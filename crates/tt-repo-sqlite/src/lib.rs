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

mod assignments;
pub mod backup;
mod competition;
pub mod migrate;
mod observations;
mod picklist;
mod standings;
pub mod storage;
mod upstream;
mod users;
mod weights;

use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;
use std::time::Duration;
use tracing::warn;
use tt_core::assignments::{Assignment, Sighting};
use tt_core::picklist::Entry;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::{Decision, ReviewState};
use tt_core::season::WeightOverrides;
use tt_core::standings::Standing;
use tt_core::user::{Session, User};
use tt_repo::{
    Credentials, Device, Health, NewAssignment, NewObservation, NewUser, Recorded, Repo, RepoError,
    Result, Scout, StoredObservation,
};

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
        user: Option<&User>,
        now: DateTime<Utc>,
    ) -> Result<Device> {
        self.touch_device_impl(device_uuid, user, now).await
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

    async fn list_scouts(&self) -> Result<Vec<Scout>> {
        self.list_scouts_impl().await
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

    async fn match_by_key(&self, key: &str) -> Result<Option<MatchRecord>> {
        self.match_by_key_impl(key).await
    }

    async fn event_matches(&self, event_key: &str) -> Result<Vec<MatchRecord>> {
        self.event_matches_impl(event_key).await
    }

    async fn team_matches(&self, event_key: &str, team_number: i32) -> Result<Vec<MatchRecord>> {
        self.team_matches_impl(event_key, team_number).await
    }

    async fn event_assignments(&self, event_key: &str) -> Result<Vec<Assignment>> {
        self.event_assignments_impl(event_key).await
    }

    async fn set_assignments(
        &self,
        assignments: &[NewAssignment],
        assigned_by: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.set_assignments_impl(assignments, assigned_by, now)
            .await
    }

    async fn unassign(&self, match_key: &str, team_number: i32) -> Result<()> {
        self.unassign_impl(match_key, team_number).await
    }

    async fn clear_assignments(&self, event_key: &str, match_key: Option<&str>) -> Result<u64> {
        self.clear_assignments_impl(event_key, match_key).await
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

    async fn record_standings(
        &self,
        event_key: &str,
        standings: &[Standing],
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.record_standings_impl(event_key, standings, now).await
    }

    async fn record_observation(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        self.record_observation_impl(observation, now).await
    }

    async fn observed_teams(&self, match_key: &str, scouter_id: i64) -> Result<Vec<i32>> {
        self.observed_teams_impl(match_key, scouter_id).await
    }

    async fn recorded_by(&self, event_key: &str, scouter_id: i64) -> Result<Vec<(String, i32)>> {
        self.recorded_by_impl(event_key, scouter_id).await
    }

    async fn event_sightings(&self, event_key: &str) -> Result<Vec<Sighting>> {
        self.event_sightings_impl(event_key).await
    }

    async fn pending_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>> {
        self.observations_in_state_impl(event_key, ReviewState::Pending)
            .await
    }

    async fn approved_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>> {
        self.observations_in_state_impl(event_key, ReviewState::Approved)
            .await
    }

    async fn weight_overrides(&self) -> Result<WeightOverrides> {
        self.weight_overrides_impl().await
    }

    async fn replace_weight_overrides(
        &self,
        overrides: &WeightOverrides,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.replace_weight_overrides_impl(overrides, now).await
    }

    async fn observation(&self, id: i64) -> Result<Option<StoredObservation>> {
        self.observation_impl(id).await
    }

    async fn review_observation(
        &self,
        id: i64,
        decision: &Decision,
        reviewer_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.review_observation_impl(id, decision, reviewer_id, now)
            .await
    }

    async fn declined_for(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<StoredObservation>> {
        self.declined_for_impl(event_key, scouter_id).await
    }

    async fn pick_list(&self, owning_team: i32, event_key: &str) -> Result<Vec<Entry>> {
        self.pick_list_impl(owning_team, event_key).await
    }

    async fn replace_pick_list(
        &self,
        owning_team: i32,
        event_key: &str,
        expected: &[Entry],
        list: &[Entry],
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.replace_pick_list_impl(owning_team, event_key, expected, list, now)
            .await
    }

    async fn append_upstream(&self, entry: &tt_repo::NewUpstream) -> Result<Option<i64>> {
        self.append_upstream_impl(entry).await
    }

    async fn upstream_since(&self, after: i64, limit: i64) -> Result<Vec<tt_repo::UpstreamEntry>> {
        self.upstream_since_impl(after, limit).await
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

    /// A database file in a directory of its own, removed on drop.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("tt-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDb(dir)
        }
        fn url(&self) -> String {
            format!("sqlite://{}", self.0.join("tealteam.db").display())
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn a_file_database_is_wal_with_normal_sync() {
        // P3. Memory databases cannot be WAL, so this needs a real file.
        let db = TempDb::new("wal");
        let repo = SqliteRepo::connect(&db.url()).expect("connect");
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(repo.pool())
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(repo.pool())
            .await
            .unwrap();
        assert_eq!(sync, 1, "NORMAL");
    }

    #[tokio::test]
    async fn there_is_one_writer_and_the_next_one_waits() {
        // P3. The pool's one connection is the single writer: while a
        // transaction is open, nothing else in the process can write, and a
        // second write waits for it rather than failing with SQLITE_BUSY.
        let db = TempDb::new("writer");
        let repo = SqliteRepo::connect(&db.url()).expect("connect");
        assert_eq!(repo.pool().options().get_max_connections(), 1);
        sqlx::query("CREATE TABLE t (n INTEGER)")
            .execute(repo.pool())
            .await
            .unwrap();

        let mut first = repo.pool().begin().await.unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(&mut *first)
            .await
            .unwrap();

        let pool = repo.pool().clone();
        let second = tokio::spawn(async move {
            sqlx::query("INSERT INTO t VALUES (2)")
                .execute(&pool)
                .await
                .map(|_| ())
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!second.is_finished(), "the second write waited");

        first.commit().await.unwrap();
        second.await.unwrap().expect("then landed");
        let rows: Vec<i64> = sqlx::query_scalar("SELECT n FROM t ORDER BY rowid")
            .fetch_all(repo.pool())
            .await
            .unwrap();
        assert_eq!(rows, [1, 2]);
    }

    #[tokio::test]
    async fn a_backup_restores_into_a_fresh_database_with_everything_in_it() {
        // Q4: the deliberate restore test.
        let db = TempDb::new("backup");
        let repo = SqliteRepo::connect(&db.url()).expect("connect");
        migrate::apply(repo.pool()).await.expect("migrate");
        let user = NewUser {
            email: "kim@example.com".into(),
            name: "Kim".into(),
            password_hash: "hash".into(),
            team_number: Some(10101),
            roles: tt_core::user::Roles::default(),
        };
        // No checkpoints: what follows stays in the -wal file, where a copy
        // of tealteam.db alone would miss it. The snapshot must not.
        sqlx::query("PRAGMA wal_autocheckpoint = 0")
            .execute(repo.pool())
            .await
            .unwrap();
        repo.create_user(user, Utc::now()).await.expect("user");
        let late = NewUser {
            email: "sam@example.com".into(),
            name: "Sam".into(),
            password_hash: "hash".into(),
            team_number: Some(10101),
            roles: tt_core::user::Roles::default(),
        };
        repo.create_user(late, Utc::now()).await.expect("late user");
        let main_file = std::fs::read(db.0.join("tealteam.db")).unwrap();
        assert!(
            !main_file.windows(15).any(|w| w == b"sam@example.com"),
            "Sam is only in the WAL"
        );

        // The server's writer is mid-transaction; the snapshot does not wait
        // for it, and does not see its uncommitted row.
        let mut open_tx = repo.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO events (tba_key, name, created_at, updated_at) VALUES ('x', 'x', '', '')",
        )
        .execute(&mut *open_tx)
        .await
        .unwrap();
        let dir = db.0.join("backups");
        std::fs::create_dir(&dir).unwrap();
        let taken = tokio::time::timeout(
            Duration::from_secs(3),
            backup::snapshot(&db.url(), &dir, Utc::now()),
        )
        .await
        .expect("did not wait for the writer")
        .expect("snapshot");
        open_tx.rollback().await.unwrap();
        assert!(backup::taken_at(taken.file_name().unwrap().to_str().unwrap()).is_some());
        assert!(!std::fs::read_dir(&dir).unwrap().any(|e| {
            e.unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "partial")
        }));

        // The check: a fresh database from the file, at this build's schema.
        let restored = backup::check(&taken).await.expect("restores");
        assert_eq!(restored.migrated, 0, "taken by this build");
        assert_eq!(restored.counts[3], ("users", 2));
        assert_eq!(
            restored.counts[4],
            ("events", 0),
            "uncommitted is not in it"
        );

        // And put back the way PI_STORAGE.md says: copied into place, then
        // opened as the server opens it.
        let fresh = db.0.join("restored.db");
        std::fs::copy(&taken, &fresh).unwrap();
        let back = SqliteRepo::connect(&format!("sqlite://{}", fresh.display())).unwrap();
        migrate::apply(back.pool()).await.unwrap();
        let sam = back
            .credentials_by_email("sam@example.com")
            .await
            .unwrap()
            .expect("the row that was only in the WAL came back");
        assert_eq!(sam.user.name, "Sam");
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_database_fails_the_check() {
        let db = TempDb::new("notdb");
        let junk = db.0.join("tealteam-20260314T094000Z.db");
        std::fs::write(&junk, b"half a file").unwrap();
        let err = backup::check(&junk).await.unwrap_err();
        assert!(err.to_string().contains("not a database"), "{err}");
    }

    #[test]
    fn the_startup_log_can_say_where_the_database_is() {
        let db = TempDb::new("where");
        let at = storage::locate(&db.url()).expect("parses");
        assert_eq!(at.path.as_deref(), Some(db.0.join("tealteam.db").as_path()));
        // This machine's disk, whatever it is; on the Pi's SD card it would
        // say so.
        assert_ne!(at.medium, storage::Medium::Memory);
    }
}
