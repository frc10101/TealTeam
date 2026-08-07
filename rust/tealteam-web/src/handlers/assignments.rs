// Per-match robot assignments: the lead scout assigns a scout or device to
// each robot slot for each match (port of Controllers/AssignmentsController.cs).

use askama::Template;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Json};
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::error;

use super::*;
use crate::models::User;
use crate::session::DEVICE_COOKIE_NAME;
use crate::state::SharedState;

const ONLINE_WINDOW_MINUTES: i64 = 3;

// ── View model types ──────────────────────────────────────────────────────

pub struct MatchAssignmentRow {
    pub match_id: i32,
    pub match_number: i32,
    pub match_type: String,
    pub played: bool,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub red_slots: Vec<SlotAssignment>,
    pub blue_slots: Vec<SlotAssignment>,
}

impl MatchAssignmentRow {
    pub fn label(&self) -> String {
        match self.match_type.as_str() {
            "qm" | "" => format!("Q{}", self.match_number),
            "sf" => format!("SF{}", self.match_number),
            "f" => format!("F{}", self.match_number),
            _ => format!("M{}", self.match_number),
        }
    }

    pub fn scheduled_display(&self) -> String {
        self.scheduled_time
            .map(|t| t.with_timezone(&chrono::Local).format("%-I:%M %p").to_string())
            .unwrap_or_default()
    }

    pub fn assigned_count(&self) -> usize {
        self.red_slots.iter().chain(&self.blue_slots).filter(|s| s.assignment_id.is_some()).count()
    }

    pub fn total_count(&self) -> usize {
        self.red_slots.iter().chain(&self.blue_slots).filter(|s| s.team_id.is_some()).count()
    }
}

#[derive(Clone)]
pub struct SlotAssignment {
    pub position: i32,
    pub team_id: Option<i32>,
    pub team_number: i32,
    pub team_name: String,
    pub assignment_id: Option<i32>,
    pub scouter_id: Option<i32>,
    pub device_id: Option<i32>,
}

impl SlotAssignment {
    pub fn is_unassigned(&self) -> bool {
        self.scouter_id.is_none() && self.device_id.is_none()
    }

    pub fn is_scouter(&self, id: &i32) -> bool {
        self.scouter_id.as_ref() == Some(id)
    }

    pub fn is_device(&self, id: &i32) -> bool {
        self.device_id.as_ref() == Some(id)
    }
}

#[derive(Clone)]
pub struct ScoutOption {
    pub id: i32,
    pub name: String,
    pub online: bool,
}

#[derive(Clone)]
pub struct DeviceOption {
    pub id: i32,
    pub name: String,
    pub online: bool,
}

#[derive(Default)]
pub struct AssignmentData {
    pub selected_event_name: String,
    pub match_rows: Vec<MatchAssignmentRow>,
    pub scouts: Vec<ScoutOption>,
    pub devices: Vec<DeviceOption>,
    pub info: String,
}

// ── Templates ─────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/assignments.html")]
struct AssignmentsTemplate {
    title: String,
    description: String,
    nav: Nav,
    selected_event_id: Option<i32>,
    info: String,
    selected_event_name: String,
    device_list_html: String,
    assignment_table_html: String,
}

#[derive(Template)]
#[template(path = "partials/assignment_table.html")]
pub struct AssignmentTableFragment {
    pub data: AssignmentData,
}

#[derive(Template)]
#[template(path = "partials/device_list.html")]
pub struct DeviceListFragment {
    pub devices: Vec<DeviceOption>,
}

// ── Internal DB row types ─────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct MatchDbRow {
    id: i32,
    match_number: i32,
    match_type: String,
    played: bool,
    scheduled_time: Option<DateTime<Utc>>,
    red1: Option<i32>,
    red2: Option<i32>,
    red3: Option<i32>,
    blue1: Option<i32>,
    blue2: Option<i32>,
    blue3: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct AssignmentDbRow {
    id: i32,
    match_id: i32,
    team_id: i32,
    scouter_id: Option<i32>,
    device_id: Option<i32>,
}

#[derive(sqlx::FromRow, Clone)]
struct TeamLookupRow {
    team_number: i32,
    team_id: i32,
    team_name: String,
}

// ── Page ──────────────────────────────────────────────────────────────────

