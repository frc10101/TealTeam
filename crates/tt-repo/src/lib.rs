//! The `Repo` trait: the seam between domain logic and storage.
//!
//! This crate exists so that [`tt_core`] never learns what a database is. A
//! handler asks a `Repo` for data; whether that resolves to SQLite on a
//! Raspberry Pi or SQLite-WASM in a browser tab is not the handler's business.
//!
//! # Why two variants
//!
//! Server futures must be `Send` -- tokio's multi-threaded runtime moves tasks
//! between worker threads. Browser futures cannot be `Send`, because everything
//! in a wasm32 environment is pinned to one thread and the underlying handles
//! are not thread-safe.
//!
//! Writing the trait twice by hand means two definitions drifting apart.
//! Instead [`trait_variant`] generates the `Send` flavour from the local one:
//!
//! - [`LocalRepo`] -- the definition. Futures need not be `Send`. Browser adapters
//!   implement this.
//! - [`Repo`] -- generated, identical, with `Send` bounds. Server adapters
//!   implement this, and axum handlers take it.
//!
//! Implement whichever matches your runtime. Do not implement both by hand.

use chrono::{DateTime, Utc};
use thiserror::Error;
use tt_core::assignments::{AssigneeKey, Assignment, Sighting};
use tt_core::picklist::Entry;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::review::{Decision, ReviewState};
use tt_core::season::{Payload, WeightOverrides};
use tt_core::standings::Standing;
use tt_core::user::{Roles, Session, User};

/// Anything that can go wrong reaching storage.
///
/// Deliberately not an alias for the adapter's own error type: `tt-web` handles
/// these without knowing whether sqlx, OPFS, or a network hop produced them.
#[derive(Debug, Error)]
pub enum RepoError {
    /// Storage is unreachable. On the server this is a dead pool; in a browser
    /// it is usually a missing or evicted OPFS handle.
    ///
    /// This is a first-class case rather than a generic failure because the app
    /// is explicitly required to keep serving when the database is down
    /// (REBUILD_SPEC.md 8) -- callers need to distinguish "no database" from
    /// "database said no".
    #[error("storage unavailable: {0}")]
    Unavailable(String),

    /// The query ran and failed.
    #[error("query failed: {0}")]
    Query(String),

    /// A uniqueness constraint rejected the write. `what` names the thing that
    /// already exists, in words safe to show a user.
    #[error("{what} already exists")]
    Conflict { what: &'static str },

    /// Schema is older or newer than this build expects.
    #[error("schema version mismatch: expected {expected}, found {found}")]
    SchemaMismatch { expected: i64, found: i64 },
}

pub type Result<T> = std::result::Result<T, RepoError>;

/// Result of a storage health probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Reachable and responding.
    Ready,
    /// Not reachable. The app still boots and serves what it can.
    Down,
}

impl Health {
    pub fn is_ready(self) -> bool {
        matches!(self, Health::Ready)
    }
}

/// Everything needed to create an account.
#[derive(Debug, Clone)]
pub struct NewUser {
    /// Already normalised and validated by `tt_core::user::validate_email`.
    pub email: String,
    pub name: String,
    /// Already hashed. The trait never sees a plaintext password.
    pub password_hash: String,
    pub team_number: Option<i32>,
    pub roles: Roles,
}

/// A stored account, including the hash that a login must verify against.
///
/// Separate from [`User`] so that the hash cannot leak into a view model by
/// accident: handlers pass `User` around, and only the login path ever holds
/// this.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub user: User,
    pub password_hash: String,
}

/// A tablet's self-reported presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: i64,
    pub device_uuid: String,
    pub name: Option<String>,
    pub team_number: Option<i32>,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Who was signed in at its latest heartbeat, if anyone.
    pub last_user_id: Option<i64>,
}

impl Device {
    /// Display name, falling back to a short form of the UUID so a lead scout
    /// can still tell two unnamed tablets apart.
    pub fn display_name(&self) -> String {
        match self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            Some(name) => name.to_string(),
            None => format!(
                "Device {}",
                &self.device_uuid[..8.min(self.device_uuid.len())]
            ),
        }
    }

    /// Whether this device has checked in recently enough to be considered
    /// present. Heartbeats are every 60s; the window allows two misses.
    pub fn is_online(&self, now: DateTime<Utc>, window: chrono::TimeDelta) -> bool {
        seen_within(self.last_seen_at, now, window)
    }
}

/// How long since a heartbeat a device, or a person, still counts as online.
pub const DEVICE_ONLINE_WINDOW: chrono::TimeDelta = chrono::TimeDelta::minutes(3);

fn seen_within(
    last_seen: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    window: chrono::TimeDelta,
) -> bool {
    last_seen.is_some_and(|seen| now.signed_duration_since(seen) <= window)
}

