//! Database introspection behind the admin DB viewer.
//!
//! This is the one place the app reads arbitrary tables, which makes it the
//! one place SQL injection could occur, so two rules hold:
//!
//! 1. A table name is only interpolated after being matched against
//!    `information_schema.tables` ([`load_page`]); anything else is rejected.
//! 2. Columns in [`SENSITIVE_COLUMNS`] are never selected for display —
//!    today that means `users.password_hash`.
//!
//! Values come back as strings because the viewer renders whatever the schema
//! happens to contain; [`format_value`] decodes the common PostgreSQL types
//! and falls back to an empty cell rather than failing the page.

use once_cell::sync::Lazy;
use sqlx::{PgPool, Row, TypeInfo, ValueRef};
use std::collections::HashMap;

// Sensitive columns never shown in the viewer.
static SENSITIVE_COLUMNS: Lazy<HashMap<&'static str, Vec<&'static str>>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("users", vec!["password_hash"]);
    m
});

/// Column metadata from `information_schema.columns`.
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: String,
}

/// One page of a table: the columns to show and their values as strings.
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: i64,
}

/// Base tables in the `public` schema, alphabetically.
pub async fn table_names(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

/// Row count for one table.
///
/// `table_name` must already be known-good — callers pass names that came
/// from [`table_names`].
pub async fn row_count(pool: &PgPool, table_name: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{table_name}\""))
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// One page of a table, ordered by its first column.
///
/// Validates `table_name` against `information_schema` first and errors on
/// anything unknown, so a crafted URL cannot reach another schema. Sensitive
/// columns are dropped before any value is read.
pub async fn load_page(
    pool: &PgPool,
    table_name: &str,
    offset: i64,
    limit: i64,
) -> anyhow::Result<TablePage> {
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

    let columns: Vec<ColumnInfo> = all_columns
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

    let rows = db_rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|col| format_value(row, &col.name))
                .collect()
        })
        .collect();

    Ok(TablePage {
        columns,
        rows,
        total_rows,
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
