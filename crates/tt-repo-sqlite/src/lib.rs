//! Server-side [`Repo`] implementation over SQLite.
//!
//! This is the only crate in the workspace that depends on sqlx. If you find
//! yourself wanting `sqlx` anywhere else, the query belongs here behind a trait
//! method instead.
//!
//! # Why SQLite, and why one writer
//!
//! The server is a Raspberry Pi at a competition venue with no cloud tier
//! (REBUILD_SPEC.md 10). SQLite in WAL mode gives concurrent readers with a
//! single writer, which matches the actual load: ~50 devices reading constantly
//! and submitting a form every couple of minutes. The retired implementation ran
//! Postgres in a container next to the app for the same workload, which bought
//! nothing and cost a second process to keep alive on battery power.
//!
//! The pool is capped at one connection deliberately -- see [`connect`].

mod competition;
pub mod migrate;
mod users;

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;
use std::time::Duration;
use tracing::warn;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::user::{Session, User};
use tt_repo::{Credentials, Device, Health, NewUser, Repo, RepoError, Result};

/// Time to wait for a connection before giving up.
///
/// Short on purpose: on a single-writer database a long queue means something is
/// already wrong, and a scout staring at a spinner is worse than an error they
/// can retry.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct SqliteRepo {
    pool: SqlitePool,
}

impl SqliteRepo {
    /// Wrap an existing pool. Useful in tests and for sharing one pool with a
    /// migration runner.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Open (creating if absent) the database at `url` without touching it.
    ///
    /// Connection is **lazy**: this returns successfully even if the file is
    /// unreadable or the disk is missing. That is required behaviour, not
    /// laziness on our part -- the server must boot and serve degraded pages when
    /// storage is unavailable (REBUILD_SPEC.md 8, F5). Call [`SqliteRepo::health`]
    /// to find out whether it actually works.
    pub fn connect(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .map_err(|e| RepoError::Unavailable(format!("bad database url {url:?}: {e}")))?
            .create_if_missing(true)
            // WAL: concurrent readers alongside the single writer.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // NORMAL is the documented safe pairing with WAL. FULL costs an fsync
            // per commit, which on a Pi's SD or USB storage is measurable.
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            // Enforce the FK constraints the schema declares. SQLite ignores them
            // unless asked -- an easy and expensive thing to forget.
            .foreign_keys(true)
            // Wait rather than immediately returning SQLITE_BUSY under write
            // contention.
            .busy_timeout(ACQUIRE_TIMEOUT);

        let pool = SqlitePoolOptions::new()
            // One connection. SQLite serialises writes anyway, and a larger pool
            // just converts lock contention into confusing timeouts.
            .max_connections(1)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_lazy_with(options);

        Ok(Self::new(pool))
    }

    /// The underlying pool, for the migration runner (D11).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

impl Repo for SqliteRepo {
    async fn health(&self) -> Health {
        match sqlx::query_scalar::<_, i64>("SELECT 1")
            .fetch_one(&self.pool)
            .await
        {
            Ok(_) => Health::Ready,
            Err(e) => {
                warn!("database health probe failed: {e}");
                Health::Down
            }
        }
    }

    async fn schema_version(&self) -> Result<Option<i64>> {
        // `user_version` is a SQLite header field, so this works on an empty
        // database with no tables -- unlike a migrations table, which has to
        // exist before it can be read.
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| RepoError::Query(format!("reading user_version: {e}")))?;

        Ok((version > 0).then_some(version))
    }

    // Each method delegates to an inherent `*_impl` in the submodule that owns
    // it. Keeping the trait impl a thin index means this block stays readable as
    // the surface grows, and the SQL sits next to related SQL.

    async fn create_user(&self, new_user: NewUser, now: DateTime<Utc>) -> Result<User> {
        self.create_user_impl(new_user, now).await
    }

    async fn credentials_by_email(&self, email: &str) -> Result<Option<Credentials>> {
        self.credentials_by_email_impl(email).await
    }

    async fn user_by_id(&self, id: i64) -> Result<Option<User>> {
        self.user_by_id_impl(id).await
    }

    async fn password_hash(&self, user_id: i64) -> Result<Option<String>> {
        self.password_hash_impl(user_id).await
    }

    async fn set_password_hash(&self, user_id: i64, hash: &str, now: DateTime<Utc>) -> Result<()> {
        self.set_password_hash_impl(user_id, hash, now).await
    }

    async fn record_login(&self, user_id: i64, now: DateTime<Utc>) -> Result<()> {
        self.record_login_impl(user_id, now).await
    }