/// Someone who can be handed a robot, and when their browser last checked in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scout {
    pub id: i64,
    pub name: String,
    pub team_number: Option<i32>,
    /// From the heartbeat of a signed-in page -- not from having a session,
    /// which outlives the person leaving by a day (REBUILD_SPEC.md 12.13).
    pub last_seen_at: Option<DateTime<Utc>>,
}

impl Scout {
    pub fn is_online(&self, now: DateTime<Utc>, window: chrono::TimeDelta) -> bool {
        seen_within(self.last_seen_at, now, window)
    }
}

/// One robot in one match for one assignee, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAssignment {
    pub match_key: String,
    pub event_key: String,
    pub team_number: i32,
    pub assignee: AssigneeKey,
}

/// One scout's record of one robot in one match, ready to store.
///
/// Arrives pending review; approving and declining it are later updates to the
/// same row (L9, L10), never a copy into another table.
#[derive(Debug, Clone)]
pub struct NewObservation {
    /// UUIDv7 minted when the form was rendered (D7). A second post carrying
    /// the same id is the same observation, not another one.
    pub client_record_id: String,
    pub match_key: String,
    pub event_key: String,
    pub team_number: i32,
    /// `"red"` or `"blue"`, taken from the match rather than asked of the scout.
    pub alliance: &'static str,
    /// Already checked against the season schema.
    pub payload: Payload,
    pub schema_version: i64,
    pub scouter_id: Option<i64>,
    pub device_id: Option<i64>,
    /// The scout's team, resolved now (L7). It decides who may read the notes,
    /// and the retired app needed a backfill migration after leaving it null.
    pub submitting_team: Option<i32>,
    /// When the match was watched. Not when the row reached the server, which
    /// in phase 3 can be much later.
    pub observed_at: DateTime<Utc>,
}

/// An observation as stored, with names resolved, for review (L8-L10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObservation {
    pub id: i64,
    pub match_key: String,
    pub event_key: String,
    pub team_number: i32,
    /// `"red"` or `"blue"`.
    pub alliance: String,
    /// Unreadable JSON comes back empty, rather than hiding the row.
    pub payload: Payload,
    pub schema_version: i64,
    pub scouter_id: Option<i64>,
    /// `None` once the account is gone.
    pub scouter_name: Option<String>,
    pub submitting_team: Option<i32>,
    pub review_state: ReviewState,
    pub review_note: Option<String>,
    pub reviewer_name: Option<String>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub observed_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
}

/// What recording an observation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// Stored now, with this row id.
    Created(i64),
    /// This `client_record_id` was already stored -- a double-tapped Save or a
    /// retried post. Nothing was written; the earlier row stands.
    Duplicate(i64),
}

#[trait_variant::make(Repo: Send)]
pub trait LocalRepo {
    // ── Health ──────────────────────────────────────────────────────────────

    /// Cheap liveness probe. Must not error: an unreachable database is a
    /// [`Health::Down`] answer, not a failure to answer.
    async fn health(&self) -> Health;

    /// Highest applied migration version, or `None` on an empty database.
    async fn schema_version(&self) -> Result<Option<i64>>;

    // ── Users ───────────────────────────────────────────────────────────────

    /// Create an account. Returns [`RepoError::Conflict`] if the email is taken.
    async fn create_user(&self, new_user: NewUser, now: DateTime<Utc>) -> Result<User>;

    /// Look up an account by email, for login. Email must already be normalised.
    async fn credentials_by_email(&self, email: &str) -> Result<Option<Credentials>>;

    async fn user_by_id(&self, id: i64) -> Result<Option<User>>;

    /// Fetch the stored hash so a password change can verify the current one.
    async fn password_hash(&self, user_id: i64) -> Result<Option<String>>;

    async fn set_password_hash(&self, user_id: i64, hash: &str, now: DateTime<Utc>) -> Result<()>;

    async fn record_login(&self, user_id: i64, now: DateTime<Utc>) -> Result<()>;

    /// Whether any account exists.
    ///
    /// Used to make the first account created on a fresh database an admin --
    /// otherwise a new deployment has nobody who can grant anybody anything.
    async fn has_any_user(&self) -> Result<bool>;

    // ── Sessions ────────────────────────────────────────────────────────────

    async fn create_session(&self, session: &Session, now: DateTime<Utc>) -> Result<()>;

    /// Resolve a session cookie to its user.
    ///
    /// Expired sessions are deleted and reported as `None`, so expiry cleans up
    /// as a side effect of normal traffic rather than needing a sweeper task.
    async fn session_user(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Session, User)>>;

    async fn delete_session(&self, session_id: &str) -> Result<()>;

    /// Remove every expired session. Cheap; call occasionally.
    async fn purge_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64>;

    // ── Devices ─────────────────────────────────────────────────────────────

