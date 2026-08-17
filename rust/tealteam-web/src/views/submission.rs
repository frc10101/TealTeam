//! Scouting submission page, its swappable panel, and the team `<select>`
//! rebuilt whenever the scout changes event.
//!
//! The whole form lives in one partial so that submitting it can re-render the
//! panel in place with a success or error banner, keeping the scout on the
//! page between matches. The panel also pre-fills itself from the scout's next
//! match assignment ([`ScoutingFormData::prefill_from_assignment`]) so the
//! common case is: glance at the chip, confirm, submit.

use askama::Template;

use super::Nav;
use crate::models::assignment::AssignedTeam;
use crate::models::event::EventOption;
use crate::models::team::TeamOption;

/// Everything the form needs: the pickers' contents and what to pre-select.
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

    /// Marks the pre-selected event option.
    pub fn is_event(&self, id: &i32) -> bool {
        self.prefill_event_id.as_ref() == Some(id)
    }
    /// Marks the pre-selected team option.
    pub fn is_team(&self, id: &i32) -> bool {
        self.prefill_team_id.as_ref() == Some(id)
    }

    /// Pre-selects the scout's next unplayed assignment, if they have one.
    pub fn prefill_from_assignment(&mut self) {
        if let Some(first) = self.assigned_teams.first() {
            self.prefill_team_id = Some(first.team_id);
            self.prefill_match_number = first.match_number;
        }
    }
}

/// The form panel, re-rendered after every submit with a banner.
#[derive(Template)]
#[template(path = "partials/submission_panel.html")]
pub struct SubmissionPanelFragment {
    pub error: String,
    pub success: String,
    pub form: ScoutingFormData,
}

/// The submission page wrapping [`SubmissionPanelFragment`].
#[derive(Template)]
#[template(path = "pages/submission.html")]
pub struct SubmissionTemplate {
    pub title: String,
    pub nav: Nav,
    pub description: String,
    pub panel_html: String,
}

impl SubmissionTemplate {
    /// Page shell around an already-rendered panel.
    pub fn new(nav: Nav, panel_html: String) -> Self {
        Self {
            title: "Scouting Submission".to_string(),
            nav,
            description: "Submit scouting data for competitions".to_string(),
            panel_html,
        }
    }
}

/// Builds the whole `<select id="team-id">` element. Unpoly matches the
/// response against up-target="#team-id", so the element is returned as a
/// whole and swapped in one piece (not just its `<option>`s).
pub struct TeamSelect {
    options: String,
}

impl TeamSelect {
    /// A select holding only the disabled placeholder option.
    pub fn new() -> Self {
        Self {
            options: r#"<option value="" disabled selected>Select team</option>"#.to_string(),
        }
    }

    /// Appends a team. The name is HTML-escaped — it comes from an external
    /// API.
    pub fn push(&mut self, team_id: i32, team_number: i32, name: &str) {
        self.options.push_str(&format!(
            r#"<option value="{}">{} - {}</option>"#,
            team_id,
            team_number,
            html_escape::encode_text(name)
        ));
    }

    /// Appends the "no teams" notice, for an event with no roster yet.
    pub fn push_empty_notice(&mut self) {
        self.options
            .push_str(r#"<option value="" disabled>No teams available for this event</option>"#);
    }

    /// The complete `<select>` element.
    pub fn render(&self) -> String {
        const OPEN: &str = r#"<select id="team-id" name="team_id" required class="w-full px-4 py-2 bg-white border border-gray-300 rounded-lg text-gray-900 focus:outline-none focus:ring-2 focus:ring-teal-500 focus:border-transparent transition-colors">"#;
        format!("{OPEN}{}</select>", self.options)
    }
}

impl Default for TeamSelect {
    fn default() -> Self {
        Self::new()
    }
}
