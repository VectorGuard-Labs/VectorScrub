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

fn deletion_error(e: DeletionError) -> ApiError {
    match e {
        DeletionError::PaidPlanRequired => (StatusCode::FORBIDDEN, "Deletion requests are part of the paid plan.".into()),
        DeletionError::ConsentRequired => (StatusCode::UNPROCESSABLE_ENTITY, "Please accept the authorization to continue.".into()),
        DeletionError::AuthorizationOutdated => (StatusCode::CONFLICT, "The authorization text has changed. Please review it and try again.".into()),
        DeletionError::NothingSelected => (StatusCode::UNPROCESSABLE_ENTITY, "Select at least one company.".into()),
        DeletionError::UnknownChannel => (StatusCode::UNPROCESSABLE_ENTITY, "One of the selected companies is not available.".into()),
        DeletionError::NotEligible => (StatusCode::FORBIDDEN, "One of the selected companies is not available for your location.".into()),
        DeletionError::OutsidePilot => (StatusCode::FORBIDDEN, "This pilot is not open in your state yet.".into()),
        DeletionError::NoRequestKind => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Choose what to ask for: deleting your data, stopping its sale or sharing, or both.".into(),
        ),
        DeletionError::KindNotOffered => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "One of the selected companies does not offer the kind of request you chose.".into(),
        ),
        DeletionError::SignatureRequired => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Please sign the authorization: agree to sign electronically and type your full name.".into(),
        ),
        DeletionError::SignatureMismatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Type your full name exactly as it appears in your saved details. It is the name the companies receive.".into(),
        ),
        DeletionError::DetailsMissing => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Save your full name and an email address in your details first. The requests are sent in that name.".into(),
        ),
        DeletionError::CampaignWithdrawn => (StatusCode::CONFLICT, "That authorization was withdrawn.".into()),
        DeletionError::Keys(e) => internal_error(e),
        DeletionError::TooLateToWithdraw => (
            StatusCode::CONFLICT,
            "That request has already been sent, or is being sent, so it can no longer be withdrawn.".into(),
        ),
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
    let residence = customer_svc::residence_state(&state.pool, &state.keys, auth.id)
        .await
        .map_err(internal_error)?;
    let paid = customer.subscription_tier != crate::models::customer::SubscriptionTier::Free;

    let in_pilot = broker_channels::in_pilot(residence, &state.config.deletion_pilot_states);
    let channels: Vec<Value> = broker_channels::catalog()
        .iter()
        .map(|c| {
            let eligible = in_pilot && broker_channels::eligible(c, residence);
            json!({
                "id": c.id, "company": c.company, "address": c.address, "registry_id": c.registry_id,
                "source_url": c.source_url, "evidence": c.evidence, "verified_on": c.verified_on,
                "applies_to": c.applies_to, "requests": c.requests, "legal_basis": c.legal_basis,
                "agent_requirements": c.agent_requirements, "notes": c.notes,
                "requires_address": c.requires_address,
                "eligible": eligible,
                "ineligible_reason": if eligible { Value::Null } else {
                    json!(if residence.is_none() {
                        "We could not tell your state from your saved address. This pilot is open only to residents of specific states, and this company's page offers this route to residents of specific states."
                    } else if !in_pilot {
                        "This pilot is not open in your state yet."
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
        "pilot_states": state.config.deletion_pilot_states,
        "in_pilot": in_pilot,
        "request_kinds": broker_channels::REQUEST_KINDS,
        "cooldown_days": deletion::COOLDOWN_DAYS,
        "channels": channels,
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateCampaign {
    pub channel_ids: Vec<String>,
    /// `delete` and/or `opt_out_sale`. Separate choices: nothing is assumed.
    #[serde(default)]
    pub request_kinds: Vec<String>,
    pub include_address: bool,
    pub consent: bool,
    pub authorization_version: String,
    /// The full name typed as a signature. Never logged.
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub esign_consent: bool,
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
    let residence = customer_svc::residence_state(&state.pool, &state.keys, auth.id)
        .await
        .map_err(internal_error)?;
    let signer = deletion::signer_for(&state.pool, &state.keys, auth.id).await.map_err(internal_error)?;
    let (campaign_id, request_ids) = deletion::create_campaign(
        &state.pool,
        &state.keys,
        auth.id,
        residence,
        &state.config.deletion_pilot_states,
        signer.as_ref(),
        NewCampaign {
            channel_ids: &req.channel_ids,
            request_kinds: &req.request_kinds,
            include_address: req.include_address,
            consent: req.consent,
            authorization_version: &req.authorization_version,
            signature: &req.signature,
            esign_consent: req.esign_consent,
        },
    )
    .await
    .map_err(deletion_error)?;

    audit::AuditEntry::builder(audit::actions::DELETION_AUTHORIZE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(campaign_id)
        .details(json!({
            "channels": req.channel_ids, "request_kinds": req.request_kinds, "include_address": req.include_address,
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

/// POST /api/v1/deletion-requests/:id/withdraw
/// Cancels one request that has not been sent. 409 once it is being sent or was sent.
pub async fn withdraw_request(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?;
    deletion::withdraw_request(&state.pool, auth.id, id).await.map_err(deletion_error)?;
    audit::AuditEntry::builder(audit::actions::DELETION_WITHDRAW, audit::resources::DELETION_REQUEST)
        .customer_id(auth.id)
        .resource_id(id)
        .log_or_warn(&state.pool)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/deletion-campaigns/:id/withdraw
/// Cancels every request of the campaign that has not been sent. Answers how many were cancelled.
pub async fn withdraw_campaign(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    enabled(&state)?;
    let withdrawn = deletion::withdraw_campaign(&state.pool, auth.id, id).await.map_err(deletion_error)?;
    audit::AuditEntry::builder(audit::actions::DELETION_WITHDRAW, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(id)
        .details(json!({ "withdrawn": withdrawn }))
        .log_or_warn(&state.pool)
        .await;
    Ok(Json(json!({ "withdrawn": withdrawn })))
}

/// GET /api/v1/deletion-campaigns/:id/authorization
/// The record of what the customer accepted and what became of each request, to keep or to show a
/// company that asks for proof. Includes the signed document (the customer's name and email), opened
/// for its owner only.
pub async fn authorization_record(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<deletion::AuthorizationRecord>, ApiError> {
    enabled(&state)?;
    deletion::authorization_record(&state.pool, &state.keys, auth.id, id).await.map(Json).map_err(deletion_error)
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
    let interrupted = deletion::fail_stale_sending(&state.pool).await.map_err(internal_error)?;
    if interrupted > 0 {
        metrics::counter!("deletion_requests_interrupted_total").increment(interrupted);
        tracing::warn!(interrupted, "Deletion request sends were interrupted; an operator must check the mail logs (`vectorscrub deletion interrupted`)");
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
            "legal_basis": channel.legal_basis, "requests": deletion::asked_of(&channel.id, &r.request_kinds),
            "include_address": r.include_address,
            "authorized_at": r.authorized_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
            "authorization_version": r.authorization_version, "authorization_text": r.authorization_text,
            "signed_document_sha256": r.signed_document_sha256,
        }));
    }
    Ok(Json(json!({ "requests": out })))
}

#[derive(Debug, Deserialize, Default)]
pub struct ClaimBody {
    /// The Message-ID the worker will send with (`<local@domain>`). Recorded before sending.
    pub message_id: Option<String>,
}

/// A Message-ID as RFC 5322 writes it: one `<...@...>` token, printable, no spaces.
fn plain_message_id(m: &str) -> bool {
    m.len() <= 300 && m.starts_with('<') && m.ends_with('>') && m.contains('@')
        && m.chars().all(|c| c.is_ascii_graphic())
}

/// POST /api/v1/internal/deletion-requests/:id/claim
/// 204 for exactly one caller; 409 for everyone else. The worker must not send unless it got 204.
/// The body may carry the Message-ID the email will be sent with.
pub async fn claim(
    _auth: AgentAuth,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?;
    let body: ClaimBody = if body.is_empty() {
        ClaimBody::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| (StatusCode::UNPROCESSABLE_ENTITY, "Invalid claim body".to_string()))?
    };
    if body.message_id.as_deref().is_some_and(|m| !plain_message_id(m)) {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "message_id must be one <local@domain> token".into()));
    }
    if deletion::claim(&state.pool, id, body.message_id.as_deref()).await.map_err(internal_error)? {
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
    let identity = identity_access::release_for_deletion(&state.pool, &state.keys, id)
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
    metrics::counter!(if req.sent { "deletion_requests_sent_total" } else { "deletion_requests_failed_total" }).increment(1);
    if req.sent {
        let mut entry = audit::AuditEntry::builder(audit::actions::DELETION_SENT, audit::resources::DELETION_REQUEST).resource_id(id);
        if let Some(c) = customer {
            entry = entry.customer_id(c);
        }
        entry.log_or_warn(&state.pool).await;
    }
    Ok(StatusCode::NO_CONTENT)
}
