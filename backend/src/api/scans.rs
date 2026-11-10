use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use super::error::internal_error;
use crate::crypto::MasterKey;
use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::scan::ScanError;
use crate::services::{audit, customer as customer_svc, scan as scan_svc};

/// POST /api/v1/scans
/// Initiates a new scan job for the authenticated customer.
pub async fn initiate(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    // Resolve tier from the customer record, not from the (possibly stale) JWT claim.
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Customer not found".into()))?;

    // A scan with no identity on file can only fail later inside the agent.
    if !customer_svc::has_identity(&state.pool, auth.id).await.map_err(internal_error)? {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "Submit your identity profile before starting a scan".into(),
        ));
    }

    let master = MasterKey::from_base64(&state.config.master_key_b64).map_err(internal_error)?;

    let job = scan_svc::initiate_scan(
        &state.pool,
        &state.redis,
        &master,
        auth.id,
        &customer.subscription_tier,
    )
    .await
    .map_err(|e| match e {
        ScanError::RateLimited(message) => (StatusCode::TOO_MANY_REQUESTS, message.to_string()),
        other => internal_error(other),
    })?;

    audit::AuditEntry::builder(audit::actions::SCAN_INITIATE, audit::resources::SCAN)
        .customer_id(auth.id)
        .resource_id(job.id)
        .log(&state.pool)
        .await
        .ok();

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
        .map_err(internal_error)?;
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
        .map_err(internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Scan job not found".into()))?;
    Ok(Json(json!({ "job": job })))
}
