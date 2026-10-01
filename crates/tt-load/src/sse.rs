//! One scout's live stream (S8), held for the whole run.
//!
//! A scouting page opens `/api/sync/stream` and a new page opens another, so
//! the stream is closed and reopened on every save, resuming from
//! `Last-Event-ID`. A dropped stream is reopened after [`RETRY`], as
//! `EventSource` does. Unlike `EventSource`, a stream silent for longer than
//! [`WATCHDOG`] counts as dropped: the server sends a comment every 15
//! seconds, so that long without one is a dead connection.
//!
//! What it keeps: every observation it was sent and when, so the run can say
//! whether each saved observation reached each scout, and how long it took.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::{Notify, watch};
use tokio::time::{Instant, timeout};

use crate::report::Shared;
use crate::seed::EVENT;

/// `EventSource`'s reconnection delay in Chromium and Firefox.
pub const RETRY: Duration = Duration::from_secs(3);
/// Two missed heartbeats, and some.
pub const WATCHDOG: Duration = Duration::from_secs(35);

#[derive(Debug, Default)]
pub struct StreamStats {
    /// Streams opened, navigations included.
    pub opened: u32,
    /// Streams lost, and when and why: `silent`, `closed`, `error`, `connect`,
    /// or a refusal's status.
    pub drops: Vec<(Duration, String)>,
    /// Observation id → when this stream first sent it.
    pub seen: HashMap<String, Duration>,
    /// Observations sent twice: a resume that went back too far.
    pub repeats: u32,
}

/// One server-sent event.
#[derive(Debug, Default, PartialEq)]
pub struct Message {
    pub event: String,
    pub data: String,
    pub id: Option<String>,
}

/// Turns the bytes of an event stream into [`Message`]s. Comments, the
/// server's heartbeats, are dropped.
#[derive(Default)]
pub struct Parser {
    line: Vec<u8>,
    current: Message,
    has_data: bool,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Message> {
        let mut out = Vec::new();
        for &b in bytes {
            if b != b'\n' {
                self.line.push(b);
                continue;
            }
            let line = String::from_utf8_lossy(&self.line)
                .trim_end_matches('\r')
                .to_string();
            self.line.clear();
            if line.is_empty() {
                let done = std::mem::take(&mut self.current);
                if std::mem::take(&mut self.has_data) || done.id.is_some() {
                    out.push(done);
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((&line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => self.current.event = value.into(),
                "id" => self.current.id = Some(value.into()),
                "data" => {
                    if self.has_data {
                        self.current.data.push('\n');
                    }
                    self.current.data.push_str(value);
                    self.has_data = true;
                }
                _ => {}
            }
        }
        out
    }
}

/// Follow the stream until `stop`. `navigated` is the scout's page changing.
pub async fn follow(
    shared: &Shared,
    base: &str,
    cookie: &str,
    mut last_id: String,
    navigated: &Notify,
    mut stop: watch::Receiver<bool>,
) -> StreamStats {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .expect("http client");
    let url = format!("{base}/api/sync/stream?event={EVENT}");
    let mut stats = StreamStats::default();

    'open: while !*stop.borrow() {
        let request = client
            .get(&url)
            .header("cookie", cookie)
            .header("last-event-id", &last_id)
            .send();
        let started = Instant::now();
        let response = tokio::select! {
            r = timeout(WATCHDOG, request) => r,
            _ = stop.changed() => break,
        };
        let mut response = match response {
            Ok(Ok(r)) if r.status().is_success() => {
                shared.sample("stream open", started, true);
                r
            }
            Ok(Ok(r)) => {
                shared.sample("stream open", started, false);
                stats
                    .drops
                    .push((shared.elapsed(), format!("refused {}", r.status().as_u16())));
                // A 503 says when to come back.
                pause(Duration::from_secs(30), &mut stop).await;
                continue;
            }
            Ok(Err(_)) | Err(_) => {
                shared.sample("stream open", started, false);
                stats.drops.push((shared.elapsed(), "connect".into()));
                pause(RETRY, &mut stop).await;
                continue;
            }
        };
        stats.opened += 1;

        let mut parser = Parser::default();
        let why = loop {
            let chunk = tokio::select! {
                c = timeout(WATCHDOG, response.chunk()) => c,
                _ = navigated.notified() => continue 'open,
                _ = stop.changed() => break 'open,
            };
            let bytes = match chunk {
                Err(_) => break "silent",
                Ok(Ok(Some(bytes))) => bytes,
                Ok(Ok(None)) => break "closed",
                Ok(Err(_)) => break "error",
            };
            for message in parser.feed(&bytes) {
                if let Some(id) = message.id {
                    last_id = id;
                }
                if message.event == "change" {
                    saw(shared, &mut stats, &message.data);
                }
            }
        };
        stats.drops.push((shared.elapsed(), why.into()));
        pause(RETRY, &mut stop).await;
    }
    stats
}

fn saw(shared: &Shared, stats: &mut StreamStats, data: &str) {
    let Ok(change) = serde_json::from_str::<serde_json::Value>(data) else {
        return;
    };
    if change["entity"] != "observation" || change["op"] != "upsert" {
        return;
    }
    let Some(id) = change["entity_pk"].as_str() else {
        return;
    };
    if stats.seen.contains_key(id) {
        stats.repeats += 1;
    } else {
        stats.seen.insert(id.to_string(), shared.elapsed());
    }
}

async fn pause(how_long: Duration, stop: &mut watch::Receiver<bool>) {
    tokio::select! {
        _ = tokio::time::sleep(how_long) => {}
        _ = stop.changed() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_split_across_chunks_come_out_whole() {
        let mut p = Parser::default();
        assert!(p.feed(b"event: change\nid: 4-0\nda").is_empty());
        let out = p.feed(b"ta: {\"a\":1}\n\n:\n\nevent: cursor\r\nid: 5-0\r\ndata: {}\r\n\r\n");
        assert_eq!(
            out,
            vec![
                Message {
                    event: "change".into(),
                    data: "{\"a\":1}".into(),
                    id: Some("4-0".into())
                },
                Message {
                    event: "cursor".into(),
                    data: "{}".into(),
                    id: Some("5-0".into())
                },
            ]
        );
    }

    #[test]
    fn a_heartbeat_is_not_a_message() {
        let mut p = Parser::default();
        assert!(p.feed(b":\n\n:\n\n").is_empty());
    }

    #[test]
    fn an_observation_seen_twice_is_a_repeat() {
        let shared = Shared::new();
        let mut stats = StreamStats::default();
        let change = r#"{"entity":"observation","op":"upsert","entity_pk":"abc"}"#;
        saw(&shared, &mut stats, change);
        saw(&shared, &mut stats, change);
        saw(
            &shared,
            &mut stats,
            r#"{"entity":"assignment","op":"upsert","entity_pk":"x"}"#,
        );
        assert_eq!(stats.seen.len(), 1);
        assert_eq!(stats.repeats, 1);
    }
}
