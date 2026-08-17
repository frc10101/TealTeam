//! Event picker and event summary fragments.
//!
//! Shown on the home page and the drive coach panel, and re-served on their
//! own whenever the picker changes. Selecting an event writes it to the
//! session ([`crate::models::session::set_selected_event`]), which is what
//! scopes the rest of the app: submissions, assignments, rankings and the
//! coach schedule all read that one value.
//!
//! The summary is nested inside the picker fragment as pre-rendered HTML so
//! one swap updates both.

use askama::Template;

use crate::models::event::{EventOption, EventSummary, EventTeamRow};

/// The summary panel's state: an event's counts and roster, or an error, or
/// nothing selected yet (the default).
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

impl EventSummaryData {
    /// Presents a loaded [`EventSummary`], turning the "your team is not on
    /// this roster" flag into the warning the template shows.
    pub fn from_summary(event_id: i32, summary: EventSummary) -> Self {
        Self {
            has_event: true,
            error: String::new(),
            event_name: summary.name,
            event_id,
            teams_count: summary.teams_count,
            matches_count: summary.matches_count,
            teams: summary.teams,
            warning: if summary.viewer_team_missing {
                "Your team is not listed for this event yet.".to_string()
            } else {
                String::new()
            },
        }
    }

    /// Summary panel showing an error instead of an event.
    pub fn error(message: &str) -> Self {
        Self {
            error: message.to_string(),
            ..Default::default()
        }
    }
}

/// The picker's state. The default is the signed-out case, where the template
/// renders a prompt to sign in instead of a `<select>`.
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

/// The summary panel, also served alone at `/hx/events/summary`.
#[derive(Template)]
#[template(path = "partials/event_summary.html")]
pub struct EventSummaryFragment {
    pub s: EventSummaryData,
}

/// The picker, with the summary already rendered into `summary_html`.
#[derive(Template)]
#[template(path = "partials/event_selection.html")]
pub struct EventSelectionFragment {
    pub d: EventSelectionData,
    pub summary_html: String,
}
