//! Sessions: the entity plus cookie and database session handling.
//!
//! A port of `Services/SessionService.cs`, compatible with the Go and .NET
//! apps' `sessions` table and `session_id` cookie: sign in against one
//! implementation and the others accept the same cookie.
//!
//! Sessions are server-side rows keyed by a 256-bit random id; the cookie
//! carries nothing but that id. They last [`DURATION_HOURS`] and are deleted
//! lazily when a request presents an expired one. The session row also holds
//! the user's currently selected event, which is why so many controllers read
//! it directly rather than just the user.
//!
//! Scouting tablets additionally carry a permanent `device_uuid` cookie (set
//! by `static/js/device.js`) so a lead scout can assign robots to a device
//! that nobody is signed in on — see [`device_uuid`] and
//! [`crate::models::device`].

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sqlx::{FromRow, PgPool};

use super::user::{self, User};

/// Session cookie name, shared with the Go and .NET ports.
pub const COOKIE_NAME: &str = "session_id";
/// Permanent per-tablet identifier cookie, set client-side by
/// `static/js/device.js`.
pub const DEVICE_COOKIE_NAME: &str = "device_uuid";
/// How long a session stays valid, for both the cookie and the database row.
pub const DURATION_HOURS: i64 = 24;

/// A `sessions` row.
#[derive(Debug, Clone, FromRow)]
pub struct Session {
    /// Random id; also the cookie value.
    pub session_id: String,
    pub user_id: i32,
    /// The event this user is currently scouting. Most pages are scoped to it,
    /// and it is stored per session rather than per user so one account can
    /// work two events from two devices.
    pub selected_event_id: Option<i32>,
    pub expires_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
}

/// 32 bytes from the OS RNG, URL-safe base64 encoded.
pub fn generate_session_id() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

/// Resolves the session cookie to a live session, deleting it if it has
/// expired. `None` covers every failure: no cookie, unknown id, expired row,
/// or an unreachable database.
pub async fn get_session(pool: &PgPool, jar: &CookieJar) -> Option<Session> {
    let session_id = jar.get(COOKIE_NAME)?.value().to_string();
    if session_id.is_empty() {
        return None;
    }

    let session: Session = sqlx::query_as(
        "SELECT session_id, user_id, selected_event_id, expires_at, created_at
         FROM sessions WHERE session_id = $1",
    )
    .bind(&session_id)
    .fetch_optional(pool)
    .await
    .ok()??;

    if Utc::now() > session.expires_at {
        let _ = sqlx::query("DELETE FROM sessions WHERE session_id = $1")
            .bind(&session_id)
            .execute(pool)
            .await;
        return None;
    }

    Some(session)
}

/// The user behind the session cookie. See [`crate::web::current_user`].
pub async fn get_session_user(pool: &PgPool, jar: &CookieJar) -> Option<User> {
    let session = get_session(pool, jar).await?;
    user::find_by_id(pool, session.user_id).await
}

/// Creates a session row and returns its id for [`session_cookie`].
pub async fn create_session(pool: &PgPool, user_id: i32) -> anyhow::Result<String> {
    let session_id = generate_session_id();
    let expires_at = Utc::now() + Duration::hours(DURATION_HOURS);

    sqlx::query("INSERT INTO sessions (session_id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(&session_id)
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(session_id)
}

/// Deletes a session (sign-out). Deleting an unknown id is not an error.
pub async fn delete_session(pool: &PgPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Persists the event a user is currently scouting on their session row.
pub async fn set_selected_event(
    pool: &PgPool,
    session_id: &str,
    event_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET selected_event_id = $1 WHERE session_id = $2")
        .bind(event_id)
        .bind(session_id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// The device UUID cookie planted by static/js/device.js, if present.
pub fn device_uuid(jar: &CookieJar) -> Option<String> {
    jar.get(DEVICE_COOKIE_NAME)
        .map(|c| c.value().trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The `session_id` cookie. Not `Secure`: event LANs serve plain HTTP, same
/// as the Go app.
pub fn session_cookie(session_id: String) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, session_id))
        .path("/")
        .max_age(time_duration())
        .http_only(true)
        .secure(false) // Plain HTTP on the LAN, same as the Go app.
        .same_site(SameSite::Lax)
        .build()
}

/// An immediately-expiring `session_id` cookie, to clear it on sign-out.
pub fn clear_session_cookie() -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Lax)
        .build()
}

fn time_duration() -> time::Duration {
    time::Duration::hours(DURATION_HOURS)
}
