//! The graph view (U21): each chosen team's matches as lines, one per metric,
//! chosen by tapping chips.

use askama::Template;

use crate::Nav;

#[derive(Template)]
#[template(path = "pages/graph.html")]
pub struct GraphPage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there is nothing to draw, when there cannot be.
    pub unavailable: String,
    pub errors: Vec<String>,
    /// Said, not refused: a limit reached, a default chosen.
    pub notices: Vec<String>,
    /// The chosen teams, in the order their colours were given.
    pub teams: Vec<Chip>,
    /// Every other team with something to draw.
    pub more_teams: Vec<Chip>,
    pub metrics: Vec<Chip>,
    /// What to tap when nothing is drawn.
    pub prompt: String,
    /// Every event this season, not only the selected one.
    pub season: bool,
    /// `{"metrics": [...], "teams": [...]}` for `static/js/graph.js`: every
    /// team on offer, not only the chosen, so a tap needs no reload.
    pub data: String,
    pub headings: Vec<String>,
    pub tables: Vec<TeamTable>,
    /// Where the numbers come from, and how old TBA's are.
    pub source: String,
    pub max_teams: usize,
    pub max_metrics: usize,
}

/// A tap-to-toggle chip: a checkbox in the page's GET form.
#[derive(Debug, Clone)]
pub struct Chip {
    pub value: String,
    pub label: String,
    /// Read out in full; the label may be short.
    pub title: String,
    pub on: bool,
    /// Its colour (a team) or line style (a metric), from 1; 0 when off.
    pub slot: usize,
}

/// One chosen team's numbers, for reading without the chart.
#[derive(Debug, Clone)]
pub struct TeamTable {
    pub number: i32,
    pub name: String,
    pub slot: usize,
    pub rows: Vec<TableRow>,
}

#[derive(Debug, Clone)]
pub struct TableRow {
    /// `"Q14"`, or `"MABIL Q14"` across events.
    pub label: String,
    /// One per chosen metric; `"—"` when not recorded.
    pub values: Vec<String>,
}
