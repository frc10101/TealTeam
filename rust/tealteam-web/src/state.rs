use sqlx::PgPool;

pub struct AppState {
    pub pool: PgPool,
}

pub type SharedState = std::sync::Arc<AppState>;
