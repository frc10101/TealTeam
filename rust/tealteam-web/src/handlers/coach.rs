// Drive coach panel (port of Controllers/CoachController.cs).

use askama::Template;
use axum::extract::State;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, FixedOffset, Local, Utc};
use chrono_tz::Tz;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::error;

use super::*;
use crate::connectivity;
use crate::first_api::FirstApiClient;
use crate::state::SharedState;

pub struct DriveCoachTeam {
    pub team_number: i32,
    pub opr: Option<f64>,
    pub dpr: Option<f64>,
}

impl DriveCoachTeam {
    pub fn opr_display(&self) -> String {
        self.opr.map(|v| format!("{v:.1}")).unwrap_or_else(|| "0.0".to_string())
    }
    pub fn dpr_display(&self) -> String {
        self.dpr.map(|v| format!("{v:.1}")).unwrap_or_else(|| "0.0".to_string())
    }
}

pub struct DriveCoachMatch {
    pub description: String,
    pub match_number: i32,
    pub time_display: String,
    pub status_label: String,
    pub status_class: String,
    pub badge_class: String,
    pub red_teams: Vec<DriveCoachTeam>,
    pub blue_teams: Vec<DriveCoachTeam>,
    pub our_alliance: String,
    pub our_partners: Vec<i32>,
    pub start_time_sort: i64,
}

#[derive(Default)]
pub struct DriveCoachSummary {
    pub present: bool,
    pub event_name: String,
    pub team_number: i32,
    pub current_matches: i32,
    pub upcoming_matches: i32,
    pub completed_matches: i32,
}

#[derive(Template)]
#[template(path = "pages/coach_viewer.html")]
struct CoachViewerTemplate {
    title: String,
    description: String,
    nav: Nav,
    summary: DriveCoachSummary,
    event_selection_html: String,
    matches_html: String,
}

#[derive(Template)]
#[template(path = "partials/drive_coach_matches.html")]
pub struct DriveCoachMatchesFragment {
    pub updated_at: String,
    pub error: String,
    pub info: String,
    pub matches: Vec<DriveCoachMatch>,
}

pub async fn coach_viewer(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.is_admin && !user.is_coach {
        return Ok(redirect("/"));
    }

    let session = current_session(&state.pool, &jar).await;
    let selected_event_id = session.and_then(|s| s.selected_event_id);

    let event_selection_html = crate::handlers::pages::render_event_selection(
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
        None => render_html(&DriveCoachMatchesFragment {
            updated_at: String::new(),
            error: String::new(),
            info: "Select an event above to load your team schedule.".to_string(),
            matches: Vec::new(),
        }),
        Some(event_id) => {
            let (fragment, s) = load_drive_coach(&state.pool, event_id, user.team_number, "").await;
            summary = s;
            render_html(&fragment)
        }
    };

    Ok(render(&CoachViewerTemplate {
        title: "Drive Coach Panel".to_string(),
        description: "Quick match schedule and alliance partners.".to_string(),
        nav: Nav::from_user(Some(&user)),
        summary,
        event_selection_html,
        matches_html,
    }))
}

pub async fn drive_coach_matches(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok((axum::http::StatusCode::UNAUTHORIZED, "Authentication required").into_response());
    };
    if !user.is_admin && !user.is_coach {
        return Ok((axum::http::StatusCode::FORBIDDEN, "Access denied").into_response());
    }

    let updated_at = Local::now().format("%-I:%M:%S %p").to_string();
    let session = current_session(&state.pool, &jar).await;

    let fragment = match session.and_then(|s| s.selected_event_id) {
        None => DriveCoachMatchesFragment {
            updated_at,
            error: String::new(),
            info: "Select an event above to load your team schedule.".to_string(),
            matches: Vec::new(),
        },
        Some(event_id) => {
            let (mut fragment, _) =
                load_drive_coach(&state.pool, event_id, user.team_number, &updated_at).await;
            fragment.updated_at = updated_at;
            fragment
        }
    };

    Ok(render(&fragment).into_response())
}

async fn load_drive_coach(
    pool: &PgPool,
    event_id: i32,
    user_team_number: Option<i32>,
    updated_at: &str,
) -> (DriveCoachMatchesFragment, DriveCoachSummary) {
    let mut fragment = DriveCoachMatchesFragment {
        updated_at: updated_at.to_string(),
        error: String::new(),
        info: String::new(),
        matches: Vec::new(),
    };
    let mut summary = DriveCoachSummary::default();

    match load_matches(pool, event_id, user_team_number).await {
        Ok((matches, s)) => {
            if matches.is_empty() {
                fragment.info =
                    "No matches were returned for your team at the selected event yet.".to_string();
            }
            fragment.matches = matches;
            summary = s;
        }
        Err(e) => fragment.error = e,
    }

    (fragment, summary)
}

