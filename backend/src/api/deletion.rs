//! Deletion requests sent to companies on the customer's behalf. See services/deletion.rs.
//!
//! Everything here answers 404 unless `DELETION_REQUESTS_ENABLED` is set.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::error::internal_error;
use crate::crypto::MasterKey;
use crate::db::AppState;
use crate::middleware::auth_extractor::{AgentAuth, AuthenticatedCustomer};
use crate::services::deletion::{self, DeletionError, NewCampaign};
use crate::services::identity_access::{self, AccessError};
use crate::services::{audit, broker_channels, customer as customer_svc};

type ApiError = (StatusCode, String);

fn enabled(state: &AppState) -> Result<(), ApiError> {
    if state.config.deletion_requests_enabled {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, "Not found".into()))
    }
}

fn master(state: &AppState) -> Result<MasterKey, ApiError> {
    MasterKey::from_base64(&state.config.master_key_b64).map_err(internal_error)
}

fn deletion_error(e: DeletionError) -> ApiError {
    match e {
        DeletionError::PaidPlanRequired => (StatusCode::FORBIDDEN, "Deletion requests are part of the paid plan.".into()),
        DeletionError::ConsentRequired => (StatusCode::UNPROCESSABLE_ENTITY, "Please accept the authorization to continue.".into()),
        DeletionError::AuthorizationOutdated => (StatusCode::CONFLICT, "The authorization text has changed. Please review it and try again.".into()),
        DeletionError::NothingSelected => (StatusCode::UNPROCESSABLE_ENTITY, "Select at least one company.".into()),
        DeletionError::UnknownChannel => (StatusCode::UNPROCESSABLE_ENTITY, "One of the selected companies is not available.".into()),
        DeletionError::NotEligible => (StatusCode::FORBIDDEN, "One of the selected companies is not available for your location.".into()),
        DeletionError::AddressRequired => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "One of the selected companies needs your address to find your records. Tick \"include my address\" or deselect it.".into(),
        ),
        DeletionError::TooSoon => (StatusCode::CONFLICT, "You already sent a request to one of these companies in the last 90 days.".into()),
        DeletionError::NotFound => (StatusCode::NOT_FOUND, "Request not found.".into()),
        DeletionError::IllegalTransition => (StatusCode::CONFLICT, "That change is not allowed for this request.".into()),
        DeletionError::Db(e) => internal_error(e),
    }
}

// ---------------------------------------------------------------------------
// Customer
// ---------------------------------------------------------------------------

