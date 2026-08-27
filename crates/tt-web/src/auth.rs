//! Password hashing, session cookies, and role guards (A1-A4).
//!
//! # Argon2id, not bcrypt
//!
//! The retired implementation used bcrypt cost 12, chosen so hashes were
//! interchangeable between its Go, C#, and Rust ports. There is one
//! implementation now, so that constraint is gone and Argon2id -- memory-hard,
//! and the current password-hashing recommendation -- is the better default.
//!
//! # Guards
//!
//! Every guarded handler in the retired app re-checked `if !user.is_admin &&
//! !user.is_lead_scout` inline. That is repetitive, and it is how its database
//! viewer ended up shipping with no check at all (REBUILD_SPEC.md 12.4).
//!
//! Here the check is a type. A handler that takes [`LeadScout`] cannot be
//! reached by anyone else, because the extractor refuses to build. Forgetting a
//! guard is visible in the signature rather than absent from the body.

use argon2::password_hash::{PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use tt_core::user::{SESSION_DURATION, Session, User};
use tt_repo::Repo;

use crate::startup::AppState;

pub const SESSION_COOKIE: &str = "tt_session";
pub const DEVICE_COOKIE: &str = "tt_device";

// ── Password hashing ────────────────────────────────────────────────────────

/// Number of random bytes in a salt. 16 is the Argon2 recommendation.
const SALT_BYTES: usize = 16;

/// Hash a password for storage.
///
/// The salt comes from the workspace's `rand` rather than argon2's re-exported
/// `rand_core`: argon2 0.5 vendors rand_core 0.6 while the workspace is on rand
/// 0.9, and generating the bytes here means the two versions never have to
/// agree on a trait.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let mut salt_bytes = [0u8; SALT_BYTES];
    rand::rng().fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes)?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Check a password against a stored hash.
///
/// Returns `false` for a malformed hash rather than erroring: a corrupt row must
/// deny access, not crash the login route.
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

// ── Session identifiers ─────────────────────────────────────────────────────

/// 256 bits from the OS CSPRNG, URL-safe base64.
pub fn generate_session_id() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Build a session for `user_id`, expiring [`SESSION_DURATION`] from now.
pub fn new_session(user_id: i64) -> Session {
    Session {
        id: generate_session_id(),
        user_id,
        expires_at: Utc::now() + SESSION_DURATION,
    }
}

/// The session cookie.
///
/// `Secure` is deliberately **off**: the event LAN is plain HTTP with no TLS
/// terminator on the Pi, and a Secure cookie would simply never be sent. The
/// threat model is a gymnasium LAN reachable only by cable.
pub fn session_cookie(id: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, id))
        .path("/")
        .max_age(time::Duration::seconds(SESSION_DURATION.num_seconds()))
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Lax)
        .build()
}

pub fn clear_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Lax)
        .build()
}

// ── Extractors ──────────────────────────────────────────────────────────────

/// The signed-in user, or a redirect to sign-in.
pub struct Auth(pub User);

/// The signed-in user, if there is one. Never rejects.
///
/// For pages that render differently but are readable either way.
pub struct MaybeAuth(pub Option<User>);

/// A user who may approve submissions, assign robots, and tune weights.
pub struct LeadScout(pub User);

/// A user who may see the coach panel.
pub struct Coach(pub User);