async fn load_matches(
    pool: &PgPool,
    event_id: i32,
    user_team_number: Option<i32>,
) -> Result<(Vec<DriveCoachMatch>, DriveCoachSummary), String> {
    let Some(user_team_number) = user_team_number else {
        return Err("Your account is missing a team number. Add one in Account settings.".to_string());
    };

    let event_row: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT name, tba_key, timezone FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    let Some((event_name, tba_key, timezone)) = event_row else {
        return Err("Unable to load selected event".to_string());
    };
    let Some(tba_key) = tba_key.filter(|k| !k.is_empty()) else {
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
    let raw_matches = match tokio::time::timeout(std::time::Duration::from_secs(15), fetch).await {
        Ok(Ok(m)) => m,
        Ok(Err(e)) => {
            if connectivity::is_internet_unavailable(&e) {
                return Err("No internet connection available. Connect network uplink and try manual sync again.".to_string());
            }
            return Err("Could not fetch match schedule from FIRST API".to_string());
        }
        Err(_) => return Err("Could not fetch match schedule from FIRST API".to_string()),
    };

    // Load OPR/DPR stats for all teams in the schedule.
    let team_numbers: Vec<i32> = {
        let mut set = std::collections::HashSet::new();
        for m in &raw_matches {
            for t in &m.teams {
                set.insert(t.team_number);
            }
        }
        set.into_iter().collect()
    };
    let mut stats_by_team: HashMap<i32, (Option<f64>, Option<f64>)> = HashMap::new();
    if !team_numbers.is_empty() {
        let rows: Vec<(i32, Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT teams.team_number, team_event_stats.opr::float8, team_event_stats.dpr::float8
             FROM teams
             LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
                 AND team_event_stats.event_id = $1
             WHERE teams.team_number = ANY($2)",
        )
        .bind(event_id)
        .bind(&team_numbers)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (team_number, opr, dpr) in rows {
            stats_by_team.insert(team_number, (opr, dpr));
        }
    }

    let event_tz: Option<Tz> = timezone.as_deref().and_then(|t| t.parse().ok());

    let now = Utc::now();
    let mut results = Vec::new();
    let (mut current, mut upcoming, mut completed) = (0, 0, 0);

    for m in &raw_matches {
        let start_time = parse_match_time(&m.start_time, event_tz);

        let mut entry = DriveCoachMatch {
            description: m.description.clone(),
            match_number: m.match_number,
            time_display: String::new(),
            status_label: "Upcoming".to_string(),
            status_class: "border-teal-600 bg-teal-900/20".to_string(),
            badge_class: "bg-teal-700 text-white".to_string(),
            red_teams: Vec::new(),
            blue_teams: Vec::new(),
            our_alliance: String::new(),
            our_partners: Vec::new(),
            start_time_sort: i64::MAX,
        };

        if let Some(start) = start_time {
            entry.time_display = start.with_timezone(&Local).format("%a %b %-d, %-I:%M %p").to_string();
            entry.start_time_sort = start.timestamp();
            let minutes_until = (start.with_timezone(&Utc) - now).num_minutes();
            if minutes_until < -15 {
                entry.status_label = "Completed".to_string();
                entry.status_class = "border-gray-700 bg-gray-900/40".to_string();
                entry.badge_class = "bg-gray-700 text-gray-200".to_string();
                completed += 1;
            } else if minutes_until <= 15 {
                entry.status_label = "Current Match".to_string();
                entry.status_class = "border-yellow-500 bg-yellow-900/20 ring-2 ring-yellow-500/40".to_string();
                entry.badge_class = "bg-yellow-600 text-white".to_string();
                current += 1;
            } else {
                upcoming += 1;
            }
        } else {
            error!("failed to parse match start time: match {} raw {}", m.match_number, m.start_time);
        }

        for mt in &m.teams {
            let (opr, dpr) = stats_by_team.get(&mt.team_number).copied().unwrap_or((None, None));
            let team = DriveCoachTeam { team_number: mt.team_number, opr, dpr };
            if mt.station.starts_with("Red") {
                entry.red_teams.push(team);
                if mt.team_number == user_team_number {
                    entry.our_alliance = "Red".to_string();
                }
            } else if mt.station.starts_with("Blue") {
                entry.blue_teams.push(team);
                if mt.team_number == user_team_number {
                    entry.our_alliance = "Blue".to_string();
                }
            }
        }

        let our_teams = match entry.our_alliance.as_str() {
            "Red" => Some(&entry.red_teams),
            "Blue" => Some(&entry.blue_teams),
            _ => None,
        };
        if let Some(our_teams) = our_teams {
            entry.our_partners = our_teams
                .iter()
                .filter(|t| t.team_number != user_team_number)
                .map(|t| t.team_number)
                .collect();
        }

        results.push(entry);
    }

    results.sort_by_key(|m| m.start_time_sort);

    let summary = DriveCoachSummary {
        present: true,
        event_name,
        team_number: user_team_number,
        current_matches: current,
        upcoming_matches: upcoming,
        completed_matches: completed,
    };

    Ok((results, summary))
}

/// Parse a FIRST schedule time. RFC3339 with offset wins; otherwise interpret
/// a naive datetime in the event timezone.
fn parse_match_time(raw: &str, event_tz: Option<Tz>) -> Option<DateTime<FixedOffset>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        if let Some(tz) = event_tz {
            use chrono::TimeZone;
            if let chrono::LocalResult::Single(dt) = tz.from_local_datetime(&naive) {
                return Some(dt.fixed_offset());
            }
        }
        // Fall back to local time interpretation.
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(dt) = Local.from_local_datetime(&naive) {
            return Some(dt.fixed_offset());
        }
    }
    None
}
