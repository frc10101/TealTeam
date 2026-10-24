//! The server's log, as it is written: the journal on the Pi, `server.log`
//! on the laptop. Tailed by a child process into a ring buffer, each line
//! given a level and a subject so the screen can show only sign-ins, or only
//! what went wrong.

use std::collections::VecDeque;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::Msg;
use crate::site::{Site, Supervisor};

/// How many lines the screen keeps. The Pi's journal keeps the rest.
pub const KEEP: usize = 3000;
/// How far back the tail starts.
const TAIL: &str = "600";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    /// Not a tracing line: a panic, a startup message, a shell's output.
    Other,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Other => "",
        }
    }
}

/// What a line is about, from its target and words. Rough on purpose: a
/// filter for a person scanning, not an audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    All,
    SignIns,
    Scouting,
    Sync,
    Backups,
    Http,
    Server,
}

impl Subject {
    pub const CYCLE: [Subject; 7] = [
        Subject::All,
        Subject::SignIns,
        Subject::Scouting,
        Subject::Sync,
        Subject::Backups,
        Subject::Http,
        Subject::Server,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Subject::All => "everything",
            Subject::SignIns => "sign-ins & accounts",
            Subject::Scouting => "scouting",
            Subject::Sync => "sync & upstream",
            Subject::Backups => "backups",
            Subject::Http => "requests",
            Subject::Server => "server",
        }
    }

    pub fn next(self) -> Subject {
        let i = Self::CYCLE.iter().position(|s| *s == self).unwrap_or(0);
        Self::CYCLE[(i + 1) % Self::CYCLE.len()]
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    pub time: String,
    pub level: Level,
    pub target: String,
    pub text: String,
    pub subject: Subject,
}

impl Line {
    pub fn parse(raw: &str) -> Line {
        let clean = strip_ansi(raw);
        let clean = clean.trim_end();
        // tracing's default: `2026-10-24T17:03:12.123456Z  INFO target: message`
        let mut words = clean.splitn(2, char::is_whitespace);
        let first = words.next().unwrap_or("");
        let looks_like_time =
            first.len() >= 19 && first.as_bytes().get(4) == Some(&b'-') && first.contains('T');
        if looks_like_time {
            let rest = words.next().unwrap_or("").trim_start();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let level = match parts.next().unwrap_or("") {
                "TRACE" => Some(Level::Trace),
                "DEBUG" => Some(Level::Debug),
                "INFO" => Some(Level::Info),
                "WARN" => Some(Level::Warn),
                "ERROR" => Some(Level::Error),
                _ => None,
            };
            if let Some(level) = level {
                let rest = parts.next().unwrap_or("").trim_start();
                let (target, text) = match rest.split_once(": ") {
                    Some((t, m)) if !t.contains(' ') => (t.to_string(), m.to_string()),
                    _ => (String::new(), rest.to_string()),
                };
                let time = first
                    .get(11..19)
                    .map(str::to_string)
                    .unwrap_or_else(|| first.to_string());
                let subject = classify(&target, &text);
                return Line {
                    time,
                    level,
                    target,
                    text,
                    subject,
                };
            }
        }
        Line {
            time: String::new(),
            level: Level::Other,
            target: String::new(),
            text: clean.to_string(),
            subject: classify("", clean),
        }
    }

    pub fn shows(&self, min: Level, subject: Subject) -> bool {
        let level_ok = self.level == Level::Other || self.level >= min;
        let subject_ok = subject == Subject::All || self.subject == subject;
        level_ok && subject_ok
    }
}

fn classify(target: &str, text: &str) -> Subject {
    let lower = text.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if target.contains("tower_http") || target.contains("hyper") {
        Subject::Http
    } else if has(&[
        "login", "sign", "session", "password", "account", "signup", "token", "role",
    ]) {
        Subject::SignIns
    } else if has(&[
        "observation",
        "submission",
        "handoff",
        "handed",
        "scout",
        "assign",
        "refused",
        "form",
    ]) {
        Subject::Scouting
    } else if has(&[
        "sync", "upstream", "ftc", "tba", "first", "bundle", "tether", "uplink", "schedule", "pull",
    ]) {
        Subject::Sync
    } else if has(&["backup", "snapshot", "restore", "prune"]) {
        Subject::Backups
    } else {
        Subject::Server
    }
}

/// Drop `ESC [ ... <letter>` sequences; tracing writes colour even to a file.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(Debug, Default)]
pub struct Ring {
    pub lines: VecDeque<Line>,
    pub source: String,
    pub trouble: Option<String>,
}

