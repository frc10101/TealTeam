//! `GET /api/sync/snapshot` (S10): a fresh device's first sync, as one file.
//!
//! The answer is a SQLite database (`tt_repo_sqlite::snapshot` says what is
//! in it) holding what the signed-in viewer may see of the events asked for,
//! `?event=` as for the pull (S3). A client writes it to OPFS, opens it, and
//! pulls from the cursors it carries, which are also in the header
//! `X-Sync-Cursor: <changes>-<upstream>`, the S8 event id's shape.
//! `static/js/snapshot.js` does the download.
//!
//! **Shared for a minute.** At the start of an event every tablet asks at
//! once, and each would be a copy of the whole database. A snapshot is kept
//! for [`FRESH`] and given to anyone asking with the same team and events, and
//! only one is made at a time. One a minute old is still right: its cursors
//! are its own, so the client pulls the minute it missed.
//!
//! With `?schema=` not the server's, a 409 as for the pull (S11): the file
//! is this build's schema, which an older client cannot read.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::warn;
use tt_core::notes::{self, Notes};
use tt_core::season::{Payload, SeasonSchema};
use tt_repo_sqlite::snapshot::{self, Audience, Snapshot};

use crate::auth::Auth;
use crate::startup::AppState;
use crate::sync::{self, Scope};

/// How long a snapshot is handed out before a new one is made.
pub const FRESH: Duration = Duration::from_secs(60);

/// Who a snapshot was made for: the viewer's team, and the events asked for.
type Key = (Option<i32>, Option<Vec<String>>);

/// The snapshots made in the last [`FRESH`]. Held across a build, so a crowd
/// of tablets asking together makes one.
#[derive(Default)]
pub struct Cache(Mutex<Vec<(Key, Instant, Arc<Snapshot>)>>);

impl Cache {
    async fn get_or_build(
        &self,
        key: Key,
        build: impl Future<Output = tt_repo::Result<Snapshot>>,
    ) -> tt_repo::Result<Arc<Snapshot>> {
        let mut kept = self.0.lock().await;
        kept.retain(|(_, made, _)| made.elapsed() < FRESH);
        if let Some((_, _, snap)) = kept.iter().find(|(k, _, _)| *k == key) {
            return Ok(snap.clone());
        }
        let snap = Arc::new(build.await?);
        kept.push((key, Instant::now(), snap.clone()));
        Ok(snap)
    }
}

/// The pull's rules (`sync::visible`), for a snapshot's rows.
struct Viewer<'a> {
    season: &'a SeasonSchema,
    team: Option<i32>,
    scope: &'a Scope,
}

impl Audience for Viewer<'_> {
    fn team(&self) -> Option<i32> {
        self.team
    }

    fn has_event(&self, event_key: &str) -> bool {
        self.scope.has_event(event_key)
    }

    fn has_upstream(&self, path: &str) -> bool {
        self.scope.has_upstream(path)
    }

    fn answers(&self, writer: Option<i32>, payload: &str) -> Option<String> {
        if Notes::for_viewer(self.team, writer).shown() {
            return None;
        }
        // Unreadable answers are sent as none: never fail open.
        let mut answers: Payload = serde_json::from_str(payload).unwrap_or_default();
        notes::redact(self.season, &mut answers);
        Some(serde_json::to_string(&answers).unwrap_or_else(|_| "{}".into()))
    }
}

pub async fn download(
    State(state): State<AppState>,
    Auth(viewer): Auth,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(refused) = sync::schema_mismatch(&state, &query) {
        return refused;
    }
    let scope = Scope::from_query(uri.query());
    let mut events = scope.events.clone();
    if let Some(events) = events.as_mut() {
        events.sort();
        events.dedup();
    }
    let audience = Viewer {
        season: &state.season,
        team: viewer.team_number,
        scope: &scope,
    };
    let built = state
        .snapshots
        .get_or_build(
            (viewer.team_number, events),
            snapshot::build(&state.repo, &audience),
        )
        .await;
    let snap = match built {
        Ok(snap) => snap,
        Err(e) => {
            warn!("sync snapshot: {e}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "storage unavailable" })),
            )
                .into_response();
        }
    };

    (
        [
            (header::CONTENT_TYPE, "application/vnd.sqlite3".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"tealteam.sqlite3\"".into(),
            ),
            // Whose notes and pick list are in it depends on who asked.
            (header::CACHE_CONTROL, "private, no-store".into()),
            (
                header::HeaderName::from_static("x-sync-cursor"),
                sync::event_id(snap.changes, snap.upstream),
            ),
            (
                header::HeaderName::from_static("x-sync-taken-at"),
                snap.taken_at.to_rfc3339(),
            ),
        ],
        snap.bytes.clone(),
    )
        .into_response()
}