pub async fn assignments_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(redirect("/"));
    }

    let session = current_session(&state.pool, &jar).await;
    let selected_event_id = session.and_then(|s| s.selected_event_id);

    let Some(event_id) = selected_event_id else {
        return Ok(render(&AssignmentsTemplate {
            title: "Match Assignments".to_string(),
            description: "Assign scouts or devices to each robot slot per match. Assignees get a pre-filled scouting form.".to_string(),
            nav: Nav::from_user(Some(&user)),
            selected_event_id: None,
            info: "Select an event on the home page first.".to_string(),
            selected_event_name: String::new(),
            device_list_html: String::new(),
            assignment_table_html: String::new(),
        }));
    };

    let data = hydrate_assignment_data(&state.pool, &user, event_id).await;
    let selected_event_name = data.selected_event_name.clone();
    let device_list_html = render_html(&DeviceListFragment { devices: data.devices.clone() });
    let assignment_table_html = render_html(&AssignmentTableFragment { data });

    Ok(render(&AssignmentsTemplate {
        title: "Match Assignments".to_string(),
        description: "Assign scouts or devices to each robot slot per match. Assignees get a pre-filled scouting form.".to_string(),
        nav: Nav::from_user(Some(&user)),
        selected_event_id: Some(event_id),
        info: String::new(),
        selected_event_name,
        device_list_html,
        assignment_table_html,
    }))
}

// ── Data hydration ────────────────────────────────────────────────────────

async fn hydrate_assignment_data(pool: &PgPool, lead: &User, event_id: i32) -> AssignmentData {
    let online_cutoff = Utc::now() - chrono::Duration::minutes(ONLINE_WINDOW_MINUTES);
    let mut data = AssignmentData::default();

    data.selected_event_name = sqlx::query_scalar("SELECT name FROM events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .unwrap_or(None)
        .unwrap_or_default();

    let matches: Vec<MatchDbRow> = sqlx::query_as(
        "SELECT id, match_number, match_type, played, scheduled_time,
                red1, red2, red3, blue1, blue2, blue3
         FROM matches WHERE event_id = $1
         ORDER BY match_number, match_type",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let team_lookup: HashMap<i32, TeamLookupRow> = sqlx::query_as::<_, TeamLookupRow>(
        "SELECT teams.team_number, teams.id AS team_id, teams.name AS team_name
         FROM teams
         JOIN event_teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|r| (r.team_number, r))
    .collect();

    let assignments: Vec<AssignmentDbRow> = sqlx::query_as(
        "SELECT sa.id, sa.match_id, sa.team_id, sa.scouter_id, sa.device_id
         FROM scout_assignments sa
         JOIN matches m ON m.id = sa.match_id
         WHERE m.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut by_match: HashMap<i32, HashMap<i32, AssignmentDbRow>> = HashMap::new();
    for a in assignments {
        by_match.entry(a.match_id).or_default().insert(a.team_id, a);
    }

    data.match_rows = matches
        .into_iter()
        .map(|m| {
            let empty = HashMap::new();
            let by_team = by_match.get(&m.id).unwrap_or(&empty);
            let red_slots = build_slots(&[(1, m.red1), (2, m.red2), (3, m.red3)], &team_lookup, by_team);
            let blue_slots = build_slots(&[(1, m.blue1), (2, m.blue2), (3, m.blue3)], &team_lookup, by_team);
            MatchAssignmentRow {
                match_id: m.id,
                match_number: m.match_number,
                match_type: m.match_type,
                played: m.played,
                scheduled_time: m.scheduled_time,
                red_slots,
                blue_slots,
            }
        })
        .collect();

    // Scouts: users on the lead's team (everyone if lead has no team).
    let scout_query = if lead.team_number.is_some() {
        "SELECT users.id, users.name,
                EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = users.id AND s.expires_at > $2) AS online
         FROM users WHERE users.team_number = $1 ORDER BY users.name"
    } else {
        "SELECT users.id, users.name,
                EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = users.id AND s.expires_at > $2) AS online
         FROM users WHERE ($1::int IS NULL OR TRUE) ORDER BY users.name"
    };
    let scout_rows: Vec<(i32, String, bool)> = sqlx::query_as(scout_query)
        .bind(lead.team_number)
        .bind(Utc::now())
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    data.scouts = scout_rows
        .into_iter()
        .map(|(id, name, online)| ScoutOption { id, name, online })
        .collect();

    let device_rows: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT id, COALESCE(NULLIF(name, ''), 'Device ' || SUBSTRING(device_uuid, 1, 8)) AS name,
                (last_seen_at >= $1) IS TRUE AS online
         FROM devices ORDER BY last_seen_at DESC NULLS LAST",
    )
    .bind(online_cutoff)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    data.devices = device_rows
        .into_iter()
        .map(|(id, name, online)| DeviceOption { id, name, online })
        .collect();

    data
}

