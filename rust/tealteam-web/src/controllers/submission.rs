//! Scouting submission page and form handling.
//!
//! This is the page scouts spend a competition in, so it is built to keep them
//! on it: the form is a fragment, and submitting re-renders it in place with a
//! banner and a fresh pre-fill for the next match.
//!
//! Submissions are queued for lead-scout review rather than written straight
//! to `scouting_data` — see [`crate::models::scouting`]. Each one is
//! attributed to the scout's team so their notes come back to their own team
//! only.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{error, warn};

use crate::models::scouting::NewSubmission;
use crate::models::{assignment, event, scouting, session, team, User};
use crate::services::first_api::FirstApiClient;
use crate::services::first_sync;
use crate::state::SharedState;
use crate::views::submission::{
    ScoutingFormData, SubmissionPanelFragment, SubmissionTemplate, TeamSelect,
};
use crate::views::{render, render_html, Nav};
use crate::web::*;

/// Loads the events, the scout's assignments and the team list backing the
/// submission form.
async fn build_form_data(pool: &PgPool, user: &User, jar: &CookieJar) -> ScoutingFormData {
    let mut data = ScoutingFormData {
        events: {
            let event_ids = event::available_ids(pool, user).await.unwrap_or_default();
            event::options(pool, &event_ids).await
        },
        ..Default::default()
    };

    let Some(event_id) = session::get_session(pool, jar).await.and_then(|s| s.selected_event_id)
    else {
        return data;
    };
    data.prefill_event_id = Some(event_id);

    // The scout's next unplayed match assignment for the selected event,
    // matched by signed-in user OR by this device's permanent UUID.
    data.assigned_teams =
        assignment::upcoming_for_scout(pool, event_id, user.id, session::device_uuid(jar)).await;
    data.team_options = team::options_for_event(pool, event_id).await;
    data.prefill_from_assignment();

    data
}

/// Renders the form panel with an optional error or success banner.
async fn render_panel(
    pool: &PgPool,
    user: &User,
    jar: &CookieJar,
    error: &str,
    success: &str,
) -> String {
    render_html(&SubmissionPanelFragment {
        error: error.to_string(),
        success: success.to_string(),
        form: build_form_data(pool, user, jar).await,
    })
}

/// `GET /submission` — the scouting form.
pub async fn submission_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let panel_html = render_panel(&state.pool, &user, &jar, "", "").await;
    Ok(render(&SubmissionTemplate::new(
        Nav::from_user(Some(&user)),
        panel_html,
    )))
}

/// `POST /submission` — validates and queues a scouting submission.
///
/// Event, team, alliance colour and starting position are required; every
/// other field is optional, because a scout who saw only part of a match
/// should still be able to record what they saw.
pub async fn submit(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let form = form_map(&body);
    let field = |name: &str| form_str(&form, name).trim().to_lowercase();

    let event_id = parse_required_int(form_str(&form, "event_id"));
    let team_id = parse_required_int(form_str(&form, "team_id"));
    let alliance_color = field("alliance_color");
    let starting_position = field("starting_position");

    let error = if event_id.is_none() {
        Some("event_id is required")
    } else if team_id.is_none() {
        Some("team_id is required")
    } else if alliance_color.is_empty() {
        Some("alliance_color is required")
    } else if starting_position.is_empty() {
        Some("starting_position is required")
    } else {
        None
    };

    if let Some(error) = error {
        if is_unpoly(&headers) {
            let html = render_panel(&state.pool, &user, &jar, error, "").await;
            return Ok(Html(html).into_response());
        }
        return Ok((StatusCode::BAD_REQUEST, error).into_response());
    }

    // Attribute the submission to the scout's team so their notes are visible
    // to teammates on the team data page.
    let submitting_team_id = match user.active_team_number() {
        Some(team_number) => team::id_by_number(&state.pool, team_number).await,
        None => None,
    };

    let submission = NewSubmission {
        event_id: event_id.unwrap(),
        team_id: team_id.unwrap(),
        alliance_color: &alliance_color,
        notes: form_str(&form, "notes").trim(),
        starting_position: &starting_position,
        defense_rating: field("defense_rating"),
        traversal: field("traversal"),
        scoring_strategy: field("teleop_strategy"),
        shooting_speed: field("shooting_speed"),
        capacity: field("capacity"),
        defendability: form_str(&form, "defendability").trim(),
        hang_level: field("hang_level"),
        auto_hang: field("auto_hang"),
        hang_position: field("hang_position"),
        scouter_id: user.id,
        submitting_team_id,
    };

    if let Err(e) = scouting::queue_submission(&state.pool, &submission).await {
        error!(
            "failed to create scouting submission (event {event_id:?}, team {team_id:?}, scouter {}): {e}",
            user.id
        );
        if is_unpoly(&headers) {
            let msg = format!("Failed to queue submission: {e}");
            let html = render_panel(&state.pool, &user, &jar, &msg, "").await;
            return Ok(Html(html).into_response());
        }
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to queue submission: {e}"),
        )
            .into_response());
    }

    if is_unpoly(&headers) {
        let html = render_panel(
            &state.pool,
            &user,
            &jar,
            "",
            "Submission queued for team scouting. Thanks for scouting!",
        )
        .await;
        return Ok(Html(html).into_response());
    }

    Ok(redirect("/submission"))
}

/// `GET /submission/event-teams` — the team `<select>` for an event.
///
/// When the local roster is empty (a scout picked an event nobody has synced
/// yet) this falls back to the FIRST API and upserts what it finds, so the
/// form is usable immediately rather than after the next sync. Unauthenticated
/// on purpose: it exposes nothing but public team names, and the form needs it
/// during first paint.
pub async fn event_teams(
    State(state): State<SharedState>,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(event_id_raw) = query.get("event_id").filter(|v| !v.is_empty()) else {
        return Ok((StatusCode::BAD_REQUEST, "event_id is required").into_response());
    };
    let Ok(event_id) = event_id_raw.parse::<i32>() else {
        return Ok((StatusCode::BAD_REQUEST, "event_id must be a number").into_response());
    };

    let event_tba_key = event::find_tba_key(&state.pool, event_id).await;
    let teams = team::options_for_event(&state.pool, event_id).await;

    let mut select = TeamSelect::new();

    if !teams.is_empty() {
        for team in &teams {
            select.push(team.id, team.team_number, &team.name);
        }
        return Ok(Html(select.render()).into_response());
    }

    // No teams locally; fall back to the FIRST API and upsert results.
    let Some(client) = FirstApiClient::from_environment() else {
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            "FIRST API credentials not configured",
        )
            .into_response());
    };

    let first_teams = match client
        .get_event_teams(
            FirstApiClient::season_from_environment(),
            event_tba_key.as_deref().unwrap_or(""),
        )
        .await
    {
        Ok(t) => t,
        Err(e) => {
            error!("failed to fetch teams from FIRST API for event {event_id}: {e}");
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch teams from FIRST API",
            )
                .into_response());
        }
    };

    if first_teams.is_empty() {
        select.push_empty_notice();
        return Ok(Html(select.render()).into_response());
    }

    for first_team in &first_teams {
        match first_sync::upsert_team(&state.pool, first_team).await {
            Ok(db_id) => select.push(db_id, first_team.team_number, &first_team.display_name()),
            Err(e) => warn!("failed to upsert team {}: {e}", first_team.team_number),
        }
    }

    Ok(Html(select.render()).into_response())
}
