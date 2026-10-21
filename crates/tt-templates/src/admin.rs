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

/// Account administration (G1): `/admin/users`.
///
/// Every account, each with a form for its roles and team number and one to
/// reset its password. The rules -- the last admin stays one, a reset ends
/// the account's sessions -- are the repo's; this only shows what it said.
#[derive(Template)]
#[template(path = "pages/admin_users.html")]
pub struct AdminUsersPage {
    pub title: String,
    pub nav: Nav,
    /// Why there is nothing below, when storage cannot be read at all.
    pub unavailable: String,
    /// What the change that led here did. Empty for none.
    pub notice: String,
    /// Whether `notice` says something was refused rather than done.
    pub notice_is_error: bool,
    pub accounts: Vec<AccountRow>,
}

/// One account, ready to render.
#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: i64,
    pub name: String,
    pub email: String,
    /// The number, or empty for none -- the input's value.
    pub team_number: String,
    pub role_labels: Vec<&'static str>,
    pub is_admin: bool,
    pub is_lead_scout: bool,
    pub is_coach: bool,
    /// `"20 minutes ago"`, or `"Never"`.
    pub last_sign_in: String,
    /// The admin looking at the page.
    pub is_you: bool,
    pub save_action: String,
    pub reset_action: String,
}