fn build_slots(
    slots: &[(i32, Option<i32>)],
    team_lookup: &HashMap<i32, TeamLookupRow>,
    by_team: &HashMap<i32, AssignmentDbRow>,
) -> Vec<SlotAssignment> {
    slots
        .iter()
        .map(|(pos, team_number)| {
            let (team_id, team_name) = team_number
                .and_then(|n| team_lookup.get(&n))
                .map(|t| (Some(t.team_id), t.team_name.clone()))
                .unwrap_or((None, "TBD".to_string()));

            let a = team_id.and_then(|id| by_team.get(&id));
            SlotAssignment {
                position: *pos,
                team_id,
                team_number: team_number.unwrap_or(0),
                team_name,
                assignment_id: a.map(|a| a.id),
                scouter_id: a.and_then(|a| a.scouter_id),
                device_id: a.and_then(|a| a.device_id),
            }
        })
        .collect()
}

async fn render_table(pool: &PgPool, user: &User, event_id: i32) -> Html<String> {
    let data = hydrate_assignment_data(pool, user, event_id).await;
    Html(render_html(&AssignmentTableFragment { data }))
}

// ── Mutations ─────────────────────────────────────────────────────────────

fn parse_assignee(assignee: &str) -> Option<(Option<i32>, Option<i32>)> {
    if let Some(rest) = assignee.strip_prefix("u:") {
        rest.parse().ok().map(|id| (Some(id), None))
    } else if let Some(rest) = assignee.strip_prefix("d:") {
        rest.parse().ok().map(|id| (None, Some(id)))
    } else {
        None
    }
}

pub async fn set_assignment(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    }

    let session = current_session(&state.pool, &jar).await;
    let Some(event_id) = session.and_then(|s| s.selected_event_id) else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let form = form_map(&body);
    let match_id = parse_required_int(form_str(&form, "match_id"));
    let team_id = parse_required_int(form_str(&form, "team_id"));
    let Some(match_id) = match_id else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "match_id is required").into_response());
    };
    let Some(team_id) = team_id else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "team_id is required").into_response());
    };

    let assignee = form_str(&form, "assignee").trim().to_string();

    let result = if assignee.is_empty() {
        sqlx::query("DELETE FROM scout_assignments WHERE match_id = $1 AND team_id = $2")
            .bind(match_id)
            .bind(team_id)
            .execute(&state.pool)
            .await
            .map(|_| ())
    } else {
        let Some((scouter_id, device_id)) = parse_assignee(&assignee) else {
            return Ok((axum::http::StatusCode::BAD_REQUEST, "Invalid assignee").into_response());
        };
        sqlx::query(
            "INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (match_id, team_id) DO UPDATE SET
                scouter_id = EXCLUDED.scouter_id, device_id = EXCLUDED.device_id,
                assigned_by = EXCLUDED.assigned_by, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(match_id)
        .bind(team_id)
        .bind(event_id)
        .bind(scouter_id)
        .bind(device_id)
        .bind(user.id)
        .execute(&state.pool)
        .await
        .map(|_| ())
    };

    if let Err(e) = result {
        error!("failed to set assignment (match {match_id}, team {team_id}): {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Failed to save assignment").into_response());
    }

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

