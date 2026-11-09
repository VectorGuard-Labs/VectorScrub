use axum::http::{header, HeaderValue, Method, StatusCode};
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
use metrics_middleware::metrics_middleware;
use security_headers::security_headers_middleware;

pub mod auth_extractor;
pub mod csrf;
pub mod metrics_middleware;
pub mod rate_limiter;
pub mod security_headers;

/// Apply the middleware stack to the provided Router and return it.
/// Using direct `.layer()` calls avoids ServiceBuilder tower version conflicts.
pub fn apply<S>(router: Router<S>, config: &AppConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // Parse allowed origins from config
    let origins: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect();

    let cors = if origins.is_empty() {
        // Fallback to permissive for development if no origins configured
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
            .allow_origin(origins)
            .allow_credentials(true)
    };

    // TODO: Add proper rate limiting using tower_governor
    // For now, we rely on timeout layer (30s) as basic protection

    router
        .layer(cors)
        .layer(axum_middleware::from_fn(csrf_middleware))
        .layer(axum_middleware::from_fn(metrics_middleware))
        .layer(axum_middleware::from_fn(security_headers_middleware))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
