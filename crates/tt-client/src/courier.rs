//! Fetching upstream with the device's own signal (S7).
//!
//! A lead scout's tablet that finds signal, in the lobby or on the bus,
//! fetches the event's matches, rankings, and OPRs from TBA itself, keeps
//! them, and hands them to the Pi as a bundle (S5) once it is back in reach.
//! Nobody has to leave on purpose (REFURBISH_PLAN.md, "The transports, in
//! priority order", transport 3).
//!
//! One [`Courier::tick`] does all of it, in order:
//!
//! 1. **Ask the Pi for the key** (`GET /api/upstream/key`). The Pi gives its
//!    TBA key to a lead scout only, and the device keeps it in its own
//!    `courier` table for when the Pi cannot be asked. A 403, or a Pi with
//!    no key, takes it away. The answer also says whether the Pi has
//!    internet itself.
//! 2. **Fetch**, unless the Pi has its own uplink, or this device reached TBA
//!    less than [`FETCH_EVERY`] ago. The requests are the Pi's loop's
//!    (matches, OPRs, rankings, and component OPRs per event), made by the
//!    same [`TbaClient`], conditional on what the device already holds. No
//!    signal is the first request failing, and it stops the fetch quietly.
//!    A new response goes into the device's upstream log as `via = 'device'`.
//! 3. **Push** what the Pi has not had, as a bundle (`POST /api/sync/bundle`),
//!    when the Pi answered step 1. The Pi's answer says how far it read, and
//!    the device moves its cursor there.
//!
//! Signal is never guessed from `navigator.onLine` alone: a tablet on the
//! venue's wifi with no internet says yes. The request finds out. The
//! [`TbaClient`] does ask `navigator.onLine` first, because its "no" can be
//! trusted (S4).
//!
//! # The queue is the device's upstream log
//!
//! Rows with `via = 'device'` and a `seq` past the pushed cursor are copied
//! into a bundle in S5's format (`tt_repo_sqlite::bundle`). Rows the device
//! pulled from the Pi are never sent back. The bundle's log id is made once
//! and kept in `courier`. A new snapshot replaces the file, table and all, so
//! it starts a new log, which the Pi reads from the start.
//!
//! The `courier` table is the device's own, made on first use, like the
//! outbox: the server has none, so a snapshot never carries one.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, MAIN_DB, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tt_repo::{LocalRepo, NewUpstream, Result};
use tt_upstream::journal::{Fetched, Recorder};
use tt_upstream::tba::TbaClient;
use tt_upstream::{Uplink, UpstreamError};

use crate::ClientRepo;
use crate::sql::{Context, from_sql, to_sql};
use crate::sync::{Reply, Stop, Transport, stopped};

/// What the device's own fetches are logged as.
pub const VIA: &str = "device";

/// The least time between two fetches that reached TBA: the Pi's loop's
/// pause while an event is live.
pub const FETCH_EVERY: TimeDelta = TimeDelta::minutes(2);

/// Where the Pi gives out its key.
pub const KEY_PATH: &str = "/api/upstream/key";

/// `tt_repo_sqlite::bundle::FORMAT`, which this crate cannot depend on.
const BUNDLE_FORMAT: &str = "1";

/// With no events named, those running today or starting tomorrow: the Pi
/// loop's live events (`tt_upstream::sync::LOOKAHEAD_DAYS`).
const LOOKAHEAD_DAYS: i64 = 1;

const COURIER: &str = "CREATE TABLE IF NOT EXISTS courier (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT";

/// S5's bundle tables, made in a database of its own.
const BUNDLE: &str = "
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE upstream (
        seq        INTEGER PRIMARY KEY,
        api        TEXT NOT NULL,
        path       TEXT NOT NULL,
        etag       TEXT,
        body       TEXT NOT NULL,
        fetched_at TEXT NOT NULL
    );";

/// A bundle ready to push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The SQLite file.
    pub bytes: Vec<u8>,
    pub log: String,
    pub rows: usize,
    /// The newest row in it: the cursor once the Pi has read it.
    pub to_seq: i64,
}

