//! Users: the account entity, credential hashing, and the queries behind
//! sign-in, sign-up, account settings and the scout picker.
//!
//! Password hashes are bcrypt at cost 12, matching the Go and .NET ports, so
//! an account created in any implementation works in all of them.
//!
//! Roles are three independent flags rather than one role column:
//! `is_admin` implies the other two ([`User::can_lead`], [`User::can_coach`]),
//! and every user can scout.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

const BCRYPT_COST: u32 = 12;

/// A `users` row.
#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub name: String,
    /// bcrypt hash; never rendered, and excluded from the DB viewer.
    pub password_hash: String,
    /// FRC team number this user scouts for. Drives which events they can
    /// select and whose scouting notes they can read.
    pub team_number: Option<i32>,
    /// Legacy free-text role from the Go schema; the boolean flags below are
    /// what authorization actually reads.
    pub role: Option<String>,
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl User {
    /// May use the lead scout panel, match assignments and manual FIRST sync.
    pub fn can_lead(&self) -> bool {
        self.is_admin || self.is_lead_scout
    }

    /// May use the drive coach panel.
    pub fn can_coach(&self) -> bool {
        self.is_admin || self.is_coach
    }

    /// Team number when it is set to a real team (0 is treated as unset).
    pub fn active_team_number(&self) -> Option<i32> {
        self.team_number.filter(|n| *n > 0)
    }
}

/// A signed-up user with an online flag, for the assignment picker.
#[derive(Debug, Clone, FromRow)]
pub struct ScoutRow {
    pub id: i32,
    pub name: String,
    pub online: bool,
}

/// Hashes a password with bcrypt at the shared cost factor.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(bcrypt::hash(password, BCRYPT_COST)?)
}

/// Verifies a password against a stored hash. A malformed or truncated hash
/// verifies as `false` rather than erroring, so a corrupt row cannot be used
/// to bypass the check.
pub fn check_password_hash(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

/// Looks up the user behind a session. `None` if the row is gone or the
/// database is unreachable, which signs the request out rather than failing.
pub async fn find_by_id(pool: &PgPool, id: i32) -> Option<User> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Looks up an account for sign-in. The error is kept distinct from "no such
/// user" so the controller can say "try again" instead of leaking whether the
/// address exists.
pub async fn find_by_email(pool: &PgPool, email: &str) -> anyhow::Result<Option<User>> {
    Ok(sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(pool)
        .await?)
}

/// Pre-checks sign-up for a duplicate address. The unique index is still the
/// real guard — [`create`] can fail on a race.
pub async fn email_exists(pool: &PgPool, email: &str) -> anyhow::Result<bool> {
    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE email = $1 LIMIT 1")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    Ok(existing.is_some())
}

/// Values for a new account, as entered on the sign-up form.
pub struct NewUser<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub password_hash: &'a str,
    pub team_number: Option<i32>,
    pub lead_scout: bool,
    pub coach: bool,
}

/// Inserts an account and returns its id. Every account starts with
/// `role = 'user'`; admin is granted out of band.
pub async fn create(pool: &PgPool, new_user: &NewUser<'_>) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (name, email, password_hash, team_number, role, is_lead_scout, is_coach)
         VALUES ($1, $2, $3, $4, 'user', $5, $6)
         RETURNING id",
    )
    .bind(new_user.name)
    .bind(new_user.email)
    .bind(new_user.password_hash)
    .bind(new_user.team_number)
    .bind(new_user.lead_scout)
    .bind(new_user.coach)
    .fetch_one(pool)
    .await
}

/// Records a successful sign-in. Best-effort: the caller logs a failure and
/// still signs the user in.
pub async fn touch_last_login(pool: &PgPool, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET last_login = $1 WHERE id = $2")
        .bind(Utc::now())
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Stores a new password hash. Existing sessions deliberately stay valid, as
/// in the other ports.
pub async fn update_password(
    pool: &PgPool,
    user_id: i32,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(password_hash)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Scouts assignable by `lead`: users on the lead's team, or everyone when the
/// lead has no team number. `online` means they hold an unexpired session.
pub async fn list_scouts(pool: &PgPool, lead_team_number: Option<i32>) -> Vec<ScoutRow> {
    let query = if lead_team_number.is_some() {
        "SELECT users.id, users.name,
                EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = users.id AND s.expires_at > $2) AS online
         FROM users WHERE users.team_number = $1 ORDER BY users.name"
    } else {
        "SELECT users.id, users.name,
                EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = users.id AND s.expires_at > $2) AS online
         FROM users WHERE ($1::int IS NULL OR TRUE) ORDER BY users.name"
    };

    sqlx::query_as(query)
        .bind(lead_team_number)
        .bind(Utc::now())
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Ids of scouts with a live session, used by auto-distribute.
pub async fn online_scout_ids(pool: &PgPool, lead_team_number: Option<i32>) -> Vec<i32> {
    let query = if lead_team_number.is_some() {
        "SELECT DISTINCT users.id FROM users
         JOIN sessions s ON s.user_id = users.id AND s.expires_at > $1
         WHERE users.team_number = $2"
    } else {
        "SELECT DISTINCT users.id FROM users
         JOIN sessions s ON s.user_id = users.id AND s.expires_at > $1
         WHERE ($2::int IS NULL OR TRUE)"
    };

    sqlx::query_scalar(query)
        .bind(Utc::now())
        .bind(lead_team_number)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}
