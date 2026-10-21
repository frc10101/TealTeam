//! Read-only diagnostic queries behind the admin database viewer (U17).
//!
//! The retired viewer had **no guard at all**: `GET /development/db` browsed
//! every table, including `sessions`, to any visitor who knew the URL, and
//! leaked every user's email along the way (REBUILD_SPEC.md 12.4). Guarding
//! the route is tt-web's job, through its `Admin` extractor. This module's job
//! is to make sure that even a guarded, honest admin cannot see a session
//! token, the offline-token signing key, or a plaintext email through it.
//!
//! Every statement below is a `SELECT`. There is no function here, and no
//! planned one, that writes.

use sqlx::sqlite::SqlitePool;
use sqlx::{Row, TypeInfo, ValueRef};
use tt_repo::Result;

use crate::users::query_err;

/// Never listed, and never queryable by name, however it is asked for: a
/// session id is a bearer credential good for the rest of its life, and
/// `token_key` is the secret that signs offline tokens (C9). Showing either
/// would recreate the exact leak this item exists to close.
pub const HIDDEN_TABLES: &[&str] = &["sessions", "token_key"];

/// Replaced with [`MASK`] wherever one of these appears, by column name: a
/// password hash is secret regardless of which table holds it, and a
/// plaintext email is exactly what the retired viewer leaked.
const MASKED_COLUMNS: &[&str] = &["email", "password_hash"];
const MASK: &str = "••••••";

const PAGE_SIZE: i64 = 50;

/// A table the viewer may show, with how many rows it currently holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticTable {
    pub name: String,
    pub row_count: i64,
}

/// One page of a table's rows, already masked -- callers never see a raw row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRows {
    pub table: String,
    pub columns: Vec<String>,
    /// `None` is a SQL `NULL`, not a masked value.
    pub rows: Vec<Vec<Option<String>>>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Every table the viewer may show, alphabetically, with its row count.
///
/// [`HIDDEN_TABLES`] never appears here, so there is no later step -- not a
/// crafted `table` parameter, not a table some future migration adds -- that
/// can surface one: [`rows`] only ever looks a name up in what this returned.
pub async fn tables(pool: &SqlitePool) -> Result<Vec<DiagnosticTable>> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '\\_%' ESCAPE '\\' \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| query_err("listing tables", e))?;

    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if HIDDEN_TABLES.contains(&name.as_str()) {
            continue;
        }
        // `name` is only ever one `sqlite_master` itself just said exists,
        // never a caller's input, so building SQL from it here cannot inject
        // anything.
        let row_count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            r#"SELECT COUNT(*) FROM "{name}""#
        )))
        .fetch_one(pool)
        .await
        .map_err(|e| query_err(&format!("counting {name}"), e))?;
        out.push(DiagnosticTable { name, row_count });
    }
    Ok(out)
}

/// A page of `table`'s rows, masked. `page` is zero-based.
///
/// `Ok(None)` for anything not in [`tables`] -- a hidden table, a typo, or not
/// a table at all -- rather than building a query from an unchecked name.
pub async fn rows(pool: &SqlitePool, table: &str, page: i64) -> Result<Option<DiagnosticRows>> {
    let Some(found) = tables(pool).await?.into_iter().find(|t| t.name == table) else {
        return Ok(None);
    };
    let page = page.max(0);
    // `?page=` is the admin's to type: a huge one is past the end, not an
    // overflow.
    let offset = page.saturating_mul(PAGE_SIZE);

    // `table` is only used below once it has matched a name `tables` itself
    // returned, so it carries no caller-chosen text into the query either.
    let columns: Vec<String> = sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"PRAGMA table_info("{table}")"#
    )))
    .fetch_all(pool)
    .await
    .map_err(|e| query_err(&format!("reading {table}'s columns"), e))?
    .into_iter()
    .map(|r| r.get::<String, _>("name"))
    .collect();

    let fetched = sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"SELECT * FROM "{table}" LIMIT {PAGE_SIZE} OFFSET {offset}"#
    )))
    .fetch_all(pool)
    .await
    .map_err(|e| query_err(&format!("reading {table}"), e))?;

    let masked_rows = fetched
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let value = cell_to_string(row, i);
                    if MASKED_COLUMNS.contains(&name.as_str()) {
                        value.map(|_| MASK.to_string())
                    } else {
                        value
                    }
                })
                .collect()
        })
        .collect();

    Ok(Some(DiagnosticRows {
        table: found.name,
        columns,
        rows: masked_rows,
        total: found.row_count,
        page,
        page_size: PAGE_SIZE,
    }))
}

