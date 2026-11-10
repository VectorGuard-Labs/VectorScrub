use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware as axum_middleware;
use axum::Router;
use std::time::Duration;
use tower_http::{
    cors::CorsLayer,
    request_id::{MakeRequestUuid, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::services::config::AppConfig;
use csrf::csrf_middleware;
use json_errors::json_errors_middleware;
use metrics_middleware::metrics_middleware;
use security_headers::security_headers_middleware;

pub mod auth_extractor;
pub mod csrf;
pub mod json_errors;
pub mod metrics_middleware;
pub mod rate_limiter;
pub mod security_headers;
pub mod session;

/// Apply the middleware stack to the provided Router and return it.
/// Using direct `.layer()` calls avoids ServiceBuilder tower version conflicts.
pub fn apply<S>(router: Router<S>, config: &AppConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // Parse allowed origins from config. AppConfig guarantees at least one entry, and in
    // production that they are https and not local. If none of them is a valid header value
    // the layer allows no cross-origin caller at all; it never falls back to "any origin".
    let origins: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect();

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-csrf-token"),
        ])
        .allow_origin(origins)
        .allow_credentials(true);

    router
        .layer(cors)
        .layer(axum_middleware::from_fn(csrf_middleware))
        .layer(axum_middleware::from_fn(metrics_middleware))
        .layer(axum_middleware::from_fn_with_state(
            config.hsts_enabled(),
            security_headers_middleware,
        ))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        // Outermost, so it sees every error the layers above produced (CSRF, timeout, ...).
        .layer(axum_middleware::from_fn(json_errors_middleware))
}
