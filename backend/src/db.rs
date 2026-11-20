use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::PgPool;

use std::sync::Arc;

use crate::services::config::AppConfig;
use crate::services::keys::KeyService;

/// Shared application state injected into all route handlers via Axum's State extractor.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub redis: redis::Client,
    pub config: AppConfig,
    pub metrics: PrometheusHandle,
    /// Where customers' data keys come from (PyHSM in production). See `services::keys`.
    pub keys: Arc<KeyService>,
}
