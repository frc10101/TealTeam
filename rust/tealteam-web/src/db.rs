//! SQL migration runner, a port of `internal/db/migrate.go`.
//!
//! Migrations live in the repo-level `migrations/` directory and are shared
//! with the Go and .NET apps: each file is applied once, in filename order,
//! inside a transaction, and recorded in the `schema_migrations` table that
//! all three implementations read. Whichever port boots first applies any
//! pending files; the others then see them as already applied.

use anyhow::Context;
use sqlx::PgPool;
use tracing::info;

/// Applies every `*.sql` file in `dir` that is not yet recorded in
/// `schema_migrations`, in filename order.
///
/// Each file runs in its own transaction together with its history row, so a
/// failing migration leaves no partial schema change behind. Empty files are
/// skipped. Errors if `dir` does not exist.
pub async fn apply_migrations(pool: &PgPool, dir: &std::path::Path) -> anyhow::Result<()> {
    if !dir.is_dir() {
        anyhow::bail!("migrations directory not found: {}", dir.display());
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id SERIAL PRIMARY KEY,
            filename VARCHAR(255) UNIQUE NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(pool)
    .await?;

    let mut files: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".sql"))
        .collect();
    files.sort();

    for file in files {
        let applied: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations WHERE filename = $1")
                .bind(&file)
                .fetch_one(pool)
                .await?;
        if applied > 0 {
            continue;
        }

        let content = std::fs::read_to_string(dir.join(&file))?;
        if content.trim().is_empty() {
            continue;
        }

        let mut tx = pool.begin().await?;
        sqlx::raw_sql(&content)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to apply migration {file}"))?;
        sqlx::query("INSERT INTO schema_migrations (filename) VALUES ($1)")
            .bind(&file)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        info!("applied migration {file}");
    }

    Ok(())
}

/// Drops the migration history table so the next [`apply_migrations`] call
/// re-runs everything.
///
/// Test databases only — [`crate::config::Config::is_test`] gates the call.
pub async fn reset_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("DROP TABLE IF EXISTS schema_migrations")
        .execute(pool)
        .await?;
    Ok(())
}
