//! Backups (Q4).
//!
//! The Pi holds the only authoritative copy of an event's scouting, so the
//! server snapshots it every ten minutes onto the SSD and keeps a day of them,
//! and one command copies a fresh snapshot to a USB stick between match blocks.
//!
//! **A snapshot is `VACUUM INTO`, never a file copy.** The database is in WAL
//! mode: recent commits live in `tealteam.db-wal` until a checkpoint, so
//! copying `tealteam.db` alone can silently lose them, and copying it while it
//! is written can give a torn file. `VACUUM INTO` writes a consistent,
//! self-contained database from one read transaction.
//!
//! Each snapshot opens its **own** connection, which only reads. The server's
//! single writer carries on while it runs, and a second process -- the
//! `tt-web backup` command -- can take one while the server is up.
//!
//! A backup is only a backup once it has been restored: [`check`] restores one
//! into a fresh database, brings it up to this build's schema, and counts
//! what is in it.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Connection, SqliteConnection};
use tt_repo::{RepoError, Result};

/// How often the server takes one.
pub const EVERY: TimeDelta = TimeDelta::minutes(10);
/// How long a timed snapshot is kept.
pub const KEEP: TimeDelta = TimeDelta::hours(24);
/// Never pruned below this many, whatever the clock says: a Pi without its
/// RTC (P1) can boot hours or years out, and a wrong clock must not delete
/// the last good copies.
pub const KEEP_AT_LEAST: usize = 6;

const PREFIX: &str = "tealteam-";
const SUFFIX: &str = ".db";
const STAMP: &str = "%Y%m%dT%H%M%SZ";

/// The tables a person would check after a restore, in the order they matter.
pub const COUNTED: [&str; 7] = [
    "observations",
    "pick_list_entries",
    "scout_assignments",
    "users",
    "events",
    "matches",
    "team_event_stats",
];

/// `tealteam-20260930T125300Z.db`, for a snapshot taken at `at`.
pub fn file_name(at: DateTime<Utc>) -> String {
    format!("{PREFIX}{}{SUFFIX}", at.format(STAMP))
}

/// When a snapshot was taken, from its name. Anything else is not ours.
pub fn taken_at(name: &str) -> Option<DateTime<Utc>> {
    let stamp = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    Some(NaiveDateTime::parse_from_str(stamp, STAMP).ok()?.and_utc())
}

/// Snapshot the database at `url` into `dir`, as `tealteam-<time>.db`.
///
/// Written under a `.partial` name and renamed when complete, so a snapshot
/// cut off by a power loss is never mistaken for a good one, then synced so a
/// USB stick can be pulled once this returns.
pub async fn snapshot(url: &str, dir: &Path, at: DateTime<Utc>) -> Result<PathBuf> {
    if !dir.is_dir() {
        return Err(RepoError::Unavailable(format!(
            "backup folder {} does not exist",
            dir.display()
        )));
    }
    let done = dir.join(file_name(at));
    let partial = dir.join(format!("{}.partial", file_name(at)));
    // VACUUM INTO refuses to overwrite; a leftover partial is garbage anyway.
    let _ = std::fs::remove_file(&partial);

    let mut conn = open(url).await?;
    let result = sqlx::query("VACUUM INTO ?")
        .bind(partial.to_str().ok_or_else(|| {
            RepoError::Unavailable(format!("{} is not a UTF-8 path", partial.display()))
        })?)
        .execute(&mut conn)
        .await;
    let _ = conn.close().await;
    if let Err(e) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(RepoError::Query(format!(
            "backing up to {}: {e}",
            dir.display()
        )));
    }

    let io = |e: std::io::Error| RepoError::Unavailable(format!("{}: {e}", done.display()));
    std::fs::File::open(&partial)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    std::fs::rename(&partial, &done).map_err(io)?;
    // The rename is only durable once the folder is.
    if let Ok(folder) = std::fs::File::open(dir) {
        let _ = folder.sync_all();
    }
    Ok(done)
}

/// Which of `names` to delete: snapshots older than `keep`, except the newest
/// [`KEEP_AT_LEAST`]. Files that are not snapshots are never touched.
pub fn to_prune(names: &[String], now: DateTime<Utc>, keep: TimeDelta) -> Vec<String> {
    let mut snapshots: Vec<(DateTime<Utc>, &String)> = names
        .iter()
        .filter_map(|n| Some((taken_at(n)?, n)))
        .collect();
    snapshots.sort();
    let spared = snapshots.len().saturating_sub(KEEP_AT_LEAST);
    snapshots[..spared]
        .iter()
        .filter(|(at, _)| now - *at > keep)
        .map(|(_, n)| (*n).clone())
        .collect()
}

