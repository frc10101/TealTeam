//! Drive coach panel: the team's own schedule with alliance partners.
//!
//! The panel is polled from the page, so the fragment endpoint carries a
//! timestamp showing when it last refreshed. Unlike the home-page schedule
//! there is no cached fallback — partner OPR/DPR is the point of the panel and
//! the cached `matches` rows do not carry line-ups — so an unreachable FIRST
//! API shows as an error line.
//!
//! Requires [`crate::models::User::can_coach`], and a team number: without one
//! there is no "our alliance" to report.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use chrono::Local;
use chrono_tz::Tz;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};

use crate::models::{event, stats};
use crate::services::connectivity;
use crate::services::first_api::FirstApiClient;
use crate::state::SharedState;
use crate::views::coach::{
    build_schedule, CoachViewerTemplate, DriveCoachMatchesFragment, DriveCoachSummary,
    NO_EVENT_INFO,
};
use crate::views::{render, render_html, Nav};
use crate::web::*;

const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// `GET /drive-coach` — the panel, with the schedule already rendered.
pub async fn coach_viewer(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.can_coach() {
        return Ok(redirect("/"));
    }

    let selected_event_id = selected_event_id(&state.pool, &jar).await;

    let event_selection_html = super::event_selection_html(
        &state.pool,
        Some(&user),
        selected_event_id,
        "",
        false,
        true,
    )
    .await;

    let mut summary = DriveCoachSummary::default();
    let matches_html = match selected_event_id {
        None => render_html(&DriveCoachMatchesFragment::info(
            String::new(),
            NO_EVENT_INFO,
        )),
        Some(event_id) => {
            let (fragment, s) =
                load_schedule(&state.pool, event_id, user.team_number, String::new()).await;
            summary = s;
            render_html(&fragment)
        }
    };

    Ok(render(&CoachViewerTemplate::new(
        Nav::from_user(Some(&user)),
        summary,
        event_selection_html,
        matches_html,
    )))
}

/// `GET /hx/drive-coach/matches` — the match list alone, for the poll.
pub async fn drive_coach_matches(
    State(state): State<SharedState>,
    jar: CookieJar,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok((StatusCode::UNAUTHORIZED, "Authentication required").into_response());
    };
    if !user.can_coach() {
        return Ok((StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let updated_at = Local::now().format("%-I:%M:%S %p").to_string();

    let fragment = match selected_event_id(&state.pool, &jar).await {
        None => DriveCoachMatchesFragment::info(updated_at, NO_EVENT_INFO),
        Some(event_id) => {
            let (fragment, _) =
                load_schedule(&state.pool, event_id, user.team_number, updated_at).await;
            fragment
        }
    };

    Ok(render(&fragment).into_response())
}

/// Fetches the team's schedule and turns it into the matches fragment.
///
/// Any failure becomes an error line inside the fragment, so the surrounding
/// page still renders.
async fn load_schedule(
    pool: &PgPool,
    event_id: i32,
    user_team_number: Option<i32>,
    updated_at: String,
) -> (DriveCoachMatchesFragment, DriveCoachSummary) {
    match load_matches(pool, event_id, user_team_number).await {
        Ok((matches, summary)) => (
            DriveCoachMatchesFragment::matches(updated_at, matches),
            summary,
        ),
        Err(e) => (
            DriveCoachMatchesFragment::error(updated_at, e),
            DriveCoachSummary::default(),
        ),
    }
}

/// Loaded schedule and its summary, or a message to show the coach.
type ScheduleResult = Result<(Vec<crate::views::coach::DriveCoachMatch>, DriveCoachSummary), String>;

/// Pulls the team's schedule from FIRST and joins it with locally synced
/// OPR/DPR for every team appearing in it.
async fn load_matches(pool: &PgPool, event_id: i32, user_team_number: Option<i32>) -> ScheduleResult {
    let Some(user_team_number) = user_team_number else {
        return Err(
            "Your account is missing a team number. Add one in Account settings.".to_string(),
        );
    };

    let Some(source) = event::find_schedule_source(pool, event_id).await else {
        return Err("Unable to load selected event".to_string());
    };
    let Some(tba_key) = source.tba_key.clone().filter(|k| !k.is_empty()) else {
        return Err("Selected event is missing schedule data".to_string());
    };

    let event_code = extract_event_code(&tba_key);
    if event_code.is_empty() {
        return Err("Unable to determine event code for schedule lookup".to_string());
    }

    let Some(client) = FirstApiClient::from_environment() else {
        return Err("FIRST API credentials are not configured".to_string());
    };
    let season = FirstApiClient::season_from_environment();

    let mut filters = HashMap::new();
    if user_team_number > 0 {
        filters.insert("teamNumber".to_string(), user_team_number.to_string());
    } else {
        filters.insert("tournamentLevel".to_string(), "Qualification".to_string());
    }

    let fetch = client.get_match_schedule(season, &event_code, &filters);
    let raw_matches = match tokio::time::timeout(FETCH_TIMEOUT, fetch).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            if connectivity::is_internet_unavailable(&e) {
                return Err("No internet connection available. Connect network uplink and try manual sync again.".to_string());
            }
            return Err("Could not fetch match schedule from FIRST API".to_string());
        }
        Err(_) => return Err("Could not fetch match schedule from FIRST API".to_string()),
    };

    // OPR/DPR for every team appearing in the schedule.
    let team_numbers: Vec<i32> = raw_matches
        .iter()
        .flat_map(|m| m.teams.iter().map(|t| t.team_number))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let stats_by_team: HashMap<i32, (Option<f64>, Option<f64>)> =
        stats::opr_dpr_by_team_number(pool, event_id, &team_numbers)
            .await
            .into_iter()
            .map(|(team_number, opr, dpr)| (team_number, (opr, dpr)))
            .collect();

    let event_tz: Option<Tz> = source.timezone.as_deref().and_then(|t| t.parse().ok());

    Ok(build_schedule(
        &raw_matches,
        &stats_by_team,
        event_tz,
        source.name,
        user_team_number,
    ))
}
