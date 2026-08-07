// SQL migration runner tracked in schema_migrations, shared with the Go and
// .NET apps (port of internal/db/migrate.go).

use anyhow::Context;
use sqlx::PgPool;
use tracing::info;

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

/// Clears migration history (test databases only).
pub async fn reset_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("DROP TABLE IF EXISTS schema_migrations")
        .execute(pool)
        .await?;
    Ok(())
}

pub fn resolve_migrations_dir() -> std::path::PathBuf {
    let candidates: [std::path::PathBuf; 4] = [
        crate::exe_relative("migrations").unwrap_or_else(|| "migrations".into()),
        "migrations".into(),
        "../../migrations".into(),
        "rust/tealteam-web/../../migrations".into(),
    ];
    for candidate in candidates {
        if candidate.is_dir() {
            return candidate;
        }
    }
    "migrations".into()
}
