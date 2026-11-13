//! Internal endpoints called by the Python agent layer.
//! Protected by the X-Agent-Key header (compared in constant time), not customer JWT.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use super::error::internal_error;
use crate::crypto::MasterKey;
use crate::db::AppState;
use crate::middleware::auth_extractor::AgentAuth;
use crate::models::customer::CleartextIdentity;
use crate::models::exposure::{ExposureSeverity, PiiCategory};
use crate::models::scan_job::{ScanJobStatus, SourceCoverage, MAX_COVERAGE_ENTRIES};
use crate::services::identity_access::{self, AccessError};
use crate::services::monitoring::{
    self as monitoring_svc, MonitoringError, MonitoringOutcome, MonitoringTarget,
    VerificationOutcome, VerificationTarget,
};
use crate::services::optout_health::{self, OptOutPageCheck, MAX_CHECKS_PER_REPORT};
use crate::services::scan::StatusOutcome;
use crate::services::{exposure as exposure_svc, scan as scan_svc, url_vault};

// ---------------------------------------------------------------------------
// Scans: claim -> progress/heartbeat -> complete | fail
// ---------------------------------------------------------------------------

/// POST /api/v1/internal/scan/:job_id/claim
/// A worker takes a queued job before processing it. Exactly one worker can win, so a
/// duplicate queue entry (after a recovery re-dispatch) is harmless. The returned
/// `attempt` is the fencing token to send with every later status update.
pub async fn claim_scan(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match scan_svc::claim_scan_job(&state.pool, job_id).await.map_err(internal_error)? {
        Some(attempt) => Ok(Json(serde_json::json!({ "attempt": attempt }))),
        None => Err((StatusCode::CONFLICT, "Scan job is not available to claim".into())),
    }
}

#[derive(Debug, Deserialize, Validate)]
pub struct ScanStatusUpdate {
    pub job_id: Uuid,
    pub status: ScanJobStatus,
    /// The attempt number returned by the claim. Updates from any other attempt are refused.
    pub attempt: Option<i32>,
    #[validate(range(min = 0, max = 10000))]
    pub sources_completed: Option<i32>,
    #[validate(range(min = 0, max = 10000))]
    pub sources_failed: Option<i32>,
    #[validate(range(min = 0, max = 100000))]
    pub exposures_found: Option<i32>,
    #[validate(length(max = 1000))]
    pub error_message: Option<String>,
    /// Per-source outcomes of the discovery step (see `SourceCoverage`). Sent with the final
    /// status. Names are checked below; an unknown outcome is refused by deserialization.
    pub coverage: Option<Vec<SourceCoverage>>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct RecordExposureRequest {
    pub customer_id: Uuid,
    pub scan_job_id: Uuid,
    #[validate(length(min = 1, max = 200))]
    pub source_name: String,
    #[validate(url, length(max = 2000))]
    pub source_url: String,
    pub pii_category: PiiCategory,
    pub severity: ExposureSeverity,
    /// Masked summary (e.g. "123 Main St***") — never the full raw PII value
    #[validate(length(min = 1, max = 500))]
    pub pii_summary: String,
    #[validate(range(min = 0.0, max = 1.0))]
    pub confidence: f32,
}

/// PUT /api/v1/internal/scan/status
/// Progress (`running`, also a heartbeat), completion, or failure from the worker that
/// holds the job. 409 means this worker no longer owns the job and must stop.
pub async fn update_scan_status(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<ScanStatusUpdate>,
) -> Result<StatusCode, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Workers may report progress, success, or failure. Queued/cancelled are not theirs to set.
    if !matches!(req.status, ScanJobStatus::Running | ScanJobStatus::Completed | ScanJobStatus::Failed) {
        return Err((StatusCode::BAD_REQUEST, "Status must be running, completed, or failed".into()));
    }

    if let Some(coverage) = &req.coverage {
        if coverage.len() > MAX_COVERAGE_ENTRIES || !coverage.iter().all(SourceCoverage::is_valid) {
            return Err((StatusCode::BAD_REQUEST, "Invalid coverage report".into()));
        }
        // Before the status, so "completed" never reaches the customer without its coverage.
        scan_svc::record_coverage(&state.pool, req.job_id, req.attempt, coverage)
            .await
            .map_err(internal_error)?;
    }