    async fn has_any_user(&self) -> Result<bool> {
        self.has_any_user_impl().await
    }

    async fn create_session(&self, session: &Session, now: DateTime<Utc>) -> Result<()> {
        self.create_session_impl(session, now).await
    }

    async fn session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Session, User)>> {
        self.session_user_impl(session_id, now).await
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.delete_session_impl(session_id).await
    }

    async fn purge_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64> {
        self.purge_expired_sessions_impl(now).await
    }

    async fn touch_device(
        &self,
        device_uuid: &str,
        team_number: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<Device> {
        self.touch_device_impl(device_uuid, team_number, now).await
    }

    async fn device_by_uuid(&self, device_uuid: &str) -> Result<Option<Device>> {
        self.device_by_uuid_impl(device_uuid).await
    }

    async fn list_devices(&self) -> Result<Vec<Device>> {
        self.list_devices_impl().await
    }

    async fn rename_device(&self, id: i64, name: &str, now: DateTime<Utc>) -> Result<()> {
        self.rename_device_impl(id, name, now).await
    }

    async fn upsert_event(&self, event: &Event, now: DateTime<Utc>) -> Result<()> {
        self.upsert_event_impl(event, now).await
    }

    async fn event(&self, key: &str) -> Result<Option<Event>> {
        self.event_impl(key).await
    }

    async fn list_events(&self) -> Result<Vec<Event>> {
        self.list_events_impl().await
    }

    async fn events_for_team(&self, team_number: i32) -> Result<Vec<Event>> {
        self.events_for_team_impl(team_number).await
    }

    async fn active_events(
        &self,
        date: chrono::NaiveDate,
        lookahead_days: i64,
    ) -> Result<Vec<Event>> {
        self.active_events_impl(date, lookahead_days).await
    }

    async fn upsert_team(&self, team: &Team, now: DateTime<Utc>) -> Result<()> {
        self.upsert_team_impl(team, now).await
    }

    async fn team(&self, number: i32) -> Result<Option<Team>> {
        self.team_impl(number).await
    }

    async fn event_teams(&self, event_key: &str) -> Result<Vec<Team>> {
        self.event_teams_impl(event_key).await
    }

    async fn link_event_team(
        &self,
        event_key: &str,
        team_number: i32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.link_event_team_impl(event_key, team_number, now).await
    }

    async fn upsert_match(&self, record: &MatchRecord, now: DateTime<Utc>) -> Result<()> {
        self.upsert_match_impl(record, now).await
    }

    async fn event_matches(&self, event_key: &str) -> Result<Vec<MatchRecord>> {
        self.event_matches_impl(event_key).await
    }

    async fn team_matches(&self, event_key: &str, team_number: i32) -> Result<Vec<MatchRecord>> {
        self.team_matches_impl(event_key, team_number).await
    }

    async fn upsert_team_stats(&self, stats: &TeamEventStats, now: DateTime<Utc>) -> Result<()> {
        self.upsert_team_stats_impl(stats, now).await
    }

    async fn team_stats(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Option<TeamEventStats>> {
        self.team_stats_impl(event_key, team_number).await
    }

    async fn event_stats(&self, event_key: &str) -> Result<Vec<TeamEventStats>> {
        self.event_stats_impl(event_key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_database_is_healthy() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        assert_eq!(repo.health().await, Health::Ready);
    }

    #[tokio::test]
    async fn fresh_database_has_no_schema_version() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        assert_eq!(repo.schema_version().await.expect("probe"), None);
    }

    #[tokio::test]
    async fn schema_version_reads_back_what_was_set() {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        // PRAGMA does not accept bind parameters.
        sqlx::query("PRAGMA user_version = 7")
            .execute(repo.pool())
            .await
            .expect("set version");

        assert_eq!(repo.schema_version().await.expect("probe"), Some(7));
    }

    #[tokio::test]
    async fn unreachable_database_reports_down_rather_than_erroring() {
        // A directory that does not exist: connect() succeeds because it is lazy,
        // and the failure surfaces as Health::Down. This is the behaviour startup
        // depends on to degrade instead of aborting.
        let repo =
            SqliteRepo::connect("sqlite:///nonexistent-dir/tealteam.db").expect("lazy connect");
        assert_eq!(repo.health().await, Health::Down);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        // SQLite ignores FK constraints unless explicitly enabled; verify we did.
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(repo.pool())
            .await
            .expect("read pragma");
        assert_eq!(enabled, 1);
    }
}