pub async fn auto_distribute(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    }

    let session = current_session(&state.pool, &jar).await;
    let Some(event_id) = session.and_then(|s| s.selected_event_id) else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    // Checked assignees come as repeated "assignees" form values.
    let mut pool_vec: Vec<String> = form_multi(&body)
        .into_iter()
        .filter(|(k, _)| k == "assignees")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();

    let mut info = String::new();

    if pool_vec.is_empty() {
        let online_cutoff = Utc::now() - chrono::Duration::minutes(ONLINE_WINDOW_MINUTES);
        let scout_query = if user.team_number.is_some() {
            "SELECT DISTINCT users.id FROM users
             JOIN sessions s ON s.user_id = users.id AND s.expires_at > $1
             WHERE users.team_number = $2"
        } else {
            "SELECT DISTINCT users.id FROM users
             JOIN sessions s ON s.user_id = users.id AND s.expires_at > $1
             WHERE ($2::int IS NULL OR TRUE)"
        };
        let scout_ids: Vec<i32> = sqlx::query_scalar(scout_query)
            .bind(Utc::now())
            .bind(user.team_number)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
        pool_vec.extend(scout_ids.into_iter().map(|id| format!("u:{id}")));

        let device_ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM devices WHERE last_seen_at >= $1")
                .bind(online_cutoff)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();
        pool_vec.extend(device_ids.into_iter().map(|id| format!("d:{id}")));
    }

    if pool_vec.is_empty() {
        info = "No online scouts or devices to distribute to.".to_string();
    } else {
        // Unassigned slots = teams in upcoming unplayed matches, no assignment.
        let unassigned: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT teams_slot.match_id, teams_slot.team_id
             FROM (
                SELECT m.id AS match_id, t.id AS team_id
                FROM matches m
                CROSS JOIN LATERAL (
                    SELECT teams.id
                    FROM teams
                    WHERE teams.team_number IN (m.red1, m.red2, m.red3, m.blue1, m.blue2, m.blue3)
                ) t(id)
                WHERE m.event_id = $1 AND m.played = FALSE
                  AND (m.red1 IS NOT NULL OR m.blue1 IS NOT NULL)
             ) teams_slot
             LEFT JOIN scout_assignments sa
                ON sa.match_id = teams_slot.match_id AND sa.team_id = teams_slot.team_id
             WHERE sa.id IS NULL
             ORDER BY teams_slot.match_id, teams_slot.team_id",
        )
        .bind(event_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();

        for (i, (match_id, team_id)) in unassigned.iter().enumerate() {
            let assignee = &pool_vec[i % pool_vec.len()];
            let Some((scouter_id, device_id)) = parse_assignee(assignee) else {
                continue;
            };
            let _ = sqlx::query(
                "INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (match_id, team_id) DO NOTHING",
            )
            .bind(match_id)
            .bind(team_id)
            .bind(event_id)
            .bind(scouter_id)
            .bind(device_id)
            .bind(user.id)
            .execute(&state.pool)
            .await;
        }
    }

    let mut data = hydrate_assignment_data(&state.pool, &user, event_id).await;
    data.info = info;
    Ok(Html(render_html(&AssignmentTableFragment { data })).into_response())
}

pub async fn clear_all(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    }
    let session = current_session(&state.pool, &jar).await;
    let Some(event_id) = session.and_then(|s| s.selected_event_id) else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let _ = sqlx::query("DELETE FROM scout_assignments WHERE event_id = $1")
        .bind(event_id)
        .execute(&state.pool)
        .await;

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

pub async fn clear_match(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(match_id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    }
    let session = current_session(&state.pool, &jar).await;
    let Some(event_id) = session.and_then(|s| s.selected_event_id) else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let _ = sqlx::query("DELETE FROM scout_assignments WHERE match_id = $1")
        .bind(match_id)
        .execute(&state.pool)
        .await;

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

pub async fn rename_device(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.is_admin && !user.is_lead_scout {
        return Ok(axum::http::StatusCode::UNAUTHORIZED.into_response());
    }

    let form = form_map(&body);
    let name = form_str(&form, "name").trim().to_string();
    if name.is_empty() {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Name is required").into_response());
    }

    let _ = sqlx::query("UPDATE devices SET name = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(&name)
        .bind(id)
        .execute(&state.pool)
        .await;

    let session = current_session(&state.pool, &jar).await;
    let devices = match session.and_then(|s| s.selected_event_id) {
        Some(event_id) => hydrate_assignment_data(&state.pool, &user, event_id).await.devices,
        None => Vec::new(),
    };
    Ok(Html(render_html(&DeviceListFragment { devices })).into_response())
}

pub async fn heartbeat(
    State(state): State<SharedState>,
    jar: CookieJar,
    _headers: HeaderMap,
) -> HandlerResult {
    let uuid = jar.get(DEVICE_COOKIE_NAME).map(|c| c.value().trim().to_string());
    let Some(uuid) = uuid.filter(|u| !u.is_empty()) else {
        return Ok((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "no device id"}))).into_response());
    };
    if uuid.len() < 8 || uuid.len() > 64 {
        return Ok((axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid device id"}))).into_response());
    }

    let user = current_user(&state.pool, &jar).await;
    let team_number = user.and_then(|u| u.team_number);

    let result = sqlx::query(
        "INSERT INTO devices (device_uuid, team_number, last_seen_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (device_uuid) DO UPDATE SET
            last_seen_at = EXCLUDED.last_seen_at,
            team_number = COALESCE(devices.team_number, EXCLUDED.team_number),
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&uuid)
    .bind(team_number)
    .bind(Utc::now())
    .execute(&state.pool)
    .await;

    if let Err(e) = result {
        error!("device heartbeat failed: {e}");
        return Ok((axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "heartbeat failed"}))).into_response());
    }

    Ok(Json(serde_json::json!({"status": "ok"})).into_response())
}
