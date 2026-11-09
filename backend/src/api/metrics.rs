use axum::{extract::State, http::StatusCode, response::IntoResponse};

use crate::db::AppState;

/// GET /api/v1/metrics
/// Prometheus metrics endpoint
///
/// Returns metrics in Prometheus text format that can be scraped by Prometheus server.
/// No authentication required - metrics are considered public monitoring data.
pub async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let metrics = state.metrics.render();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        metrics,
    )
}
