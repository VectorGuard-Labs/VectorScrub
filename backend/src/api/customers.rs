use axum::{extract::State, http::StatusCode, Json};

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::customer::{CustomerProfile, SubmitIdentityRequest};
use crate::services::{audit, customer as customer_svc};
use crate::crypto::MasterKey;
use validator::Validate;

/// GET /api/v1/me
pub async fn get_profile(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<CustomerProfile>, (StatusCode, String)> {
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?
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
        .map_err(super::error::internal_error)?;

    customer_svc::save_identity(&state.pool, &master, &req, auth.id)
        .await
        .map_err(super::error::internal_error)?;

    // Record that identity data was stored or replaced (never its contents).
    audit::AuditEntry::builder(audit::actions::IDENTITY_SUBMIT, audit::resources::IDENTITY)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .log_or_warn(&state.pool)
        .await;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "message": "Identity profile saved successfully" })),
    ))
}

/// GET /api/v1/me/residence
/// The US state of the signed-in customer's saved address, as two letters, or null if there is
/// no address or its state cannot be told. The opt-out guide uses it to offer California's DROP
/// service to California residents only. Only the state is returned, never the address.
pub async fn get_residence(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let master = MasterKey::from_base64(&state.config.master_key_b64)
        .map_err(super::error::internal_error)?;
    let found = customer_svc::residence_state(&state.pool, &master, auth.id)
        .await
        .map_err(super::error::internal_error)?;
    Ok(Json(serde_json::json!({ "state": found })))
}

#[derive(serde::Deserialize)]
pub struct AddListingRequest {
    pub source_url: String,
}

/// POST /api/v1/me/listings
/// The customer found their own listing on a site we cannot scan and wants it tracked: it becomes
/// an exposure with a manual-action-required removal, followed by the usual self-report and
/// re-checks. See services/listing.rs.
pub async fn add_listing(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<AddListingRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    use crate::services::{audit, listing, url_vault};
    let (site, normalized) = listing::site_of(&req.source_url).map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Customer not found".into()))?;
    let tier = format!("{:?}", customer.subscription_tier).to_lowercase();

    let master = MasterKey::from_base64(&state.config.master_key_b64).map_err(super::error::internal_error)?;
    let sealed = url_vault::seal(&master, auth.id, &normalized).map_err(super::error::internal_error)?;

    let mut exposure = listing::add_listing(&state.pool, auth.id, &tier, &site, &sealed)
        .await
        .map_err(|e| match e.downcast_ref::<listing::ListingError>() {
            Some(le @ listing::ListingError::AlreadyTracked) => (StatusCode::CONFLICT, le.to_string()),
            Some(le @ listing::ListingError::TooMany(_)) => (StatusCode::CONFLICT, le.to_string()),
            Some(le @ (listing::ListingError::BadUrl | listing::ListingError::NotAListing)) => (StatusCode::UNPROCESSABLE_ENTITY, le.to_string()),
            None => super::error::internal_error(format!("{e:?}")),
        })?;
    url_vault::open_all(&master, std::slice::from_mut(&mut exposure)).map_err(super::error::internal_error)?;

    audit::AuditEntry::builder(audit::actions::LISTING_ADD, audit::resources::EXPOSURE)
        .customer_id(auth.id)
        .resource_id(exposure.id)
        .details(serde_json::json!({ "site": site }))
        .log_or_warn(&state.pool)
        .await;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "exposure": exposure,
            "message": "Added. Follow the steps on the Removals page, then tell us when you have submitted it and we will re-check the listing.",
        })),
    ))
}
