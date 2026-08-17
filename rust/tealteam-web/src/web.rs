//! HTTP plumbing shared by every controller: the error type, form decoding,
//! Unpoly response helpers, and the current-user lookups.
//!
//! # Unpoly
//!
//! The UI is server-rendered HTML progressively enhanced by
//! [Unpoly](https://unpoly.com), vendored under `static/js/`. A controller
//! generally serves a full page, and re-serves a fragment of it when the same
//! route is hit by an Unpoly request — [`is_unpoly`] is the test. Two response
//! shapes need help beyond plain HTML:
//!
//! - [`up_navigate`] asks the browser to navigate (the analog of htmx's
//!   `HX-Redirect`), used after login, logout and submission review.
//! - [`redirect`] is an ordinary redirect for non-Unpoly navigation.
//!
//! # Forms
//!
//! Bodies arrive as `String` and are decoded here rather than through axum's
//! `Form` extractor, because several endpoints accept repeated keys or
//! partially-filled forms that must round-trip back into a re-rendered
//! fragment instead of failing extraction with a 4xx the user cannot see.

use std::collections::HashMap;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;

use crate::models::session;
use crate::models::{Session, User};

// ── Error type ────────────────────────────────────────────────────────────

/// Catch-all controller error: logs, and renders as a 500.
///
/// Controllers return this only for genuinely unexpected failures. Expected
/// problems — a missing event, an unreachable FIRST API, a wrong password —
/// are part of the page and get rendered into the view instead.
pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("handler error: {:#}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("internal error: {}", self.0)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

/// Return type of every controller action.
pub type HandlerResult = Result<Response, AppError>;

// ── Responses ─────────────────────────────────────────────────────────────

/// True when the request came from Unpoly (any `up-follow`/`up-submit`/
/// `up.render`), meaning the caller wants a fragment rather than a page.
pub fn is_unpoly(headers: &HeaderMap) -> bool {
    headers.get("X-Up-Version").is_some()
}

/// Ordinary browser redirect, used for full-page navigation.
pub fn redirect(to: &str) -> Response {
    Redirect::to(to).into_response()
}

/// Server-driven full navigation — the Unpoly analog of htmx's HX-Redirect.
/// Emits a `tt:navigate` event via X-Up-Events (tt-unpoly.js turns it into
/// window.location) and skips fragment rendering with X-Up-Target: :none.
pub fn up_navigate(to: &str) -> Response {
    let mut response = ().into_response();
    let events = format!(r#"[{{"type":"tt:navigate","url":"{to}"}}]"#);
    if let Ok(value) = events.parse() {
        response.headers_mut().insert("X-Up-Events", value);
    }
    response
        .headers_mut()
        .insert("X-Up-Target", ":none".parse().unwrap());
    response
}

// ── Form parsing ──────────────────────────────────────────────────────────

/// Decodes a URL-encoded body into a key/value map, empty on malformed input.
pub fn form_map(body: &str) -> HashMap<String, String> {
    serde_urlencoded::from_str(body).unwrap_or_default()
}

/// Decodes a URL-encoded body preserving repeated keys, for checkbox groups
/// such as the assignment auto-distribute pool.
pub fn form_multi(body: &str) -> Vec<(String, String)> {
    serde_urlencoded::from_str(body).unwrap_or_default()
}

/// Field value, or the empty string when the field was not submitted.
pub fn form_str<'a>(form: &'a HashMap<String, String>, key: &str) -> &'a str {
    form.get(key).map(|s| s.as_str()).unwrap_or("")
}

/// Parses a form field as an integer; `None` when blank or malformed.
pub fn parse_required_int(value: &str) -> Option<i32> {
    value.trim().parse().ok()
}

/// Converts a TBA event key into a FIRST event code.
///
/// TBA keys are `{year}{event_code}`, e.g. `2026mndu` -> `mndu`; the FIRST API
/// expects the lowercase code alone. Returns an empty string if the key is not
/// in that shape, which callers treat as "this event cannot be looked up".
pub fn extract_event_code(tba_key: &str) -> String {
    let mut chars = tba_key.chars();
    let prefix: String = chars.by_ref().take(4).collect();
    if prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) {
        let rest: String = chars.collect();
        if !rest.is_empty() {
            return rest.to_lowercase();
        }
    }
    String::new()
}

// ── Session convenience wrappers ──────────────────────────────────────────

/// The signed-in user for this request, or `None` when the session cookie is
/// missing, expired, or the database is unreachable.
pub async fn current_user(pool: &PgPool, jar: &CookieJar) -> Option<User> {
    session::get_session_user(pool, jar).await
}

/// The session row itself, needed when a controller has to write to it (event
/// selection) rather than just identify the user.
pub async fn current_session(pool: &PgPool, jar: &CookieJar) -> Option<Session> {
    session::get_session(pool, jar).await
}

/// The event the signed-in user selected on the home page, if any.
pub async fn selected_event_id(pool: &PgPool, jar: &CookieJar) -> Option<i32> {
    current_session(pool, jar).await.and_then(|s| s.selected_event_id)
}
