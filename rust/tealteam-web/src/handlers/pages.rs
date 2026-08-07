// Home page, event selection, event summary fragment, and the Help page
// (port of Controllers/PagesController.cs).

use askama::Template;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use tracing::{error, info, warn};

use super::*;
use crate::state::SharedState;

// ── Templates ─────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "partials/event_summary.html")]
pub struct EventSummaryFragment {
    pub s: EventSummaryData,
}

#[derive(Template)]
#[template(path = "partials/event_selection.html")]
pub struct EventSelectionFragment {
    pub d: EventSelectionData,
    pub summary_html: String,
}

#[derive(Template)]
#[template(path = "pages/index.html")]
struct IndexTemplate {
    title: String,
    nav: Nav,
    message: String,
    event_selection_html: String,
}

#[derive(Template)]
#[template(path = "pages/help.html")]
struct HelpTemplate {
    title: String,
    nav: Nav,
}

pub async fn render_event_selection(
    pool: &sqlx::PgPool,
    user: Option<&crate::models::User>,
    selected_event_id: Option<i32>,
    event_error: &str,
    event_updated: bool,
    with_summary: bool,
) -> String {
    let mut d = load_event_selection(pool, user, selected_event_id).await;
    d.event_error = event_error.to_string();
    d.event_updated = event_updated;

    let summary = match (with_summary, selected_event_id) {
        (true, Some(event_id)) => load_event_summary(pool, event_id, user).await,
        _ => EventSummaryData::default(),
    };
    let summary_html = render_html(&EventSummaryFragment { s: summary });

    render_html(&EventSelectionFragment { d, summary_html })
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub async fn index(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let session = current_session(&state.pool, &jar).await;
    let selected_event_id = session.and_then(|s| s.selected_event_id);

    let event_selection_html = render_event_selection(
        &state.pool,
        user.as_ref(),
        selected_event_id,
        "",
        false,
        true,
    )
    .await;

    Ok(render(&IndexTemplate {
        title: "Home".to_string(),
        nav: Nav::from_user(user.as_ref()),
        message: "TealTeam Scouting".to_string(),
        event_selection_html,
    }))
}

pub async fn help(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    Ok(render(&HelpTemplate {
        title: "Help".to_string(),
        nav: Nav::from_user(user.as_ref()),
    }))
}

pub async fn select_event(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    body: String,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let Some(user) = user else {
        return Ok(redirect("/sign-in"));
    };

    let form = form_map(&body);
    let selected_event_id = parse_required_int(form_str(&form, "event_id"));

    let Some(selected_event_id) = selected_event_id else {
        if is_htmx(&headers) {
            let session = current_session(&state.pool, &jar).await;
            let html = render_event_selection(
                &state.pool,
                Some(&user),
                session.and_then(|s| s.selected_event_id),
                "event_id is required",
                false,
                false,
            )
            .await;
            return Ok(axum::response::Html(html).into_response());
        }
        return Ok((axum::http::StatusCode::BAD_REQUEST, "event_id is required").into_response());
    };

    let Some(session) = current_session(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let update = sqlx::query("UPDATE sessions SET selected_event_id = $1 WHERE session_id = $2")
        .bind(selected_event_id)
        .bind(&session.session_id)
        .execute(&state.pool)
        .await;

    match update {
        Ok(_) => info!(
            "event selection updated: session {} event {selected_event_id}",
            session.session_id
        ),
        Err(e) => {
            error!(
                "failed to update selected event (session {}, event {selected_event_id}): {e}",
                session.session_id
            );
            if is_htmx(&headers) {
                let html = render_event_selection(
                    &state.pool,
                    Some(&user),
                    session.selected_event_id,
                    "Failed to save event selection",
                    false,
                    false,
                )
                .await;
                return Ok(axum::response::Html(html).into_response());
            }
            return Ok((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save event selection",
            )
                .into_response());
        }
    }

    if is_htmx(&headers) {
        let html = render_event_selection(
            &state.pool,
            Some(&user),
            Some(selected_event_id),
            "",
            true,
            true,
        )
        .await;
        // Trigger match schedule reload via HTMX event.
        let mut response = axum::response::Html(html).into_response();
        response
            .headers_mut()
            .insert("HX-Trigger", "eventSelected".parse().unwrap());
        return Ok(response);
    }

    Ok(redirect("/"))
}

pub async fn event_summary(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let Some(user) = user else {
        return Ok(redirect("/sign-in"));
    };

    let mut event_id_raw = query.get("event_id").cloned().unwrap_or_default();
    if event_id_raw.is_empty() {
        if let Some(session) = current_session(&state.pool, &jar).await {
            if let Some(id) = session.selected_event_id {
                event_id_raw = id.to_string();
            }
        }
    }

    if event_id_raw.is_empty() {
        return Ok(render(&EventSummaryFragment {
            s: EventSummaryData::default(),
        }));
    }

    let Ok(event_id) = event_id_raw.parse::<i32>() else {
        warn!("invalid event id in summary request: {event_id_raw}");
        return Ok(render(&EventSummaryFragment {
            s: EventSummaryData {
                error: "Invalid event ID".to_string(),
                ..Default::default()
            },
        }));
    };

    let s = load_event_summary(&state.pool, event_id, Some(&user)).await;
    Ok(render(&EventSummaryFragment { s }))
}
