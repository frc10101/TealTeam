//! Home page, event selection, event summary fragment, and the Help page.
//!
//! Selecting an event is the hinge of the whole app: it writes
//! `selected_event_id` onto the session, and nearly every other page reads it.
//! [`select_event`] therefore answers in two shapes — an Unpoly request gets
//! the re-rendered picker (including any error), a plain form post gets a
//! redirect home.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use tracing::{error, info, warn};

use super::event_selection_html;
use crate::models::{event, session};
use crate::state::SharedState;
use crate::views::events::{EventSummaryData, EventSummaryFragment};
use crate::views::home::{HelpTemplate, IndexTemplate};
use crate::views::{render, Nav};
use crate::web::*;

/// `GET /` — home page: the event picker, its summary, and the live match
/// schedule (which loads itself).
pub async fn index(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    let selected_event_id = selected_event_id(&state.pool, &jar).await;

    let event_selection_html =
        event_selection_html(&state.pool, user.as_ref(), selected_event_id, "", false, true).await;

    Ok(render(&IndexTemplate {
        title: "Home".to_string(),
        nav: Nav::from_user(user.as_ref()),
        message: "TealTeam Scouting".to_string(),
        event_selection_html,
    }))
}

/// `GET /help` — static usage guide.
pub async fn help(State(state): State<SharedState>, jar: CookieJar) -> HandlerResult {
    let user = current_user(&state.pool, &jar).await;
    Ok(render(&HelpTemplate {
        title: "Help".to_string(),
        nav: Nav::from_user(user.as_ref()),
    }))
}

/// `POST /api/events/select` — stores the chosen event on the session.
pub async fn select_event(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    body: String,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let form = form_map(&body);
    let Some(new_event_id) = parse_required_int(form_str(&form, "event_id")) else {
        if is_unpoly(&headers) {
            let current = selected_event_id(&state.pool, &jar).await;
            let html = event_selection_html(
                &state.pool,
                Some(&user),
                current,
                "event_id is required",
                false,
                false,
            )
            .await;
            return Ok(Html(html).into_response());
        }
        return Ok((StatusCode::BAD_REQUEST, "event_id is required").into_response());
    };

    let Some(session) = current_session(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    match session::set_selected_event(&state.pool, &session.session_id, new_event_id).await {
        Ok(()) => info!(
            "event selection updated: session {} event {new_event_id}",
            session.session_id
        ),
        Err(e) => {
            error!(
                "failed to update selected event (session {}, event {new_event_id}): {e}",
                session.session_id
            );
            if is_unpoly(&headers) {
                let html = event_selection_html(
                    &state.pool,
                    Some(&user),
                    session.selected_event_id,
                    "Failed to save event selection",
                    false,
                    false,
                )
                .await;
                return Ok(Html(html).into_response());
            }
            return Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save event selection",
            )
                .into_response());
        }
    }

    if is_unpoly(&headers) {
        let html = event_selection_html(
            &state.pool,
            Some(&user),
            Some(new_event_id),
            "",
            true,
            true,
        )
        .await;
        // The event-selection form re-emits `eventSelected`/`reload-matches`
        // client-side via up-on-inserted, so no server trigger header is needed.
        return Ok(Html(html).into_response());
    }

    Ok(redirect("/"))
}

/// `GET /hx/events/summary` — the summary panel alone, for the event id in
/// the query string or the one on the session.
pub async fn event_summary(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/sign-in"));
    };

    let mut event_id_raw = query.get("event_id").cloned().unwrap_or_default();
    if event_id_raw.is_empty() {
        if let Some(id) = selected_event_id(&state.pool, &jar).await {
            event_id_raw = id.to_string();
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
            s: EventSummaryData::error("Invalid event ID"),
        }));
    };

    let summary = event::summary(&state.pool, event_id, Some(&user)).await;
    Ok(render(&EventSummaryFragment {
        s: EventSummaryData::from_summary(event_id, summary),
    }))
}
