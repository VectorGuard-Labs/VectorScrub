use crate::db::AppState;
use crate::middleware::auth_extractor::{AgentAuth, AuthenticatedCustomer};
use crate::models::removal_request::{RemovalAuditEntry, RemovalRequest, RemovalStatus};
use crate::services::{audit, removal, url_vault};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Open the stored (sealed) listing links on rows about to leave the API. See services/url_vault.rs.
fn open_links<T: url_vault::CarriesLink>(state: &AppState, rows: &mut [T]) -> anyhow::Result<()> {
    let master = crate::crypto::MasterKey::from_base64(&state.config.master_key_b64)?;
    url_vault::open_all(&master, rows)
}

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
    let mut requests = removal::list_removal_requests(&state.pool, auth.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    open_links(&state, &mut requests).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ListRemovalRequestsResponse { requests }))
}

/// POST /api/v1/removal-requests/:id/done-by-hand
/// The customer says they completed, by hand, an opt-out our automation could not finish (the site
/// blocks automated browsers). The request becomes `submitted` and its listing is re-checked
/// later; only evidence that the listing is gone marks it removed. See
/// `removal::mark_done_by_hand`.
pub async fn done_by_hand(
    auth: AuthenticatedCustomer,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(report): Json<removal::ManualReport>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let new_link = match report.listing_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        None => None,
        Some(raw) => {
            let (site, normalized) = crate::services::listing::site_of(raw)
                .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, match e {
                crate::services::listing::ListingError::NotAListing => e.to_string(),
                _ => "That profile link is not a valid https:// link to the site.".to_string(),
            }))?;
            let master = crate::crypto::MasterKey::from_base64(&state.config.master_key_b64).map_err(super::error::internal_error)?;
            let sealed = url_vault::seal(&master, auth.id, &normalized).map_err(super::error::internal_error)?;
            Some(removal::NewListingLink { site, sealed })
        }
    };
    let mut request = removal::mark_done_by_hand(&state.pool, auth.id, id, report, new_link)
        .await
        .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
            Some(removal::RemovalError::OtherSiteLink) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "That profile link is on a different site from this listing.".to_string(),
            ),
            Some(removal::RemovalError::BadReport) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "The date you submitted it cannot be in the future or more than 60 days ago.".to_string(),
            ),
            Some(removal::RemovalError::NotFound) => (StatusCode::NOT_FOUND, "Removal request not found.".to_string()),
            Some(removal::RemovalError::AlreadyInFlight) => (
                StatusCode::CONFLICT,
                "Another request for this listing is already in progress.".to_string(),
            ),
            Some(removal::RemovalError::NotActionable) => (
                StatusCode::CONFLICT,
                "This listing is no longer active, so there is nothing left to remove.".to_string(),
            ),
            Some(removal::RemovalError::IllegalTransition { .. }) => (
                StatusCode::CONFLICT,
                "Only a request that failed can be marked as done by hand.".to_string(),
            ),
            _ => super::error::internal_error(format!("{e:?}")),
        })?;
    open_links(&state, std::slice::from_mut(&mut request)).map_err(super::error::internal_error)?;

    audit::AuditEntry::builder(audit::actions::REMOVAL_MANUAL_SUBMIT, audit::resources::EXPOSURE)
        .customer_id(auth.id)
        .resource_id(request.exposure_id)
        .details(serde_json::json!({ "removal_request_id": request.id }))
        .log_or_warn(&state.pool)
        .await;

    Ok(Json(serde_json::json!({
        "request": request,
        "message": "Thanks, we have recorded what you told us. We will re-check the listing on a schedule (about 3, 7, 14 and 30 days after you submitted it, up to 45) and tell you what we find. Some sites take weeks.",
    })))
}

#[derive(Deserialize)]
pub struct ListingLinkRequest {
    listing_url: String,
}

/// POST /api/v1/removal-requests/:id/listing-link
/// Replace the link we re-check (same site only) on a request still open. See
/// `removal::update_listing_link`.
pub async fn update_listing_link(
    auth: AuthenticatedCustomer,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<ListingLinkRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (site, normalized) = crate::services::listing::site_of(&body.listing_url)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, match e {
                crate::services::listing::ListingError::NotAListing => e.to_string(),
                _ => "That profile link is not a valid https:// link to the site.".to_string(),
            }))?;
    let master = crate::crypto::MasterKey::from_base64(&state.config.master_key_b64).map_err(super::error::internal_error)?;
    let sealed = url_vault::seal(&master, auth.id, &normalized).map_err(super::error::internal_error)?;
    let mut request = removal::update_listing_link(&state.pool, auth.id, id, removal::NewListingLink { site, sealed })
        .await
        .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
            Some(removal::RemovalError::NotFound) => (StatusCode::NOT_FOUND, "Removal request not found.".to_string()),
            Some(removal::RemovalError::OtherSiteLink) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "That profile link is on a different site from this listing.".to_string(),
            ),
            Some(removal::RemovalError::NotActionable) => (
                StatusCode::CONFLICT,
                "This request is closed, so its link can no longer be changed.".to_string(),
            ),
            _ => super::error::internal_error(format!("{e:?}")),
        })?;
    open_links(&state, std::slice::from_mut(&mut request)).map_err(super::error::internal_error)?;
    Ok(Json(serde_json::json!({
        "request": request,
        "message": "Updated. Our re-checks will look at this page from now on.",
    })))
}