impl Ring {
    pub fn push(&mut self, line: Line) {
        if self.lines.len() >= KEEP {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// The newest warnings and errors, newest last.
    pub fn alarms(&self, n: usize) -> Vec<&Line> {
        let mut out: Vec<&Line> = self
            .lines
            .iter()
            .rev()
            .filter(|l| l.level >= Level::Warn && l.level != Level::Other)
            .take(n)
            .collect();
        out.reverse();
        out
    }
}

/// The command that follows the log here, and a name for it.
fn tail_command(site: &Site) -> Option<(String, Vec<String>, String)> {
    match &site.supervisor {
        Supervisor::Systemd { unit } => {
            let args = vec![
                "-u".to_string(),
                unit.clone(),
                "-n".to_string(),
                TAIL.to_string(),
                "-f".to_string(),
                "-o".to_string(),
                "cat".to_string(),
                "--no-pager".to_string(),
            ];
            // Reading the journal needs the systemd-journal group or root;
            // try as ourselves first, then sudo -n, and say which worked.
            let plain_ok = std::process::Command::new("journalctl")
                .args(["-u", unit, "-n", "1", "-o", "cat", "--no-pager", "-q"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if plain_ok {
                Some(("journalctl".into(), args, format!("journalctl -u {unit}")))
            } else {
                let mut full = vec!["-n".to_string(), "journalctl".to_string()];
                full.extend(args);
                Some(("sudo".into(), full, format!("sudo journalctl -u {unit}")))
            }
        }
        Supervisor::Pidfile { log, .. } => Some((
            "tail".into(),
            vec![
                "-n".into(),
                TAIL.into(),
                "-F".into(),
                log.display().to_string(),
            ],
            format!("tail -F {}", log.display()),
        )),
        Supervisor::None => {
            let log = site.dir.join("server.log");
            log.is_file().then(|| {
                (
                    "tail".to_string(),
                    vec![
                        "-n".into(),
                        TAIL.into(),
                        "-F".into(),
                        log.display().to_string(),
                    ],
                    format!("tail -F {}", log.display()),
                )
            })
        }
    }
}

/// Follow the log until the tool exits, one [`Msg::Log`] per line.
pub async fn follow(site: Site, tx: UnboundedSender<Msg>) {
    let Some((program, args, name)) = tail_command(&site) else {
        let _ = tx.send(Msg::LogSource {
            name: "no log".into(),
            trouble: Some("no unit, no pidfile and no server.log here".into()),
        });
        return;
    };
    let child = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Msg::LogSource {
                name,
                trouble: Some(format!("could not run {program}: {e}")),
            });
            return;
        }
    };
    let _ = tx.send(Msg::LogSource {
        name: name.clone(),
        trouble: None,
    });
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let errors = tokio::spawn(async move {
        let mut collected = Vec::new();
        if let Some(stderr) = stderr {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push(line);
            }
        }
        collected
    });
    if let Some(stdout) = stdout {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(Msg::Log(Line::parse(&line))).is_err() {
                return;
            }
        }
    }
    let status = child.wait().await;
    let errors = errors.await.unwrap_or_default();
    let mut trouble = format!(
        "{name} ended ({})",
        status
            .map(|s| s.to_string())
            .unwrap_or_else(|e| e.to_string())
    );
    if let Some(last) = errors.last() {
        trouble.push_str(": ");
        trouble.push_str(last);
    }
    let _ = tx.send(Msg::LogSource {
        name,
        trouble: Some(trouble),
    });
}

/// Whether a log file exists to tail, for the screen's hint.
pub fn describe_source(site: &Site) -> String {
    match tail_command(site) {
        Some((_, _, name)) => name,
        None => "none".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_tracing_line() {
        let raw = "\u{1b}[2m2026-10-24T17:03:12.123456Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \u{1b}[2mtt_web::handlers\u{1b}[0m\u{1b}[2m:\u{1b}[0m manual sync requested \u{1b}[3muser\u{1b}[0m\u{1b}[2m=\u{1b}[0mlead@x";
        let line = Line::parse(raw);
        assert_eq!(line.level, Level::Info);
        assert_eq!(line.time, "17:03:12");
        assert_eq!(line.target, "tt_web::handlers");
        assert!(line.text.starts_with("manual sync requested"));
        assert_eq!(line.subject, Subject::Sync);
    }

    #[test]
    fn other_lines_keep_their_text() {
        let line = Line::parse("thread 'main' panicked at src/main.rs");
        assert_eq!(line.level, Level::Other);
        assert_eq!(line.text, "thread 'main' panicked at src/main.rs");
        assert!(line.shows(Level::Error, Subject::All));
    }

    #[test]
    fn filters_by_level_and_subject() {
        let warn = Line::parse("2026-10-24T17:03:12.1Z  WARN tt_web::backups: backups are off");
        assert_eq!(warn.subject, Subject::Backups);
        assert!(warn.shows(Level::Warn, Subject::Backups));
        assert!(!warn.shows(Level::Error, Subject::All));
        assert!(!warn.shows(Level::Info, Subject::SignIns));
    }
}
