// Team lookup page and HTMX fragments (port of Controllers/TeamsController.cs).

use askama::Template;
use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::warn;

use super::*;
use crate::models::{ScoutingData, Team, TeamEventStats};
use crate::state::SharedState;

// ── View models ───────────────────────────────────────────────────────────

pub struct NoteEntry {
    pub note: String,
    pub scouted_display: String,
    pub match_index: usize,
}

/// Pre-formatted stats for display (empty string = value absent).
#[derive(Default)]
pub struct StatsView {
    pub present: bool,
    pub rank: String,
    pub record: String,
    pub matches: String,
    pub qual_average: String,
    pub avg_match_points: String,
    pub dq_count: String,
    pub opr: String,
    pub dpr: String,
    pub ccwm: String,
    pub auto_opr: String,
    pub teleop_opr: String,
    pub endgame_opr: String,
    pub qual_points: String,
    pub elim_points: String,
    pub award_points: String,
    pub alliance_points: String,
}

#[derive(Default)]
pub struct TeamDataView {
    pub has_data: bool,
    pub event_name: String,
    pub stats: Option<StatsView>,
    pub scouting_count: usize,
    pub most_common_start_pos: String,
    pub most_common_defense: String,
    pub most_common_traversal: String,
    pub most_common_scoring_strategy: String,
    pub most_common_hang_level: String,
    pub most_common_hang_position: String,
    pub most_common_accuracy: String,
    pub shooting_speed: String,
    pub capacity: String,
    pub defendability: String,
    pub scoring_strategy: String,
    pub notes: Vec<NoteEntry>,
    pub recent_alliances: Vec<String>,
}

// ── Templates ─────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/team.html")]
struct TeamPageTemplate {
    title: String,
    nav: Nav,
    team_search_value: String,
    team_error: String,
    team_info_html: String,
}

#[derive(Template)]
#[template(path = "partials/team_info.html")]
struct TeamInfoFragment {
    signed_in: bool,
    team: TeamView,
    events: Vec<EventOption>,
    selected_event_id: Option<i32>,
}

pub struct TeamView {
    pub team_number: i32,
    pub name: String,
    pub school_name: String,
    pub city_line: String,
    pub rookie_year: String,
    pub nickname: String,
    pub website: String,
    pub motto: String,
}

#[derive(Template)]
#[template(path = "partials/team_data.html")]
struct TeamDataFragment {
    v: TeamDataView,
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub async fn team_page(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let team = query.get("team").cloned().unwrap_or_default();

    let mut team_error = String::new();
    let mut team_info_html = String::new();

    if !team.is_empty() {
        match build_team_info(&state.pool, user.as_ref(), &team).await {
            Ok(html) => team_info_html = html,
            Err(msg) => team_error = msg,
        }
    }

    Ok(render(&TeamPageTemplate {
        title: "Teams".to_string(),
        nav: Nav::from_user(user.as_ref()),
        team_search_value: team,
        team_error,
        team_info_html,
    }))
}

pub async fn team_search(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let team = query.get("team").cloned().unwrap_or_default();

    match build_team_info(&state.pool, user.as_ref(), &team).await {
        Ok(html) => Ok(Html(html).into_response()),
        Err(msg) => Ok(Html(format!(
            r#"<div id="team-info-container"><div class="card"><div class="card-body text-center text-red-400 py-8">{}</div></div></div>"#,
            html_escape::encode_text(&msg)
        ))
        .into_response()),
    }
}

pub async fn fetch_past_events(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let form = form_map(&body);
    let team = form_str(&form, "team").trim().to_string();
    if team.is_empty() {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Team number is required").into_response());
    }
    let Ok(team_number) = team.parse::<i32>() else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Invalid team number").into_response());
    };

    if let Err(e) = crate::first_sync::sync_team_for_user(&state.pool, team_number).await {
        warn!("failed to fetch past events for team {team_number}: {e}");
    }

    match build_team_info(&state.pool, user.as_ref(), &team).await {
        Ok(html) => Ok(Html(html).into_response()),
        Err(msg) => Ok((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!(
                r#"<div id="team-info-container"><div class="card"><div class="card-body text-center text-red-400 py-8">{}</div></div></div>"#,
                html_escape::encode_text(&msg)
            )),
        )
            .into_response()),
    }
}

