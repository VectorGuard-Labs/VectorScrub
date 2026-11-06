use axum::http::{header, Method, StatusCode};
use axum::Router;
use std::time::Duration;
use tower_http::{
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::services::config::AppConfig;

pub mod auth_extractor;

/// Apply the middleware stack to the provided Router and return it.
/// Using direct `.layer()` calls avoids ServiceBuilder tower version conflicts.
pub fn apply<S>(router: Router<S>, config: &AppConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let cors = if config.is_production() {
        CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
            // TODO: replace Any with your production origin
            .allow_origin(Any)
    } else {
        CorsLayer::permissive()
    };

    router
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
