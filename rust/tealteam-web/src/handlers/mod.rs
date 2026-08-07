// Shared handler plumbing: router, error type, HTMX helpers, and the
// event-selection/summary hydration used across pages (port of
// Controllers/AppController.cs).

pub mod api;
pub mod assignments;
pub mod auth;
pub mod coach;
pub mod db_viewer;
pub mod lead_scout;
pub mod matches;
pub mod pages;
pub mod submission;
pub mod teams;

use std::collections::HashMap;

use askama::Template;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;

use crate::models::User;
use crate::state::SharedState;

pub fn router(state: SharedState) -> Router {
    Router::new()
        // Pages
        .route("/", get(pages::index))
        .route("/help", get(pages::help))
        .route("/api/events/select", post(pages::select_event))
        .route("/hx/events/summary", get(pages::event_summary))
        // Auth
        .route("/sign-in", get(auth::sign_in_page))
        .route("/sign-up", get(auth::sign_up_page))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/signup", post(auth::signup))
        .route("/api/auth/logout", post(auth::logout))
        .route("/account", get(auth::account_page))
        .route("/api/account/change-password", post(auth::change_password))
        // Scouting submission
        .route("/submission", get(submission::submission_page).post(submission::submit))
        .route("/submission/event-teams", get(submission::event_teams))
        // Lead scout
        .route("/lead-scout", get(lead_scout::admin_viewer))
        .route("/lead-scout/submissions/:id", get(lead_scout::view_submission))
        .route("/hx/lead-scout/submissions/:id/approve", post(lead_scout::approve_submission))
        .route("/hx/lead-scout/submissions/:id/decline", post(lead_scout::decline_submission))
        .route("/lead-scout/weights", get(lead_scout::weights_page).post(lead_scout::weights_update))
        // Assignments
        .route("/lead-scout/assignments", get(assignments::assignments_page))
        .route("/hx/assignments/set", post(assignments::set_assignment))
        .route("/hx/assignments/auto", post(assignments::auto_distribute))
        .route("/hx/assignments/clear-all", post(assignments::clear_all))
        .route("/hx/assignments/clear-match/:match_id", post(assignments::clear_match))
        .route("/hx/devices/:id/rename", post(assignments::rename_device))
        .route("/api/device/heartbeat", post(assignments::heartbeat))
        // Teams
        .route("/teams", get(teams::team_page))
        .route("/hx/teams/search", get(teams::team_search))
        .route("/hx/teams/fetch-past-events", post(teams::fetch_past_events))
        .route("/hx/teams/data", get(teams::team_event_data))
        // Matches / coach
        .route("/hx/matches/schedule", get(matches::match_schedule))
        .route("/drive-coach", get(coach::coach_viewer))
        .route("/hx/drive-coach/matches", get(coach::drive_coach_matches))
        // DB viewer
        .route("/development/db", get(db_viewer::db_viewer))
        .route("/hx/development/db/table/:name", get(db_viewer::table_content))
        // JSON APIs
        .route("/api/frc/sync", post(api::frc_sync))
        .route("/api/pick-list", get(api::get_pick_list))
        .route("/api/pick-list/entry", post(api::save_pick_list_entry))
        .route("/api/pick-list/entry", delete(api::delete_pick_list_entry))
        .route("/hx/network/status", get(api::network_status_badge))
        .route("/api/network/status", get(api::network_status))
        .with_state(state)
}

// ── Error type ────────────────────────────────────────────────────────────

pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("handler error: {:#}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("internal error: {}", self.0)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

pub type HandlerResult = Result<Response, AppError>;

// ── Rendering / HTMX helpers ──────────────────────────────────────────────

pub fn render<T: Template>(t: &T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("template render failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("template error: {e}")).into_response()
        }
    }
}

pub fn render_html<T: Template>(t: &T) -> String {
    t.render().unwrap_or_else(|e| {
        tracing::error!("template render failed: {e}");
        String::new()
    })
}

pub fn is_htmx(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .map(|v| v.as_bytes() == b"true")
        .unwrap_or(false)
}

pub fn redirect(to: &str) -> Response {
    Redirect::to(to).into_response()
}

pub fn hx_redirect_header(response: Response, to: &str) -> Response {
    let mut response = response;
    if let Ok(value) = to.parse() {
        response.headers_mut().insert("HX-Redirect", value);
    }
    response
}

/// HTML fragments returned by the auth endpoints (port of sendAuthResponse).
pub fn auth_response(success: bool, message: &str, hx_redirect: &str) -> Response {
    let encoded = html_escape::encode_text(message);
    let html = if success {
        format!(
            r##"<div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded mb-4" role="alert">
    <div class="flex items-center gap-2">
        <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
        </svg>
        <span class="block sm:inline font-medium">{encoded}</span>
    </div>
</div>"##
        )
    } else {
        format!(
            r##"<div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded mb-4" role="alert">
    <div class="flex items-center gap-2">
        <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
        </svg>
        <span class="block sm:inline font-medium">{encoded}</span>
    </div>
</div>"##
        )
    };

    let mut response = Html(html).into_response();
    if !hx_redirect.is_empty() {
        if let Ok(value) = hx_redirect.parse() {
            response.headers_mut().insert("HX-Redirect", value);
        }
    }
    response
}

// ── Form parsing ──────────────────────────────────────────────────────────

pub fn form_map(body: &str) -> HashMap<String, String> {
    serde_urlencoded::from_str(body).unwrap_or_default()
}

