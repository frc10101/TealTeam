//! The admin database diagnostic viewer (U17): `/admin/db`.
//!
//! Read-only, admin-only. The retired viewer had no guard at all and exposed
//! every user's email and every session row to anyone who knew the URL
//! (REBUILD_SPEC.md 12.4). This view model never holds a session or a token
//! key row -- `tt_repo_sqlite::admin` leaves them out before the handler ever
//! sees them -- and nothing here renders a form: there is no write path to
//! misuse even if the guard were somehow bypassed.

use askama::Template;

use crate::Nav;

#[derive(Template)]
#[template(path = "pages/admin.html")]
pub struct AdminPage {
    pub title: String,
    pub nav: Nav,
    /// Why there is nothing below, when storage cannot be read at all.
    pub unavailable: String,
    pub tables: Vec<AdminTable>,
    /// Set once a table is chosen and read without error.
    pub selected: Option<AdminTableView>,
}

/// One row of the table list.
#[derive(Debug, Clone)]
pub struct AdminTable {
    pub name: String,
    pub row_count: i64,
    pub href: String,
    pub selected: bool,
}

/// A page of one table's rows, ready to render.
#[derive(Debug, Clone)]
pub struct AdminTableView {
    pub name: String,
    pub total: i64,
    pub columns: Vec<String>,
    pub rows: Vec<AdminRow>,
    pub has_prev: bool,
    pub has_next: bool,
    pub prev_href: String,
    pub next_href: String,
}

/// One row, each cell already paired with its column name so the template
/// needs no second list to zip against -- `.cards`' narrow layout (U16) reads
/// that pairing straight off each cell as `data-label`.
#[derive(Debug, Clone)]
pub struct AdminRow {
    pub cells: Vec<AdminCell>,
}

#[derive(Debug, Clone)]
pub struct AdminCell {
    pub label: String,
    /// `"—"` for SQL `NULL`, never empty.
    pub value: String,
}
