//! Compare teams (DR1): chosen teams side by side, a column each, every
//! number with how many matches it rests on and how spread out they were.

use askama::Template;

use crate::{Chip, Nav};

#[derive(Template)]
#[template(path = "pages/compare.html")]
pub struct ComparePage {
    pub title: String,
    pub nav: Nav,
    pub event_name: String,
    /// Why there is nothing to compare, when there cannot be.
    pub unavailable: String,
    pub errors: Vec<String>,
    /// Said, not refused: a limit reached, a default chosen.
    pub notices: Vec<String>,
    /// The chosen teams, in column order.
    pub teams: Vec<Chip>,
    /// Every other team with something scouted.
    pub more_teams: Vec<Chip>,
    pub max_teams: usize,
    /// Every match, or the last few.
    pub windows: Vec<Choice>,
    /// The team the others are measured against; the first is "nobody".
    pub references: Vec<Choice>,
    pub columns: Vec<CompareColumn>,
    pub groups: Vec<CompareGroup>,
    /// The chosen teams' scout estimate, match by match, on Graph.
    pub graph_href: String,
    /// Where the numbers come from.
    pub source: String,
}

/// An option in a select.
#[derive(Debug, Clone)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone)]
pub struct CompareColumn {
    pub number: i32,
    pub name: String,
    /// Its colour, as on Graph.
    pub slot: usize,
    pub team_href: String,
    /// `"6 matches"`.
    pub matches: String,
    /// Fewer matches than can be trusted.
    pub thin: bool,
    /// The team the others are measured against.
    pub reference: bool,
}

#[derive(Debug, Clone)]
pub struct CompareGroup {
    pub label: String,
    pub rows: Vec<CompareRow>,
}

#[derive(Debug, Clone)]
pub struct CompareRow {
    pub label: String,
    /// `"higher is better"`, said so a reader knows what the mark means.
    pub hint: String,
    pub cells: Vec<CompareCell>,
}

#[derive(Debug, Clone, Default)]
pub struct CompareCell {
    /// `"8.0"`, `"67%"`, `"Played 5 · Disabled 1"`, or `"—"`.
    pub main: String,
    /// `"median 8 · 5–11 · ±3.2 · 4 matches"`.
    pub detail: String,
    /// Against the reference team: `"+2.0"`; empty without one.
    pub delta: String,
    /// `"ahead"`, `"behind"` or empty, by which way is better.
    pub delta_class: String,
    /// The row's best.
    pub best: bool,
    /// Where the values lie on the row's shared scale, when it has one.
    pub plot: Option<BoxPlot>,
}

/// A box plot on a 0–100 scale shared across the row, so boxes line up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxPlot {
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
}

impl BoxPlot {
    /// The box's width, at least a hairline so a single value still shows.
    pub fn width(&self) -> f64 {
        (self.q3 - self.q1).max(0.8)
    }
}
