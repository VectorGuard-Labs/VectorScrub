use axum::{body::Bytes, extract::State, http::HeaderMap, http::StatusCode, Json};
use serde_json::json;
use time::OffsetDateTime;

use super::error::internal_error;
use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::billing::{self, BillingError, BillingView};
use crate::services::customer as customer_svc;

/// POST /api/v1/webhooks/stripe
///
/// Called by Stripe, not by a browser: no session, no CSRF token. The only authentication is
/// the `Stripe-Signature` header, checked against the RAW request body before anything is
/// read from it. Answers 2xx once the event is handled or deliberately ignored; a 5xx makes
/// Stripe retry, which is what we want when the database was unavailable.
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/stripe",
    tag = "billing",
    request_body(content = String, description = "Raw Stripe event JSON; the signature covers these exact bytes", content_type = "application/json"),
    params(("Stripe-Signature" = String, Header, description = "t=<unix>,v1=<HMAC-SHA256 hex>")),
    responses(
        (status = 200, description = "Event handled or deliberately ignored"),
        (status = 400, description = "Invalid signature or unreadable event"),
        (status = 404, description = "Billing is not configured"),
        (status = 500, description = "Database failure; Stripe will retry"),
    ),
)]
pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Billing is off unless configured; the endpoint does not exist as far as callers know.
    let Some(cfg) = state.config.stripe.as_ref() else {
        return Err((StatusCode::NOT_FOUND, "Not found".into()));
    };

    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if !billing::verify_signature(&cfg.webhook_secret, signature, &body, now) {
        tracing::warn!("Stripe webhook refused: invalid signature");
        return Err((StatusCode::BAD_REQUEST, "Invalid signature".into()));
    }

    match billing::process_event(&state.pool, cfg, &body).await {
        Ok(outcome) => {
            tracing::info!(outcome = ?outcome, "Stripe webhook processed");
            Ok(Json(json!({ "received": true })))
        }
        Err(BillingError::Malformed) => {
            tracing::warn!("Stripe webhook refused: unreadable event");
            Err((StatusCode::BAD_REQUEST, "Unreadable event".into()))
        }
        Err(BillingError::Database(e)) => Err(internal_error(e)),
    }
}

/// GET /api/v1/me/billing
/// What the billing page needs: the plans on offer (with this customer's checkout links), the
/// current subscription if any, and the customer-portal link.
#[utoipa::path(
    get,
    path = "/api/v1/me/billing",
    tag = "billing",
    responses(
        (status = 200, description = "Plans on offer, current subscription, portal link", body = BillingView),
        (status = 401, description = "Not signed in"),
    ),
)]
pub async fn get_billing(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<BillingView>, (StatusCode, String)> {
    let customer = customer_svc::find_by_id(&state.pool, auth.id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Customer not found".into()))?;

    let view = billing::billing_view(&state.pool, state.config.stripe.as_ref(), customer.id, &customer.email)
        .await
        .map_err(internal_error)?;
    Ok(Json(view))
}
