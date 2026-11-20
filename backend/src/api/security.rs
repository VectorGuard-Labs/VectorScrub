//! Account security: two-factor authentication and the customer's own sessions.
//!
//! Changing two-factor settings needs the password again (and, to turn it off, a current code),
//! so a stolen session alone cannot weaken the account. Turning it on or off also ends the
//! customer's other sessions: whoever else was signed in has to do it again under the new rules.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::auth::reauthenticate;
use super::error::internal_error;
use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::services::{audit, mfa as mfa_svc, session as session_svc};

fn invalid(message: &str) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, message.to_string()).into_response()
}

/// GET /api/v1/me/mfa
#[utoipa::path(
    get,
    path = "/api/v1/me/mfa",
    tag = "auth",
    responses((status = 200, description = "Whether two-factor authentication is on", body = mfa_svc::Status)),
)]
pub async fn mfa_status(auth: AuthenticatedCustomer, State(state): State<AppState>) -> Result<Json<mfa_svc::Status>, Response> {
    mfa_svc::status(&state.pool, auth.id)
        .await
        .map(Json)
        .map_err(|e| internal_error(e).into_response())
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct MfaSetupRequest {
    #[validate(length(max = 1024))]
    pub password: String,
}

/// POST /api/v1/me/mfa/setup
/// Step 1 of turning two-factor on: returns a new secret and the `otpauth://` link for the
/// authenticator app. Nothing is protected until step 2 confirms a code.
#[utoipa::path(
    post,
    path = "/api/v1/me/mfa/setup",
    tag = "auth",
    request_body = MfaSetupRequest,
    responses(
        (status = 200, description = "Secret to add to an authenticator app", body = mfa_svc::Setup),
        (status = 401, description = "Wrong password"),
        (status = 409, description = "Two-factor authentication is already on"),
    ),
)]
pub async fn mfa_setup(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<MfaSetupRequest>,
) -> Result<Json<mfa_svc::Setup>, Response> {
    req.validate().map_err(|e| invalid(&e.to_string()))?;
    // Checked first: with two-factor on, re-authentication would ask for a code, but there is
    // nothing to set up either way and the answer should say so.
    if mfa_svc::is_enabled(&state.pool, auth.id).await.map_err(|e| internal_error(e).into_response())? {
        return Err((StatusCode::CONFLICT, "Two-factor authentication is already on.").into_response());
    }
    let customer = reauthenticate(&state, auth.id, &req.password, None).await?;
    match mfa_svc::begin_setup(&state.pool, &state.keys, auth.id, &customer.email).await {
        Ok(setup) => Ok(Json(setup)),
        Err(mfa_svc::MfaError::AlreadyEnabled) => {
            Err((StatusCode::CONFLICT, "Two-factor authentication is already on.").into_response())
        }
        Err(e) => Err(internal_error(e).into_response()),
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct MfaEnableRequest {
    #[validate(length(min = 1, max = 64))]
    pub code: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MfaEnabledResponse {
    /// Shown once. Each works once, in place of an authenticator code.
    pub recovery_codes: Vec<String>,
}

/// POST /api/v1/me/mfa/enable
/// Step 2: a code from the app proves it is set up, switches two-factor on, and returns the
/// recovery codes. Other sessions are ended.
#[utoipa::path(
    post,
    path = "/api/v1/me/mfa/enable",
    tag = "auth",
    request_body = MfaEnableRequest,
    responses(
        (status = 200, description = "Two-factor authentication is on", body = MfaEnabledResponse),
        (status = 409, description = "Already on"),
        (status = 422, description = "Wrong code, or setup not started"),
    ),
)]
pub async fn mfa_enable(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<MfaEnableRequest>,
) -> Result<Json<MfaEnabledResponse>, Response> {
    req.validate().map_err(|e| invalid(&e.to_string()))?;
    let codes = match mfa_svc::confirm_setup(&state.pool, &state.keys, auth.id, &req.code).await {
        Ok(codes) => codes,
        Err(mfa_svc::MfaError::AlreadyEnabled) => {
            return Err((StatusCode::CONFLICT, "Two-factor authentication is already on.").into_response())
        }
        Err(mfa_svc::MfaError::NotStarted) => return Err(invalid("Start two-factor setup first.")),
        Err(mfa_svc::MfaError::BadCode) => {
            return Err(invalid("That code is not valid. Check that your phone's clock is correct and try the next code."))
        }
        Err(e) => return Err(internal_error(e).into_response()),
    };
    let ended = session_svc::revoke_all(&state.pool, auth.id, Some(auth.session_id))
        .await
        .map_err(|e| internal_error(e).into_response())?;
    audit::AuditEntry::builder(audit::actions::MFA_ENABLE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .details(serde_json::json!({ "other_sessions_ended": ended }))
        .log_or_warn(&state.pool)
        .await;
    Ok(Json(MfaEnabledResponse { recovery_codes: codes }))
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct MfaDisableRequest {
    #[validate(length(max = 1024))]
    pub password: String,
    #[validate(length(max = 64))]
    pub code: String,
}

/// POST /api/v1/me/mfa/disable
/// Turns two-factor off. Needs the password and a current code (or a recovery code). Other
/// sessions are ended.
#[utoipa::path(
    post,
    path = "/api/v1/me/mfa/disable",
    tag = "auth",
    request_body = MfaDisableRequest,
    responses(
        (status = 204, description = "Two-factor authentication is off"),
        (status = 401, description = "Wrong password or code"),
        (status = 409, description = "It was not on"),
    ),
)]
pub async fn mfa_disable(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<MfaDisableRequest>,
) -> Result<StatusCode, Response> {
    req.validate().map_err(|e| invalid(&e.to_string()))?;
    if !mfa_svc::is_enabled(&state.pool, auth.id).await.map_err(|e| internal_error(e).into_response())? {
        return Err((StatusCode::CONFLICT, "Two-factor authentication is not on.").into_response());
    }
    reauthenticate(&state, auth.id, &req.password, Some(&req.code)).await?;
    mfa_svc::disable(&state.pool, auth.id).await.map_err(|e| internal_error(e).into_response())?;
    let ended = session_svc::revoke_all(&state.pool, auth.id, Some(auth.session_id))
        .await
        .map_err(|e| internal_error(e).into_response())?;
    audit::AuditEntry::builder(audit::actions::MFA_DISABLE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .details(serde_json::json!({ "other_sessions_ended": ended }))
        .log_or_warn(&state.pool)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ChangePasswordRequest {
    #[validate(length(max = 1024))]
    pub current_password: String,
    #[validate(length(min = 12, max = 128, message = "Password must be between 12 and 128 characters"))]
    pub new_password: String,
    /// Required when the account has two-factor authentication on.
    #[validate(length(max = 64))]
    pub mfa_code: Option<String>,
}

/// POST /api/v1/me/password
/// Changes the password. Needs the current password (and a code, if two-factor is on), shares the
/// login lockout, and ends every other session so a copy of the old login stops working.
#[utoipa::path(
    post,
    path = "/api/v1/me/password",
    tag = "auth",
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed; other sessions ended"),
        (status = 401, description = "Wrong current password, or a missing or wrong two-factor code"),
        (status = 422, description = "The new password is too short, too long, or the same as the old one"),
        (status = 429, description = "Too many failed attempts"),
    ),
)]
pub async fn change_password(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, Response> {
    req.validate().map_err(|e| invalid(&e.to_string()))?;
    let customer = reauthenticate(&state, auth.id, &req.current_password, req.mfa_code.as_deref()).await?;

    if req.new_password == req.current_password {
        return Err(invalid("Choose a password you have not been using."));
    }
    let new_hash = crate::services::auth::hash_password_async(req.new_password.clone())
        .await
        .map_err(|e| internal_error(e).into_response())?;
    crate::services::customer::set_password_hash(&state.pool, customer.id, &new_hash)
        .await
        .map_err(|e| internal_error(e).into_response())?;
    let ended = session_svc::revoke_all(&state.pool, auth.id, Some(auth.session_id))
        .await
        .map_err(|e| internal_error(e).into_response())?;
    audit::AuditEntry::builder(audit::actions::PASSWORD_CHANGE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .details(serde_json::json!({ "other_sessions_ended": ended }))
        .log_or_warn(&state.pool)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionView {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String)]
    pub last_seen_at: time::OffsetDateTime,
    pub user_agent: Option<String>,
    /// True for the session making this request.
    pub current: bool,
}

/// GET /api/v1/me/sessions
/// The customer's live sessions, most recently used first.
#[utoipa::path(
    get,
    path = "/api/v1/me/sessions",
    tag = "auth",
    responses((status = 200, description = "Live sessions", body = Vec<SessionView>)),
)]
pub async fn list_sessions(auth: AuthenticatedCustomer, State(state): State<AppState>) -> Result<Json<Vec<SessionView>>, Response> {
    let rows = session_svc::list(&state.pool, auth.id)
        .await
        .map_err(|e| internal_error(e).into_response())?;
    Ok(Json(
        rows.into_iter()
            .map(|s| SessionView {
                current: s.id == auth.session_id,
                id: s.id,
                created_at: s.created_at,
                last_seen_at: s.last_seen_at,
                user_agent: s.user_agent,
            })
            .collect(),
    ))
}

/// DELETE /api/v1/me/sessions/:id
/// Ends one of the customer's sessions (another browser or device).
#[utoipa::path(
    delete,
    path = "/api/v1/me/sessions/{id}",
    tag = "auth",
    params(("id" = Uuid, Path, description = "Session id from the list")),
    responses((status = 204, description = "Session ended"), (status = 404, description = "No such live session")),
)]
pub async fn revoke_session(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Response> {
    let ended = session_svc::revoke(&state.pool, auth.id, id)
        .await
        .map_err(|e| internal_error(e).into_response())?;
    if !ended {
        return Err((StatusCode::NOT_FOUND, "No such session.").into_response());
    }
    audit::AuditEntry::builder(audit::actions::SESSION_REVOKE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .details(serde_json::json!({ "scope": "one" }))
        .log_or_warn(&state.pool)
        .await;
    // Ending the current session this way is allowed; the next request will simply be refused.
    Ok(StatusCode::NO_CONTENT)
}