pub fn form_multi(body: &str) -> Vec<(String, String)> {
    serde_urlencoded::from_str(body).unwrap_or_default()
}

pub fn form_str<'a>(form: &'a HashMap<String, String>, key: &str) -> &'a str {
    form.get(key).map(|s| s.as_str()).unwrap_or("")
}

pub fn parse_required_int(value: &str) -> Option<i32> {
    value.trim().parse().ok()
}

/// TBA key format: {year}{event_code}, e.g. "2026mndu" -> "mndu".
/// FIRST API expects lowercase event codes.
pub fn extract_event_code(tba_key: &str) -> String {
    let mut chars = tba_key.chars();
    let prefix: String = chars.by_ref().take(4).collect();
    if prefix.len() == 4 && prefix.chars().all(|c| c.is_ascii_digit()) {
        let rest: String = chars.collect();
        if !rest.is_empty() {
            return rest.to_lowercase();
        }
    }
    String::new()
}

// ── Nav (layout chrome shared by every page) ──────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub signed_in: bool,
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
}

impl Nav {
    pub fn from_user(user: Option<&User>) -> Self {
        match user {
            Some(u) => Self {
                signed_in: true,
                is_admin: u.is_admin,
                is_lead_scout: u.is_lead_scout,
                is_coach: u.is_coach,
            },
            None => Self::default(),
        }
    }

    pub fn show_lead(&self) -> bool {
        self.signed_in && (self.is_admin || self.is_lead_scout)
    }

    pub fn show_coach(&self) -> bool {
        self.signed_in && (self.is_admin || self.is_coach)
    }
}

// ── Event selection / summary hydration ───────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventOption {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventTeamRow {
    pub team_number: i32,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventSummaryData {
    pub has_event: bool,
    pub error: String,
    pub event_name: String,
    pub event_id: i32,
    pub teams_count: i64,
    pub matches_count: i64,
    pub teams: Vec<EventTeamRow>,
    pub warning: String,
}

#[derive(Debug, Clone, Default)]
pub struct EventSelectionData {
    pub signed_in: bool,
    pub events: Vec<EventOption>,
    pub selected_event_id: Option<i32>,
    pub event_error: String,
    pub event_updated: bool,
}

impl EventSelectionData {
    /// Askama passes loop-variable fields by reference, hence `&i32`.
    pub fn is_selected(&self, id: &i32) -> bool {
        self.selected_event_id.as_ref() == Some(id)
    }
}

/// Events available to a user: their team's events, or all events for
/// team-less users.
pub async fn available_event_ids(pool: &PgPool, user: &User) -> anyhow::Result<Vec<i32>> {
    if let Some(team_number) = user.team_number.filter(|n| *n > 0) {
        return events_for_team(pool, team_number).await;
    }

    Ok(sqlx::query_scalar("SELECT id FROM events ORDER BY start_date")
        .fetch_all(pool)
        .await?)
}

pub async fn events_for_team(pool: &PgPool, team_number: i32) -> anyhow::Result<Vec<i32>> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT event_teams.event_id
         FROM event_teams
         JOIN teams ON teams.id = event_teams.team_id
         WHERE teams.team_number = $1",
    )
    .bind(team_number)
    .fetch_all(pool)
    .await?)
}

pub async fn load_event_options(pool: &PgPool, event_ids: &[i32]) -> Vec<EventOption> {
    if event_ids.is_empty() {
        return Vec::new();
    }
    sqlx::query_as("SELECT id, name FROM events WHERE id = ANY($1) ORDER BY start_date")
        .bind(event_ids)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn load_event_selection(
    pool: &PgPool,
    user: Option<&User>,
    selected_event_id: Option<i32>,
) -> EventSelectionData {
    let Some(user) = user else {
        return EventSelectionData::default();
    };

    let mut data = EventSelectionData {
        signed_in: true,
        selected_event_id,
        ..Default::default()
    };

    let event_ids = available_event_ids(pool, user).await.unwrap_or_default();
    data.events = load_event_options(pool, &event_ids).await;
    data
}

pub async fn load_event_summary(
    pool: &PgPool,
    event_id: i32,
    viewer: Option<&User>,
) -> EventSummaryData {
    let mut data = EventSummaryData {
        has_event: true,
        event_id,
        ..Default::default()
    };

    let event_name: Option<String> = sqlx::query_scalar("SELECT name FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    data.event_name = event_name.unwrap_or_default();

    data.teams_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM event_teams WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    data.matches_count = sqlx::query_scalar("SELECT COUNT(*) FROM matches WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    data.teams = sqlx::query_as(
        "SELECT teams.team_number, teams.name
         FROM event_teams
         JOIN teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1
         ORDER BY teams.team_number",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    if let Some(team_number) = viewer.and_then(|u| u.team_number) {
        let user_team_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM event_teams
             JOIN teams ON teams.id = event_teams.team_id
             WHERE event_teams.event_id = $1 AND teams.team_number = $2",
        )
        .bind(event_id)
        .bind(team_number)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if user_team_count == 0 {
            data.warning = "Your team is not listed for this event yet.".to_string();
        }
    }

    data
}

// ── Session convenience wrappers ──────────────────────────────────────────

pub async fn current_user(pool: &PgPool, jar: &CookieJar) -> Option<User> {
    crate::session::get_session_user(pool, jar).await
}

pub async fn current_session(pool: &PgPool, jar: &CookieJar) -> Option<crate::models::Session> {
    crate::session::get_session(pool, jar).await
}