/// What one tick did, for the page that asked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tick {
    /// The Pi answered.
    pub pi: bool,
    /// This device holds a key to fetch with.
    pub key: bool,
    /// Why nothing was fetched, when nothing was.
    pub skipped: Option<String>,
    /// New responses from TBA this time.
    pub fetched: usize,
    /// What TBA said other than "no signal", such as an event it does not
    /// know.
    pub problems: Vec<String>,
    /// Responses the Pi took from this device's bundle that were new to it.
    pub pushed: usize,
    /// Responses still waiting for the Pi.
    pub waiting: usize,
    /// Why the push did not land, when it did not.
    pub stopped: Option<String>,
}

/// The Pi's answer at [`KEY_PATH`].
#[derive(Deserialize)]
struct KeyReply {
    tba: Option<String>,
    /// TBA, or a test's stub.
    base: Option<String>,
    /// The Pi's own uplink is answering, so a fetch here would only repeat
    /// it.
    uplink_online: bool,
}

pub struct Courier<T> {
    transport: T,
    /// The events to fetch; none is those live today.
    events: Vec<String>,
}

impl<T: Transport> Courier<T> {
    pub fn new(transport: T, events: Vec<String>) -> Self {
        let events = events
            .into_iter()
            .map(|e| e.trim().to_ascii_lowercase())
            .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()))
            .collect();
        Self { transport, events }
    }

    /// Ask for the key, fetch, push. `fresh_log` names this device's log
    /// the first time a bundle is made: random, and new with every file.
    pub async fn tick(
        &self,
        repo: &ClientRepo,
        now: DateTime<Utc>,
        fresh_log: impl FnOnce() -> String,
    ) -> Result<Tick> {
        repo.ensure_courier()?;
        let mut tick = Tick::default();
        let pi_online = self.ask_for_key(repo, &mut tick).await?;
        repo.flush().await?;
        tick.key = repo.courier_value("tba_key")?.is_some();

        if pi_online {
            tick.skipped = Some("the Pi has its own connection".into());
        } else {
            self.fetch(repo, now, &mut tick).await?;
            repo.flush().await?;
        }

        if tick.pi && repo.waiting_upstream()? > 0 {
            self.push(repo, fresh_log, &mut tick).await?;
            repo.flush().await?;
        }
        tick.waiting = repo.waiting_upstream()?;
        Ok(tick)
    }

    /// Step 1. Whether the Pi has internet of its own.
    async fn ask_for_key(&self, repo: &ClientRepo, tick: &mut Tick) -> Result<bool> {
        let Ok(reply) = self.transport.get(KEY_PATH).await else {
            return Ok(false);
        };
        tick.pi = true;
        match reply.status {
            200 => match serde_json::from_str::<KeyReply>(&reply.body) {
                Ok(answer) => {
                    repo.keep_upstream_key(answer.tba.as_deref(), answer.base.as_deref())?;
                    Ok(answer.uplink_online)
                }
                // A sign-in page where JSON should be: the session is gone,
                // and the key kept still works.
                Err(_) => Ok(false),
            },
            // No longer a lead scout: whatever this device fetched next
            // could not be pushed.
            403 => {
                repo.keep_upstream_key(None, None)?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// Step 2.
    async fn fetch(&self, repo: &ClientRepo, now: DateTime<Utc>, tick: &mut Tick) -> Result<()> {
        let Some(key) = repo.courier_value("tba_key")? else {
            tick.skipped = Some("this device has no key to fetch with".into());
            return Ok(());
        };
        if let Some(last) = repo
            .courier_value("fetched_at")?
            .as_deref()
            .and_then(from_sql)
            && now - last < FETCH_EVERY
        {
            tick.skipped = Some("fetched less than two minutes ago".into());
            return Ok(());
        }
        let events = if self.events.is_empty() {
            repo.active_events(now.date_naive(), LOOKAHEAD_DAYS)
                .await?
                .into_iter()
                .map(|e| e.key)
                .collect()
        } else {
            self.events.clone()
        };
        if events.is_empty() {
            tick.skipped = Some("no event is on today".into());
            return Ok(());
        }

        let fetched: Arc<Mutex<Vec<Fetched>>> = Arc::default();
        let into = fetched.clone();
        let Ok(tba) = TbaClient::new(key, Uplink::new()) else {
            tick.skipped = Some("this device has no key to fetch with".into());
            return Ok(());
        };
        let mut tba = tba.with_recorder(Recorder::new(move |f| {
            if let Ok(mut fetched) = into.lock() {
                fetched.push(f);
            }
        }));
        if let Some(base) = repo.courier_value("tba_base")? {
            tba = tba.with_base_url(base);
        }
        for (path, etag, body) in repo.held_tba(&events)? {
            tba.remember(&path, &etag, body);
        }

        let mut reached = false;
        'events: for event in &events {
            // What the Pi's sync_event and sync_stats ask, in that order,
            // one at a time: with no signal, the first is the only one.
            for step in 0..4 {
                let answer = match step {
                    0 => tba.matches(event).await.map(|_| ()),
                    1 => tba.oprs(event).await.map(|_| ()),
                    2 => tba.rankings(event).await.map(|_| ()),
                    _ => tba.component_oprs(event).await.map(|_| ()),
                };
                match answer {
                    Ok(()) => reached = true,
                    Err(UpstreamError::Offline | UpstreamError::Transport { .. }) => {
                        if !reached {
                            tick.skipped = Some("no signal".into());
                        }
                        break 'events;
                    }
                    Err(e) => {
                        reached = true;
                        tick.problems.push(e.to_string());
                    }
                }
            }
        }
        if reached {
            repo.set_courier_value("fetched_at", Some(&to_sql(now)))?;
        }

        let fetched = std::mem::take(&mut *fetched.lock().unwrap_or_else(|e| e.into_inner()));
        for f in fetched {
            let appended = repo.append_upstream_impl(&NewUpstream {
                api: f.api.to_string(),
                path: f.path,
                etag: f.etag,
                body: f.body.to_string(),
                fetched_at: f.fetched_at,
                via: VIA.into(),
            })?;
            tick.fetched += usize::from(appended.is_some());
        }
        Ok(())
    }

    /// Step 3.
    async fn push(
        &self,
        repo: &ClientRepo,
        fresh_log: impl FnOnce() -> String,
        tick: &mut Tick,
    ) -> Result<()> {
        let Some(bundle) = repo.bundle(fresh_log)? else {
            return Ok(());
        };
        let schema = match repo.schema()? {
            Some(n) => n.to_string(),
            None => String::new(),
        };
        let path = format!("/api/sync/bundle?schema={schema}");
        let reply = match self.transport.post_file(&path, bundle.bytes).await {
            Ok(reply) => reply,
            Err(why) => {
                tick.stopped = Some(describe(&Stop::Offline(why)));
                return Ok(());
            }
        };
        if reply.status != 200 {
            tick.stopped = Some(describe(&refused(&reply)));
            return Ok(());
        }
        let answer: JsonValue = serde_json::from_str(&reply.body).unwrap_or_default();
        let cursor = answer.get("cursor").and_then(JsonValue::as_i64);
        match (answer.get("log").and_then(JsonValue::as_str), cursor) {
            (Some(log), Some(cursor)) if log == bundle.log => {
                repo.bundle_pushed(cursor)?;
                tick.pushed = answer
                    .get("appended")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0) as usize;
            }
            _ => tick.stopped = Some("the Pi's answer was unreadable".into()),
        }
        Ok(())
    }
}

