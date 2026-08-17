//! Controller layer: one module per area of the site.
//!
//! A controller action reads the request, calls [`crate::models`] for data and
//! [`crate::services`] for the outside world, then hands the result to
//! [`crate::views`]. It holds no SQL and no markup.
//!
//! # Shape of an action
//!
//! Actions take axum extractors, return [`crate::web::HandlerResult`], and
//! follow the same order: resolve the user, check authorization, resolve the
//! selected event, load, render.
//!
//! # Authorization
//!
//! Every protected action checks the session itself rather than relying on a
//! middleware, because the right response differs by route: a page redirects
//! to `/sign-in` or `/`, an Unpoly fragment endpoint returns a status code,
//! and a JSON API returns a JSON error. The checks are
//! [`crate::models::User::can_lead`] and
//! [`crate::models::User::can_coach`]; the DB viewer requires `is_admin`.
//!
//! # Pages and fragments
//!
//! Several actions serve both a full page and, for an Unpoly request, just the
//! fragment that changed — see [`crate::web::is_unpoly`]. Mutating actions
//! generally respond with the re-rendered fragment rather than a redirect, so
//! the page updates in place.
//!
//! # Failure
//!
//! Expected failures are rendered, not returned as errors: an unreachable
//! FIRST API becomes a note in the schedule panel, a duplicate email becomes a
//! banner on the form. [`crate::web::AppError`] is reserved for the
//! unexpected.

pub mod api;
pub mod assignments;
pub mod auth;
pub mod coach;
pub mod db_viewer;
pub mod home;
pub mod lead_scout;
pub mod matches;
pub mod submission;
pub mod teams;

use sqlx::PgPool;

use crate::models::event;
use crate::models::User;
use crate::views::events::{
    EventSelectionData, EventSelectionFragment, EventSummaryData, EventSummaryFragment,
};
use crate::views::render_html;

/// Renders the event picker, optionally with its summary, for embedding in a
/// page or serving on its own.
///
/// Shared by the home page and the drive coach panel. `event_error` and
/// `event_updated` carry the outcome of a just-submitted selection back into
/// the re-rendered picker. A signed-out viewer gets the picker's signed-out
/// state without touching the database.
pub async fn event_selection_html(
    pool: &PgPool,
    user: Option<&User>,
    selected_event_id: Option<i32>,
    event_error: &str,
    event_updated: bool,
    with_summary: bool,
) -> String {
    let Some(user) = user else {
        return render_html(&EventSelectionFragment {
            d: EventSelectionData::default(),
            summary_html: render_html(&EventSummaryFragment {
                s: EventSummaryData::default(),
            }),
        });
    };

    let event_ids = event::available_ids(pool, user).await.unwrap_or_default();
    let d = EventSelectionData {
        signed_in: true,
        events: event::options(pool, &event_ids).await,
        selected_event_id,
        event_error: event_error.to_string(),
        event_updated,
    };

    let summary = match (with_summary, selected_event_id) {
        (true, Some(event_id)) => EventSummaryData::from_summary(
            event_id,
            event::summary(pool, event_id, Some(user)).await,
        ),
        _ => EventSummaryData::default(),
    };
    let summary_html = render_html(&EventSummaryFragment { s: summary });

    render_html(&EventSelectionFragment { d, summary_html })
}
