//! The browser's [`LocalRepo`] (C4): the server's schema and SQL, over SQLite
//! compiled to wasm, kept in the origin private file system.
//!
//! # What it opens
//!
//! A device's database is a snapshot (S10): the server's own file, cut down
//! to what this viewer may see, written to OPFS as `tealteam.sqlite3` by
//! `static/js/snapshot.js`. It is this build's schema, migrations table and
//! triggers included, so nothing here creates or migrates a table. The SQL in
//! each module is `tt_repo_sqlite`'s, statement for statement, and the
//! round-trip tests run both against one snapshot (C11 widens them).
//!
//! # Why the whole file is held in memory
//!
//! The database is read out of OPFS into memory with `sqlite3_deserialize`,
//! and every change is written back whole, swapped in by `createWritable`
//! only when complete. SQLite's OPFS file systems need a synchronous access
//! handle, which only a dedicated worker has, and the handler dispatch (C5)
//! runs in the service worker. A snapshot of a few events is a few
//! megabytes, and a write is a scout's form every couple of minutes.
//!
//! One context owns the file. A page and the service worker each opening it
//! would each write back their own copy, and the last one would win.
//!
//! # Foreign keys are off
//!
//! On the server they are on. Here they cannot be: a snapshot carries no
//! `devices` and only the `users` its rows name, by name (S10b), so a
//! scout's own observation can name a `scouter_id` this file has no row for. The server
//! enforces the keys when the row reaches it (C7); on the device they would
//! only refuse every offline write.
//!
//! Only in a secure context: https, or the device itself. Over the event
//! LAN's plain http there is no OPFS at all (open decision 9).

mod assignments;
mod competition;
mod observations;
#[cfg(target_arch = "wasm32")]
pub mod opfs;
mod picklist;
mod sql;
mod upstream;
mod users;

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, MAIN_DB, Params, Row};
use tt_core::assignments::{Assignment, Sighting};
use tt_core::picklist::Entry;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::{Decision, ReviewState};
use tt_core::season::WeightOverrides;
use tt_core::standings::Standing;
use tt_core::user::{Session, User};
use tt_repo::{
    Change, Credentials, Device, Health, LocalRepo, NewAssignment, NewObservation, NewUpstream,
    NewUser, Recorded, RepoError, Result, Scout, StoredObservation, UpstreamEntry,
};

use crate::sql::Context;

/// The first sixteen bytes of every SQLite database file.
const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Writes the whole database somewhere it survives the page: OPFS in a
/// browser ([`opfs::open`]), anything at all in a test.
pub type Saver = Box<dyn Fn(Vec<u8>) -> Pin<Box<dyn Future<Output = Result<()>>>>>;

pub struct ClientRepo {
    conn: Connection,
    saver: Option<Saver>,
    /// `total_changes` when the file was last written: anything above it is
    /// in memory only.
    saved: Cell<u64>,
    saving: Cell<bool>,
}

impl ClientRepo {
    /// Open a database file's bytes, such as a snapshot. Nothing is written
    /// anywhere until a saver is given ([`ClientRepo::saving_with`]).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if !bytes.starts_with(MAGIC) {
            return Err(RepoError::Refused("this is not a database".into()));
        }
        // A file left in WAL mode opens in memory only once it says it is
        // not: bytes 18 and 19 are the write and read versions, 2 for WAL.
        // A snapshot is already 1 (S10); a backup copied over by hand is not.
        let mut file = bytes.to_vec();
        if file.len() > 19 && file[18] == 2 && file[19] == 2 {
            file[18] = 1;
            file[19] = 1;
        }
        let mut conn = Connection::open_in_memory().ctx("opening the device's database")?;
        conn.deserialize_read_exact(MAIN_DB, file.as_slice(), file.len(), false)
            .ctx("reading the device's database")?;
        conn.execute_batch("PRAGMA foreign_keys = OFF")
            .ctx("opening the device's database")?;
        // Deserializing checks nothing; the first read does.
        conn.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |_| Ok(()))
            .ctx("reading the device's database")?;
        Ok(Self {
            saved: Cell::new(conn.total_changes()),
            conn,
            saver: None,
            saving: Cell::new(false),
        })
    }

    /// Write the database back with `saver` after every change.
    pub fn saving_with(mut self, saver: Saver) -> Self {
        self.saver = Some(saver);
        self
    }

    /// The whole database as a file, as [`ClientRepo::from_bytes`] reads it.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let data = self
            .conn
            .serialize(MAIN_DB)
            .ctx("copying the device's database")?;
        Ok(data.to_vec())
    }

    /// The newest migration this file was made with: the number a sync
    /// compares with the server's (S11), as the snapshot's
    /// `_sqlx_migrations` recorded it.
    pub fn schema(&self) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT MAX(version) FROM _sqlx_migrations WHERE success",
                [],
                |row| row.get(0),
            )
            .ctx("reading the schema version")
    }

    /// Whether every change so far has reached the saver.
    pub fn is_saved(&self) -> bool {
        self.conn.total_changes() == self.saved.get()
    }

    /// Write the file if anything changed since it was last written.
    ///
    /// Called after every trait method, so a read costs one comparison. A
    /// change made while a write is under way is picked up by that write's
    /// loop rather than starting a second one, which could finish first and
    /// leave the older copy in place.
    pub async fn flush(&self) -> Result<()> {
        let Some(saver) = &self.saver else {
            return Ok(());
        };
        if self.saving.replace(true) {
            return Ok(());
        }
        let result = async {
            loop {
                let seen = self.conn.total_changes();
                if seen == self.saved.get() {
                    return Ok(());
                }
                saver(self.to_bytes()?).await?;
                self.saved.set(seen);
            }
        }
        .await;
        self.saving.set(false);
        result
    }

    /// Every row of a query, mapped.
    fn all<T>(
        &self,
        sql: &str,
        params: impl Params,
        map: impl FnMut(&Row) -> rusqlite::Result<T>,
        what: &str,
    ) -> Result<Vec<T>> {
        let mut statement = self.conn.prepare(sql).ctx(what)?;
        statement
            .query_map(params, map)
            .and_then(|rows| rows.collect())
            .ctx(what)
    }

    /// A method's answer, once whatever it changed is saved.
    async fn saved<T>(&self, answer: Result<T>) -> Result<T> {
        let answer = answer?;
        self.flush().await?;
        Ok(answer)
    }
}

