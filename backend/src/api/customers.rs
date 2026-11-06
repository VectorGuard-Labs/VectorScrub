use axum::{extract::State, http::StatusCode, Json};

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::customer::{CustomerProfile, SubmitIdentityRequest};
use crate::services::customer as customer_svc;
use crate::crypto::MasterKey;
use validator::Validate;

/// GET /api/v1/me
pub async fn get_profile(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<CustomerProfile>, (StatusCode, String)> {
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Customer not found".into()))?;

    Ok(Json(customer.into()))
}

/// POST /api/v1/me/identity
/// Accepts a customer's PII, encrypts it, and persists the encrypted profile.
pub async fn submit_identity(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<SubmitIdentityRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let master = MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    customer_svc::save_identity(&state.pool, &master, &req, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "message": "Identity profile saved successfully" })),
    ))
}
