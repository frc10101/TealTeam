// Rust/axum port of the TealTeam scouting server (cmd/web/main.go and
// dotnet/TealTeam.Web/Program.cs). Same routes, same PostgreSQL schema and
// migrations, same session cookie — all three ports can share one database.

// Entity/view structs mirror the full DB schema; not every column is read back
// out in every code path, which is expected for a faithful port.
#![allow(dead_code)]

mod connectivity;
mod db;
mod first_api;
mod first_sync;
mod handlers;
mod models;
mod scouting_points;
mod session;
mod state;
mod stats_syncer;
mod tba;
mod tba_stats_sync;

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use state::AppState;
use tower_http::services::ServeDir;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env for local development (app dir first, then repo root).
    load_dotenv(".env");
    load_dotenv("../../.env");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    // "test" resets migration history on boot (like the Go app's -env=test).
    let app_env = std::env::var("TEALTEAM_ENV")
        .unwrap_or_else(|_| "test".into())
        .trim()
        .to_lowercase();
    if app_env != "test" && app_env != "prod" {
        anyhow::bail!("invalid environment: {app_env}");
    }

    let port = std::env::var("PORT")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "8080".into());

    let database_url = if app_env == "prod" {
        std::env::var("RENDER_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| {
                anyhow::anyhow!(
                    "production mode requires RENDER_DATABASE_URL or DATABASE_URL environment variable"
                )
            })?
    } else {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable".into())
    };

    info!("running in {} mode", app_env.to_uppercase());

    let pool = PgPoolOptions::new()
        .max_connections(25)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_lazy(&database_url)?;

    // Like the Go app, the server still starts if the database is unavailable;
    // DB-backed pages degrade.
    let db_available = sqlx::query("SELECT 1").execute(&pool).await.is_ok();
    if db_available {
        info!("database connected successfully");

        if app_env == "test" {
            db::reset_migrations(&pool).await?;
        }
        db::apply_migrations(&pool, &db::resolve_migrations_dir()).await?;

        first_sync::sync_on_boot(&pool).await;
    } else {
        warn!("database connection failed, running without database");
    }

    let state = Arc::new(AppState { pool: pool.clone() });

    // Background TBA stats/matches sync loop.
    tokio::spawn(stats_syncer::run(pool.clone()));

    let app = handlers::router(state)
        .nest_service("/static", ServeDir::new(resolve_static_dir()))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Minimal .env loader mirroring godotenv (ignored if missing; existing
/// environment variables win).
fn load_dotenv(path: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(idx) = line.find('=') else { continue };
        if idx == 0 {
            continue;
        }
        let key = line[..idx].trim();
        let value = line[idx + 1..].trim().trim_matches('"');
        if std::env::var(key).is_err() {
            std::env::set_var(key, value);
        }
    }
}

fn resolve_static_dir() -> std::path::PathBuf {
    // Next to the binary in a publish layout, or the source tree in dev.
    let candidates = [
        exe_relative("static"),
        Some("static".into()),
        Some("rust/tealteam-web/static".into()),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.is_dir() {
            return candidate;
        }
    }
    "static".into()
}

fn exe_relative(name: &str) -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(name))
}
