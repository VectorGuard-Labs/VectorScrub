use crate::db::AppState;
use crate::middleware::auth_extractor::{AgentAuth, AuthenticatedCustomer};
use crate::models::removal_request::{RemovalAuditEntry, RemovalRequest, RemovalStatus};
use crate::services::removal;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

// Customer endpoints

#[derive(Serialize)]
pub struct ListRemovalRequestsResponse {
    requests: Vec<RemovalRequest>,
}

/// GET /api/v1/removal-requests
/// Lists all removal requests for the authenticated customer.
pub async fn list_requests(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<ListRemovalRequestsResponse>, StatusCode> {
    let requests = removal::list_removal_requests(&state.pool, auth.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ListRemovalRequestsResponse { requests }))
}

#[derive(Serialize)]
pub struct AuthorizeRemovalResponse {
    request: RemovalRequest,
    message: String,
}

/// POST /api/v1/removal-requests/:exposure_id/authorize
/// Authorizes a removal request for a specific exposure.
pub async fn authorize(
    auth: AuthenticatedCustomer,
    Path(exposure_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<AuthorizeRemovalResponse>, StatusCode> {
    // Verify exposure belongs to customer
    let exposure = crate::services::exposure::get_exposure(&state.pool, exposure_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if exposure.customer_id != auth.id {
        return Err(StatusCode::FORBIDDEN);
    }

    if !exposure.is_active {
        return Err(StatusCode::BAD_REQUEST);
    }

    let request = removal::authorize_removal(&state.pool, auth.id, exposure_id)
        .await
        .map_err(|e| {
            if e.to_string().contains("already authorized") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;

    Ok(Json(AuthorizeRemovalResponse {
        request,
        message: "Removal request authorized and queued for processing".to_string(),
    }))
}

#[derive(Serialize)]
pub struct GetRemovalRequestResponse {
    request: RemovalRequest,
    audit_log: Vec<RemovalAuditEntry>,
}

/// GET /api/v1/removal-requests/:id
/// Gets a specific removal request with audit log.
pub async fn get_request(
    auth: AuthenticatedCustomer,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<GetRemovalRequestResponse>, StatusCode> {
    let request = removal::get_removal_request(&state.pool, id, auth.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let audit_log = removal::get_removal_audit_log(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(GetRemovalRequestResponse {
        request,
        audit_log,
    }))
}

// Internal agent endpoints

#[derive(Deserialize)]
pub struct ListPendingQuery {
    status: Option<RemovalStatus>,
}

#[derive(Serialize)]
pub struct ListPendingResponse {
    requests: Vec<RemovalRequestWithExposure>,
}

#[derive(Serialize)]
pub struct RemovalRequestWithExposure {
    #[serde(flatten)]
    pub request: RemovalRequest,
    pub source_name: String,
    pub source_url: String,
}

/// GET /api/v1/internal/removal-requests?status=authorized
/// Lists removal requests by status for agent processing.
pub async fn list_pending(
    _auth: AgentAuth,
    Query(params): Query<ListPendingQuery>,
    State(state): State<AppState>,
) -> Result<Json<ListPendingResponse>, StatusCode> {
    let status = params.status.unwrap_or(RemovalStatus::Authorized);

    let requests = removal::list_removal_requests_by_status(&state.pool, status)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Join with exposures to get source info
    let mut requests_with_exposure = Vec::new();
    for request in requests {
        let exposure = crate::services::exposure::get_exposure(&state.pool, request.exposure_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        requests_with_exposure.push(RemovalRequestWithExposure {
            request,
            source_name: exposure.source_name,
            source_url: exposure.source_url,
        });
    }

    Ok(Json(ListPendingResponse {
        requests: requests_with_exposure,
    }))
}

#[derive(Deserialize)]
pub struct UpdateStatusRequest {
    status: RemovalStatus,
    confirmation_ref: Option<String>,
    evidence_hash: Option<String>,
    next_check_days: Option<i32>,
}

/// PUT /api/v1/internal/removal-requests/:id/status
/// Updates removal request status.
pub async fn update_status(
    _auth: AgentAuth,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<UpdateStatusRequest>,
) -> Result<StatusCode, StatusCode> {
    let next_check_at = req.next_check_days.map(|days| {
        OffsetDateTime::now_utc() + time::Duration::days(days as i64)
    });

    removal::update_removal_status(
        &state.pool,
        id,
        req.status,
        req.confirmation_ref,
        req.evidence_hash,
        next_check_at,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RecordAuditRequest {
    customer_id: Uuid,
    action: String,
    actor: String,
    details: serde_json::Value,
}

/// POST /api/v1/internal/removal-requests/:id/audit
/// Records an audit log entry.
pub async fn record_audit(
    _auth: AgentAuth,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(req): Json<RecordAuditRequest>,
) -> Result<StatusCode, StatusCode> {
    removal::record_audit_entry(
        &state.pool,
        id,
        req.customer_id,
        req.action,
        req.actor,
        req.details,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::CREATED)
}