/// What a failed guard does.
///
/// Anonymous visitors are sent to sign in. Signed-in users who simply lack the
/// role are sent home rather than shown a 403 -- being told a page exists that
/// you cannot open is a worse experience than it simply not being in your nav.
pub struct AuthRedirect(&'static str);

impl IntoResponse for AuthRedirect {
    fn into_response(self) -> Response {
        Redirect::to(self.0).into_response()
    }
}

async fn current_user(state: &AppState, parts: &Parts) -> Option<User> {
    let jar = CookieJar::from_headers(&parts.headers);
    let session_id = jar.get(SESSION_COOKIE)?.value().to_string();
    if session_id.is_empty() {
        return None;
    }

    match state.repo.session_user(&session_id, Utc::now()).await {
        Ok(Some((_, user))) => Some(user),
        Ok(None) => None,
        Err(e) => {
            // Storage down means nobody is authenticated, which is correct: the
            // degraded app serves public pages only.
            tracing::warn!("resolving session: {e}");
            None
        }
    }
}

/// The device UUID this browser reports, if any.
pub fn device_uuid(parts: &Parts) -> Option<String> {
    let jar = CookieJar::from_headers(&parts.headers);
    let raw = jar.get(DEVICE_COOKIE)?.value().trim().to_string();
    // Bound the length: this value is written by the client and lands in the
    // database.
    (8..=64).contains(&raw.len()).then_some(raw)
}

impl FromRequestParts<AppState> for MaybeAuth {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeAuth(current_user(state, parts).await))
    }
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = AuthRedirect;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        current_user(state, parts)
            .await
            .map(Auth)
            .ok_or(AuthRedirect("/sign-in"))
    }
}

/// Generates the role guards, which differ only in which predicate they call.
macro_rules! role_guard {
    ($guard:ident, $check:ident) => {
        impl FromRequestParts<AppState> for $guard {
            type Rejection = AuthRedirect;

            async fn from_request_parts(
                parts: &mut Parts,
                state: &AppState,
            ) -> Result<Self, Self::Rejection> {
                match current_user(state, parts).await {
                    Some(user) if user.roles.$check() => Ok($guard(user)),
                    // Signed in but unauthorised: home, not a 403.
                    Some(_) => Err(AuthRedirect("/")),
                    None => Err(AuthRedirect("/sign-in")),
                }
            }
        }
    };
}

role_guard!(LeadScout, can_lead);
role_guard!(Coach, can_coach);

// NOTE: an `Admin` guard belongs here too, but it would be dead code until the
// database viewer lands (U17) -- and that viewer shipping *unguarded* is exactly
// the defect this module exists to prevent. Add it with the route.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hashed_password_verifies() {
        let hash = hash_password("correct horse battery").expect("hash");
        assert!(verify_password("correct horse battery", &hash));
    }

    #[test]
    fn a_wrong_password_does_not_verify() {
        let hash = hash_password("correct horse battery").expect("hash");
        assert!(!verify_password("Correct horse battery", &hash));
        assert!(!verify_password("", &hash));
    }

    #[test]
    fn hashing_is_salted_so_equal_passwords_differ() {
        let a = hash_password("same password").expect("hash");
        let b = hash_password("same password").expect("hash");
        assert_ne!(
            a, b,
            "identical passwords must not produce identical hashes"
        );
        assert!(verify_password("same password", &a));
        assert!(verify_password("same password", &b));
    }

    #[test]
    fn hashes_are_argon2id() {
        assert!(
            hash_password("x12345678")
                .expect("hash")
                .starts_with("$argon2id$")
        );
    }

    #[test]
    fn a_corrupt_hash_denies_access_rather_than_panicking() {
        for bad in [
            "",
            "not-a-hash",
            "$argon2id$garbage",
            "$2b$12$oldbcrypthash",
        ] {
            assert!(!verify_password("anything", bad), "{bad:?} must not verify");
        }
    }

    #[test]
    fn session_ids_are_unpredictable_and_url_safe() {
        let a = generate_session_id();
        let b = generate_session_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43, "256 bits, unpadded base64");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn a_new_session_expires_a_day_out() {
        let session = new_session(7);
        assert_eq!(session.user_id, 7);
        let life = session.expires_at - Utc::now();
        assert!(life > SESSION_DURATION - chrono::TimeDelta::minutes(1));
        assert!(life <= SESSION_DURATION);
    }

    #[test]
    fn the_session_cookie_is_http_only_and_lax() {
        let cookie = session_cookie("abc".into());
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        // Plain HTTP on the event LAN; a Secure cookie would never be sent.
        assert_eq!(cookie.secure(), Some(false));
    }

    #[test]
    fn clearing_the_cookie_expires_it_immediately() {
        let cookie = clear_session_cookie();
        assert_eq!(cookie.value(), "");
        assert_eq!(cookie.max_age(), Some(time::Duration::ZERO));
    }
}
