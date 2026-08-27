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
        self.last_seen_at
            .is_some_and(|seen| now.signed_duration_since(seen) <= window)
    }
}

/// How long since a heartbeat a device still counts as online.
pub const DEVICE_ONLINE_WINDOW: chrono::TimeDelta = chrono::TimeDelta::minutes(3);

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
    /// `team_number` fills in only if the device does not already have one, so a
    /// borrowed tablet is not relabelled by whoever picks it up.
    async fn touch_device(
        &self,
        device_uuid: &str,
        team_number: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<Device>;

    async fn device_by_uuid(&self, device_uuid: &str) -> Result<Option<Device>>;

    async fn list_devices(&self) -> Result<Vec<Device>>;

    async fn rename_device(&self, id: i64, name: &str, now: DateTime<Utc>) -> Result<()>;
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