/// A bundle push's refusal. A 403 here is the role, not the session.
fn refused(reply: &Reply) -> Stop {
    if reply.status == 403 {
        let why = serde_json::from_str::<JsonValue>(&reply.body)
            .ok()
            .and_then(|v| v.get("error").and_then(JsonValue::as_str).map(String::from));
        return Stop::Refused(why.unwrap_or_else(|| "only a lead scout may push".into()));
    }
    stopped(reply)
}

fn describe(stop: &Stop) -> String {
    match stop {
        Stop::Offline(why) => format!("the Pi could not be reached: {why}"),
        Stop::Reload => "this device's build is older than the Pi's; reload".into(),
        Stop::ServerBehind => "the Pi runs an older build than this device".into(),
        Stop::SignedOut => "signed out; sign in again to hand the Pi what was fetched".into(),
        Stop::Refused(why) => format!("the Pi refused it: {why}"),
    }
}

impl ClientRepo {
    pub(crate) fn ensure_courier(&self) -> Result<()> {
        self.conn
            .execute_batch(COURIER)
            .ctx("making the device's courier table")
    }

    fn courier_value(&self, key: &str) -> Result<Option<String>> {
        self.ensure_courier()?;
        self.conn
            .query_row("SELECT value FROM courier WHERE key = ?", [key], |row| {
                row.get(0)
            })
            .optional()
            .ctx("reading the courier table")
    }

