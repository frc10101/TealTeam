//! Users, sessions, and devices against SQLite.

use chrono::{DateTime, Utc};
use sqlx::Row;
use tt_core::user::{Roles, Session, User};
use tt_repo::{Credentials, Device, NewUser, RepoError, Result};

use crate::SqliteRepo;

/// SQLite has no native timestamp type. ISO-8601 UTC sorts lexicographically,
/// which is what makes `expires_at > ?` and `ORDER BY created_at` work as plain
/// string comparisons.
pub(crate) fn to_sql(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn from_sql(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(crate) fn query_err(context: &str, e: sqlx::Error) -> RepoError {
    // A closed pool or missing file means storage is gone, which callers treat
    // differently from a query that ran and failed.
    match &e {
        sqlx::Error::PoolClosed | sqlx::Error::PoolTimedOut => {
            RepoError::Unavailable(format!("{context}: {e}"))
        }
        sqlx::Error::Database(db) if db.message().contains("unable to open database") => {
            RepoError::Unavailable(format!("{context}: {e}"))
        }
        _ => RepoError::Query(format!("{context}: {e}")),
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.is_unique_violation())
}

fn user_from_row(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: row.get("id"),
        email: row.get("email"),
        name: row.get("name"),
        team_number: row.get("team_number"),
        roles: Roles {
            is_admin: row.get::<i64, _>("is_admin") != 0,
            is_lead_scout: row.get::<i64, _>("is_lead_scout") != 0,
            is_coach: row.get::<i64, _>("is_coach") != 0,
        },
    }
}

fn device_from_row(row: &sqlx::sqlite::SqliteRow) -> Device {
    Device {
        id: row.get("id"),
        device_uuid: row.get("device_uuid"),
        name: row.get("name"),
        team_number: row.get("team_number"),
        last_seen_at: row
            .get::<Option<String>, _>("last_seen_at")
            .as_deref()
            .and_then(from_sql),
    }
}

// sqlx 0.9 requires query strings to be &'static str -- dynamically assembled
// SQL is rejected at compile time. Columns are therefore written out literally
// at each call site rather than interpolated from a constant. Slightly more
// typing, no possibility of an injected fragment.

impl SqliteRepo {
    pub(crate) async fn create_user_impl(
        &self,
        new_user: NewUser,
        now: DateTime<Utc>,
    ) -> Result<User> {
        let ts = to_sql(now);
        let result = sqlx::query(
            "INSERT INTO users (email, name, password_hash, team_number, \
                                is_admin, is_lead_scout, is_coach, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
        )
        .bind(&new_user.email)
        .bind(&new_user.name)
        .bind(&new_user.password_hash)
        .bind(new_user.team_number)
        .bind(new_user.roles.is_admin as i64)
        .bind(new_user.roles.is_lead_scout as i64)
        .bind(new_user.roles.is_coach as i64)
        .bind(&ts)
        .bind(&ts)
        .fetch_one(&self.pool)
        .await;

        let row = match result {
            Ok(row) => row,
            Err(e) if is_unique_violation(&e) => {
                return Err(RepoError::Conflict {
                    what: "An account with that email",
                });
            }
            Err(e) => return Err(query_err("creating user", e)),
        };

        Ok(User {
            id: row.get("id"),
            email: new_user.email,
            name: new_user.name,
            team_number: new_user.team_number,
            roles: new_user.roles,
        })
    }

    pub(crate) async fn credentials_by_email_impl(
        &self,
        email: &str,
    ) -> Result<Option<Credentials>> {
        // lower() on both sides so this uses the unique index and matches the
        // constraint exactly.
        let row = sqlx::query(
            "SELECT id, email, name, team_number, is_admin, is_lead_scout, is_coach, \
                    password_hash \
             FROM users WHERE lower(email) = lower(?)",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("looking up credentials", e))?;

        Ok(row.map(|row| Credentials {
            user: user_from_row(&row),
            password_hash: row.get("password_hash"),
        }))
    }

    pub(crate) async fn user_by_id_impl(&self, id: i64) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, email, name, team_number, is_admin, is_lead_scout, is_coach \
             FROM users WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading user", e))?;
        Ok(row.as_ref().map(user_from_row))
    }

    pub(crate) async fn password_hash_impl(&self, user_id: i64) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| query_err("loading password hash", e))
    }

    pub(crate) async fn set_password_hash_impl(
        &self,
        user_id: i64,
        hash: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(hash)
            .bind(to_sql(now))
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("updating password", e))?;
        Ok(())
    }

    pub(crate) async fn record_login_impl(&self, user_id: i64, now: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
            .bind(to_sql(now))
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("recording login", e))?;
        Ok(())
    }

    pub(crate) async fn has_any_user_impl(&self) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| query_err("counting users", e))?;
        Ok(count > 0)
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    pub(crate) async fn create_session_impl(
        &self,
        session: &Session,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&session.id)
        .bind(session.user_id)
        .bind(to_sql(session.expires_at))
        .bind(to_sql(now))
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("creating session", e))?;
        Ok(())
    }

    pub(crate) async fn session_user_impl(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Session, User)>> {
        let row = sqlx::query(
            "SELECT s.id AS session_id, s.user_id, s.expires_at, \
                    u.id, u.email, u.name, u.team_number, \
                    u.is_admin, u.is_lead_scout, u.is_coach \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading session", e))?;

        let Some(row) = row else { return Ok(None) };

        let expires_at = row
            .get::<String, _>("expires_at")
            .as_str()
            .pipe(from_sql)
            .ok_or_else(|| RepoError::Query("session has an unparsable expiry".into()))?;

        let session = Session {
            id: row.get("session_id"),
            user_id: row.get("user_id"),
            expires_at,
        };

        // Expiry cleans up as a side effect of normal traffic, so there is no
        // sweeper task to forget to start.
        if session.is_expired(now) {
            self.delete_session_impl(session_id).await?;
            return Ok(None);
        }

        Ok(Some((session, user_from_row(&row))))
    }

    pub(crate) async fn delete_session_impl(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("deleting session", e))?;
        Ok(())
    }

    pub(crate) async fn purge_expired_sessions_impl(&self, now: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(to_sql(now))
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("purging sessions", e))?;
        Ok(result.rows_affected())
    }

    // ── Devices ─────────────────────────────────────────────────────────────

    pub(crate) async fn touch_device_impl(
        &self,
        device_uuid: &str,
        team_number: Option<i32>,
        now: DateTime<Utc>,
    ) -> Result<Device> {
        let ts = to_sql(now);
        // NOTE: no `--` comments inside this string. Rust's backslash line
        // continuation removes the newline, so a SQL line comment would swallow
        // the rest of the statement -- silently, because it stays valid SQL.
        //
        // The COALESCE on team_number is what makes a borrowed tablet keep the
        // team it was first seen with, rather than being relabelled by whoever
        // picks it up next.
        let row = sqlx::query(
            "INSERT INTO devices (device_uuid, team_number, last_seen_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (device_uuid) DO UPDATE SET \
                last_seen_at = excluded.last_seen_at, \
                team_number  = COALESCE(devices.team_number, excluded.team_number), \
                updated_at   = excluded.updated_at \
             RETURNING id, device_uuid, name, team_number, last_seen_at",
        )
        .bind(device_uuid)
        .bind(team_number)
        .bind(&ts)
        .bind(&ts)
        .bind(&ts)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| query_err("recording device heartbeat", e))?;

        Ok(device_from_row(&row))
    }

    pub(crate) async fn device_by_uuid_impl(&self, device_uuid: &str) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT id, device_uuid, name, team_number, last_seen_at FROM devices \
             WHERE device_uuid = ?",
        )
        .bind(device_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading device", e))?;
        Ok(row.as_ref().map(device_from_row))
    }

    pub(crate) async fn list_devices_impl(&self) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            "SELECT id, device_uuid, name, team_number, last_seen_at FROM devices \
             ORDER BY last_seen_at DESC NULLS LAST, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing devices", e))?;
        Ok(rows.iter().map(device_from_row).collect())
    }

    pub(crate) async fn rename_device_impl(
        &self,
        id: i64,
        name: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query("UPDATE devices SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name.trim())
            .bind(to_sql(now))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| query_err("renaming device", e))?;
        Ok(())
    }
}

/// Small helper so `.pipe(from_sql)` reads left-to-right above.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl Pipe for &str {}
