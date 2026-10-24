//! What the database says, read straight from the file.
//!
//! The server need not be up for any of this: the terminal opens the same
//! SQLite file the server does (WAL lets a reader in beside a writer), so the
//! counts, the people, and the backups are there even while the server is
//! being restarted. Everything is best effort: a query that fails leaves its
//! card empty and the failure in [`Snapshot::problems`], never the whole
//! screen blank.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::Row;
use tt_core::records::Event;
use tt_repo::{DEVICE_ONLINE_WINDOW, Device, Repo, Scout};
use tt_repo_sqlite::SqliteRepo;
use tt_repo_sqlite::accounts::Account;

use crate::site::Site;

#[derive(Debug, Clone, Default)]
pub struct EventSummary {
    pub key: String,
    pub name: String,
    pub dates: String,
    pub matches: usize,
    pub teams: usize,
    pub observations: i64,
    pub pending: i64,
    pub assignments: i64,
}

#[derive(Debug, Clone)]
pub struct Recent {
    pub at: DateTime<Utc>,
    pub kind: &'static str,
    pub what: String,
}

#[derive(Debug, Clone)]
pub struct BackupFile {
    pub path: PathBuf,
    pub name: String,
    pub taken_at: Option<DateTime<Utc>>,
    pub bytes: u64,
}

/// Open sessions per account: how many, and when the newest was made.
#[derive(Debug, Clone, Copy)]
pub struct Sessions {
    pub count: i64,
    pub newest: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub storage_ready: bool,
    pub schema_version: Option<i64>,
    pub tables: Vec<(String, i64)>,
    pub accounts: Vec<Account>,
    pub sessions: HashMap<i64, Sessions>,
    pub scouts: Vec<Scout>,
    pub devices: Vec<Device>,
    pub events: Vec<EventSummary>,
    /// The event the pages would show by default, if any.
    pub default_event: Option<String>,
    pub recent: Vec<Recent>,
    pub db_bytes: Option<u64>,
    pub db_location: String,
    pub backups: Vec<BackupFile>,
    pub problems: Vec<String>,
}

impl Snapshot {
    pub fn account(&self, id: i64) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn online_scouts(&self, now: DateTime<Utc>) -> usize {
        self.scouts
            .iter()
            .filter(|s| s.is_online(now, DEVICE_ONLINE_WINDOW))
            .count()
    }

    pub fn online_devices(&self, now: DateTime<Utc>) -> usize {
        self.devices
            .iter()
            .filter(|d| d.is_online(now, DEVICE_ONLINE_WINDOW))
            .count()
    }
}

/// Read everything once. Cheap at event scale (hundreds of rows); the app
/// asks every few seconds.
pub async fn load(repo: &SqliteRepo, site: &Site) -> Snapshot {
    let now = Utc::now();
    let mut out = Snapshot {
        db_location: site
            .db_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "in memory".into()),
        db_bytes: site
            .db_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len() + wal_bytes(site.db_path.as_deref())),
        backups: list_backups(site.backup_dir.as_deref()),
        ..Default::default()
    };

    out.storage_ready = repo.health().await.is_ready();
    if !out.storage_ready {
        out.problems.push("database not reachable".into());
        return out;
    }
    fn problem(problems: &mut Vec<String>, what: &str, e: &dyn std::fmt::Display) {
        problems.push(format!("{what}: {e}"));
    }

    match repo.schema_version().await {
        Ok(v) => out.schema_version = v,
        Err(e) => problem(&mut out.problems, "schema version", &e),
    }
    match tt_repo_sqlite::admin::tables(repo.pool()).await {
        Ok(tables) => out.tables = tables.into_iter().map(|t| (t.name, t.row_count)).collect(),
        Err(e) => problem(&mut out.problems, "table counts", &e),
    }
    match repo.accounts().await {
        Ok(a) => out.accounts = a,
        Err(e) => problem(&mut out.problems, "accounts", &e),
    }
    match sessions(repo).await {
        Ok(s) => out.sessions = s,
        Err(e) => problem(&mut out.problems, "sessions", &e),
    }
    match repo.list_scouts().await {
        Ok(s) => out.scouts = s,
        Err(e) => problem(&mut out.problems, "scouts", &e),
    }
    match repo.list_devices().await {
        Ok(d) => out.devices = d,
        Err(e) => problem(&mut out.problems, "devices", &e),
    }
    match repo.list_events().await {
        Ok(events) => {
            out.default_event =
                tt_core::records::default_event(&events, now).map(|e| e.key.clone());
            for event in &events {
                out.events
                    .push(summarise(repo, event, &mut out.problems).await);
            }
        }
        Err(e) => problem(&mut out.problems, "events", &e),
    }
    match recent(repo, &out.accounts).await {
        Ok(r) => out.recent = r,
        Err(e) => problem(&mut out.problems, "recent activity", &e),
    }
    site.fix_wal_ownership();
    out
}

