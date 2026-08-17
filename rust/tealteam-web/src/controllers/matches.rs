//! Live match schedule fragment, refreshed from the FIRST API with the local
//! database as an offline fallback.
//!
//! This runs on a timer on the home page during an event, so it never fails
//! loudly: every problem — signed out, no event selected, no credentials, no
//! internet — becomes a sentence in the panel. When FIRST cannot be reached
//! but matches have been synced from TBA, the cached schedule is shown with a
//! note saying so, which is the difference between a useless panel and a
//! slightly stale one in a venue with no uplink.

use axum::extract::State;
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use tracing::error;

use crate::models::{assignment, event};
use crate::services::connectivity;
use crate::services::first_api::FirstApiClient;
use crate::state::SharedState;
use crate::views::matches::{MatchDisplay, MatchScheduleFragment};
use crate::views::render;
use crate::web::*;

const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// `GET /hx/matches/schedule` — matches starting around now.
///
/// The FIRST schedule endpoint requires at least one filter, so the request is
/// narrowed to the user's team, or to qualification matches for a user without
/// one.
pub async fn match_schedule(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let msg = |m: &str| render(&MatchScheduleFragment::message(m));

    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(msg("Sign in to view match schedule."));
    };
    let Some(session) = current_session(&state.pool, &jar).await else {
        return Ok(msg("Session error. Please refresh the page."));
    };
    let Some(event_id) = session.selected_event_id else {
        return Ok(msg("Select an event to view live matches."));
    };

    let Some(source) = event::find_schedule_source(&state.pool, event_id).await else {
        return Ok(msg("Unable to load selected event."));
    };
    let Some(tba_key) = source.tba_key.filter(|k| !k.is_empty()) else {
        return Ok(msg("Selected event is missing schedule data."));
    };

    let event_code = extract_event_code(&tba_key);
    if event_code.is_empty() {
        error!("failed to extract event code from TBA key {tba_key}");
        return Ok(msg("Unable to determine event code for schedule lookup."));
    }

    let Some(client) = FirstApiClient::from_environment() else {
        return Ok(msg("FIRST API credentials are not configured."));
    };
    let season = FirstApiClient::season_from_environment();

    // FIRST schedule endpoint requires at least one filter parameter.
    let mut filters = HashMap::new();
    match user.active_team_number() {
        Some(n) => {
            filters.insert("teamNumber".to_string(), n.to_string());
        }
        None => {
            filters.insert("tournamentLevel".to_string(), "Qualification".to_string());
        }
    }

    let fetch = client.get_match_schedule(season, &event_code, &filters);
    let raw_matches = match tokio::time::timeout(FETCH_TIMEOUT, fetch).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            error!("failed to fetch match schedule for {event_code}: {e}");
            let stale = cached_matches(&state.pool, event_id).await;
            if !stale.is_empty() {
                let message = if connectivity::is_internet_unavailable(&e) {
                    "Offline: showing cached schedule from local DB."
                } else {
                    "Using cached schedule data from local DB."
                };
                return Ok(render(&MatchScheduleFragment::with_matches(message, stale)));
            }
            let message = if connectivity::is_internet_unavailable(&e) {
                "No internet connection available. Connect uplink and retry manual sync."
            } else {
                "Could not fetch match schedule from FIRST API."
            };
            return Ok(msg(message));
        }
        Err(_) => {
            let stale = cached_matches(&state.pool, event_id).await;
            if !stale.is_empty() {
                return Ok(render(&MatchScheduleFragment::with_matches(
                    "Using cached schedule data from local DB.",
                    stale,
                )));
            }
            return Ok(msg("Could not fetch match schedule from FIRST API."));
        }
    };

    Ok(render(&MatchScheduleFragment::with_matches(
        "",
        MatchDisplay::from_schedule(&raw_matches),
    )))
}

/// Locally synced schedule, the fallback when FIRST is unreachable.
async fn cached_matches(pool: &sqlx::PgPool, event_id: i32) -> Vec<MatchDisplay> {
    MatchDisplay::from_cached(assignment::scheduled_times(pool, event_id).await)
}
