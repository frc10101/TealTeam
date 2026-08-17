//! Admin database viewer.
//!
//! Admin-only (`is_admin`, not lead scout), read-only, and useful at an event
//! for answering "did that actually save?" without a psql session on the Pi.
//! Table names are validated and sensitive columns filtered in
//! [`crate::models::schema`].

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use std::collections::HashMap;
use tracing::error;

use crate::models::schema;
use crate::state::SharedState;
use crate::views::db_viewer::{DbTableContentFragment, DbViewerTemplate, TableInfo};
use crate::views::{render, Nav};
use crate::web::*;

const DEFAULT_PAGE_SIZE: i64 = 50;

/// `GET /development/db` — tabs and the table list with row counts.
pub async fn db_viewer(
    State(state): State<SharedState>,
    jar: CookieJar,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.is_admin {
        return Ok(redirect("/"));
    }

    let active_tab = query.get("tab").cloned().unwrap_or_else(|| "core".to_string());
    let selected_table = query.get("table").cloned().unwrap_or_default();

    let mut tables = Vec::new();
    for name in schema::table_names(&state.pool).await {
        let row_count = schema::row_count(&state.pool, &name).await;
        tables.push(TableInfo::new(name, row_count, &active_tab, &selected_table));
    }

    Ok(render(&DbViewerTemplate::new(
        Nav::from_user(Some(&user)),
        active_tab,
        tables,
        selected_table,
    )))
}

/// `GET /hx/development/db/table/:name` — one page of one table.
///
/// `limit` and `offset` come from the query string and are clamped to sane
/// values; an unknown table name is a 500 from the model's validation.
pub async fn table_content(
    State(state): State<SharedState>,
    jar: CookieJar,
    Path(name): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> HandlerResult {
    let Some(user) = current_user(&state.pool, &jar).await else {
        return Ok(redirect("/"));
    };
    if !user.is_admin {
        return Ok(redirect("/"));
    }
    if name.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Table name is required").into_response());
    }

    let mut limit: i64 = query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PAGE_SIZE);
    let mut offset: i64 = query.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    if limit <= 0 {
        limit = DEFAULT_PAGE_SIZE;
    }
    if offset < 0 {
        offset = 0;
    }

    match schema::load_page(&state.pool, &name, offset, limit).await {
        Ok(page) => Ok(render(&DbTableContentFragment::new(&name, page, offset, limit)).into_response()),
        Err(e) => {
            error!("failed to get table data for {name}: {e}");
            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get table data: {e}"),
            )
                .into_response())
        }
    }
}
