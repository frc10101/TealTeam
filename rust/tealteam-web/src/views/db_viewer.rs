//! Admin database viewer: the table list per tab, and a paginated dump of one
//! table.
//!
//! A debugging aid for admins at an event — "did that submission actually
//! land?" — not an editor: everything here is read-only, and sensitive columns
//! are filtered out in [`crate::models::schema`] before they reach a view.
//!
//! Tabs group the tables that tend to be inspected together; every table is
//! still listed, with the current tab's ones highlighted.

use askama::Template;

use super::Nav;
use crate::models::schema::{ColumnInfo, TablePage};

/// One table in the sidebar list.
pub struct TableInfo {
    pub name: String,
    pub row_count: i64,
    pub description: String,
    pub visible: bool,
    pub selected: bool,
}

impl TableInfo {
    /// Presents a table, marking whether this tab shows it and whether it is
    /// the one currently open.
    pub fn new(name: String, row_count: i64, active_tab: &str, selected_table: &str) -> Self {
        let description = table_description(&name, row_count);
        let visible = tab_tables(active_tab).contains(&name.as_str());
        let selected = selected_table == name;
        Self {
            name,
            row_count,
            description,
            visible,
            selected,
        }
    }
}

/// Which tables each tab of the viewer shows.
fn tab_tables(tab: &str) -> &'static [&'static str] {
    match tab {
        "relationships" => &["event_teams"],
        "match_data" => &["matches", "scouting_data"],
        "submissions" => &["scouting_submissions", "sessions"],
        _ => &["teams", "events", "users"],
    }
}

fn table_description(name: &str, row_count: i64) -> String {
    match name {
        "scouting_data" => "Scouting data".to_string(),
        "event_teams" => "Team-event mapping".to_string(),
        "scouting_submissions" => "Pending submissions".to_string(),
        "sessions" => "User sessions".to_string(),
        _ => format!("{row_count} rows"),
    }
}

/// The viewer page: tabs and the table list.
#[derive(Template)]
#[template(path = "pages/db_viewer.html")]
pub struct DbViewerTemplate {
    pub title: String,
    pub nav: Nav,
    pub active_tab: String,
    pub tables: Vec<TableInfo>,
    pub selected_table: String,
}

impl DbViewerTemplate {
    /// The viewer for one tab, with one table optionally open.
    pub fn new(
        nav: Nav,
        active_tab: String,
        tables: Vec<TableInfo>,
        selected_table: String,
    ) -> Self {
        Self {
            title: "Database Viewer".to_string(),
            nav,
            active_tab,
            tables,
            selected_table,
        }
    }

    /// Highlights the active tab. Called from the template.
    fn tab_class(&self, tab: &str, accent: &str) -> String {
        if self.active_tab == tab || (tab == "core" && self.active_tab.is_empty()) {
            format!("border-{accent}-500 text-{accent}-300")
        } else {
            "border-transparent text-gray-400 hover:text-gray-300".to_string()
        }
    }
}

/// One page of one table, with its pagination controls.
#[derive(Template)]
#[template(path = "partials/db_table_content.html")]
pub struct DbTableContentFragment {
    pub selected_table: String,
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: i64,
    pub offset: i64,
    pub limit: i64,
    pub shown_end: i64,
    pub prev_offset: i64,
    pub next_offset: i64,
    pub page: i64,
    pub total_pages: i64,
}

impl DbTableContentFragment {
    /// Presents a loaded page and works out the pagination numbers around it.
    pub fn new(table_name: &str, page: TablePage, offset: i64, limit: i64) -> Self {
        let shown_end = offset + page.rows.len() as i64;
        Self {
            selected_table: table_name.to_string(),
            columns: page.columns,
            rows: page.rows,
            total_rows: page.total_rows,
            offset,
            limit,
            shown_end,
            prev_offset: (offset - limit).max(0),
            next_offset: offset + limit,
            page: offset / limit + 1,
            total_pages: page.total_rows / limit + 1,
        }
    }
}
