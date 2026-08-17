//! Application state handed to every controller by axum.

use sqlx::PgPool;

/// Shared state for the whole process. The connection pool is currently the
/// only thing in it; anything else with a process lifetime belongs here too.
pub struct AppState {
    /// Lazy PostgreSQL pool. Because it is lazy, holding it does not imply the
    /// database is reachable — queries may still fail, and controllers are
    /// written to degrade rather than panic when they do.
    pub pool: PgPool,
}

/// What controllers receive as `State<SharedState>`.
pub type SharedState = std::sync::Arc<AppState>;