/// One cell, read without assuming its type ahead of time.
///
/// SQLite is dynamically typed per value, not per column, and guessing wrong
/// has a silent failure mode worth avoiding: decoding a `REAL` as `i64` calls
/// `sqlite3_value_int64`, which truncates rather than erroring, so a careless
/// "try `i64` first" would quietly show the wrong number instead of failing
/// loudly. Checking the value's own type first avoids that.
fn cell_to_string(row: &sqlx::sqlite::SqliteRow, index: usize) -> Option<String> {
    let raw = row.try_get_raw(index).ok()?;
    if raw.is_null() {
        return None;
    }
    match raw.type_info().name() {
        "INTEGER" => row.try_get::<i64, _>(index).ok().map(|v| v.to_string()),
        "REAL" => row.try_get::<f64, _>(index).ok().map(|v| v.to_string()),
        "TEXT" => row.try_get::<String, _>(index).ok(),
        "BLOB" => row
            .try_get::<Vec<u8>, _>(index)
            .ok()
            .map(|b| format!("<{} bytes>", b.len())),
        other => Some(format!("<{other}>")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteRepo;

    async fn migrated() -> SqlitePool {
        let repo = SqliteRepo::connect("sqlite::memory:").expect("connect");
        crate::migrate::apply(repo.pool()).await.expect("migrate");
        repo.pool().clone()
    }

    async fn insert_user(pool: &SqlitePool, email: &str, name: &str) {
        sqlx::query(
            "INSERT INTO users (email, name, password_hash, created_at, updated_at) \
             VALUES (?, ?, 'argon2-hash', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(email)
        .bind(name)
        .execute(pool)
        .await
        .expect("insert user");
    }

    #[tokio::test]
    async fn sessions_and_the_token_key_never_appear_or_answer() {
        let pool = migrated().await;
        let names: Vec<String> = tables(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        for hidden in HIDDEN_TABLES {
            assert!(
                !names.contains(&hidden.to_string()),
                "{hidden} must not be listed"
            );
        }
        assert!(
            names.contains(&"users".to_string()),
            "ordinary tables still show"
        );

        // Asking for a hidden table by name directly, rather than only
        // through the list, must be refused the same way -- this is the
        // check the retired viewer skipped entirely (REBUILD_SPEC.md 12.4).
        for hidden in HIDDEN_TABLES {
            assert!(rows(&pool, hidden, 0).await.unwrap().is_none(), "{hidden}");
        }
        assert!(rows(&pool, "not_a_real_table", 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn emails_and_password_hashes_are_masked_but_other_columns_are_not() {
        let pool = migrated().await;
        insert_user(&pool, "sam@example.com", "Sam").await;

        let page = rows(&pool, "users", 0)
            .await
            .unwrap()
            .expect("users is listed");
        let col = |name: &str| page.columns.iter().position(|c| c == name).unwrap();

        assert_eq!(page.rows[0][col("email")].as_deref(), Some(MASK));
        assert_eq!(page.rows[0][col("password_hash")].as_deref(), Some(MASK));
        assert_eq!(page.rows[0][col("name")].as_deref(), Some("Sam"));
    }

    #[tokio::test]
    async fn a_null_cell_is_none_not_a_mask_or_a_placeholder() {
        let pool = migrated().await;
        insert_user(&pool, "sam@example.com", "Sam").await;

        let page = rows(&pool, "users", 0).await.unwrap().unwrap();
        let col = |name: &str| page.columns.iter().position(|c| c == name).unwrap();
        assert_eq!(page.rows[0][col("team_number")], None);
    }

    #[tokio::test]
    async fn row_counts_match_and_a_page_holds_what_it_says() {
        let pool = migrated().await;
        for n in 1..=3 {
            insert_user(&pool, &format!("user{n}@example.com"), &format!("User {n}")).await;
        }

        let listed = tables(&pool).await.unwrap();
        let users = listed.iter().find(|t| t.name == "users").unwrap();
        assert_eq!(users.row_count, 3);

        let page = rows(&pool, "users", 0).await.unwrap().unwrap();
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.total, 3);
        assert_eq!(page.page, 0);
    }

    #[tokio::test]
    async fn a_page_past_any_end_is_empty_not_an_overflow() {
        let pool = migrated().await;
        insert_user(&pool, "sam@example.com", "Sam").await;

        let page = rows(&pool, "users", i64::MAX).await.unwrap().unwrap();
        assert!(page.rows.is_empty());
        assert_eq!(page.total, 1);
    }
}
