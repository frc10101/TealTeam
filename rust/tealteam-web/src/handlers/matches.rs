// Match schedule HTMX fragment (port of Controllers/MatchesController.cs).

use askama::Template;
use axum::extract::State;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Local, Utc};
use std::collections::HashMap;
use tracing::{error, warn};

use super::*;
use crate::connectivity;
use crate::first_api::FirstApiClient;
use crate::state::SharedState;

pub struct MatchDisplay {
    pub description: String,
    pub time_display: String,
    pub time_status: String,
    pub minutes_until: i64,
    pub red_teams: Vec<i32>,
    pub blue_teams: Vec<i32>,
}

#[derive(Template)]
#[template(path = "partials/match_schedule.html")]
pub struct MatchScheduleFragment {
    pub message: String,
    pub matches: Vec<MatchDisplay>,
}

pub async fn match_schedule(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let msg = |m: &str| {
        render(&MatchScheduleFragment {
            message: m.to_string(),
            matches: Vec::new(),
        })
    };

    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(msg("Sign in to view match schedule."));
    };
    let Some(session) = current_session(&state.pool, &jar).await else {
        return Ok(msg("Session error. Please refresh the page."));
    };
    let Some(event_id) = session.selected_event_id else {
        return Ok(msg("Select an event to view live matches."));
    };

    let event_row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT name, tba_key FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let Some((event_name, tba_key)) = event_row else {
        return Ok(msg("Unable to load selected event."));
    };
    let Some(tba_key) = tba_key.filter(|k| !k.is_empty()) else {
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
    match user.team_number.filter(|n| *n > 0) {
        Some(n) => {
            filters.insert("teamNumber".to_string(), n.to_string());
        }
        None => {
            filters.insert("tournamentLevel".to_string(), "Qualification".to_string());
        }
    }

    let fetch =
        client.get_match_schedule(season, &event_code, &filters);
    let raw_matches = match tokio::time::timeout(std::time::Duration::from_secs(15), fetch).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            error!("failed to fetch match schedule for {event_code}: {e}");
            let stale = load_stale_matches(&state.pool, event_id).await;
            if !stale.is_empty() {
                let message = if connectivity::is_internet_unavailable(&e) {
                    "Offline: showing cached schedule from local DB."
                } else {
                    "Using cached schedule data from local DB."
                };
                return Ok(render(&MatchScheduleFragment {
                    message: message.to_string(),
                    matches: stale,
                }));
            }
            let message = if connectivity::is_internet_unavailable(&e) {
                "No internet connection available. Connect uplink and retry manual sync."
            } else {
                "Could not fetch match schedule from FIRST API."
            };
            return Ok(msg(message));
        }
        Err(_) => {
            let stale = load_stale_matches(&state.pool, event_id).await;
            if !stale.is_empty() {
                return Ok(render(&MatchScheduleFragment {
                    message: "Using cached schedule data from local DB.".to_string(),
                    matches: stale,
                }));
            }
            return Ok(msg("Could not fetch match schedule from FIRST API."));
        }
    };

    // Only show matches within a ±15 minute window.
    let now = Local::now();
    let window_start = now - chrono::Duration::minutes(15);
    let window_end = now + chrono::Duration::minutes(15);

    let mut display: Vec<(DateTime<Local>, MatchDisplay)> = Vec::new();
    for m in &raw_matches {
        let Ok(start_time) = DateTime::parse_from_rfc3339(&m.start_time) else {
            warn!("failed to parse match start time: match {} time {}", m.match_number, m.start_time);
            continue;
        };
        let start_time = start_time.with_timezone(&Local);
        if start_time < window_start || start_time > window_end {
            continue;
        }

        let mut red = Vec::new();
        let mut blue = Vec::new();
        for mt in &m.teams {
            if mt.station.starts_with("Red") {
                red.push(mt.team_number);
            } else if mt.station.starts_with("Blue") {
                blue.push(mt.team_number);
            }
        }

        let minutes_until = (start_time - now).num_minutes();
        let time_status = if minutes_until < -15 {
            "past"
        } else if minutes_until <= 5 {
            "current"
        } else {
            "upcoming"
        };

        display.push((
            start_time,
            MatchDisplay {
                description: m.description.clone(),
                time_display: start_time.format("%a %b %-d, %-I:%M %p").to_string(),
                time_status: time_status.to_string(),
                minutes_until,
                red_teams: red,
                blue_teams: blue,
            },
        ));
    }

    display.sort_by_key(|(t, _)| *t);
    let matches: Vec<MatchDisplay> = display.into_iter().map(|(_, m)| m).collect();

    let _ = event_name;
    Ok(render(&MatchScheduleFragment {
        message: String::new(),
        matches,
    }))
}

async fn load_stale_matches(pool: &sqlx::PgPool, event_id: i32) -> Vec<MatchDisplay> {
    let rows: Vec<(i32, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT match_number, scheduled_time FROM matches WHERE event_id = $1 ORDER BY scheduled_time",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let now = Utc::now();
    rows.into_iter()
        .map(|(match_number, scheduled_time)| {
            let mut item = MatchDisplay {
                description: format!("Match {match_number}"),
                time_display: String::new(),
                time_status: "upcoming".to_string(),
                minutes_until: 0,
                red_teams: Vec::new(),
                blue_teams: Vec::new(),
            };
            if let Some(scheduled) = scheduled_time {
                item.time_display = scheduled
                    .with_timezone(&Local)
                    .format("%a %b %-d, %-I:%M %p")
                    .to_string();
                let minutes_until = (scheduled - now).num_minutes();
                item.minutes_until = minutes_until;
                item.time_status = if minutes_until < -15 {
                    "past".to_string()
                } else if minutes_until <= 5 {
                    "current".to_string()
                } else {
                    "upcoming".to_string()
                };
            }
            item
        })
        .collect()
}
