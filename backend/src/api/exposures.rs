use axum::{extract::State, http::StatusCode, Json};

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::exposure as exposure_svc;
use crate::services::url_vault;

/// GET /api/v1/exposures
pub async fn list(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut exposures = exposure_svc::list_exposures(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?;
    let master = crate::crypto::MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(super::error::internal_error)?;
    url_vault::open_all(&master, &mut exposures).map_err(super::error::internal_error)?;

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