    fn set_courier_value(&self, key: &str, value: Option<&str>) -> Result<()> {
        self.ensure_courier()?;
        match value {
            Some(value) => self.conn.execute(
                "INSERT INTO courier (key, value) VALUES (?1, ?2) \
                 ON CONFLICT (key) DO UPDATE SET value = ?2 WHERE value <> ?2",
                params![key, value],
            ),
            None => self
                .conn
                .execute("DELETE FROM courier WHERE key = ?", [key]),
        }
        .map(|_| ())
        .ctx("writing the courier table")
    }

    /// Keep the Pi's TBA key and where to send it, or forget them.
    pub fn keep_upstream_key(&self, key: Option<&str>, base: Option<&str>) -> Result<()> {
        let key = key.map(str::trim).filter(|k| !k.is_empty());
        self.set_courier_value("tba_key", key)?;
        self.set_courier_value("tba_base", key.and(base))
    }

    /// The newest tagged TBA response per path for `events`, to make this
    /// device's next requests conditional (S4's `remember`).
    fn held_tba(&self, events: &[String]) -> Result<Vec<(String, String, String)>> {
        let mut held = Vec::new();
        for event in events {
            held.extend(self.all(
                "SELECT path, etag, body FROM upstream AS u \
                 WHERE api = 'tba' AND etag IS NOT NULL AND path LIKE ?1 \
                 AND seq = (SELECT MAX(seq) FROM upstream WHERE api = 'tba' AND path = u.path)",
                [format!("/event/{event}/%")],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                "reading the upstream log",
            )?);
        }
        Ok(held)
    }

    fn pushed_cursor(&self) -> Result<i64> {
        Ok(self
            .courier_value("pushed")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    /// Responses this device fetched that the Pi has not read.
    pub fn waiting_upstream(&self) -> Result<usize> {
        let after = self.pushed_cursor()?;
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM upstream WHERE via = ? AND seq > ?",
                params![VIA, after],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .ctx("counting what waits for the Pi")
    }

    /// What the Pi has not read of this device's fetches, as S5's bundle,
    /// or `None` when there is nothing.
    pub fn bundle(&self, fresh_log: impl FnOnce() -> String) -> Result<Option<Bundle>> {
        let after = self.pushed_cursor()?;
        let rows = self.all(
            "SELECT seq, api, path, etag, body, fetched_at FROM upstream \
             WHERE via = ? AND seq > ? ORDER BY seq",
            params![VIA, after],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
            "reading what waits for the Pi",
        )?;
        let Some(to_seq) = rows.last().map(|r| r.0) else {
            return Ok(None);
        };
        let log = match self.courier_value("log")? {
            Some(log) => log,
            None => {
                let log = fresh_log();
                self.set_courier_value("log", Some(&log))?;
                log
            }
        };

        let out = Connection::open_in_memory().ctx("making a bundle")?;
        out.execute_batch(BUNDLE).ctx("making a bundle")?;
        out.execute(
            "INSERT INTO meta (key, value) VALUES ('format', ?1), ('log', ?2)",
            params![BUNDLE_FORMAT, log],
        )
        .ctx("making a bundle")?;
        for (seq, api, path, etag, body, fetched_at) in &rows {
            out.execute(
                "INSERT INTO upstream (seq, api, path, etag, body, fetched_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![seq, api, path, etag, body, fetched_at],
            )
            .ctx("making a bundle")?;
        }
        let bytes = out.serialize(MAIN_DB).ctx("making a bundle")?.to_vec();
        Ok(Some(Bundle {
            bytes,
            log,
            rows: rows.len(),
            to_seq,
        }))
    }

    /// The Pi has read this device's log up to `cursor`.
    pub fn bundle_pushed(&self, cursor: i64) -> Result<()> {
        if cursor > self.pushed_cursor()? {
            self.set_courier_value("pushed", Some(&cursor.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_bundle_says_why_in_words() {
        let reply = Reply {
            status: 403,
            body: r#"{"error":"only a lead scout may push upstream data"}"#.into(),
        };
        assert_eq!(
            describe(&refused(&reply)),
            "the Pi refused it: only a lead scout may push upstream data"
        );
        let reply = Reply {
            status: 0,
            body: String::new(),
        };
        assert_eq!(refused(&reply), Stop::SignedOut);
    }
}
