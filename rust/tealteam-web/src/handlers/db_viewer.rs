// Admin database viewer (port of Controllers/DbViewerController.cs).

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use once_cell::sync::Lazy;
use sqlx::{Row, TypeInfo, ValueRef};
use std::collections::HashMap;
use tracing::error;

use super::*;
use crate::state::SharedState;

// Sensitive columns never shown in the viewer.
static SENSITIVE_COLUMNS: Lazy<HashMap<&'static str, Vec<&'static str>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("users", vec!["password_hash"]);
    m
});

pub struct TableInfo {
    pub name: String,
    pub row_count: i64,
    pub description: String,
    pub visible: bool,
    pub selected: bool,
}

pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: String,
}

#[derive(Template)]
#[template(path = "pages/db_viewer.html")]
struct DbViewerTemplate {
    title: String,
    nav: Nav,
    active_tab: String,
    tables: Vec<TableInfo>,
    selected_table: String,
}

#[derive(Template)]
#[template(path = "partials/db_table_content.html")]
struct DbTableContentFragment {
    selected_table: String,
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<String>>,
    total_rows: i64,
    offset: i64,
    limit: i64,
    shown_end: i64,
    prev_offset: i64,
    next_offset: i64,
    page: i64,
    total_pages: i64,
}

impl DbViewerTemplate {
    fn tab_class(&self, tab: &str, accent: &str) -> String {
        if self.active_tab == tab || (tab == "core" && self.active_tab.is_empty()) {
            format!("border-{accent}-500 text-{accent}-300")
        } else {
            "border-transparent text-gray-400 hover:text-gray-300".to_string()
        }
    }
}

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
    let visible = tab_tables(&active_tab);

    let table_names: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
         ORDER BY table_name",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut tables = Vec::new();
    for name in table_names {
        let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{name}\""))
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
        let description = table_description(&name, row_count);
        let is_visible = visible.contains(&name.as_str());
        let selected = selected_table == name;
        tables.push(TableInfo { name, row_count, description, visible: is_visible, selected });
    }

    Ok(render(&DbViewerTemplate {
        title: "Database Viewer".to_string(),
        nav: Nav::from_user(Some(&user)),
        active_tab,
        tables,
        selected_table,
    }))
}

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
        return Ok((axum::http::StatusCode::BAD_REQUEST, "Table name is required").into_response());
    }

    let mut limit: i64 = query.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let mut offset: i64 = query.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    if limit <= 0 {
        limit = 50;
    }
    if offset < 0 {
        offset = 0;
    }

    match load_table_data(&state.pool, &name, offset, limit).await {
        Ok(fragment) => Ok(render(&fragment).into_response()),
        Err(e) => {
            error!("failed to get table data for {name}: {e}");
            Ok((
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to get table data: {e}"),
            )
                .into_response())
        }
    }
}

async fn load_table_data(
    pool: &sqlx::PgPool,
    table_name: &str,
    offset: i64,
    limit: i64,
) -> anyhow::Result<DbTableContentFragment> {
    // Validate the table name against information_schema (prevents injection).
    let allowed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name = $1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await?;
    if allowed == 0 {
        anyhow::bail!("invalid table name");
    }

    let all_columns: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT column_name, data_type, is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1
         ORDER BY ordinal_position",
    )
    .bind(table_name)
    .fetch_all(pool)
    .await?;

    let excluded: Vec<&str> = SENSITIVE_COLUMNS.get(table_name).cloned().unwrap_or_default();

    let display_columns: Vec<ColumnInfo> = all_columns
        .iter()
        .filter(|(name, _, _, _)| !excluded.contains(&name.as_str()))
        .map(|(name, type_name, nullable, default)| ColumnInfo {
            name: name.clone(),
            type_name: type_name.clone(),
            nullable: nullable == "YES",
            default: default.clone().unwrap_or_default(),
        })
        .collect();

    let total_rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{table_name}\""))
        .fetch_one(pool)
        .await?;

    let order_by = all_columns
        .first()
        .map(|(n, _, _, _)| format!(" ORDER BY \"{n}\""))
        .unwrap_or_default();
    let sql = format!("SELECT * FROM \"{table_name}\"{order_by} LIMIT {limit} OFFSET {offset}");
    let db_rows = sqlx::query(&sql).fetch_all(pool).await?;

    let mut rows = Vec::new();
    for row in &db_rows {
        let mut values = Vec::new();
        for col in &display_columns {
            values.push(format_value(row, &col.name));
        }
        rows.push(values);
    }

    let shown_end = offset + db_rows.len() as i64;
    Ok(DbTableContentFragment {
        selected_table: table_name.to_string(),
        columns: display_columns,
        rows,
        total_rows,
        offset,
        limit,
        shown_end,
        prev_offset: (offset - limit).max(0),
        next_offset: offset + limit,
        page: offset / limit + 1,
        total_pages: total_rows / limit + 1,
    })
}

/// Best-effort stringification of an arbitrary column value for display.
fn format_value(row: &sqlx::postgres::PgRow, col_name: &str) -> String {
    let Ok(value) = row.try_get_raw(col_name) else {
        return String::new();
    };
    if value.is_null() {
        return String::new();
    }

    let type_info = value.type_info();
    let type_name = type_info.name();
    // Try the common types in turn; fall back to the type name if decoding fails.
    match type_name {
        "INT2" => row.try_get::<i16, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "INT4" => row.try_get::<i32, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "INT8" => row.try_get::<i64, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "FLOAT4" => row.try_get::<f32, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "FLOAT8" => row.try_get::<f64, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "NUMERIC" => row
            .try_get::<f64, _>(col_name)
            .map(|v| v.to_string())
            .or_else(|_| row.try_get::<i64, _>(col_name).map(|v| v.to_string()))
            .unwrap_or_default(),
        "BOOL" => row.try_get::<bool, _>(col_name).map(|v| v.to_string()).unwrap_or_default(),
        "TIMESTAMPTZ" => row
            .try_get::<chrono::DateTime<chrono::Utc>, _>(col_name)
            .map(|v| v.to_rfc3339())
            .unwrap_or_default(),
        "TIMESTAMP" => row
            .try_get::<chrono::NaiveDateTime, _>(col_name)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        "DATE" => row
            .try_get::<chrono::NaiveDate, _>(col_name)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        _ => row.try_get::<String, _>(col_name).unwrap_or_else(|_| String::new()),
    }
}
