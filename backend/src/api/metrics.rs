use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::db::AppState;
use crate::services::config::Environment;

/// GET /api/v1/metrics
/// Prometheus metrics in text format.
///
/// The figures describe traffic, errors and queue behaviour, which is information an attacker can
/// use, so the endpoint is closed unless configured:
///   * `METRICS_TOKEN` set: `Authorization: Bearer <token>` is required (Prometheus sends it with
///     `authorization: { credentials_file: ... }`). Compared in constant time.
///   * no token, development: open, for local work.
///   * no token, staging or production: 404, as if the endpoint did not exist.
pub async fn get_metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match &state.config.metrics_token {
        Some(token) => {
            let presented = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split_once(' '))
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map(|(_, t)| t.trim())
                .unwrap_or("");
            if !crate::crypto::secrets_equal(presented.as_bytes(), token.as_bytes()) {
                return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")], "Metrics token required").into_response();
            }
        }
        None if state.config.environment == Environment::Development => {}
        None => return StatusCode::NOT_FOUND.into_response(),
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
        .into_response()
}
