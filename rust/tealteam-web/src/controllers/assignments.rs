//! Per-match robot assignments: the lead scout assigns a scout or a device to
//! each robot slot in each match.
//!
//! Every mutation answers with the whole re-rendered table rather than a
//! status code, so the grid, its coverage counts and the online indicators
//! stay consistent after each change.
//!
//! [`heartbeat`] is the exception to the pattern here: it is posted by every
//! tablet on a timer, needs no session, and is what makes a device appear as
//! online and assignable.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Json};
use axum_extra::extract::cookie::CookieJar;
use sqlx::PgPool;
use tracing::error;

use crate::models::assignment::{self, Assignee};
use crate::models::{device, event, session, team, user, User};
use crate::state::SharedState;
use crate::views::assignments::{
    AssignmentData, AssignmentTableFragment, AssignmentsTemplate, DeviceListFragment,
};
use crate::views::{render, render_html, Nav};
use crate::web::*;

// ── Page ──────────────────────────────────────────────────────────────────

/// `GET /lead-scout/assignments` — the assignment grid and device list for
/// the selected event.
pub async fn assignments_page(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };
    if !user.can_lead() {
        return Ok(redirect("/"));
    }

    let Some(event_id) = selected_event_id(&state.pool, &jar).await else {
        return Ok(render(&AssignmentsTemplate::without_event(Nav::from_user(
            Some(&user),
        ))));
    };

    let data = load_assignment_data(&state.pool, &user, event_id).await;
    let selected_event_name = data.selected_event_name.clone();
    let device_list_html = render_html(&DeviceListFragment {
        devices: data.devices.clone(),
    });
    let assignment_table_html = render_html(&AssignmentTableFragment { data });

    Ok(render(&AssignmentsTemplate::new(
        Nav::from_user(Some(&user)),
        event_id,
        selected_event_name,
        device_list_html,
        assignment_table_html,
    )))
}

// ── Data loading ──────────────────────────────────────────────────────────

/// Loads the five lists the grid is built from: event name, schedule, roster,
/// existing assignments, and the assignable scouts and devices.
async fn load_assignment_data(pool: &PgPool, lead: &User, event_id: i32) -> AssignmentData {
    let online_cutoff = assignment::online_cutoff();

    AssignmentData::build(
        event::find_name(pool, event_id).await.unwrap_or_default(),
        assignment::matches_for_event(pool, event_id).await,
        team::lookup_for_event(pool, event_id).await,
        assignment::for_event(pool, event_id).await,
        user::list_scouts(pool, lead.team_number).await,
        device::list(pool, online_cutoff).await,
    )
}

/// Re-renders the grid, the standard response to any mutation.
async fn render_table(pool: &PgPool, user: &User, event_id: i32) -> Html<String> {
    let data = load_assignment_data(pool, user, event_id).await;
    Html(render_html(&AssignmentTableFragment { data }))
}

// ── Mutations ─────────────────────────────────────────────────────────────

