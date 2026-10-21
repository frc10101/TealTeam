//! Offline auth tokens on the Pi (C9): issued to a signed-in device, taken
//! by the sync in place of a session.
//!
//! `POST /api/auth/token` gives the signed-in user a token for the device
//! whose cookie came with the request, and the device key that goes with it
//! (`tt_core::token` says what is in each). `static/js/token.js` keeps both
//! on the device.
//!
//! [`sync_user`] is who a pull or push is from: the session's user, or, with
//! no live session, the user an `Authorization: Bearer` credential (token
//! `~` key) names. That is the scout who saved all Saturday with no signal
//! and whose 24-hour session ran out overnight: the outbox still goes as
//! them. The token must verify, the key must be its own, it must name this
//! device and the user's current token epoch, and its user must still
//! exist. Sent with no device cookie at all (the downloadable scouting
//! page, C15), it is taken as from the tablet it names. Pages still need a
//! session; a token signs no one in.
//! [`holder_key`] checks who a QR handoff (S13) names, which carries no
//! token at all.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use serde_json::json;
use tokio::sync::OnceCell;
use tracing::{info, warn};
use tt_core::token::{self, Claims, Holder};
use tt_core::user::User;
use tt_repo::{Repo, RepoError};

use crate::auth::{Auth, current_user, device_uuid};
use crate::startup::AppState;

/// The signing seed, read from the database once.
#[derive(Default)]
pub struct Keys(OnceCell<[u8; 32]>);

pub(crate) async fn seed(state: &AppState) -> Result<[u8; 32], RepoError> {
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
pub struct Issued {
    pub token: String,
    /// The device key (T1), unpadded base64url: kept beside the token, sent
    /// with it as the credential, and the key of a handoff's and a
    /// receipt's MAC. Never inside the token.
    pub key: String,
    pub expires_at: DateTime<Utc>,
    /// The Pi's Ed25519 public key, unpadded base64url, for a device that
    /// wants to verify a token itself.
    pub public_key: String,
}

/// A token and its key for `user` on `device`: what `POST /api/auth/token`
/// answers, and what the downloadable scouting page (C15) is cut with.
pub async fn issue_to(state: &AppState, user: &User, device: &str) -> Result<Issued, RepoError> {
    let seed = seed(state).await?;
    let epoch = state.repo.token_epoch(user.id).await?;
    let claims = Claims::new(user, device, epoch, Utc::now());
    Ok(Issued {
        token: token::sign(&seed, &claims),
        key: URL_SAFE_NO_PAD.encode(token::device_key(&seed, &Holder::of(&claims))),
        expires_at: claims.expires_at,
        public_key: URL_SAFE_NO_PAD.encode(token::public_key(&seed)),
    })
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
    match issue_to(&state, &user, &device).await {
        Ok(issued) => ([(header::CACHE_CONTROL, "no-store")], Json(issued)).into_response(),
        Err(e) => {
            warn!("issuing a token: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "storage unavailable" })),
            )
                .into_response()
        }
    }
}

/// Who a sync request is from: the session's user, else a credential's.
pub async fn sync_user(state: &AppState, headers: &HeaderMap) -> Option<User> {
    sync_caller(state, headers).await.map(|caller| caller.user)
}

/// Who a sync request is from, and the tablet it is from.
pub struct Caller {
    pub user: User,
    /// The device cookie's id, or, for a credential sent with none, the
    /// tablet its token was issued to.
    pub device: Option<String>,
}

/// [`sync_user`], with the tablet. A credential sent with no device cookie
/// is taken as from the tablet its token names: that is the downloadable
/// scouting page (C15), a file on the tablet whose posts carry no cookie of
/// the Pi's. The cookie never was the secret -- anyone holding the token
/// and key can send it too -- so a missing one costs nothing, and one that
/// names another tablet is still refused.
pub async fn sync_caller(state: &AppState, headers: &HeaderMap) -> Option<Caller> {
    let cookie = device_uuid(headers);
    if let Some(user) = current_user(state, headers).await {
        return Some(Caller {
            user,
            device: cookie,
        });
    }
    let bearer = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    let shown = cookie.as_deref().unwrap_or("-");
    // A token alone, as kept before T1, is no credential.
    let Some((bearer, key)) = token::split_credential(bearer) else {
        info!(device = %shown, "refused a token sent without its key");
        return None;
    };
    let claims = match verify(state, bearer).await {
        Ok(claims) => claims,
        Err(e) => {
            info!(device = %shown, "refused a token: {e}");
            return None;
        }
    };
    if cookie
        .as_ref()
        .is_some_and(|device| claims.device != *device)
    {
        info!(device = %shown, issued_to = %claims.device, "refused a token from another device");
        return None;
    }
    match holder_key(state, &Holder::of(&claims)).await {
        Ok((user, expected)) if token::same(&expected, &key) => Some(Caller {
            user,
            device: Some(claims.device),
        }),
        Ok(_) => {
            info!(device = %shown, "refused a token with another key");
            None
        }
        Err(e) => {
            info!(device = %shown, "refused a token: {e}");
            None
        }
    }
}

/// What a token says, if the Pi signed it and it has not expired. Not tied
/// to any device or key: [`sync_user`] checks those.
pub async fn verify(state: &AppState, token: &str) -> Result<Claims, String> {
    let seed = seed(state).await.map_err(|e| {
        warn!("token key: {e}");
        "storage unavailable".to_string()
    })?;
    token::verify(&token::public_key(&seed), token, Utc::now()).map_err(|e| e.to_string())
}

/// The user a token's holder names, as they are now, and the device key
/// for that token, if it can still be used: not expired, the account still
/// there, and not signed out everywhere since. A handoff (S13,
/// `crate::handoff`) checks its MAC with the key; the sync compares it.
pub async fn holder_key(state: &AppState, holder: &Holder) -> Result<(User, [u8; 32]), String> {
    let unavailable = |e: RepoError| {
        warn!("checking a token's holder: {e}");
        "storage unavailable".to_string()
    };
    if Utc::now() >= holder.expires_at() {
        return Err("the token has expired".into());
    }
    let seed = seed(state).await.map_err(unavailable)?;
    // Roles and team as they are now, and nobody for a deleted account.
    let user = state
        .repo
        .user_by_id(holder.user_id)
        .await
        .map_err(unavailable)?
        .ok_or("the account is gone")?;
    let epoch = state
        .repo
        .token_epoch(holder.user_id)
        .await
        .map_err(unavailable)?;
    if epoch != holder.epoch {
        return Err("signed out everywhere since".into());
    }
    Ok((user, token::device_key(&seed, holder)))
}
