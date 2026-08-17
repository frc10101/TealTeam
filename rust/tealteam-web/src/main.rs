//! Rust/axum port of the TealTeam FRC scouting server.
//!
//! This is one of three interchangeable implementations of the same
//! application — the others are the Go original (`cmd/web/main.go`) and the
//! ASP.NET Core port (`dotnet/TealTeam.Web/Program.cs`). All three serve the
//! same routes, run the same SQL migrations against the same PostgreSQL
//! schema, and issue the same `session_id` cookie with bcrypt password hashes,
//! so they can run side by side against one database and a user created in any
//! of them can sign into the others.
//!
//! It is built to run on a LAN server at a competition (often a Raspberry Pi)
//! with scouting tablets connected over wired ethernet, so every page degrades
//! sensibly when the internet — or even the database — is unreachable.
//!
//! # Layers
//!
//! The crate is organised as MVC, mirroring the C# port's
//! Controllers/Models/Views split:
//!
//! | Module | Role |
//! |---|---|
//! | [`models`] | Entities and **every** SQL statement in the app |
//! | [`views`] | Askama template structs, view models, and all formatting |
//! | [`controllers`] | Request handling: no SQL, no markup |
//! | [`routes`] | The URL table — one route per controller action |
//! | [`services`] | The outside world: FIRST/TBA clients and background sync |
//! | [`web`] | HTTP plumbing shared by controllers |
//! | [`config`], [`db`], [`state`] | Startup configuration, migrations, shared state |
//!
//! Dependencies run one way: controllers use models, views and services; views
//! use models but never the database; models and services know nothing about
//! HTTP.
//!
//! # Request flow
//!
//! ```text
//! Browser ──▶ routes ──▶ controllers ──▶ models   ──▶ PostgreSQL
//!                            │        └─▶ services ──▶ FIRST / TBA
//!                            ▼
//!                          views ──▶ Askama templates ──▶ HTML
//! ```
//!
//! Most pages are full HTML documents; interactive regions are re-rendered as
//! Unpoly fragments returned from the same controllers (see [`web::is_unpoly`]).
//!
//! # Startup
//!
//! [`main`] reads [`config::Config`] from the environment, opens a lazy
//! connection pool, applies migrations from the shared `migrations/`
//! directory, kicks off the boot-time FIRST sync and the background TBA sync
//! loop, then serves [`routes::router`] plus `/static`.

// Entity/view structs mirror the full DB schema; not every column is read back
// out in every code path, which is expected for a faithful port.
#![allow(dead_code)]

mod config;
mod controllers;
mod db;
mod models;
mod routes;
mod services;
mod state;
mod views;
mod web;

use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use state::AppState;
use tower_http::services::ServeDir;
use tracing::{info, warn};

/// Boots the server: configuration, database, background jobs, then serve.
///
/// The pool is lazy and the boot-time `SELECT 1` is only a probe — if the
/// database is down the server still starts (matching the Go app) and
/// DB-backed pages degrade rather than the process refusing to run, which
/// matters when the box powers on before the database container does.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = config::Config::from_environment()?;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    info!("running in {} mode", config.app_env.to_uppercase());

    let pool = PgPoolOptions::new()
        .max_connections(25)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_lazy(&config.database_url)?;

    // Like the Go app, the server still starts if the database is unavailable;
    // DB-backed pages degrade.
    let db_available = sqlx::query("SELECT 1").execute(&pool).await.is_ok();
    if db_available {
        info!("database connected successfully");

        if config.is_test() {
            db::reset_migrations(&pool).await?;
        }
        db::apply_migrations(&pool, &config::migrations_dir()).await?;

        services::first_sync::sync_on_boot(&pool).await;
    } else {
        warn!("database connection failed, running without database");
    }

    let state = Arc::new(AppState { pool: pool.clone() });

    // Background TBA stats/matches sync loop.
    tokio::spawn(services::stats_syncer::run(pool.clone()));

    let app = routes::router(state)
        .nest_service("/static", ServeDir::new(config::static_dir()))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