/// Delete old snapshots from `dir`; returns how many went.
pub fn prune(dir: &Path, now: DateTime<Utc>) -> std::io::Result<usize> {
    let names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    let old = to_prune(&names, now, KEEP);
    for name in &old {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(old.len())
}

/// What a restored backup holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// Rows per table in [`COUNTED`], in that order.
    pub counts: Vec<(&'static str, i64)>,
    /// Migrations this build applied on top: nonzero for a backup taken by an
    /// older build, which is fine and worth knowing.
    pub migrated: usize,
}

/// The restore test: copy `backup` into a fresh database in a temporary
/// folder, open it as the server would, apply this build's migrations, check
/// its integrity, and count what is in it. The backup file itself is only
/// read. The temporary copy is removed afterwards.
pub async fn check(backup: &Path) -> Result<Restored> {
    let scratch = std::env::temp_dir().join(format!(
        "tealteam-restore-check-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&scratch)
        .map_err(|e| RepoError::Unavailable(format!("{}: {e}", scratch.display())))?;
    let result = restore_into(backup, &scratch.join("tealteam.db")).await;
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

async fn restore_into(backup: &Path, fresh: &Path) -> Result<Restored> {
    // A snapshot is a whole database with no -wal of its own, which is what
    // makes a plain copy the right way to put one back.
    std::fs::copy(backup, fresh)
        .map_err(|e| RepoError::Unavailable(format!("reading {}: {e}", backup.display())))?;
    let url = format!("sqlite://{}", fresh.display());
    let repo = crate::SqliteRepo::connect(&url)?;

    let ok: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(repo.pool())
        .await
        .map_err(|e| RepoError::Query(format!("{} is not a database: {e}", backup.display())))?;
    if ok != "ok" {
        return Err(RepoError::Query(format!(
            "{} is damaged: {ok}",
            backup.display()
        )));
    }

    let before = applied(&repo).await;
    crate::migrate::apply(repo.pool()).await?;
    let migrated = applied(&repo).await.saturating_sub(before);

    let mut counts = Vec::new();
    for table in COUNTED {
        // Table names from COUNTED, a constant: nothing from outside.
        let sql = sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"));
        let n: i64 = sqlx::query_scalar(sql)
            .fetch_one(repo.pool())
            .await
            .map_err(|e| RepoError::Query(format!("counting {table}: {e}")))?;
        counts.push((table, n));
    }
    repo.pool().close().await;
    Ok(Restored { counts, migrated })
}

async fn applied(repo: &crate::SqliteRepo) -> usize {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(repo.pool())
        .await
        .map(|n| n as usize)
        .unwrap_or(0)
}

/// A connection of its own, for reading. Waits out the writer rather than
/// failing if it happens to be mid-checkpoint.
async fn open(url: &str) -> Result<SqliteConnection> {
    SqliteConnectOptions::from_str(url)
        .map_err(|e| RepoError::Unavailable(format!("bad database url {url:?}: {e}")))?
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(5))
        .connect()
        .await
        .map_err(|e| RepoError::Unavailable(format!("opening {url} to back it up: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, h, m, 0).unwrap()
    }

    #[test]
    fn names_carry_the_time_and_read_back() {
        let name = file_name(at(9, 40));
        assert_eq!(name, "tealteam-20260314T094000Z.db");
        assert_eq!(taken_at(&name), Some(at(9, 40)));
        assert_eq!(taken_at("tealteam.db"), None);
        assert_eq!(taken_at("tealteam-20260314T094000Z.db.partial"), None);
        assert_eq!(taken_at("notes.txt"), None);
    }

    #[test]
    fn a_day_is_kept_and_nothing_else_is_touched() {
        // One every 10 minutes from 00:00 to 23:50, then "now" is 12:05 the
        // next day: everything before 12:05 yesterday goes.
        let mut names: Vec<String> = (0..144)
            .map(|i| file_name(at(0, 0) + TimeDelta::minutes(10 * i)))
            .collect();
        names.push("tealteam.db".into());
        names.push("tealteam-20260314T000000Z.db.partial".into());
        let now = at(12, 5) + TimeDelta::days(1);

        let gone = to_prune(&names, now, KEEP);
        assert_eq!(gone.len(), 73, "00:00 to 12:00");
        assert!(gone.iter().all(|n| taken_at(n).unwrap() < at(12, 5)));
        assert!(!gone.contains(&"tealteam.db".to_string()));
    }

    #[test]
    fn a_wrong_clock_never_prunes_the_last_copies() {
        // The Pi booted with no RTC and thinks it is 2030: every snapshot is
        // "years old", and the newest six still stay.
        let names: Vec<String> = (0..10)
            .map(|i| file_name(at(9, 0) + TimeDelta::minutes(10 * i)))
            .collect();
        let now = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
        let gone = to_prune(&names, now, KEEP);
        assert_eq!(gone, names[..4].to_vec());
    }
}