    let outcome = scan_svc::update_scan_status(
        &state.pool,
        req.job_id,
        req.status,
        req.sources_completed,
        req.sources_failed,
        req.exposures_found,
        req.error_message,
        req.attempt,
    )
    .await
    .map_err(internal_error)?;

    match outcome {
        StatusOutcome::Applied => Ok(StatusCode::NO_CONTENT),
        StatusOutcome::Stale => Err((
            StatusCode::CONFLICT,
            "Scan job is not running under this attempt".into(),
        )),
    }
}

/// POST /api/v1/internal/exposures
/// Records an exposure found by a scan. The scan job must belong to the customer and be
/// running: the agent cannot attach findings to arbitrary customers.
pub async fn record_exposure(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<RecordExposureRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM scan_jobs WHERE id = $1 AND customer_id = $2 AND status = 'running'",
    )
    .bind(req.scan_job_id)
    .bind(req.customer_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error)?;
    if owned.is_none() {
        return Err((
            StatusCode::CONFLICT,
            "Scan job is not running for this customer".into(),
        ));
    }

    // The link can contain the person's name: store it sealed with their key.
    let sealed_url = url_vault::seal(&master(&state)?, req.customer_id, &req.source_url).map_err(internal_error)?;
    exposure_svc::record_exposure(
        &state.pool,
        req.customer_id,
        req.scan_job_id,
        &req.source_name,
        &sealed_url,
        req.pii_category,
        req.severity,
        &req.pii_summary,
        req.confidence,
    )
    .await
    .map_err(internal_error)?;

    Ok(StatusCode::CREATED)
}

// ---------------------------------------------------------------------------
// Identity: released only against a concrete, actionable work item
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Validate)]
pub struct ScanIdentityRequest {
    pub customer_id: Uuid,
    pub job_id: Uuid,
    /// The token minted for exactly this job (see services/identity_access.rs).
    #[validate(length(min = 1, max = 2000))]
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

impl From<&CleartextIdentity> for IdentityResponse {
    fn from(i: &CleartextIdentity) -> Self {
        Self {
            full_name: i.full_name.clone(),
            current_address: i.current_address.clone(),
            previous_addresses: i.previous_addresses.clone(),
            phone_numbers: i.phone_numbers.clone(),
            email_addresses: i.email_addresses.clone(),
            date_of_birth: i.date_of_birth.clone(),
        }
    }
}

fn access_error(e: AccessError) -> (StatusCode, String) {
    match e {
        // Deliberately the same for every reason: do not tell a caller why it was refused.
        AccessError::Denied => (StatusCode::FORBIDDEN, "Identity access denied".into()),
        other => internal_error(other),
    }
}

fn master(state: &AppState) -> Result<MasterKey, (StatusCode, String)> {
    MasterKey::from_base64(&state.config.master_key_b64).map_err(internal_error)
}

/// POST /api/v1/internal/identity
/// Identity for a scan job. Requires the job-scoped token that was minted when the job
/// was dispatched, and the job must still be queued or running.
pub async fn scan_identity(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<ScanIdentityRequest>,
) -> Result<Json<IdentityResponse>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let identity = identity_access::release_for_scan(
        &state.pool,
        &master(&state)?,
        req.customer_id,
        req.job_id,
        &req.identity_token,
    )
    .await
    .map_err(access_error)?;
    Ok(Json(IdentityResponse::from(&identity)))
}

/// POST /api/v1/internal/removal-requests/:id/identity
/// Identity to submit a customer-authorized removal, or to verify a submitted one that is
/// due. The customer is taken from the request, never from the caller.
pub async fn removal_identity(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
) -> Result<Json<IdentityResponse>, (StatusCode, String)> {
    let identity = identity_access::release_for_removal(&state.pool, &master(&state)?, request_id)
        .await
        .map_err(access_error)?;
    Ok(Json(IdentityResponse::from(&identity)))
}

/// POST /api/v1/internal/exposures/:id/identity
/// Identity for a reappearance check on a removed exposure that is due. The customer is
/// taken from the exposure, never from the caller.
pub async fn monitoring_identity(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(exposure_id): Path<Uuid>,
) -> Result<Json<IdentityResponse>, (StatusCode, String)> {
    let identity = identity_access::release_for_monitoring(&state.pool, &master(&state)?, exposure_id)
        .await
        .map_err(access_error)?;
    Ok(Json(IdentityResponse::from(&identity)))
}

// ---------------------------------------------------------------------------
// Monitoring: reappearance checks and removal verification
// ---------------------------------------------------------------------------