/// GET /api/v1/deletion-requests/info
/// The authorization text, and every company with where its request would go, the evidence for
/// that address, and whether this customer may be offered it.
pub async fn info(auth: AuthenticatedCustomer, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Customer not found".to_string()))?;
    let residence = customer_svc::residence_state(&state.pool, &master(&state)?, auth.id)
        .await
        .map_err(internal_error)?;
    let paid = customer.subscription_tier != crate::models::customer::SubscriptionTier::Free;

    let channels: Vec<Value> = broker_channels::catalog()
        .iter()
        .map(|c| {
            let eligible = broker_channels::eligible(c, residence);
            json!({
                "id": c.id, "company": c.company, "address": c.address,
                "source_url": c.source_url, "evidence": c.evidence, "verified_on": c.verified_on,
                "applies_to": c.applies_to, "requests": c.requests, "legal_basis": c.legal_basis,
                "agent_requirements": c.agent_requirements, "notes": c.notes,
                "requires_address": c.requires_address,
                "eligible": eligible,
                "ineligible_reason": if eligible { Value::Null } else {
                    json!(if residence.is_none() {
                        "We could not tell your state from your saved address. This company's page offers this route to residents of specific states."
                    } else {
                        "This company's page offers this route only to residents of other states."
                    })
                },
            })
        })
        .collect();

    Ok(Json(json!({
        "authorization": { "version": deletion::AUTHORIZATION_VERSION, "text": deletion::AUTHORIZATION_TEXT },
        "plan_ok": paid,
        "cooldown_days": deletion::COOLDOWN_DAYS,
        "channels": channels,
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateCampaign {
    pub channel_ids: Vec<String>,
    pub include_address: bool,
    pub consent: bool,
    pub authorization_version: String,
}

/// POST /api/v1/deletion-campaigns
pub async fn create_campaign(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<CreateCampaign>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    enabled(&state)?;
    if req.channel_ids.len() > 50 {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "Too many companies selected.".into()));
    }
    let residence = customer_svc::residence_state(&state.pool, &master(&state)?, auth.id)
        .await
        .map_err(internal_error)?;
    let (campaign_id, request_ids) = deletion::create_campaign(
        &state.pool,
        auth.id,
        residence,
        NewCampaign {
            channel_ids: &req.channel_ids,
            include_address: req.include_address,
            consent: req.consent,
            authorization_version: &req.authorization_version,
        },
    )
    .await
    .map_err(deletion_error)?;

    audit::AuditEntry::builder(audit::actions::DELETION_AUTHORIZE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(campaign_id)
        .details(json!({
            "channels": req.channel_ids, "include_address": req.include_address,
            "authorization_version": deletion::AUTHORIZATION_VERSION,
            "authorization_sha256": deletion::authorization_sha256(),
        }))
        .log_or_warn(&state.pool)
        .await;

    Ok((StatusCode::ACCEPTED, Json(json!({
        "campaign_id": campaign_id, "requests": request_ids,
        "message": "Your requests are queued. We will send them shortly.",
    }))))
}

/// GET /api/v1/deletion-requests
pub async fn list(auth: AuthenticatedCustomer, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let requests = deletion::list_for_customer(&state.pool, auth.id).await.map_err(internal_error)?;
    Ok(Json(json!({ "requests": requests })))
}

#[derive(Debug, Deserialize)]
pub struct Outcome {
    pub outcome: String,
}

/// POST /api/v1/deletion-requests/:id/outcome
/// The customer reports what the company answered (replied, removed or denied).
pub async fn outcome(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<Outcome>,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?;
    deletion::report_outcome(&state.pool, auth.id, id, &req.outcome).await.map_err(deletion_error)?;
    audit::AuditEntry::builder(audit::actions::DELETION_OUTCOME, audit::resources::DELETION_REQUEST)
        .customer_id(auth.id)
        .resource_id(id)
        .details(json!({ "reported": req.outcome }))
        .log_or_warn(&state.pool)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Agent (authenticated by the agent key)
// ---------------------------------------------------------------------------

/// GET /api/v1/internal/deletion-requests
/// Requests waiting to be sent, each with the company, the address (from the catalog, never from
/// the request), and the authorization the customer accepted. Empty unless the feature is on.
pub async fn queued(_auth: AgentAuth, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    if !state.config.deletion_requests_enabled {
        return Ok(Json(json!({ "requests": [] })));
    }
    let rows = deletion::list_queued(&state.pool).await.map_err(internal_error)?;
    let mut out = Vec::new();
    for r in rows {
        let Some(channel) = broker_channels::find(&r.channel_id) else {
            deletion::fail_queued(&state.pool, r.id, "This company is no longer available.").await.map_err(internal_error)?;
            continue;
        };
        out.push(json!({
            "id": r.id, "customer_id": r.customer_id, "channel_id": channel.id,
            "company": channel.company, "to_address": channel.address, "subject": channel.subject,
            "legal_basis": channel.legal_basis, "requests": channel.requests,
            "include_address": r.include_address,
            "authorized_at": r.authorized_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
            "authorization_version": r.authorization_version, "authorization_text": r.authorization_text,
        }));
    }
    Ok(Json(json!({ "requests": out })))
}

/// POST /api/v1/internal/deletion-requests/:id/claim
/// 204 for exactly one caller; 409 for everyone else. The worker must not send unless it got 204.
pub async fn claim(_auth: AgentAuth, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    enabled(&state)?;
    if deletion::claim(&state.pool, id).await.map_err(internal_error)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::CONFLICT, "Request is not available to claim".into()))
    }
}

/// POST /api/v1/internal/deletion-requests/:id/identity
/// The details for this one email: name, first email address, and the address only if the
/// customer agreed to share it. Only for a request that has been claimed and is being sent.
pub async fn identity(_auth: AgentAuth, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let identity = identity_access::release_for_deletion(&state.pool, &master(&state)?, id)
        .await
        .map_err(|e| match e {
            AccessError::Denied => (StatusCode::FORBIDDEN, "Not available".to_string()),
            AccessError::Db(e) => internal_error(e),
            AccessError::Internal(e) => internal_error(e),
        })?;
    Ok(Json(json!({
        "full_name": identity.full_name,
        "email_addresses": identity.email_addresses,
        "current_address": identity.current_address,
    })))
}

#[derive(Debug, Deserialize)]
pub struct SendResult {
    pub sent: bool,
    pub message_id: Option<String>,
    pub error: Option<String>,
}

/// PUT /api/v1/internal/deletion-requests/:id/result
pub async fn result(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SendResult>,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?;
    let customer = deletion::customer_of(&state.pool, id).await.map_err(internal_error)?;
    let applied = deletion::finish(&state.pool, id, req.sent, req.message_id.as_deref(), req.error.as_deref())
        .await
        .map_err(internal_error)?;
    if !applied {
        return Err((StatusCode::CONFLICT, "Request is not being sent".into()));
    }
    if req.sent {
        let mut entry = audit::AuditEntry::builder(audit::actions::DELETION_SENT, audit::resources::DELETION_REQUEST).resource_id(id);
        if let Some(c) = customer {
            entry = entry.customer_id(c);
        }
        entry.log_or_warn(&state.pool).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
