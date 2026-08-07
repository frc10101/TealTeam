// Cookie + database session handling, compatible with the Go/.NET apps'
// sessions table and session_id cookie (port of Services/SessionService.cs).

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use chrono::{Duration, Utc};
use rand::RngCore;
use sqlx::PgPool;

use crate::models::{Session, User};

pub const COOKIE_NAME: &str = "session_id";
pub const DEVICE_COOKIE_NAME: &str = "device_uuid";
const DURATION_HOURS: i64 = 24;
const BCRYPT_COST: u32 = 12;

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(bcrypt::hash(password, BCRYPT_COST)?)
}

pub fn check_password_hash(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

pub fn generate_session_id() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}

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

pub async fn get_session_user(pool: &PgPool, jar: &CookieJar) -> Option<User> {
    let session = get_session(pool, jar).await?;
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(session.user_id)
        .fetch_optional(pool)
        .await
        .ok()?
}

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

pub fn session_cookie(session_id: String) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, session_id))
        .path("/")
        .max_age(time_duration())
        .http_only(true)
        .secure(false) // Plain HTTP on the LAN, same as the Go app.
        .same_site(SameSite::Lax)
        .build()
}

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

pub async fn delete_session(pool: &PgPool, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE session_id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}