    /// Record a heartbeat, creating the device on first sight.
    ///
    /// With someone signed in, they are marked as seen too (A6), and become
    /// the device's last user. Their team fills in the device's only if it
    /// does not already have one, so a borrowed tablet is not relabelled by
    /// whoever picks it up.
    async fn touch_device(
        &self,
        device_uuid: &str,
        user: Option<&User>,
        now: DateTime<Utc>,
    ) -> Result<Device>;

    async fn device_by_uuid(&self, device_uuid: &str) -> Result<Option<Device>>;

    async fn list_devices(&self) -> Result<Vec<Device>>;

    async fn rename_device(&self, id: i64, name: &str, now: DateTime<Utc>) -> Result<()>;

    /// Every account, by name, with when each was last seen.
    async fn list_scouts(&self) -> Result<Vec<Scout>>;

    // ── Competition graph ───────────────────────────────────────────────────

    /// Insert or update an event, keyed on its TBA key.
    ///
    /// A real upsert, not select-then-insert: the retired schema lacked the
    /// unique constraint that makes this possible, and paid for it with a race
    /// (REBUILD_SPEC.md 12.11).
    async fn upsert_event(&self, event: &Event, now: DateTime<Utc>) -> Result<()>;

    async fn event(&self, key: &str) -> Result<Option<Event>>;

    /// Every event, earliest first.
    async fn list_events(&self) -> Result<Vec<Event>>;

    /// Events a given team is attending.
    async fn events_for_team(&self, team_number: i32) -> Result<Vec<Event>>;

    /// Events running on `date`, or starting within `lookahead_days` of it.
    ///
    /// Drives the sync cadence: fast during an event, slow between them (I7).
    async fn active_events(
        &self,
        date: chrono::NaiveDate,
        lookahead_days: i64,
    ) -> Result<Vec<Event>>;

    async fn upsert_team(&self, team: &Team, now: DateTime<Utc>) -> Result<()>;

    async fn team(&self, number: i32) -> Result<Option<Team>>;

    /// The roster for an event, by team number.
    async fn event_teams(&self, event_key: &str) -> Result<Vec<Team>>;

    /// Record that a team is attending an event. Idempotent.
    async fn link_event_team(
        &self,
        event_key: &str,
        team_number: i32,
        now: DateTime<Utc>,
    ) -> Result<()>;

    // ── Matches ─────────────────────────────────────────────────────────────

    async fn upsert_match(&self, record: &MatchRecord, now: DateTime<Utc>) -> Result<()>;

    async fn match_by_key(&self, key: &str) -> Result<Option<MatchRecord>>;

    /// An event's matches in playing order.
    async fn event_matches(&self, event_key: &str) -> Result<Vec<MatchRecord>>;

    /// Matches involving a team, in playing order.
    async fn team_matches(&self, event_key: &str, team_number: i32) -> Result<Vec<MatchRecord>>;

    // ── Assignments ─────────────────────────────────────────────────────────

    /// Every assignment at an event, with the assignee's name resolved.
    ///
    /// Where a row names both a scout and a tablet, the scout is reported: a
    /// person is the more specific instruction.
    async fn event_assignments(&self, event_key: &str) -> Result<Vec<Assignment>>;

    /// Assign each robot, replacing whoever had it. All or nothing.
    ///
    /// Like an observation, an assignment may name a robot that the schedule
    /// lists and no roster sync has created yet.
    async fn set_assignments(
        &self,
        assignments: &[NewAssignment],
        assigned_by: i64,
        now: DateTime<Utc>,
    ) -> Result<()>;

    /// Remove one robot's assignment. Removing one that does not exist is fine.
    async fn unassign(&self, match_key: &str, team_number: i32) -> Result<()>;

    /// Remove every assignment at an event, or in one match of it. Returns how
    /// many went.
    async fn clear_assignments(&self, event_key: &str, match_key: Option<&str>) -> Result<u64>;

    // ── Statistics ──────────────────────────────────────────────────────────

    async fn upsert_team_stats(&self, stats: &TeamEventStats, now: DateTime<Utc>) -> Result<()>;

    async fn team_stats(&self, event_key: &str, team_number: i32)
    -> Result<Option<TeamEventStats>>;

    /// Every team's stats at an event, best rank first.
    async fn event_stats(&self, event_key: &str) -> Result<Vec<TeamEventStats>>;

    /// Replace an event's ranking with one typed off the audience display
    /// (I14), in one transaction. Teams not listed lose their rank; OPRs are
    /// kept.
    async fn record_standings(
        &self,
        event_key: &str,
        standings: &[Standing],
        now: DateTime<Utc>,
    ) -> Result<()>;

    // ── Observations ────────────────────────────────────────────────────────