async fn summarise(repo: &SqliteRepo, event: &Event, problems: &mut Vec<String>) -> EventSummary {
    let mut summary = EventSummary {
        key: event.key.clone(),
        name: event.name.clone(),
        dates: match (event.start_date, event.end_date) {
            (Some(a), Some(b)) if a == b => a.to_string(),
            (Some(a), Some(b)) => format!("{a} to {b}"),
            (Some(a), None) => a.to_string(),
            _ => String::new(),
        },
        ..Default::default()
    };
    match repo.event_matches(&event.key).await {
        Ok(m) => summary.matches = m.len(),
        Err(e) => problems.push(format!("matches for {}: {e}", event.key)),
    }
    match repo.event_teams(&event.key).await {
        Ok(t) => summary.teams = t.len(),
        Err(e) => problems.push(format!("teams for {}: {e}", event.key)),
    }
    let counts = sqlx::query(
        "SELECT COUNT(*) AS total, \
                SUM(CASE WHEN review_state = 'pending' THEN 1 ELSE 0 END) AS pending \
         FROM observations WHERE event_key = ?",
    )
    .bind(&event.key)
    .fetch_one(repo.pool())
    .await;
    match counts {
        Ok(row) => {
            summary.observations = row.get::<i64, _>("total");
            summary.pending = row.get::<Option<i64>, _>("pending").unwrap_or(0);
        }
        Err(e) => problems.push(format!("observations for {}: {e}", event.key)),
    }
    match repo.event_assignments(&event.key).await {
        Ok(a) => summary.assignments = a.len() as i64,
        Err(e) => problems.push(format!("assignments for {}: {e}", event.key)),
    }
    summary
}

async fn sessions(repo: &SqliteRepo) -> Result<HashMap<i64, Sessions>, sqlx::Error> {
    let rows = sqlx::query("SELECT user_id, expires_at, created_at FROM sessions")
        .fetch_all(repo.pool())
        .await?;
    let now = Utc::now();
    let mut out: HashMap<i64, Sessions> = HashMap::new();
    for row in rows {
        let expires = parse_when(&row.get::<String, _>("expires_at"));
        if expires.is_some_and(|t| t < now) {
            continue;
        }
        let created = parse_when(&row.get::<String, _>("created_at"));
        let entry = out.entry(row.get::<i64, _>("user_id")).or_insert(Sessions {
            count: 0,
            newest: None,
        });
        entry.count += 1;
        if created > entry.newest {
            entry.newest = created;
        }
    }
    Ok(out)
}

/// The newest submissions and sign-ins, together, newest first.
async fn recent(repo: &SqliteRepo, accounts: &[Account]) -> Result<Vec<Recent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT o.match_key, o.team_number, o.review_state, o.created_at, u.name \
         FROM observations o LEFT JOIN users u ON u.id = o.scouter_id \
         ORDER BY o.id DESC LIMIT 40",
    )
    .fetch_all(repo.pool())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let Some(at) = parse_when(&row.get::<String, _>("created_at")) else {
            continue;
        };
        let who = row
            .get::<Option<String>, _>("name")
            .unwrap_or_else(|| "someone".into());
        let state = row.get::<String, _>("review_state");
        let suffix = if state == "pending" {
            String::new()
        } else {
            format!(" ({state})")
        };
        out.push(Recent {
            at,
            kind: "form",
            what: format!(
                "{who} scouted {} in {}{suffix}",
                row.get::<i64, _>("team_number"),
                row.get::<String, _>("match_key"),
            ),
        });
    }
    for account in accounts {
        if let Some(at) = account.last_login_at {
            out.push(Recent {
                at,
                kind: "sign-in",
                what: format!("{} signed in", account.name),
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.at));
    out.truncate(60);
    Ok(out)
}

/// The timed snapshots and any `tt-web backup` copies in the folder, newest
/// first. A missing or unreadable folder is an empty list.
pub fn list_backups(dir: Option<&Path>) -> Vec<BackupFile> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupFile> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().ok()?;
            if !meta.is_file() || !name.ends_with(".db") {
                return None;
            }
            let taken_at = tt_repo_sqlite::backup::taken_at(&name)
                .or_else(|| meta.modified().ok().map(DateTime::<Utc>::from));
            Some(BackupFile {
                path: entry.path(),
                name,
                taken_at,
                bytes: meta.len(),
            })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.taken_at));
    out
}

fn wal_bytes(db: Option<&Path>) -> u64 {
    let Some(db) = db else { return 0 };
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(wal).map(|m| m.len()).unwrap_or(0)
}

/// The store writes RFC 3339; be lenient about anything else that parses.
pub fn parse_when(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(t.and_utc());
        }
    }
    None
}

/// "3 min ago", "2 h ago", "never".
pub fn ago(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "never".into();
    };
    let secs = (now - at).num_seconds();
    if secs < 0 {
        return "just now".into();
    }
    match secs {
        0..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h {} min ago", secs / 3600, (secs % 3600) / 60),
        _ => format!("{} d ago", secs / 86_400),
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_timestamps() {
        assert!(parse_when("2026-10-24T12:00:00Z").is_some());
        assert!(parse_when("2026-10-24T12:00:00.123456+00:00").is_some());
        assert!(parse_when("2026-10-24 12:00:00").is_some());
        assert!(parse_when("nonsense").is_none());
    }

    #[test]
    fn ages_read_naturally() {
        let now = Utc::now();
        assert_eq!(ago(None, now), "never");
        assert_eq!(
            ago(Some(now - chrono::TimeDelta::seconds(30)), now),
            "30 s ago"
        );
        assert_eq!(
            ago(Some(now - chrono::TimeDelta::minutes(5)), now),
            "5 min ago"
        );
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
    }
}
