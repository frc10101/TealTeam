//! Offline auth tokens on the Pi (C9): issued to a signed-in device, taken
//! by the sync in place of a session.
//!
//! `POST /api/auth/token` gives the signed-in user a token for the device
//! whose cookie came with the request (`tt_core::token` says what is in it).
//! `static/js/token.js` keeps it on the device.
//!
//! [`sync_user`] is who a pull or push is from: the session's user, or, with
//! no live session, the user an `Authorization: Bearer` token names. That is
//! the scout who saved all Saturday with no signal and whose 24-hour session
//! ran out overnight: the outbox still goes as them. The token must verify,
//! must name this device, and its user must still exist. Pages still need a
//! session; a token signs no one in. [`verify`] checks one without a device,
//! for a handoff carried by QR (S13).

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use serde_json::json;
use tokio::sync::OnceCell;
use tracing::{info, warn};
use tt_core::token::{self, Claims};
use tt_core::user::User;
use tt_repo::{Repo, RepoError};

use crate::auth::{Auth, current_user, device_uuid};
use crate::startup::AppState;

/// The signing seed, read from the database once.
#[derive(Default)]
pub struct Keys(OnceCell<[u8; 32]>);

async fn seed(state: &AppState) -> Result<[u8; 32], RepoError> {
    state
        .tokens
        .0
        .get_or_try_init(|| async {
            let mut fresh = [0u8; 32];
            rand::rng().fill_bytes(&mut fresh);
            state.repo.token_seed(fresh, Utc::now()).await
        })
        .await
        .copied()
}

#[derive(Serialize)]
struct Issued {
    token: String,
    expires_at: DateTime<Utc>,
    /// The Pi's Ed25519 public key, unpadded base64url, for a device that
    /// wants to verify a token itself.
    public_key: String,
}

/// `POST /api/auth/token`: a token for the signed-in user on this device.
pub async fn issue(
    State(state): State<AppState>,
    Auth(user): Auth,
    headers: HeaderMap,
) -> Response {
    let Some(device) = device_uuid(&headers) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "this browser has no device id; reload the page" })),
        )
            .into_response();
    };
    let seed = match seed(&state).await {
        Ok(seed) => seed,
        Err(e) => {
            warn!("token key: {e}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "storage unavailable" })),
            )
                .into_response();
        }
    };
    let claims = Claims::new(&user, &device, Utc::now());
    let issued = Issued {
        token: token::sign(&seed, &claims),
        expires_at: claims.expires_at,
        public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(token::public_key(&seed)),
    };
    ([(header::CACHE_CONTROL, "no-store")], Json(issued)).into_response()
}

/// Who a sync request is from: the session's user, else a token's.
pub async fn sync_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    if let Some(user) = current_user(state, headers).await {
        return Some(user);
    }
    let bearer = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    let device = device_uuid(headers)?;
    let claims = match verify(state, bearer).await {
        Ok(claims) => claims,
        Err(e) => {
            info!(%device, "refused a token: {e}");
            return None;
        }
    };
    if claims.device != device {
        info!(%device, issued_to = %claims.device, "refused a token from another device");
        return None;
    }
    // Roles and team as they are now, and nobody for a deleted account.
    state
        .repo
        .user_by_id(claims.user_id)
        .await
        .inspect_err(|e| warn!("loading a token's user: {e}"))
        .ok()?
}

/// What a token says, if the Pi signed it and it has not expired. Not tied
/// to any device: a handoff (S13, `crate::handoff`) brings a scout's token
/// from their tablet on a lead's screen.
pub async fn verify(state: &AppState, token: &str) -> Result<Claims, String> {
    let seed = seed(state).await.map_err(|e| {
        warn!("token key: {e}");
        "storage unavailable".to_string()
    })?;
    token::verify(&token::public_key(&seed), token, Utc::now()).map_err(|e| e.to_string())
}
