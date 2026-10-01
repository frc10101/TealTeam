//! The notes view (U22): every note the viewer's team may read at the selected
//! event, each with when it was recorded, narrowed by team, scout, and words.

use askama::Template;

use crate::Nav;

#[derive(Template)]
#[template(path = "pages/notes.html")]
pub struct NotesPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there are no notes to show, when there cannot be.
    pub unavailable: String,
    pub errors: Vec<String>,
    /// Whose notes these are: the viewer's team.
    pub own_team: i32,
    pub teams: Vec<FilterOption>,
    pub scouts: Vec<FilterOption>,
    /// The words searched for, as typed.
    pub query: String,
    /// `"newest"` or `"schedule"`.
    pub order: &'static str,
    /// Readable notes at the event before any filter.
    pub total: usize,
    /// Some filter is set.
    pub filtered: bool,
    /// The page with no filter, keeping the event and the order.
    pub clear_href: String,
    pub notes: Vec<NoteEntry>,
    /// Observations of the viewer's team with notes, still waiting for review
    /// and so not listed.
    pub waiting: usize,
}

/// One choice in a filter's menu.
#[derive(Debug, Clone)]
pub struct FilterOption {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

/// One note, from one approved observation.
#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub team: i32,
    pub team_name: String,
    /// `"Q14"`.
    pub match_label: String,
    pub scout: String,
    /// The field it was written in; empty when the form has only one.
    pub label: String,
    pub text: String,
    /// `"Sat 2:14 PM CDT"`, on the event's clock; empty when unrecorded.
    pub when: String,
    /// `"12 minutes ago"`.
    pub ago: String,
    /// This page narrowed to the note's team.
    pub filter_href: String,
    pub profile_href: String,
}
