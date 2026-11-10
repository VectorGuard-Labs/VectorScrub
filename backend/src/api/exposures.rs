use axum::{extract::State, http::StatusCode, Json};

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::exposure as exposure_svc;

/// GET /api/v1/exposures
pub async fn list(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let exposures = exposure_svc::list_exposures(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?;

    Ok(Json(serde_json::json!({ "exposures": exposures })))
}

/// GET /api/v1/exposures/summary
pub async fn summary(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let summary = exposure_svc::get_exposure_summary(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?;

    Ok(Json(serde_json::json!({ "summary": summary })))
}