async fn build_team_info(
    pool: &PgPool,
    user: Option<&crate::models::User>,
    team_number_raw: &str,
) -> Result<String, String> {
    if team_number_raw.is_empty() {
        return Err("Team number is required".to_string());
    }
    let Ok(team_number) = team_number_raw.parse::<i32>() else {
        return Err("Invalid team number".to_string());
    };

    let team: Option<Team> = sqlx::query_as("SELECT * FROM teams WHERE team_number = $1 LIMIT 1")
        .bind(team_number)
        .fetch_optional(pool)
        .await
        .unwrap_or(None);
    let Some(team) = team else {
        return Err(format!("Team {team_number} not found"));
    };

    // Team events (sync from FIRST if none locally).
    let mut event_ids = events_for_team(pool, team_number).await.unwrap_or_default();
    if event_ids.is_empty() {
        if let Err(e) = crate::first_sync::sync_team_for_user(pool, team_number).await {
            warn!("failed to sync events for team {team_number}: {e}");
        } else {
            event_ids = events_for_team(pool, team_number).await.unwrap_or_default();
        }
    }
    let events = load_event_options(pool, &event_ids).await;

    let city_line = {
        let mut parts = Vec::new();
        if let Some(city) = team.city.as_deref().filter(|s| !s.is_empty()) {
            parts.push(city.to_string());
        }
        let mut line = parts.join("");
        if let Some(state) = team.state.as_deref().filter(|s| !s.is_empty()) {
            if line.is_empty() {
                line = state.to_string();
            } else {
                line = format!("{line}, {state}");
            }
        }
        if let Some(country) = team.country.as_deref().filter(|s| !s.is_empty()) {
            line = if line.is_empty() { country.to_string() } else { format!("{line} • {country}") };
        }
        line
    };

    let view = TeamView {
        team_number: team.team_number,
        name: team.name.clone(),
        school_name: team.school_name.clone().unwrap_or_default(),
        city_line,
        rookie_year: team.rookie_year.map(|y| y.to_string()).unwrap_or_default(),
        nickname: team.nickname.clone().unwrap_or_default(),
        website: team.website.clone().unwrap_or_default(),
        motto: team.motto.clone().unwrap_or_default(),
    };

    Ok(render_html(&TeamInfoFragment {
        signed_in: user.is_some(),
        team: view,
        events,
        selected_event_id: None,
    }))
}

pub async fn team_event_data(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let team = query.get("team").cloned().unwrap_or_default();
    let event_id_raw = query.get("event_id").cloned().unwrap_or_default();

    if team.is_empty() {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Team number is required").into_response());
    }
    let Ok(team_number) = team.parse::<i32>() else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Invalid team number").into_response());
    };
    if event_id_raw.is_empty() {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Event ID is required").into_response());
    }
    let Ok(event_id) = event_id_raw.parse::<i32>() else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Invalid event ID").into_response());
    };

    let team_record: Option<Team> =
        sqlx::query_as("SELECT * FROM teams WHERE team_number = $1 LIMIT 1")
            .bind(team_number)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None);
    let Some(team_record) = team_record else {
        return Ok((axum::http::StatusCode::NOT_FOUND, "Team not found").into_response());
    };

    // Resolve viewer's team id for note filtering.
    let mut viewer_team_id = 0;
    let user = current_user(&state.pool, &jar).await;
    if let Some(team_number) = user.and_then(|u| u.team_number) {
        viewer_team_id = sqlx::query_scalar("SELECT id FROM teams WHERE team_number = $1 LIMIT 1")
            .bind(team_number)
            .fetch_optional(&state.pool)
            .await
            .unwrap_or(None)
            .unwrap_or(0);
    }

    let v = hydrate_team_event_data(&state.pool, team_record.id, event_id, viewer_team_id).await;
    Ok(Html(render_html(&TeamDataFragment { v })).into_response())
}

