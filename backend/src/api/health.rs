//! Liveness and readiness probes (unauthenticated, not rate limited).

use axum::{extract::State, http::StatusCode, Json};
use serde_json::json;

use crate::db::AppState;

/// GET /api/v1/health/live: the process is up and serving requests.
pub async fn live() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

/// GET /api/v1/health/ready: dependencies are reachable (PostgreSQL and Redis).
/// Returns 503 naming the failing component, never the underlying error.
pub async fn ready(State(state): State<AppState>) -> (StatusCode, Json<serde_json::Value>) {
    let database = sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool).await.is_ok();

    let redis = match state.redis.get_multiplexed_async_connection().await {
        Ok(mut conn) => redis::cmd("PING").query_async::<String>(&mut conn).await.is_ok(),
        Err(_) => false,
    };

    let ok = database && redis;
    (
        if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        Json(json!({
            "status": if ok { "ready" } else { "unavailable" },
            "database": database,
            "redis": redis,
        })),
    )
}