    /// Store a scout's observation, pending review.
    ///
    /// Idempotent on `client_record_id`: see [`Recorded::Duplicate`]. A second,
    /// different observation of the same robot in the same match by the same
    /// scout is a [`RepoError::Conflict`] while the first one is not declined.
    ///
    /// The robot need not be on a synced roster. Match schedules and rosters
    /// arrive from different feeds, and a scout must be able to record a robot
    /// the moment it is on the field.
    async fn record_observation(
        &self,
        observation: &NewObservation,
        now: DateTime<Utc>,
    ) -> Result<Recorded>;

    /// Robots in a match that a scout has an observation of, other than a
    /// declined one, by team number.
    async fn observed_teams(&self, match_key: &str, scouter_id: i64) -> Result<Vec<i32>>;

    /// Every robot a scout has an observation of at an event, other than a
    /// declined one, as `(match key, team number)`.
    async fn recorded_by(&self, event_key: &str, scouter_id: i64) -> Result<Vec<(String, i32)>>;

    /// Every observation at an event other than a declined one, as coverage
    /// sees it (L6).
    async fn event_sightings(&self, event_key: &str) -> Result<Vec<Sighting>>;

    // ── Point weights (L12) ─────────────────────────────────────────────────

    /// The lead scout's overrides of the season schema's point values.
    async fn weight_overrides(&self) -> Result<WeightOverrides>;

    /// Replace every override with these. All or nothing.
    async fn replace_weight_overrides(
        &self,
        overrides: &WeightOverrides,
        now: DateTime<Utc>,
    ) -> Result<()>;

    // ── Review (L8-L10) ─────────────────────────────────────────────────────

    /// An event's observations waiting for review, oldest first.
    async fn pending_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>>;

    /// An event's approved observations, oldest first: what rankings are
    /// computed from (L11).
    async fn approved_observations(&self, event_key: &str) -> Result<Vec<StoredObservation>>;

    async fn observation(&self, id: i64) -> Result<Option<StoredObservation>>;

    /// Approve or decline a pending observation. `false` when it was not
    /// pending -- already reviewed, perhaps by another lead a moment ago --
    /// and nothing changed.
    ///
    /// Approval resolves the scout's team if the row lacks it: it decides who
    /// may read the notes, and the retired app needed a backfill migration
    /// after leaving it null (REBUILD_SPEC.md 5.3).
    async fn review_observation(
        &self,
        id: i64,
        decision: &Decision,
        reviewer_id: i64,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    /// A scout's declined observations at an event that they have not since
    /// recorded again, newest review first: what the scout needs to be told.
    async fn declined_for(
        &self,
        event_key: &str,
        scouter_id: i64,
    ) -> Result<Vec<StoredObservation>>;

    // ── Pick list (U20) ─────────────────────────────────────────────────────

    /// A team's pick list for an event, best first.
    async fn pick_list(&self, owning_team: i32, event_key: &str) -> Result<Vec<Entry>>;

    /// Store `list` as the team's pick list, if the stored one still reads
    /// `expected`. `false` when someone changed it in between, and nothing was
    /// written: read it again and redo the edit on what is there now.
    async fn replace_pick_list(
        &self,
        owning_team: i32,
        event_key: &str,
        expected: &[Entry],
        list: &[Entry],
        now: DateTime<Utc>,
    ) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 14, 12, minute, 0).unwrap()
    }

    fn device(name: Option<&str>, seen: Option<DateTime<Utc>>) -> Device {
        Device {
            id: 1,
            device_uuid: "0191f7ac-1234-7000-8000-abcdefabcdef".into(),
            name: name.map(str::to_string),
            team_number: None,
            last_seen_at: seen,
            last_user_id: None,
        }
    }

    #[test]
    fn named_devices_show_their_name() {
        assert_eq!(
            device(Some("Stands Left"), None).display_name(),
            "Stands Left"
        );
    }

    #[test]
    fn unnamed_devices_fall_back_to_a_uuid_prefix() {
        assert_eq!(device(None, None).display_name(), "Device 0191f7ac");
        // A whitespace-only name is not a name.
        assert_eq!(device(Some("   "), None).display_name(), "Device 0191f7ac");
    }

    #[test]
    fn a_device_that_has_never_checked_in_is_not_online() {
        assert!(!device(None, None).is_online(at(0), DEVICE_ONLINE_WINDOW));
    }

    #[test]
    fn the_online_window_allows_two_missed_heartbeats() {
        // Heartbeats are every 60s and the window is 3 minutes.
        let d = device(None, Some(at(0)));
        assert!(d.is_online(at(2), DEVICE_ONLINE_WINDOW));
        assert!(d.is_online(at(3), DEVICE_ONLINE_WINDOW));
        assert!(!d.is_online(at(4), DEVICE_ONLINE_WINDOW));
    }
}
