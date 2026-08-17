//! JSON APIs: manual FIRST sync, pick list persistence, and network status.
//!
//! These endpoints are called by JavaScript rather than navigated to, so
//! failures answer with a JSON `error` field and a status code instead of a
//! rendered page. The one exception is [`network_status_badge`], which returns
//! the badge fragment for the same data.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{error, info};

use crate::models::pick_list::{self, PickListEntry};
use crate::services::{connectivity, first_sync};
use crate::state::SharedState;
use crate::views::network::NetworkStatusBadgeFragment;
use crate::views::render;
use crate::web::*;

const SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
const STATUS_REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// `POST /api/frc/sync` — runs a full FIRST sync now and reports the counts.
///
/// Lead scouts and admins only. Bounded by [`SYNC_TIMEOUT`], since it walks
/// every event of the season; missing credentials are a 400 rather than a 500,
/// because that is a configuration problem the user can fix.
pub async fn frc_sync(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok((StatusCode::UNAUTHORIZED, "Unauthorized").into_response());
    };
    if !user.can_lead() {
        return Ok((StatusCode::UNAUTHORIZED, "Unauthorized").into_response());
    }

    let fut = first_sync::sync_now(&state.pool);
    match tokio::time::timeout(SYNC_TIMEOUT, fut).await {
        Ok(Ok(result)) => {
            info!(
                "FRC sync completed: events={} teams={} event_teams={}",
                result.events, result.teams, result.event_teams
            );
            Ok(Json(serde_json::json!({
                "season": result.season,
                "events": result.events,
                "teams": result.teams,
                "eventTeams": result.event_teams,
            }))
            .into_response())
        }
        Ok(Err(e)) if e.is::<first_sync::SyncSkipped>() => {
            Ok((StatusCode::BAD_REQUEST, "FIRST API credentials missing").into_response())
        }
        Ok(Err(e)) => {
            error!("FRC sync failed: {e}");
            Ok((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
        Err(_) => Ok((StatusCode::INTERNAL_SERVER_ERROR, "sync timed out").into_response()),
    }
}

// ── Pick list ─────────────────────────────────────────────────────────────

/// `{"error": "..."}` with a status code.
fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// The pick list is per (team, event), so both must be resolvable.
async fn pick_list_context(
    state: &SharedState,
    jar: &CookieJar,
) -> Result<(i32, i32), Response> {
    let Some(user) = current_user(&state.pool, jar).await else {
        return Err(json_error(StatusCode::UNAUTHORIZED, "not authenticated"));
    };
    let Some(session) = current_session(&state.pool, jar).await else {
        return Err(json_error(StatusCode::UNAUTHORIZED, "not authenticated"));
    };
    let Some(team_number) = user.team_number.filter(|n| *n != 0) else {
        return Err(json_error(
            StatusCode::BAD_REQUEST,
            "user has no team assigned",
        ));
    };
    let Some(event_id) = session.selected_event_id else {
        return Err(json_error(StatusCode::BAD_REQUEST, "no event selected"));
    };
    Ok((team_number, event_id))
}

/// `GET /api/pick-list` — the viewer team's list for the selected event.
pub async fn get_pick_list(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let (team_number, event_id) = match pick_list_context(&state, &jar).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let entries: Vec<_> = pick_list::entries(&state.pool, team_number, event_id)
        .await
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "picked_team_number": e.picked_team_number,
                "color": e.color,
                "crossed": e.crossed.unwrap_or(false),
                "position": e.position.unwrap_or(0),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "entries": entries })).into_response())
}

/// Body of a pick list save.
#[derive(Deserialize)]
pub struct PickListEntryRequest {
    pub picked_team_number: i32,
    pub color: Option<String>,
    #[serde(default)]
    pub crossed: bool,
    #[serde(default)]
    pub position: i32,
}

/// `POST /api/pick-list/entry` — inserts or updates one entry.
pub async fn save_pick_list_entry(
    State(state): State<SharedState>,
    jar: CookieJar,
    Json(request): Json<PickListEntryRequest>,
) -> HandlerResult {
    let (team_number, event_id) = match pick_list_context(&state, &jar).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };
    if request.picked_team_number == 0 {
        return Ok(json_error(StatusCode::BAD_REQUEST, "invalid request"));
    }

    let entry = PickListEntry {
        picked_team_number: request.picked_team_number,
        color: request.color,
        crossed: Some(request.crossed),
        position: Some(request.position),
    };

    match pick_list::save_entry(&state.pool, team_number, event_id, &entry).await {
        Ok(()) => Ok(Json(serde_json::json!({"status": "saved"})).into_response()),
        Err(e) => {
            error!("failed to save pick list entry: {e}");
            Ok(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to save entry",
            ))
        }
    }
}

/// `DELETE /api/pick-list/entry?team=` — removes one entry.
pub async fn delete_pick_list_entry(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let (team_number, event_id) = match pick_list_context(&state, &jar).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let Some(team) = query.get("team").filter(|v| !v.is_empty()) else {
        return Ok(json_error(StatusCode::BAD_REQUEST, "team number required"));
    };
    let Ok(picked_team_number) = team.parse::<i32>() else {
        return Ok(json_error(StatusCode::BAD_REQUEST, "invalid team number"));
    };

    match pick_list::delete_entry(&state.pool, team_number, event_id, picked_team_number).await {
        Ok(()) => Ok(Json(serde_json::json!({"status": "deleted"})).into_response()),
        Err(e) => {
            error!("failed to delete pick list entry: {e}");
            Ok(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to delete entry",
            ))
        }
    }
}

// ── Network status ────────────────────────────────────────────────────────

/// `GET /hx/network/status` — the status badge fragment.
pub async fn network_status_badge(State(_state): State<SharedState>) -> HandlerResult {
    let snapshot = refreshed_snapshot().await;
    Ok(render(&NetworkStatusBadgeFragment::from_snapshot(
        snapshot.classify(),
        &snapshot,
    )))
}

/// `GET /api/network/status` — the same status as JSON, for scripts and the
/// Pi's boot checks.
pub async fn network_status(State(_state): State<SharedState>) -> HandlerResult {
    let snapshot = refreshed_snapshot().await;
    Ok(Json(serde_json::json!({
        "status": snapshot.classify(),
        "data": snapshot,
    }))
    .into_response())
}

/// Probes connectivity, but never blocks the response for long: on timeout
/// the last known snapshot is returned rather than making the caller wait.
async fn refreshed_snapshot() -> connectivity::NetworkStatusSnapshot {
    tokio::time::timeout(STATUS_REFRESH_TIMEOUT, connectivity::refresh())
        .await
        .ok();
    connectivity::snapshot()
}
