// Scouting submission page + form handling (port of
// Controllers/SubmissionController.cs).

use askama::Template;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{error, warn};

use super::*;
use crate::first_api::FirstApiClient;
use crate::models::User;
use crate::session;
use crate::state::SharedState;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AssignedTeam {
    pub team_id: i32,
    pub team_number: i32,
    pub team_name: String,
    pub event_id: i32,
    pub match_number: Option<i32>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TeamOption {
    pub id: i32,
    pub team_number: i32,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct ScoutingFormData {
    pub events: Vec<EventOption>,
    pub assigned_teams: Vec<AssignedTeam>,
    pub team_options: Vec<TeamOption>,
    pub prefill_event_id: Option<i32>,
    pub prefill_team_id: Option<i32>,
    pub prefill_match_number: Option<i32>,
}

impl ScoutingFormData {
    // Askama passes loop-variable fields by reference, hence `&i32`.
    pub fn is_event(&self, id: &i32) -> bool {
        self.prefill_event_id.as_ref() == Some(id)
    }
    pub fn is_team(&self, id: &i32) -> bool {
        self.prefill_team_id.as_ref() == Some(id)
    }
}

#[derive(Template)]
#[template(path = "partials/submission_panel.html")]
pub struct SubmissionPanelFragment {
    pub error: String,
    pub success: String,
    pub form: ScoutingFormData,
}

#[derive(Template)]
#[template(path = "pages/submission.html")]
struct SubmissionTemplate {
    title: String,
    nav: Nav,
    description: String,
    panel_html: String,
}

async fn build_form_data(pool: &PgPool, user: &User, jar: &CookieJar) -> ScoutingFormData {
    let mut data = ScoutingFormData::default();

    let session = crate::session::get_session(pool, jar).await;
    let selected_event_id = session.and_then(|s| s.selected_event_id);

    let event_ids = available_event_ids(pool, user).await.unwrap_or_default();
    data.events = load_event_options(pool, &event_ids).await;

    // Find the scout's next unplayed match assignment for the selected event
    // (matched by signed-in user OR by this device's permanent UUID).
    let device_uuid = jar
        .get(session::DEVICE_COOKIE_NAME)
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty());

    let Some(event_id) = selected_event_id else {
        return data;
    };
    data.prefill_event_id = Some(event_id);

    data.assigned_teams = sqlx::query_as(
        "SELECT sa.team_id, teams.team_number, teams.name AS team_name,
                m.event_id, m.match_number
         FROM scout_assignments sa
         JOIN matches m ON m.id = sa.match_id
         JOIN teams ON teams.id = sa.team_id
         LEFT JOIN devices ON devices.id = sa.device_id
         WHERE m.event_id = $1
           AND m.played = FALSE
           AND (sa.scouter_id = $2 OR (devices.device_uuid = $3 AND $3 IS NOT NULL))
         ORDER BY m.match_number ASC",
    )
    .bind(event_id)
    .bind(user.id)
    .bind(device_uuid)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    data.team_options = sqlx::query_as(
        "SELECT teams.id, teams.team_number, teams.name
         FROM teams
         JOIN event_teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    if let Some(first) = data.assigned_teams.first() {
        data.prefill_team_id = Some(first.team_id);
        data.prefill_match_number = first.match_number;
    }

    data
}

async fn render_panel(pool: &PgPool, user: &User, jar: &CookieJar, error: &str, success: &str) -> String {
    let form = build_form_data(pool, user, jar).await;
    render_html(&SubmissionPanelFragment {
        error: error.to_string(),
        success: success.to_string(),
        form,
    })
}

pub async fn submission_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let panel_html = render_panel(&state.pool, &user, &jar, "", "").await;
    Ok(render(&SubmissionTemplate {
        title: "Scouting Submission".to_string(),
        nav: Nav::from_user(Some(&user)),
        description: "Submit scouting data for competitions".to_string(),
        panel_html,
    }))
}

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
        if is_htmx(&headers) {
            let html = render_panel(&state.pool, &user, &jar, error, "").await;
            return Ok(Html(html).into_response());
        }
        return Ok((StatusCode::BAD_REQUEST, error).into_response());
    }

    // Attribute the submission to the scout's team so their notes are visible
    // to teammates on the team data page.
    let submitting_team_id: Option<i32> = match user.team_number.filter(|n| *n > 0) {
        Some(team_number) => {
            sqlx::query_scalar("SELECT id FROM teams WHERE team_number = $1 LIMIT 1")
                .bind(team_number)
                .fetch_optional(&state.pool)
                .await
                .unwrap_or(None)
        }
        None => None,
    };

    let insert = sqlx::query(
        "INSERT INTO scouting_submissions (event_id, team_id, alliance_color, notes, starting_position,
            defense_rating, traversal, scoring_strategy, shooting_speed, capacity, defendability,
            hang_level, auto_hang, hang_position, scouted_at, scouter_id, submitting_team_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
    )
    .bind(event_id.unwrap())
    .bind(team_id.unwrap())
    .bind(&alliance_color)
    .bind(form_str(&form, "notes").trim())
    .bind(&starting_position)
    .bind(field("defense_rating"))
    .bind(field("traversal"))
    .bind(field("teleop_strategy"))
    .bind(field("shooting_speed"))
    .bind(field("capacity"))
    .bind(form_str(&form, "defendability").trim())
    .bind(field("hang_level"))
    .bind(field("auto_hang"))
    .bind(field("hang_position"))
    .bind(Utc::now())
    .bind(user.id)
    .bind(submitting_team_id)
    .execute(&state.pool)
    .await;

    if let Err(e) = insert {
        error!(
            "failed to create scouting submission (event {event_id:?}, team {team_id:?}, scouter {}): {e}",
            user.id
        );
        if is_htmx(&headers) {
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

    if is_htmx(&headers) {
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

/// Team options for a selected event, upserting from FIRST API when the local
/// DB is empty (returns raw <option> HTML).
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

    let event_tba_key: Option<String> =
        sqlx::query_scalar::<_, Option<String>>("SELECT tba_key FROM events WHERE id = $1")
            .bind(event_id)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            .flatten();

    let teams: Vec<TeamOption> = sqlx::query_as(
        "SELECT teams.id, teams.team_number, teams.name
         FROM teams
         JOIN event_teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut html = String::from(r#"<option value="" disabled selected>Select team</option>"#);

    if !teams.is_empty() {
        for team in &teams {
            html.push_str(&format!(
                r#"<option value="{}">{} - {}</option>"#,
                team.id,
                team.team_number,
                html_escape::encode_text(&team.name)
            ));
        }
        return Ok(Html(html).into_response());
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
        html.push_str(r#"<option value="" disabled>No teams available for this event</option>"#);
        return Ok(Html(html).into_response());
    }

    for first_team in &first_teams {
        match crate::first_sync::upsert_team(&state.pool, first_team).await {
            Ok(db_id) => {
                html.push_str(&format!(
                    r#"<option value="{}">{} - {}</option>"#,
                    db_id,
                    first_team.team_number,
                    html_escape::encode_text(&first_team.display_name())
                ));
            }
            Err(e) => warn!("failed to upsert team {}: {e}", first_team.team_number),
        }
    }

    Ok(Html(html).into_response())
}