/// `POST /hx/assignments/set` — assigns one slot, or clears it when the
/// picker is set back to "unassigned".
pub async fn set_assignment(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.can_lead() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }

    let Some(event_id) = selected_event_id(&state.pool, &jar).await else {
        return Ok((StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let form = form_map(&body);
    let Some(match_id) = parse_required_int(form_str(&form, "match_id")) else {
        return Ok((StatusCode::BAD_REQUEST, "match_id is required").into_response());
    };
    let Some(team_id) = parse_required_int(form_str(&form, "team_id")) else {
        return Ok((StatusCode::BAD_REQUEST, "team_id is required").into_response());
    };

    let assignee_raw = form_str(&form, "assignee").trim().to_string();

    let result = if assignee_raw.is_empty() {
        assignment::clear_slot(&state.pool, match_id, team_id).await
    } else {
        let Some(assignee) = Assignee::parse(&assignee_raw) else {
            return Ok((StatusCode::BAD_REQUEST, "Invalid assignee").into_response());
        };
        assignment::set(&state.pool, match_id, team_id, event_id, assignee, user.id).await
    };

    if let Err(e) = result {
        error!("failed to set assignment (match {match_id}, team {team_id}): {e}");
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "Failed to save assignment").into_response());
    }

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

/// `POST /hx/assignments/auto` — fills every unassigned slot in upcoming
/// matches, round-robin.
///
/// The pool is whoever was checked on the form, or everyone currently online
/// if nothing was. Existing assignments are never overwritten, so this can be
/// run repeatedly as more scouts arrive.
pub async fn auto_distribute(
    State(state): State<SharedState>,
    jar: CookieJar,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.can_lead() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }

    let Some(event_id) = selected_event_id(&state.pool, &jar).await else {
        return Ok((StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    // Checked assignees come as repeated "assignees" form values.
    let mut assignees: Vec<String> = form_multi(&body)
        .into_iter()
        .filter(|(k, _)| k == "assignees")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();

    let mut info = String::new();

    // Nothing checked: fall back to everyone currently online.
    if assignees.is_empty() {
        let online_cutoff = assignment::online_cutoff();
        assignees.extend(
            user::online_scout_ids(&state.pool, user.team_number)
                .await
                .into_iter()
                .map(|id| format!("u:{id}")),
        );
        assignees.extend(
            device::online_ids(&state.pool, online_cutoff)
                .await
                .into_iter()
                .map(|id| format!("d:{id}")),
        );
    }

    if assignees.is_empty() {
        info = "No online scouts or devices to distribute to.".to_string();
    } else {
        let unassigned = assignment::unassigned_slots(&state.pool, event_id).await;
        for (i, (match_id, team_id)) in unassigned.iter().enumerate() {
            let Some(assignee) = Assignee::parse(&assignees[i % assignees.len()]) else {
                continue;
            };
            let _ = assignment::set_if_absent(
                &state.pool,
                *match_id,
                *team_id,
                event_id,
                assignee,
                user.id,
            )
            .await;
        }
    }

    let mut data = load_assignment_data(&state.pool, &user, event_id).await;
    data.info = info;
    Ok(Html(render_html(&AssignmentTableFragment { data })).into_response())
}

/// `POST /hx/assignments/clear-all` — unassigns every slot at the event.
pub async fn clear_all(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.can_lead() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    let Some(event_id) = selected_event_id(&state.pool, &jar).await else {
        return Ok((StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let _ = assignment::clear_event(&state.pool, event_id).await;

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

/// `POST /hx/assignments/clear-match/:match_id` — unassigns one match.
pub async fn clear_match(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(match_id): Path<i32>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.can_lead() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    let Some(event_id) = selected_event_id(&state.pool, &jar).await else {
        return Ok((StatusCode::BAD_REQUEST, "No event selected").into_response());
    };

    let _ = assignment::clear_match(&state.pool, match_id).await;

    Ok(render_table(&state.pool, &user, event_id).await.into_response())
}

/// `POST /hx/devices/:id/rename` — names a tablet, and re-renders the device
/// list.
pub async fn rename_device(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(id): Path<i32>,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if !user.can_lead() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }

    let form = form_map(&body);
    let name = form_str(&form, "name").trim().to_string();
    if name.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Name is required").into_response());
    }

    let _ = device::rename(&state.pool, id, &name).await;

    let devices = match selected_event_id(&state.pool, &jar).await {
        Some(event_id) => load_assignment_data(&state.pool, &user, event_id).await.devices,
        None => Vec::new(),
    };
    Ok(Html(render_html(&DeviceListFragment { devices })).into_response())
}

/// `POST /api/device/heartbeat` — registers a tablet and refreshes its
/// last-seen time.
///
/// Identified by the `device_uuid` cookie, not a session: a tablet passed
/// between scouts still reports as the same device. If someone is signed in,
/// their team number is recorded the first time so the device can be
/// attributed.
pub async fn heartbeat(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let Some(uuid) = session::device_uuid(&jar) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "no device id"})),
        )
            .into_response());
    };
    if uuid.len() < 8 || uuid.len() > 64 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid device id"})),
        )
            .into_response());
    }

    let team_number = current_user(&state.pool, &jar).await.and_then(|u| u.team_number);

    if let Err(e) = device::heartbeat(&state.pool, &uuid, team_number).await {
        error!("device heartbeat failed: {e}");
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "heartbeat failed"})),
        )
            .into_response());
    }

    Ok(Json(serde_json::json!({"status": "ok"})).into_response())
}
