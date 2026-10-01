//! `POST /api/sync/bundle` (S5): a lead scout's device, back from somewhere
//! with signal, hands the Pi the FIRST and TBA responses it fetched.
//!
//! The body is the bundle itself, a SQLite file
//! (`tt_repo_sqlite::bundle` has the format). It is written to a temporary
//! file, imported, and deleted; then the responses it added are applied to
//! the tables with the Pi's own parsers (`tt_upstream::project`), and every
//! other client gets them through the upstream log as usual.
//!
//! **Only a lead scout may push.** Upstream data is public and
//! last-write-wins, so the defence against a bored student's fake rankings
//! is the role, the audit row that names who pushed, and the Pi's next fetch
//! of its own (RefurbishInstructions.md, "About the API keys"). No signing.
//!
//! The pusher is the session's user, or an offline token's (C9), as for the
//! pull: a lead scout back from the lobby may find the session ran out.
//!
//! Refusals are status codes with a JSON reason, never a redirect: a client
//! must be able to tell that its bundle did not land, and keep it.
//!   - 401 not signed in, 403 not a lead scout,
//!   - 409 another schema (S11, as for the pull; `?schema=`),
//!   - 413 bigger than [`MAX_BUNDLE_BYTES`],
//!   - 422 not a bundle, with why,
//!   - 503 storage down.
//!
//! `GET /api/upstream/key` is where a lead scout's device gets the TBA key to
//! fetch with (S7, `tt_client::courier`). The key is a read-only credential
//! for public data, given to the role that may push and to no one else
//! (REFURBISH_PLAN.md, "About the API keys"). The answer also says whether
//! the Pi's own uplink is answering, so a device leaves fetching to it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde_json::json;
use tracing::{info, warn};
use tt_core::connectivity::{self, UplinkState};
use tt_repo::RepoError;
use tt_repo_sqlite::bundle::{BundleImport, Pusher};
use tt_upstream::project;

use crate::auth::device_uuid;
use crate::startup::AppState;
use crate::token::sync_user;

/// The largest bundle accepted. A whole event's responses are a few MB, the
/// biggest being playoff matches with score breakdowns.
pub const MAX_BUNDLE_BYTES: usize = 32 * 1024 * 1024;

fn refuse(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "error": error.into() }))).into_response()
}

/// A file of this process's own, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "tt-bundle-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub async fn push(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let Some(user) = sync_user(&state, &headers).await else {
        return refuse(StatusCode::UNAUTHORIZED, "sign in to push a bundle");
    };
    if !user.roles.can_lead() {
        return refuse(
            StatusCode::FORBIDDEN,
            "only a lead scout may push upstream data",
        );
    }
    if let Some(refused) = crate::sync::schema_mismatch(&state, &query) {
        return refused;
    }

    let file = Scratch::new();
    if let Err(e) = tokio::fs::write(&file.0, &body).await {
        warn!("writing a pushed bundle to {}: {e}", file.0.display());
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "could not store the bundle",
        );
    }
    let device = device_uuid(&headers);
    let pusher = Pusher {
        user_id: user.id,
        device: device.as_deref(),
    };
    let imported = match state.repo.import_bundle(&file.0, pusher, Utc::now()).await {
        Ok(imported) => imported,
        Err(RepoError::Refused(why)) => {
            info!(user = %user.email, "bundle refused: {why}");
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, why);
        }
        Err(e) => {
            warn!("importing a bundle: {e}");
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable");
        }
    };
    drop(file);

    let projected = project::project(&*state.repo, &imported.appended).await;
    info!(
        user = %user.email,
        device = device.as_deref().unwrap_or("-"),
        log = %imported.log,
        "bundle {}: {} new, {} unchanged, {} stale, {} refused; applied {}",
        imported.id,
        imported.appended.len(),
        imported.unchanged,
        imported.stale,
        imported.refused,
        projected.report.summary(),
    );

    Json(json!({
        "status": "imported",
        "log": imported.log,
        // What the Pi has read of this log: the client need not send rows at
        // or before it again.
        "cursor": imported.to_seq,
        "read": imported.read(),
        "appended": imported.appended.len(),
        "unchanged": imported.unchanged,
        "stale": imported.stale,
        "refused": imported.refused,
        "applied": {
            "matches": projected.report.matches,
            "stats": projected.report.stats,
        },
        "logged_only": projected.logged_only,
        "problems": projected.report.problems,
    }))
    .into_response()
}

/// `GET /api/upstream/key`: the TBA key, to a lead scout's device only.
///
/// `{"tba": key or null, "base": where to send it, "uplink_online": bool}`.
/// `null` when the Pi has no key, which tells a device to forget its own.
pub async fn key(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(user) = sync_user(&state, &headers).await else {
        return refuse(StatusCode::UNAUTHORIZED, "sign in to fetch upstream data");
    };
    if !user.roles.can_lead() {
        return refuse(
            StatusCode::FORBIDDEN,
            "only a lead scout may fetch upstream data",
        );
    }
    let tba = state.upstream.tba.as_ref();
    let online = state.upstream.uplink.snapshot().classify(Utc::now()) == UplinkState::Online;
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "tba": tba.map(|c| c.auth_key()),
            "base": tba.map(|c| c.base_url()),
            "uplink_online": online,
        })),
    )
        .into_response()
}

/// [`describe`] for the newest import in storage.
pub async fn last_import(state: &AppState) -> String {
    match state.repo.bundle_imports(1).await {
        Ok(imports) => describe(imports.first(), Utc::now()),
        Err(e) => {
            warn!("reading bundle imports: {e}");
            String::new()
        }
    }
}

/// The lead scout page's line about the newest bundle, e.g. "Ana, from
/// Tablet 3, 12 minutes ago: 4 new responses". Empty when there has been none.
pub fn describe(last: Option<&BundleImport>, now: DateTime<Utc>) -> String {
    let Some(last) = last else {
        return String::new();
    };
    let who = last.user.as_deref().unwrap_or("A deleted account");
    let from = last
        .device
        .as_deref()
        .map(|d| format!(", from {d}"))
        .unwrap_or_default();
    let age = connectivity::describe_age(now - last.imported_at);
    let what = match last.appended {
        0 => "nothing new".to_string(),
        1 => "1 new response".to_string(),
        n => format!("{n} new responses"),
    };
    let refused = match last.refused {
        0 => String::new(),
        n => format!(", {n} refused"),
    };
    format!("{who}{from}, {age}: {what}{refused}")
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    #[test]
    fn the_newest_bundle_says_who_from_where_and_what() {
        let now = Utc::now();
        let mut last = BundleImport {
            imported_at: now - TimeDelta::minutes(12),
            user: Some("Ana".into()),
            device: Some("Tablet 3".into()),
            appended: 4,
            unchanged: 2,
            stale: 0,
            refused: 0,
        };
        let line = describe(Some(&last), now);
        assert!(line.starts_with("Ana, from Tablet 3, "), "{line}");
        assert!(line.ends_with(": 4 new responses"), "{line}");

        last.device = None;
        last.appended = 0;
        last.refused = 2;
        let line = describe(Some(&last), now);
        assert!(line.starts_with("Ana, "), "{line}");
        assert!(line.ends_with(": nothing new, 2 refused"), "{line}");

        assert_eq!(describe(None, now), "");
    }
}
