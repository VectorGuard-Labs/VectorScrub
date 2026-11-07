//! Internal endpoints called by the Python agent layer.
//! Protected by X-Agent-Key header, not customer JWT.

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use uuid::Uuid;

use crate::db::AppState;
use crate::middleware::auth_extractor::AgentAuth;
use crate::models::exposure::{ExposureSeverity, PiiCategory};
use crate::models::scan_job::ScanJobStatus;
use crate::services::{exposure as exposure_svc, scan as scan_svc};

#[derive(Debug, Deserialize)]
pub struct ScanStatusUpdate {
    pub job_id: Uuid,
    pub status: ScanJobStatus,
    pub sources_completed: Option<i32>,
    pub sources_failed: Option<i32>,
    pub exposures_found: Option<i32>,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecordExposureRequest {
    pub customer_id: Uuid,
    pub scan_job_id: Uuid,
    pub source_name: String,
    pub source_url: String,
    pub pii_category: PiiCategory,
    pub severity: ExposureSeverity,
    /// Masked summary (e.g. "123 Main St***") — never the full raw PII value
    pub pii_summary: String,
    pub confidence: f32,
}

/// PUT /api/v1/internal/scan/status
pub async fn update_scan_status(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<ScanStatusUpdate>,
) -> Result<StatusCode, (StatusCode, String)> {
    scan_svc::update_scan_status(
        &state.pool,
        req.job_id,
        req.status,
        req.sources_completed,
        req.sources_failed,
        req.exposures_found,
        req.error_message,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/internal/exposures
pub async fn record_exposure(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<RecordExposureRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.confidence < 0.0 || req.confidence > 1.0 {
        return Err((StatusCode::BAD_REQUEST, "Confidence must be between 0.0 and 1.0".into()));
    }

    exposure_svc::record_exposure(
        &state.pool,
        req.customer_id,
        req.scan_job_id,
        &req.source_name,
        &req.source_url,
        req.pii_category,
        req.severity,
        &req.pii_summary,
        req.confidence,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::CREATED)
}

#[derive(Debug, Deserialize)]
pub struct GetIdentityRequest {
    pub customer_id: Uuid,
    pub identity_token: String,
}

#[derive(Debug, serde::Serialize)]
pub struct IdentityResponse {
    pub full_name: String,
    pub current_address: String,
    pub previous_addresses: Vec<String>,
    pub phone_numbers: Vec<String>,
    pub email_addresses: Vec<String>,
    pub date_of_birth: Option<String>,
}

/// POST /api/v1/internal/identity
/// Decrypts and returns customer identity for scan processing
pub async fn get_identity(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<GetIdentityRequest>,
) -> Result<Json<IdentityResponse>, (StatusCode, String)> {
    use crate::crypto::MasterKey;
    use crate::services::customer as customer_svc;

    let master = MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let identity = customer_svc::load_cleartext_identity(&state.pool, &master, req.customer_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(IdentityResponse {
        full_name: identity.full_name.clone(),
        current_address: identity.current_address.clone(),
        previous_addresses: identity.previous_addresses.clone(),
        phone_numbers: identity.phone_numbers.clone(),
        email_addresses: identity.email_addresses.clone(),
        date_of_birth: identity.date_of_birth.clone(),
    }))
}

/// GET /api/v1/internal/customers/:customer_id/identity
/// Decrypts and returns customer identity by customer_id only (for removal worker)
pub async fn get_identity_by_customer_id(
    _auth: AgentAuth,
    State(state): State<AppState>,
    axum::extract::Path(customer_id): axum::extract::Path<Uuid>,
) -> Result<Json<IdentityResponse>, (StatusCode, String)> {
    use crate::crypto::MasterKey;
    use crate::services::customer as customer_svc;

    let master = MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let identity = customer_svc::load_cleartext_identity(&state.pool, &master, customer_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(IdentityResponse {
        full_name: identity.full_name.clone(),
        current_address: identity.current_address.clone(),
        previous_addresses: identity.previous_addresses.clone(),
        phone_numbers: identity.phone_numbers.clone(),
        email_addresses: identity.email_addresses.clone(),
        date_of_birth: identity.date_of_birth.clone(),
    }))
}
