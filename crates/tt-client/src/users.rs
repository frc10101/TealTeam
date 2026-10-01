//! Users, sessions, and devices: `tt_repo_sqlite::users`, on the device.
//!
//! A snapshot carries no rows of these tables (S10): they never leave the
//! server. They are here so that the browser's repo is the whole trait, and
//! the same SQL answers the same way on either side (C11).

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use tt_core::user::{Roles, Session, User};
use tt_repo::{Credentials, Device, NewUser, RepoError, Result, Scout};

use crate::ClientRepo;
use crate::sql::{Context, is_unique_violation, query_err, to_sql, ts_column};

fn user_from_row(row: &Row) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get("id")?,
        email: row.get("email")?,
        name: row.get("name")?,
        team_number: row.get("team_number")?,
        roles: Roles {
            is_admin: row.get::<_, i64>("is_admin")? != 0,
            is_lead_scout: row.get::<_, i64>("is_lead_scout")? != 0,
            is_coach: row.get::<_, i64>("is_coach")? != 0,
        },
    })
}

fn device_from_row(row: &Row) -> rusqlite::Result<Device> {
    Ok(Device {
        id: row.get("id")?,
        device_uuid: row.get("device_uuid")?,
        name: row.get("name")?,
        team_number: row.get("team_number")?,
        last_seen_at: ts_column(row.get("last_seen_at")?),
        last_user_id: row.get("last_user_id")?,
        clock_offset_ms: row.get("clock_offset_ms")?,
    })
}

impl ClientRepo {
    pub(crate) fn create_user_impl(&self, new_user: NewUser, now: DateTime<Utc>) -> Result<User> {
        let ts = to_sql(now);
        let id = self.conn.query_row(
            "INSERT INTO users (email, name, password_hash, team_number, \
                                is_admin, is_lead_scout, is_coach, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING id",
            params![
                new_user.email,
                new_user.name,
                new_user.password_hash,
                new_user.team_number,
                new_user.roles.is_admin as i64,
                new_user.roles.is_lead_scout as i64,
                new_user.roles.is_coach as i64,
                ts,
                ts,
            ],
            |row| row.get("id"),
        );
        let id = match id {
            Ok(id) => id,
            Err(e) if is_unique_violation(&e) => {
                return Err(RepoError::Conflict {
                    what: "An account with that email",
                });
            }
            Err(e) => return Err(query_err("creating user", e)),
        };
        Ok(User {
            id,
            email: new_user.email,
            name: new_user.name,
            team_number: new_user.team_number,
            roles: new_user.roles,
        })
    }

    pub(crate) fn credentials_by_email_impl(&self, email: &str) -> Result<Option<Credentials>> {
        self.conn
            .query_row(
                "SELECT id, email, name, team_number, is_admin, is_lead_scout, is_coach, \
                        password_hash \
                 FROM users WHERE lower(email) = lower(?)",
                [email],
                |row| {
                    Ok(Credentials {
                        user: user_from_row(row)?,
                        password_hash: row.get("password_hash")?,
                    })
                },
            )
            .optional()
            .ctx("looking up credentials")
    }

    pub(crate) fn user_by_id_impl(&self, id: i64) -> Result<Option<User>> {
        self.conn
            .query_row(
                "SELECT id, email, name, team_number, is_admin, is_lead_scout, is_coach \
                 FROM users WHERE id = ?",
                [id],
                user_from_row,
            )
            .optional()
            .ctx("loading user")
    }

