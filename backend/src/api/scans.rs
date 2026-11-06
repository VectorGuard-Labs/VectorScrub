use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use uuid::Uuid;

use crate::crypto::{derive_customer_key, encrypt_str, MasterKey};
use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::{customer as customer_svc, scan as scan_svc};

/// POST /api/v1/scans
/// Initiates a new scan job for the authenticated customer.
pub async fn initiate(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    // Resolve tier from customer record
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Customer not found".into()))?;

    // Load identity (verifies it exists before queuing)
    let master = MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Create a short-lived identity token the Python agent can use to
    // fetch only what it needs via the internal API.
    // Format: base64(encrypted JSON {customer_id, issued_at})
    let token_payload = serde_json::json!({
        "customer_id": auth.id,
        "issued_at": time::OffsetDateTime::now_utc().unix_timestamp()
    })
    .to_string();
    let key = derive_customer_key(&master, auth.id);
    let encrypted = encrypt_str(&key, &token_payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let identity_token = BASE64.encode(&encrypted);

    let job = scan_svc::initiate_scan(
        &state.pool,
        &state.redis,
        auth.id,
        &customer.subscription_tier,
        identity_token,
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "job_id": job.id,
            "status": job.status,
            "message": "Scan job queued. Your AI agents are working."
        })),
    ))
}

/// GET /api/v1/scans
pub async fn list(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let jobs = scan_svc::list_scan_jobs(&state.pool, auth.id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "jobs": jobs })))
}

/// GET /api/v1/scans/:job_id
pub async fn get_job(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job = scan_svc::get_scan_job(&state.pool, job_id, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Scan job not found".into()))?;
    Ok(Json(json!({ "job": job })))
}
