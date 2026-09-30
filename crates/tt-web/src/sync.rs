//! `GET /api/sync/pull` (S2): what changed since a client last asked.
//!
//! Two streams, two cursors, one request. `changes` is the venue stream: what
//! scouts and leads wrote, written by triggers so deletions are in it too.
//! `upstream` is S1's log of FIRST and TBA responses. Each comes back with the
//! cursor to send next time; `more` says to ask again straight away.
//!
//! Changes are served only once they are [`LAG`] old. A change still being
//! committed has its `seq` already, and a client that read past it would never
//! see it; waiting two seconds costs nothing a person would notice.
//!
//! What a viewer may see is decided per change in [`visible`], the one place
//! S3's subscription scope will go.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{TimeDelta, Utc};
use serde_json::{Value as JsonValue, json};
use tracing::warn;
use tt_core::notes::{self, Notes};
use tt_core::season::{Payload, SeasonSchema};
use tt_core::user::User;
use tt_repo::{Change, Repo};

use crate::auth::Auth;
use crate::startup::AppState;

/// How old a change must be before it is served.
pub const LAG: TimeDelta = TimeDelta::seconds(2);
/// Most changes per pull. Small rows; a whole event is a few thousand.
pub const CHANGES_PER_PULL: i64 = 500;
/// Most upstream responses per pull. These are whole API bodies.
pub const UPSTREAM_PER_PULL: i64 = 20;

/// `GET /api/sync/pull?changes=<cursor>&upstream=<cursor>`. Cursors default to
/// 0, which is everything.
pub async fn pull(
    State(state): State<AppState>,
    Auth(viewer): Auth,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let cursor = |name: &str| {
        query
            .get(name)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
            .max(0)
    };
    let (after_change, after_upstream) = (cursor("changes"), cursor("upstream"));

    let changes = state
        .repo
        .changes_since(after_change, CHANGES_PER_PULL, Utc::now() - LAG)
        .await;
    let upstream = state
        .repo
        .upstream_since(after_upstream, UPSTREAM_PER_PULL)
        .await;
    let (changes, upstream) = match (changes, upstream) {
        (Ok(c), Ok(u)) => (c, u),
        (Err(e), _) | (_, Err(e)) => {
            warn!("sync pull: {e}");
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "storage unavailable" })),
            )
                .into_response();
        }
    };

    // The cursor moves past every change read, shown or not, so a client is
    // never sent back for what it may not see.
    let changes_cursor = changes.last().map_or(after_change, |c| c.seq);
    let changes_more = changes.len() as i64 == CHANGES_PER_PULL;
    let upstream_cursor = upstream.last().map_or(after_upstream, |u| u.seq);
    let upstream_more = upstream.len() as i64 == UPSTREAM_PER_PULL;

    let shown: Vec<JsonValue> = changes
        .into_iter()
        .filter_map(|c| visible(&state.season, &viewer, c))
        .collect();
    let upstream: Vec<JsonValue> = upstream
        .into_iter()
        .map(|u| {
            json!({
                "seq": u.seq,
                "api": u.entry.api,
                "path": u.entry.path,
                "etag": u.entry.etag,
                "body": u.entry.body,
                "fetched_at": u.entry.fetched_at,
            })
        })
        .collect();

    Json(json!({
        "changes": shown,
        "changes_cursor": changes_cursor,
        "changes_more": changes_more,
        "upstream": upstream,
        "upstream_cursor": upstream_cursor,
        "upstream_more": upstream_more,
    }))
    .into_response()
}

/// What `viewer` may see of `change`, or nothing.
///
/// Two rules hold now and always: a team-scoped change (a pick list) goes to
/// that team only, and an observation's notes go only to the team that wrote
/// them (U13). Everything else is public.
///
/// **S3 hook:** a client's subscription scope (its event, what it asked for)
/// filters here, before these two rules, never instead of them.
pub fn visible(schema: &SeasonSchema, viewer: &User, change: Change) -> Option<JsonValue> {
    if change
        .team_scope
        .is_some_and(|team| viewer.team_number != Some(team))
    {
        return None;
    }

    let mut row: Option<JsonValue> = change
        .payload
        .as_deref()
        .map(|raw| serde_json::from_str(raw).unwrap_or_else(|_| json!({})));
    if change.entity == "observation"
        && let Some(JsonValue::Object(fields)) = row.as_mut()
    {
        let writer = fields
            .get("submitting_team")
            .and_then(JsonValue::as_i64)
            .and_then(|t| i32::try_from(t).ok());
        if !Notes::for_viewer(viewer.team_number, writer).shown() {
            // Unreadable answers are sent as none: never fail open.
            let mut answers: Payload = fields
                .get("payload")
                .cloned()
                .and_then(|p| serde_json::from_value(p).ok())
                .unwrap_or_default();
            notes::redact(schema, &mut answers);
            fields.insert(
                "payload".into(),
                serde_json::to_value(answers).unwrap_or_else(|_| json!({})),
            );
        }
    }

    Some(json!({
        "seq": change.seq,
        "entity": change.entity,
        "entity_pk": change.entity_pk,
        "op": change.op,
        "row": row,
        "event_key": change.event_key,
        "at": change.created_at,
    }))
}