    pub(crate) fn password_hash_impl(&self, user_id: i64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT password_hash FROM users WHERE id = ?",
                [user_id],
                |row| row.get(0),
            )
            .optional()
            .ctx("loading password hash")
    }

    pub(crate) fn set_password_hash_impl(
        &self,
        user_id: i64,
        hash: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
                params![hash, to_sql(now), user_id],
            )
            .ctx("updating password")?;
        Ok(())
    }

    pub(crate) fn record_login_impl(&self, user_id: i64, now: DateTime<Utc>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE users SET last_login_at = ? WHERE id = ?",
                params![to_sql(now), user_id],
            )
            .ctx("recording login")?;
        Ok(())
    }

    pub(crate) fn has_any_user_impl(&self) -> Result<bool> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
            .ctx("counting users")?;
        Ok(count > 0)
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    pub(crate) fn create_session_impl(&self, session: &Session, now: DateTime<Utc>) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO sessions (id, user_id, expires_at, created_at) VALUES (?, ?, ?, ?)",
                params![
                    session.id,
                    session.user_id,
                    to_sql(session.expires_at),
                    to_sql(now)
                ],
            )
            .ctx("creating session")?;
        Ok(())
    }

    pub(crate) fn session_user_impl(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Session, User)>> {
        let found = self
            .conn
            .query_row(
                "SELECT s.id AS session_id, s.user_id, s.expires_at, \
                        u.id, u.email, u.name, u.team_number, \
                        u.is_admin, u.is_lead_scout, u.is_coach \
                 FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.id = ?",
                [session_id],
                |row| {
                    Ok((
                        row.get::<_, String>("session_id")?,
                        row.get::<_, i64>("user_id")?,
                        row.get::<_, String>("expires_at")?,
                        user_from_row(row)?,
                    ))
                },
            )
            .optional()
            .ctx("loading session")?;
        let Some((id, user_id, expires_at, user)) = found else {
            return Ok(None);
        };
        let expires_at = crate::sql::from_sql(&expires_at)
            .ok_or_else(|| RepoError::Query("session has an unparsable expiry".into()))?;
        let session = Session {
            id,
            user_id,
            expires_at,
        };
        if session.is_expired(now) {
            self.delete_session_impl(session_id)?;
            return Ok(None);
        }
        Ok(Some((session, user)))
    }

    pub(crate) fn delete_session_impl(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?", [session_id])
            .ctx("deleting session")?;
        Ok(())
    }

    pub(crate) fn purge_expired_sessions_impl(&self, now: DateTime<Utc>) -> Result<u64> {
        let gone = self
            .conn
            .execute("DELETE FROM sessions WHERE expires_at <= ?", [to_sql(now)])
            .ctx("purging sessions")?;
        Ok(gone as u64)
    }

    // ── Devices ─────────────────────────────────────────────────────────────

    pub(crate) fn record_clock_offset_impl(
        &self,
        device_uuid: &str,
        offset_ms: i64,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE devices SET clock_offset_ms = ?, clock_checked_at = ? \
                 WHERE device_uuid = ?",
                params![offset_ms, to_sql(now), device_uuid],
            )
            .ctx("recording a device's clock")?;
        Ok(())
    }

    pub(crate) fn touch_device_impl(
        &self,
        device_uuid: &str,
        user: Option<&User>,
        now: DateTime<Utc>,
    ) -> Result<Device> {
        let ts = to_sql(now);
        let tx = self
            .conn
            .unchecked_transaction()
            .ctx("starting device heartbeat")?;
        if let Some(user) = user {
            tx.execute(
                "UPDATE users SET last_seen_at = ? WHERE id = ?",
                params![ts, user.id],
            )
            .ctx("recording user heartbeat")?;
        }
        // The server's statement, COALESCE and all; see its comments.
        let device = tx
            .query_row(
                "INSERT INTO devices \
                     (device_uuid, team_number, last_seen_at, last_user_id, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (device_uuid) DO UPDATE SET \
                    last_seen_at = excluded.last_seen_at, \
                    last_user_id = excluded.last_user_id, \
                    team_number  = COALESCE(devices.team_number, excluded.team_number), \
                    updated_at   = excluded.updated_at \
                 RETURNING id, device_uuid, name, team_number, last_seen_at, last_user_id, \
                           clock_offset_ms",
                params![
                    device_uuid,
                    user.and_then(|u| u.team_number),
                    ts,
                    user.map(|u| u.id),
                    ts,
                    ts,
                ],
                device_from_row,
            )
            .ctx("recording device heartbeat")?;
        tx.commit().ctx("committing device heartbeat")?;
        Ok(device)
    }

    pub(crate) fn device_by_uuid_impl(&self, device_uuid: &str) -> Result<Option<Device>> {
        self.conn
            .query_row(
                "SELECT id, device_uuid, name, team_number, last_seen_at, last_user_id, \
                        clock_offset_ms \
                 FROM devices WHERE device_uuid = ?",
                [device_uuid],
                device_from_row,
            )
            .optional()
            .ctx("loading device")
    }

    pub(crate) fn list_devices_impl(&self) -> Result<Vec<Device>> {
        self.all(
            "SELECT id, device_uuid, name, team_number, last_seen_at, last_user_id, \
                    clock_offset_ms \
             FROM devices ORDER BY last_seen_at DESC NULLS LAST, id",
            [],
            device_from_row,
            "listing devices",
        )
    }

    pub(crate) fn rename_device_impl(&self, id: i64, name: &str, now: DateTime<Utc>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE devices SET name = ?, updated_at = ? WHERE id = ?",
                params![name.trim(), to_sql(now), id],
            )
            .ctx("renaming device")?;
        Ok(())
    }

    pub(crate) fn list_scouts_impl(&self) -> Result<Vec<Scout>> {
        self.all(
            "SELECT id, name, team_number, last_seen_at FROM users \
             ORDER BY name COLLATE NOCASE, id",
            [],
            |row| {
                Ok(Scout {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    team_number: row.get("team_number")?,
                    last_seen_at: ts_column(row.get("last_seen_at")?),
                })
            },
            "listing scouts",
        )
    }
}