const DEFAULT_BATCH: i64 = 25;
const MAX_BATCH: i64 = 100;

#[derive(Debug, Deserialize)]
pub struct BatchQuery {
    limit: Option<i64>,
}

impl BatchQuery {
    fn limit(&self) -> i64 {
        self.limit.unwrap_or(DEFAULT_BATCH).clamp(1, MAX_BATCH)
    }
}

#[derive(Debug, Serialize)]
pub struct DueExposuresResponse {
    exposures: Vec<MonitoringTarget>,
}

#[derive(Debug, Serialize)]
pub struct DueVerificationsResponse {
    requests: Vec<VerificationTarget>,
}

#[derive(Debug, Deserialize)]
pub struct MonitoringResultRequest {
    pub customer_id: Uuid,
    pub outcome: MonitoringOutcome,
}

#[derive(Debug, Deserialize)]
pub struct VerificationResultRequest {
    pub customer_id: Uuid,
    pub outcome: VerificationOutcome,
}

fn monitoring_error(e: MonitoringError) -> (StatusCode, String) {
    match e {
        MonitoringError::NotFound => (StatusCode::NOT_FOUND, "Target not found".to_string()),
        MonitoringError::WrongState => (
            StatusCode::CONFLICT,
            "Target is not in a state that accepts this result".to_string(),
        ),
        other => {
            tracing::error!(error = %other, "Monitoring callback failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error while recording monitoring result".to_string(),
            )
        }
    }
}

fn db_error(e: sqlx::Error) -> (StatusCode, String) {
    tracing::error!(error = %e, "Monitoring query failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal error while loading monitoring work".to_string(),
    )
}

/// GET /api/v1/internal/exposures/due-for-monitoring?limit=N
/// Removed exposures whose tier interval has elapsed (Premium weekly, Personal monthly).
pub async fn exposures_due_for_monitoring(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Query(q): Query<BatchQuery>,
) -> Result<Json<DueExposuresResponse>, (StatusCode, String)> {
    let mut exposures = monitoring_svc::list_exposures_due_for_monitoring(&state.pool, q.limit())
        .await
        .map_err(db_error)?;
    url_vault::open_all(&master(&state)?, &mut exposures).map_err(internal_error)?;
    Ok(Json(DueExposuresResponse { exposures }))
}

/// POST /api/v1/internal/exposures/:exposure_id/monitoring-result
/// Records the outcome of a reappearance check.
pub async fn record_monitoring_result(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(exposure_id): Path<Uuid>,
    Json(req): Json<MonitoringResultRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    monitoring_svc::record_monitoring_result(&state.pool, exposure_id, req.customer_id, req.outcome)
        .await
        .map_err(monitoring_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/internal/removal-requests/due-for-verification?limit=N
/// Submitted removal requests whose check time has passed.
pub async fn removals_due_for_verification(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Query(q): Query<BatchQuery>,
) -> Result<Json<DueVerificationsResponse>, (StatusCode, String)> {
    let mut requests = monitoring_svc::list_removals_due_for_verification(&state.pool, q.limit())
        .await
        .map_err(db_error)?;
    url_vault::open_all(&master(&state)?, &mut requests).map_err(internal_error)?;
    Ok(Json(DueVerificationsResponse { requests }))
}

/// POST /api/v1/internal/removal-requests/:id/verification-result
/// Records the outcome of a removal verification check.
pub async fn record_verification_result(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(removal_request_id): Path<Uuid>,
    Json(req): Json<VerificationResultRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    monitoring_svc::record_verification_result(
        &state.pool,
        removal_request_id,
        req.customer_id,
        req.outcome,
    )
    .await
    .map_err(monitoring_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct OptOutHealthReport {
    pub checks: Vec<OptOutPageCheck>,
}

/// PUT /api/v1/internal/optout-health
/// The nightly report of which brokers' opt-out pages loaded. Public pages only: no customer
/// data is involved, so no work item is required, just the agent key.
pub async fn report_optout_health(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Json(req): Json<OptOutHealthReport>,
) -> Result<StatusCode, (StatusCode, String)> {
    if req.checks.len() > MAX_CHECKS_PER_REPORT || !req.checks.iter().all(OptOutPageCheck::is_valid) {
        return Err((StatusCode::BAD_REQUEST, "Invalid opt-out health report".into()));
    }
    optout_health::record_checks(&state.pool, &req.checks).await.map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}