async fn hydrate_team_event_data(
    pool: &PgPool,
    team_id: i32,
    event_id: i32,
    viewer_team_id: i32,
) -> TeamDataView {
    let mut v = TeamDataView::default();

    v.event_name = sqlx::query_scalar("SELECT name FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    let raw_stats = sqlx::query_as::<_, TeamEventStats>(&format!(
        "{} WHERE team_id = $1 AND event_id = $2 LIMIT 1",
        TeamEventStats::SELECT
    ))
    .bind(team_id)
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    v.stats = raw_stats.map(build_stats_view);

    let scouting_data: Vec<ScoutingData> = sqlx::query_as(
        "SELECT * FROM scouting_data WHERE team_id = $1 AND event_id = $2 ORDER BY scouted_at DESC",
    )
    .bind(team_id)
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    v.scouting_count = scouting_data.len();
    if scouting_data.is_empty() {
        v.has_data = v.stats.is_some();
        return v;
    }
    v.has_data = true;

    let most_common = |f: &dyn Fn(&ScoutingData) -> Option<String>| -> String {
        let mut counts: HashMap<String, i32> = HashMap::new();
        for sd in &scouting_data {
            if let Some(value) = f(sd).filter(|s| !s.is_empty()) {
                *counts.entry(value).or_insert(0) += 1;
            }
        }
        counts.into_iter().max_by_key(|(_, c)| *c).map(|(v, _)| v).unwrap_or_default()
    };

    v.most_common_start_pos = most_common(&|sd| sd.starting_position.clone());
    v.most_common_defense = most_common(&|sd| sd.defense_rating.clone());
    v.most_common_traversal = most_common(&|sd| sd.traversal.clone());
    v.most_common_scoring_strategy = most_common(&|sd| sd.scoring_strategy.clone());
    v.most_common_hang_level = most_common(&|sd| sd.hang_level.clone());
    v.most_common_hang_position = most_common(&|sd| sd.hang_position.clone());
    v.most_common_accuracy = most_common(&|sd| sd.accuracy_rating.clone());

    let latest = &scouting_data[0];
    v.shooting_speed = latest.shooting_speed.clone().unwrap_or_default();
    v.capacity = latest.capacity.clone().unwrap_or_default();
    v.defendability = latest.defendability.clone().unwrap_or_default();
    v.scoring_strategy = latest.scoring_strategy.clone().unwrap_or_default();

    // Notes only from the viewer's own team.
    for (i, sd) in scouting_data.iter().enumerate() {
        if let Some(notes) = sd.notes.as_deref().filter(|n| !n.is_empty()) {
            if viewer_team_id > 0 && sd.submitting_team_id == Some(viewer_team_id) {
                v.notes.push(NoteEntry {
                    note: notes.to_string(),
                    scouted_display: sd
                        .scouted_at
                        .map(|t| t.with_timezone(&chrono::Local).format("%b %-d %-I:%M %p").to_string())
                        .unwrap_or_default(),
                    match_index: i + 1,
                });
            }
        }
    }

    v.recent_alliances = scouting_data.iter().take(5).map(|sd| sd.alliance_color.clone()).collect();

    v
}

fn build_stats_view(s: TeamEventStats) -> StatsView {
    let f1 = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or_default();
    let f2 = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_default();
    let int = |v: Option<i32>| v.map(|x| x.to_string()).unwrap_or_default();

    let matches_played = s.matches_played.unwrap_or(0);
    let record = {
        let w = s.wins.unwrap_or(0);
        let l = s.losses.unwrap_or(0);
        let t = s.ties.unwrap_or(0);
        if t > 0 {
            format!("{w}W {l}L {t}T")
        } else {
            format!("{w}W {l}L")
        }
    };
    let dq = s.dq_count.unwrap_or(0);

    StatsView {
        present: true,
        rank: int(s.rank),
        record,
        matches: if matches_played > 0 { matches_played.to_string() } else { String::new() },
        qual_average: f1(s.qual_average),
        avg_match_points: f2(s.avg_match_points),
        dq_count: if dq > 0 { dq.to_string() } else { String::new() },
        opr: f2(s.opr),
        dpr: f2(s.dpr),
        ccwm: f2(s.ccwm),
        auto_opr: f1(s.auto_opr),
        teleop_opr: f1(s.teleop_opr),
        endgame_opr: f1(s.endgame_opr),
        qual_points: int(s.qual_points),
        elim_points: int(s.elim_points),
        award_points: int(s.award_points),
        alliance_points: int(s.alliance_points),
    }
}
