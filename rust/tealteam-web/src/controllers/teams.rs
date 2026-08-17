//! Team lookup page and its Unpoly fragments.
//!
//! Open to signed-out visitors: anyone can look up a team and see its synced
//! stats. Being signed in adds the private layer — scouting notes are only
//! rendered for the viewer's own team, resolved in [`team_event_data`].
//!
//! A team with no events locally triggers a synchronous FIRST sync on first
//! lookup, so a scout searching a team they have never opened still gets an
//! event list.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::warn;

use crate::models::{event, scouting, stats, team, User};
use crate::services::first_sync;
use crate::state::SharedState;
use crate::views::teams::{
    team_info_error_html, TeamDataFragment, TeamDataView, TeamInfoFragment, TeamPageTemplate,
    TeamView,
};
use crate::views::{render, render_html, Nav};
use crate::web::*;

/// `GET /teams` — the search page, pre-filled from `?team=`.
pub async fn team_page(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let team_number_raw = query.get("team").cloned().unwrap_or_default();

    let mut team_error = String::new();
    let mut team_info_html = String::new();

    if !team_number_raw.is_empty() {
        match build_team_info(&state.pool, user.as_ref(), &team_number_raw).await {
            Ok(html) => team_info_html = html,
            Err(msg) => team_error = msg,
        }
    }

    Ok(render(&TeamPageTemplate {
        title: "Teams".to_string(),
        nav: Nav::from_user(user.as_ref()),
        team_search_value: team_number_raw,
        team_error,
        team_info_html,
    }))
}

/// `GET /hx/teams/search` — the team card for a search, or an error card.
pub async fn team_search(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let team_number_raw = query.get("team").cloned().unwrap_or_default();

    match build_team_info(&state.pool, user.as_ref(), &team_number_raw).await {
        Ok(html) => Ok(Html(html).into_response()),
        Err(msg) => Ok(Html(team_info_error_html(&msg)).into_response()),
    }
}

/// `POST /hx/teams/fetch-past-events` — forces a FIRST re-sync for a team,
/// then re-renders its card. A sync failure still re-renders whatever is
/// already known.
pub async fn fetch_past_events(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let form = form_map(&body);
    let team_number_raw = form_str(&form, "team").trim().to_string();
    if team_number_raw.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Team number is required").into_response());
    }
    let Ok(team_number) = team_number_raw.parse::<i32>() else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid team number").into_response());
    };

    if let Err(e) = first_sync::sync_team_for_user(&state.pool, team_number).await {
        warn!("failed to fetch past events for team {team_number}: {e}");
    }

    match build_team_info(&state.pool, user.as_ref(), &team_number_raw).await {
        Ok(html) => Ok(Html(html).into_response()),
        Err(msg) => Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(team_info_error_html(&msg)),
        )
            .into_response()),
    }
}

/// Renders the team identity card and its event picker, syncing the team's
/// events from FIRST the first time we see it.
async fn build_team_info(
    pool: &PgPool,
    user: Option<&User>,
    team_number_raw: &str,
) -> Result<String, String> {
    if team_number_raw.is_empty() {
        return Err("Team number is required".to_string());
    }
    let Ok(team_number) = team_number_raw.parse::<i32>() else {
        return Err("Invalid team number".to_string());
    };

    let Some(team_record) = team::find_by_number(pool, team_number).await else {
        return Err(format!("Team {team_number} not found"));
    };

    let mut event_ids = event::ids_for_team(pool, team_number).await.unwrap_or_default();
    if event_ids.is_empty() {
        if let Err(e) = first_sync::sync_team_for_user(pool, team_number).await {
            warn!("failed to sync events for team {team_number}: {e}");
        } else {
            event_ids = event::ids_for_team(pool, team_number).await.unwrap_or_default();
        }
    }

    Ok(render_html(&TeamInfoFragment {
        signed_in: user.is_some(),
        team: TeamView::from_team(&team_record),
        events: event::options(pool, &event_ids).await,
        selected_event_id: None,
    }))
}

/// `GET /hx/teams/data` — stats and scouting summary for one team at one
/// event.
pub async fn team_event_data(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let team_number_raw = query.get("team").cloned().unwrap_or_default();
    let event_id_raw = query.get("event_id").cloned().unwrap_or_default();

    if team_number_raw.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Team number is required").into_response());
    }
    let Ok(team_number) = team_number_raw.parse::<i32>() else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid team number").into_response());
    };
    if event_id_raw.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Event ID is required").into_response());
    }
    let Ok(event_id) = event_id_raw.parse::<i32>() else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid event ID").into_response());
    };

    let Some(team_record) = team::find_by_number(&state.pool, team_number).await else {
        return Ok((StatusCode::NOT_FOUND, "Team not found").into_response());
    };

    // Resolve the viewer's team id; notes are only shown to their own team.
    let viewer_team_id = match current_user(&state.pool, &jar).await.and_then(|u| u.team_number) {
        Some(viewer_team_number) => team::id_by_number(&state.pool, viewer_team_number)
            .await
            .unwrap_or(0),
        None => 0,
    };

    let v = TeamDataView::build(
        event::find_name(&state.pool, event_id).await.unwrap_or_default(),
        stats::find(&state.pool, team_record.id, event_id).await,
        scouting::data_for_team_event(&state.pool, team_record.id, event_id).await,
        viewer_team_id,
    );

    Ok(Html(render_html(&TeamDataFragment { v })).into_response())
}
