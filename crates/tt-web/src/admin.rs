//! The admin database diagnostic viewer (U17): `/admin/db`.
//!
//! The retired viewer had **no guard at all**: `GET /development/db` and its
//! paging route browsed every table -- including `sessions`, exposing every
//! user's email and all session rows to anyone who knew the URL
//! (REBUILD_SPEC.md 12.4). This one is reached only through the `Admin`
//! extractor (`crate::auth`), which a non-admin's request cannot satisfy, and
//! `tt_repo_sqlite::admin` leaves `sessions` and `token_key` out of the table
//! list before this module ever sees a row.

use tracing::warn;
use tt_templates::{AdminCell, AdminPage, AdminRow, AdminTable, AdminTableView, Nav};

use crate::startup::AppState;

/// `None` for the empty cell SQL `NULL` is.
fn cell(label: &str, value: Option<String>) -> AdminCell {
    AdminCell {
        label: label.to_string(),
        value: value.unwrap_or_else(|| "—".into()),
    }
}

/// `table` and `page` come straight off the query string: `None` shows the
/// table list only, and an out-of-range or unreadable page simply shows
/// nothing selected rather than erroring.
pub async fn page(state: &AppState, nav: Nav, table: Option<&str>, page: Option<i64>) -> AdminPage {
    let mut out = AdminPage {
        title: "Database".into(),
        nav,
        unavailable: String::new(),
        tables: Vec::new(),
        selected: None,
    };

    let listed = match tt_repo_sqlite::admin::tables(state.repo.pool()).await {
        Ok(listed) => listed,
        Err(e) => {
            warn!("listing diagnostic tables: {e}");
            out.unavailable = "Could not read the database. Reload to try again.".into();
            return out;
        }
    };
    out.tables = listed
        .iter()
        .map(|t| AdminTable {
            name: t.name.clone(),
            row_count: t.row_count,
            href: format!("/admin/db?table={}", t.name),
            selected: table == Some(t.name.as_str()),
        })
        .collect();

    let Some(table) = table else {
        return out;
    };
    let requested_page = page.unwrap_or(0).max(0);
    match tt_repo_sqlite::admin::rows(state.repo.pool(), table, requested_page).await {
        Ok(Some(view)) => {
            let rows = view
                .rows
                .into_iter()
                .map(|row| AdminRow {
                    cells: view
                        .columns
                        .iter()
                        .zip(row)
                        .map(|(label, value)| cell(label, value))
                        .collect(),
                })
                .collect();
            out.selected = Some(AdminTableView {
                name: view.table.clone(),
                total: view.total,
                columns: view.columns,
                rows,
                has_prev: view.page > 0,
                has_next: (view.page + 1) * view.page_size < view.total,
                prev_href: format!("/admin/db?table={}&page={}", view.table, view.page - 1),
                next_href: format!("/admin/db?table={}&page={}", view.table, view.page + 1),
            });
        }
        // Not a table this viewer shows -- hidden, mistyped, or gone -- so
        // just fall back to the table list rather than guessing at one.
        Ok(None) => {}
        Err(e) => {
            warn!("reading diagnostic table {table}: {e}");
            out.unavailable = "Could not read that table. Reload to try again.".into();
        }
    }
    out
}