// Each method runs its module's `*_impl`, then saves if it wrote anything:
// the same thin index as `tt_repo_sqlite`'s, so the two read side by side.
impl LocalRepo for ClientRepo {
    async fn health(&self) -> Health {
        match self.conn.query_row("SELECT 1", [], |_| Ok(())) {
            Ok(()) => Health::Ready,
            Err(e) => {
                tracing::warn!("device database health probe failed: {e}");
                Health::Down
            }
        }
    }

    async fn schema_version(&self) -> Result<Option<i64>> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .ctx("reading user_version")?;
        Ok((version > 0).then_some(version))
    }

    async fn create_user(&self, new_user: NewUser, now: DateTime<Utc>) -> Result<User> {
        self.saved(self.create_user_impl(new_user, now)).await
    }

    async fn credentials_by_email(&self, email: &str) -> Result<Option<Credentials>> {
        self.credentials_by_email_impl(email)
    }

    async fn user_by_id(&self, id: i64) -> Result<Option<User>> {
        self.user_by_id_impl(id)
    }

    async fn password_hash(&self, user_id: i64) -> Result<Option<String>> {
        self.password_hash_impl(user_id)
    }

    async fn set_password_hash(&self, user_id: i64, hash: &str, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.set_password_hash_impl(user_id, hash, now))
            .await
    }

    async fn record_login(&self, user_id: i64, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.record_login_impl(user_id, now)).await
    }

    async fn has_any_user(&self) -> Result<bool> {
        self.has_any_user_impl()
    }

    async fn create_session(&self, session: &Session, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.create_session_impl(session, now)).await
    }

    async fn session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Session, User)>> {
        // May delete an expired session.
        self.saved(self.session_user_impl(session_id, now)).await
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.saved(self.delete_session_impl(session_id)).await
    }

    async fn purge_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64> {
        self.saved(self.purge_expired_sessions_impl(now)).await
    }

    async fn touch_device(
        &self,
        device_uuid: &str,
        user: Option<&User>,
        now: DateTime<Utc>,
    ) -> Result<Device> {
        self.saved(self.touch_device_impl(device_uuid, user, now))
            .await
    }

    async fn device_by_uuid(&self, device_uuid: &str) -> Result<Option<Device>> {
        self.device_by_uuid_impl(device_uuid)
    }

    async fn record_clock_offset(
        &self,
        device_uuid: &str,
        offset_ms: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.saved(self.record_clock_offset_impl(device_uuid, offset_ms, now))
            .await
    }

    async fn list_devices(&self) -> Result<Vec<Device>> {
        self.list_devices_impl()
    }

    async fn rename_device(&self, id: i64, name: &str, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.rename_device_impl(id, name, now)).await
    }

    async fn list_scouts(&self) -> Result<Vec<Scout>> {
        self.list_scouts_impl()
    }

    async fn upsert_event(&self, event: &Event, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.upsert_event_impl(event, now)).await
    }

    async fn event(&self, key: &str) -> Result<Option<Event>> {
        self.event_impl(key)
    }

    async fn list_events(&self) -> Result<Vec<Event>> {
        self.list_events_impl()
    }

    async fn events_for_team(&self, team_number: i32) -> Result<Vec<Event>> {
        self.events_for_team_impl(team_number)
    }

    async fn active_events(
        &self,
        date: chrono::NaiveDate,
        lookahead_days: i64,
    ) -> Result<Vec<Event>> {
        self.active_events_impl(date, lookahead_days)
    }

    async fn upsert_team(&self, team: &Team, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.upsert_team_impl(team, now)).await
    }

    async fn team(&self, number: i32) -> Result<Option<Team>> {
        self.team_impl(number)
    }

    async fn event_teams(&self, event_key: &str) -> Result<Vec<Team>> {
        self.event_teams_impl(event_key)
    }

    async fn link_event_team(
        &self,
        event_key: &str,
        team_number: i32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.saved(self.link_event_team_impl(event_key, team_number, now))
            .await
    }

    async fn upsert_match(&self, record: &MatchRecord, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.upsert_match_impl(record, now)).await
    }

    async fn match_by_key(&self, key: &str) -> Result<Option<MatchRecord>> {
        self.match_by_key_impl(key)
    }

    async fn event_matches(&self, event_key: &str) -> Result<Vec<MatchRecord>> {
        self.event_matches_impl(event_key)
    }

    async fn team_matches(&self, event_key: &str, team_number: i32) -> Result<Vec<MatchRecord>> {
        self.team_matches_impl(event_key, team_number)
    }

    async fn event_assignments(&self, event_key: &str) -> Result<Vec<Assignment>> {
        self.event_assignments_impl(event_key)
    }

    async fn set_assignments(
        &self,
        assignments: &[NewAssignment],
        assigned_by: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.saved(self.set_assignments_impl(assignments, assigned_by, now))
            .await
    }

    async fn unassign(&self, match_key: &str, team_number: i32) -> Result<()> {
        self.saved(self.unassign_impl(match_key, team_number)).await
    }

    async fn clear_assignments(&self, event_key: &str, match_key: Option<&str>) -> Result<u64> {
        self.saved(self.clear_assignments_impl(event_key, match_key))
            .await
    }

    async fn upsert_team_stats(&self, stats: &TeamEventStats, now: DateTime<Utc>) -> Result<()> {
        self.saved(self.upsert_team_stats_impl(stats, now)).await
    }

    async fn team_stats(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Option<TeamEventStats>> {
        self.team_stats_impl(event_key, team_number)
    }

    async fn event_stats(&self, event_key: &str) -> Result<Vec<TeamEventStats>> {
        self.event_stats_impl(event_key)
    }

    async fn record_standings(
        &self,
        event_key: &str,
        standings: &[Standing],
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.saved(self.record_standings_impl(event_key, standings, now))
            .await
    }

    async fn record_observation(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded> {
        self.saved(self.record_observation_impl(observation, now))
            .await
    }

    async fn observed_teams(&self, match_key: &str, scouter_id: i64) -> Result<Vec<i32>> {
        self.observed_teams_impl(match_key, scouter_id)
    }

    async fn recorded_by(&self, event_key: &str, scouter_id: i64) -> Result<Vec<(String, i32)>> {
        self.recorded_by_impl(event_key, scouter_id)
    }

    async fn event_sightings(&self, event_key: &str) -> Result<Vec<Sighting>> {
        self.event_sightings_impl(event_key)
    }

    async fn weight_overrides(&self) -> Result<WeightOverrides> {
        self.weight_overrides_impl()
    }

    async fn replace_weight_overrides(
        &self,
        overrides: &WeightOverrides,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.saved(self.replace_weight_overrides_impl(overrides, now))
            .await
    }

    async fn pending_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>> {
        self.observations_in_state_impl(event_key, ReviewState::Pending)
    }

    async fn approved_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>> {
        self.observations_in_state_impl(event_key, ReviewState::Approved)
    }

    async fn observation(&self, id: i64) -> Result<Option<StoredObservation>> {
        self.observation_impl(id)
    }

    async fn review_observation(
        &self,
        id: i64,
        decision: &Decision,
        reviewer_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.saved(self.review_observation_impl(id, decision, reviewer_id, now))
            .await
    }

    async fn declined_for(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<StoredObservation>> {
        self.declined_for_impl(event_key, scouter_id)
    }

    async fn pick_list(&self, owning_team: i32, event_key: &str) -> Result<Vec<Entry>> {
        self.pick_list_impl(owning_team, event_key)
    }

    async fn pick_list_doc(
        &self,
        owning_team: i32,
        event_key: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>> {
        // Stores a document made from the rows, the first time.
        self.saved(self.pick_list_doc_impl(owning_team, event_key, now))
            .await
    }

    async fn merge_pick_list(
        &self,
        owning_team: i32,
        event_key: &str,
        update: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Vec<Entry>> {
        self.saved(self.merge_pick_list_impl(owning_team, event_key, update, now))
            .await
    }

    async fn append_upstream(&self, entry: &NewUpstream) -> Result<Option<i64>> {
        self.saved(self.append_upstream_impl(entry)).await
    }

    async fn upstream_since(&self, after: i64, limit: i64) -> Result<Vec<UpstreamEntry>> {
        self.upstream_since_impl(after, limit)
    }

    async fn latest_upstream(&self, api: &str, path: &str) -> Result<Option<UpstreamEntry>> {
        self.latest_upstream_impl(api, path)
    }

    async fn changes_since(
        &self,
        after: i64,
        limit: i64,
        settled_before: DateTime<Utc>,
    ) -> Result<Vec<Change>> {
        self.changes_since_impl(after, limit, settled_before)
    }

    async fn log_heads(&self) -> Result<(i64, i64)> {
        self.log_heads_impl()
    }
}