/// POST /api/v1/removal-requests/:id/request-assistance
/// Asks for a person to help with a request automation could not finish. A hand-worked pilot
/// queue: it records the ask, promises no result or time, and asks for no free text. See
/// `removal::request_assistance`.
pub async fn request_assistance(
    auth: AuthenticatedCustomer,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut request = removal::request_assistance(&state.pool, auth.id, id)
        .await
        .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
            Some(removal::RemovalError::NotFound) => (StatusCode::NOT_FOUND, "Removal request not found.".to_string()),
            Some(removal::RemovalError::AssistanceNotApplicable) => (
                StatusCode::CONFLICT,
                "Help can only be requested for a removal that needs manual action.".to_string(),
            ),
            Some(removal::RemovalError::TooManyAssistanceRequests) => (
                StatusCode::CONFLICT,
                format!("You already have {} open help requests. We will get to those first.", removal::MAX_OPEN_ASSISTANCE_PER_CUSTOMER),
            ),
            _ => super::error::internal_error(format!("{e:?}")),
        })?;
    open_links(&state, std::slice::from_mut(&mut request)).map_err(super::error::internal_error)?;

    audit::AuditEntry::builder(audit::actions::REMOVAL_ASSISTANCE_REQUEST, audit::resources::EXPOSURE)
        .customer_id(auth.id)
        .resource_id(request.exposure_id)
        .details(serde_json::json!({ "removal_request_id": request.id }))
        .log_or_warn(&state.pool)
        .await;

    Ok(Json(serde_json::json!({
        "request": request,
        "message": "We have your request. A person will write to you at your account email. This is a free pilot service with no guaranteed response time, and we will never ask for your email password.",
    })))
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
) -> Result<Json<AuthorizeRemovalResponse>, (StatusCode, String)> {
    // Verify exposure belongs to customer
    let exposure = crate::services::exposure::get_exposure(&state.pool, exposure_id)
        .await
        .map_err(super::error::internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Exposure not found".to_string()))?;

    if exposure.customer_id != auth.id {
        return Err((StatusCode::FORBIDDEN, "Not your exposure".to_string()));
    }

    if !exposure.is_active {
        return Err((StatusCode::BAD_REQUEST, "This exposure is no longer active".to_string()));
    }

    let mut request = removal::authorize_removal(&state.pool, auth.id, exposure_id)
        .await
        .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
            Some(removal::RemovalError::AlreadyInFlight) => (
                StatusCode::CONFLICT,
                "A removal request for this exposure is already in progress".to_string(),
            ),
            Some(removal::RemovalError::NotFound) => (StatusCode::NOT_FOUND, "Exposure not found".to_string()),
            Some(removal::RemovalError::FreeLimitReached) => (
                StatusCode::FORBIDDEN,
                format!(
                    "Free accounts can submit up to {} opt-out requests. Upgrade to Personal to continue.",
                    removal::FREE_REMOVAL_LIMIT
                ),
            ),
            _ => super::error::internal_error(format!("{e:?}")),
        })?;
    open_links(&state, std::slice::from_mut(&mut request)).map_err(super::error::internal_error)?;

    audit::AuditEntry::builder(audit::actions::EXPOSURE_AUTHORIZE_REMOVAL, audit::resources::EXPOSURE)
        .customer_id(auth.id)
        .resource_id(exposure_id)
        .details(serde_json::json!({ "removal_request_id": request.id }))
        .log_or_warn(&state.pool)
        .await;

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
    let mut request = removal::get_removal_request(&state.pool, id, auth.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    open_links(&state, std::slice::from_mut(&mut request)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    requests: Vec<RemovalRequest>,
}

/// GET /api/v1/internal/removal-requests?status=authorized
/// Lists removal requests by status for agent processing (oldest first, at most 100).
/// Each request already carries the source name and URL it was authorized for.
pub async fn list_pending(
    _auth: AgentAuth,
    Query(params): Query<ListPendingQuery>,
    State(state): State<AppState>,
) -> Result<Json<ListPendingResponse>, StatusCode> {
    let status = params.status.unwrap_or(RemovalStatus::Authorized);

    let mut requests = removal::list_removal_requests_by_status(&state.pool, status)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    open_links(&state, &mut requests).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ListPendingResponse { requests }))
}

/// POST /api/v1/internal/removal-requests/:id/claim
/// Claims an authorized request for submission. 204 for exactly one caller; 409 for every
/// other (already claimed, no longer authorized, or unknown), and the agent must then not
/// submit. This is what stops a restart or a second worker from sending the same request
/// to a broker twice.
pub async fn claim_submission(
    _auth: AgentAuth,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    match removal::claim_submission(&state.pool, id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(StatusCode::CONFLICT),
        Err(e) => {
            tracing::error!(error = ?e, "Failed to claim removal submission");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
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
    .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
        Some(removal::RemovalError::NotFound) => StatusCode::NOT_FOUND,
        Some(removal::RemovalError::IllegalTransition { .. }) => StatusCode::CONFLICT,
        Some(
            removal::RemovalError::AlreadyInFlight
            | removal::RemovalError::FreeLimitReached
            | removal::RemovalError::NotActionable
            | removal::RemovalError::BadReport
            | removal::RemovalError::OtherSiteLink
            | removal::RemovalError::AssistanceNotApplicable
            | removal::RemovalError::TooManyAssistanceRequests,
        )
        | None => {
            tracing::error!(error = %e, "Failed to update removal status");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;

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
    .map_err(|e| match e.downcast_ref::<removal::RemovalError>() {
        Some(removal::RemovalError::NotFound) => StatusCode::NOT_FOUND,
        _ => {
            tracing::error!(error = %e, "Failed to record removal audit entry");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;

    Ok(StatusCode::CREATED)
}
