use sqlx::PgPool;

use crate::services::config::AppConfig;

/// Shared application state injected into all route handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub redis: redis::Client,
    pub config: AppConfig,
}
