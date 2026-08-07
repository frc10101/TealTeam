// JSON APIs: manual FIRST sync, pick list persistence, and network status
// (port of Controllers/ApiController.cs).

use askama::Template;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Local, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{error, info};

use super::*;
use crate::connectivity::{self, NetworkStatusSnapshot};
use crate::models::PickListEntry;
use crate::state::SharedState;

pub async fn frc_sync(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok((axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok((axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response());
    }

    let fut = crate::first_sync::sync_now(&state.pool);
    match tokio::time::timeout(std::time::Duration::from_secs(90), fut).await {
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
        Ok(Err(e)) if e.is::<crate::first_sync::SyncSkipped>() => {
            Ok((axum::http::StatusCode::BAD_REQUEST, "FIRST API credentials missing").into_response())
        }
        Ok(Err(e)) => {
            error!("FRC sync failed: {e}");
            Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
        }
        Err(_) => Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "sync timed out").into_response()),
    }
}

async fn pick_list_context(
    state: &SharedState,
    jar: &CookieJar,
) -> Result<(i32, i32), axum::response::Response> {
    let user = current_user(&state.pool, jar).await;
    let Some(user) = user else {
        return Err((axum::http::StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "not authenticated"}))).into_response());
    };
    let session = current_session(&state.pool, jar).await;
    let Some(session) = session else {
        return Err((axum::http::StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "not authenticated"}))).into_response());
    };
    let Some(team_number) = user.team_number.filter(|n| *n != 0) else {
        return Err((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "user has no team assigned"}))).into_response());
    };
    let Some(event_id) = session.selected_event_id else {
        return Err((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "no event selected"}))).into_response());
    };
    Ok((team_number, event_id))
}

pub async fn get_pick_list(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let (team_number, event_id) = match pick_list_context(&state, &jar).await {
        Ok(v) => v,
        Err(resp) => return Ok(resp),
    };

    let entries: Vec<PickListEntry> = sqlx::query_as(
        "SELECT picked_team_number, color, crossed, position FROM pick_list_entries
         WHERE team_number = $1 AND event_id = $2 ORDER BY position",
    )
    .bind(team_number)
    .bind(event_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let json: Vec<_> = entries
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

    Ok(Json(serde_json::json!({ "entries": json })).into_response())
}

#[derive(Deserialize)]
pub struct PickListEntryRequest {
    pub picked_team_number: i32,
    pub color: Option<String>,
    #[serde(default)]
    pub crossed: bool,
    #[serde(default)]
    pub position: i32,
}

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
        return Ok((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid request"}))).into_response());
    }

    let result = sqlx::query(
        "INSERT INTO pick_list_entries (team_number, event_id, picked_team_number, color, crossed, position)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (team_number, event_id, picked_team_number) DO UPDATE SET
            color = EXCLUDED.color, crossed = EXCLUDED.crossed, position = EXCLUDED.position,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(team_number)
    .bind(event_id)
    .bind(request.picked_team_number)
    .bind(&request.color)
    .bind(request.crossed)
    .bind(request.position)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Ok(Json(serde_json::json!({"status": "saved"})).into_response()),
        Err(e) => {
            error!("failed to save pick list entry: {e}");
            Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "failed to save entry"}))).into_response())
        }
    }
}

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
        return Ok((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "team number required"}))).into_response());
    };
    let Ok(picked_team_number) = team.parse::<i32>() else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid team number"}))).into_response());
    };

    let result = sqlx::query(
        "DELETE FROM pick_list_entries
         WHERE team_number = $1 AND event_id = $2 AND picked_team_number = $3",
    )
    .bind(team_number)
    .bind(event_id)
    .bind(picked_team_number)
    .execute(&state.pool)
    .await;

    match result {
        Ok(_) => Ok(Json(serde_json::json!({"status": "deleted"})).into_response()),
        Err(e) => {
            error!("failed to delete pick list entry: {e}");
            Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "failed to delete entry"}))).into_response())
        }
    }
}

// ── Network status ─────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "partials/network_status_badge.html")]
struct NetworkStatusBadgeFragment {
    status: String,
    label: String,
    css_class: String,
    last_sync: String,
    last_api_success: String,
    last_api_error_text: String,
    last_api_error: String,
    internet_error: String,
}

pub async fn network_status_badge(State(_state): State<SharedState>) -> HandlerResult {
    tokio::time::timeout(std::time::Duration::from_secs(2), connectivity::refresh())
        .await
        .ok();

    let snapshot = connectivity::snapshot();
    let status = classify_network_state(&snapshot);
    let (label, css_class) = match status.as_str() {
        "internet-ok" => ("Internet OK", "bg-teal-900/40 text-teal-200 border-teal-600"),
        "api-error" => ("API Error", "bg-amber-900/40 text-amber-200 border-amber-600"),
        _ => ("Offline", "bg-red-900/40 text-red-200 border-red-600"),
    };

    Ok(render(&NetworkStatusBadgeFragment {
        status: status.clone(),
        label: label.to_string(),
        css_class: css_class.to_string(),
        last_sync: format_status_time(snapshot.last_successful_sync),
        last_api_success: format_status_time(snapshot.last_api_success_at),
        last_api_error_text: format_status_time(snapshot.last_api_error_at),
        last_api_error: snapshot.last_api_error.clone(),
        internet_error: snapshot.internet_error.clone(),
    }))
}

pub async fn network_status(State(_state): State<SharedState>) -> HandlerResult {
    tokio::time::timeout(std::time::Duration::from_secs(2), connectivity::refresh())
        .await
        .ok();

    let snapshot = connectivity::snapshot();
    Ok(Json(serde_json::json!({
        "status": classify_network_state(&snapshot),
        "data": snapshot,
    }))
    .into_response())
}

fn format_status_time(t: Option<DateTime<Utc>>) -> String {
    match t {
        Some(t) => t.with_timezone(&Local).format("%b %-d, %-I:%M:%S %p").to_string(),
        None => "Never".to_string(),
    }
}

fn classify_network_state(s: &NetworkStatusSnapshot) -> String {
    let recent_window = chrono::Duration::minutes(10);
    if let Some(success) = s.last_api_success_at {
        if Utc::now().signed_duration_since(success) <= recent_window {
            let newer_than_error = s
                .last_api_error_at
                .map(|err| success > err)
                .unwrap_or(true);
            if newer_than_error {
                return "internet-ok".to_string();
            }
        }
    }

    if !s.internet_reachable {
        return "offline".to_string();
    }
    if let Some(err) = s.last_api_error_at {
        let newer_than_success = s.last_api_success_at.map(|ok| err > ok).unwrap_or(true);
        if newer_than_success {
            return "api-error".to_string();
        }
    }
    "internet-ok".to_string()
}
