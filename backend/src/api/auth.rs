use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

use super::client_ip::ClientIp;
use super::error::internal_error;
use crate::db::AppState;
use crate::middleware::session::{cleared_session_cookie, session_cookie};
use crate::models::customer::{CreateAccountRequest, CustomerProfile};
use crate::services::{audit, auth as auth_svc, customer as customer_svc, login_throttle};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email, length(max = 254))]
    #[schema(example = "user@example.com")]
    pub email: String,
    /// Capped far above anything a person types; it only stops megabyte bodies reaching the hasher.
    #[validate(length(max = 1024))]
    #[schema(example = "SecurePassword123!")]
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    /// The session itself is not in the body: it is set as an `HttpOnly` cookie
    /// (`vs_session`) that scripts cannot read. See `middleware::session`.
    pub customer: CustomerProfile,
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(300).collect())
}

fn err(status: StatusCode, message: &str) -> Response {
    (status, message.to_string()).into_response()
}

/// POST /api/v1/auth/register
#[utoipa::path(
    post,
    path = "/api/v1/auth/register",
    tag = "auth",
    request_body = CreateAccountRequest,
    responses(
        (status = 201, description = "Account created successfully", body = AuthResponse),
        (status = 409, description = "Email already exists"),
        (status = 422, description = "Validation error"),
        (status = 429, description = "Too many requests from this address"),
    ),
)]
pub async fn register(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, [(header::HeaderName, String); 1], Json<AuthResponse>), (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let customer = customer_svc::create_account(&state.pool, &req)
        .await
        .map_err(|e| {
            // Only the known "email taken" outcome is the caller's problem; anything else
            // (database errors, hashing failures) is ours and must not be echoed back.
            if e.to_string() == customer_svc::EMAIL_TAKEN {
                (StatusCode::CONFLICT, customer_svc::EMAIL_TAKEN.to_string())
            } else {
                internal_error(e)
            }
        })?;

    let token = auth_svc::issue_token(
        customer.id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(internal_error)?;

    let mut entry = audit::AuditEntry::builder(audit::actions::USER_REGISTER, audit::resources::CUSTOMER)
        .customer_id(customer.id)
        .user_email(&customer.email)
        .resource_id(customer.id)
        .details(serde_json::json!({
            "subscription_tier": format!("{:?}", customer.subscription_tier),
        }));
    if let Some(ip) = ip {
        entry = entry.ip_address(ip);
    }
    if let Some(ua) = user_agent(&headers) {
        entry = entry.user_agent(ua);
    }
    entry.log_or_warn(&state.pool).await; // Don't fail the request if audit logging fails

    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, session_cookie(&token, state.config.secure_cookies()).to_string())],
        Json(AuthResponse { customer: customer.into() }),
    ))
}

/// POST /api/v1/auth/login
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = AuthResponse),
        (status = 401, description = "Invalid credentials"),
        (status = 422, description = "Validation error"),
        (status = 429, description = "Too many failed attempts for this email, or too many requests from this address (see Retry-After)"),
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<([(header::HeaderName, String); 1], Json<AuthResponse>), Response> {
    req.validate()
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()))?;

    let audit_entry = |action: &'static str, status: audit::AuditStatus, note: Option<&str>| {
        let mut b = audit::AuditEntry::builder(action, audit::resources::CUSTOMER)
            .user_email(&req.email)
            .status(status);
        if let Some(ip) = ip {
            b = b.ip_address(ip);
        }
        if let Some(ua) = user_agent(&headers) {
            b = b.user_agent(ua);
        }
        if let Some(n) = note {
            b = b.error_message(n);
        }
        b
    };

    // 1. Locked out? Checked first, before any password work, and identical whether or
    //    not the email has an account. A failure to check is a failure to log in.
    if let Some(wait) = login_throttle::locked_for(&state.pool, &req.email)
        .await
        .map_err(|e| internal_error(e).into_response())?
    {
        audit_entry(audit::actions::USER_LOGIN, audit::AuditStatus::Failure, Some("locked out"))
            .log_or_warn(&state.pool)
            .await;
        let minutes = (wait + 59) / 60;
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, wait.to_string())],
            format!("Too many failed login attempts. Try again in {minutes} minute(s)."),
        )
            .into_response());
    }

    // 2. Look up and verify. An unknown email still pays for one Argon2 verification.
    let customer = customer_svc::find_by_email(&state.pool, &req.email)
        .await
        .map_err(|e| internal_error(e).into_response())?;
    let valid = match &customer {
        // Verification runs on the blocking pool (see services::auth). An unknown email still
        // pays for one, so "no such account" is not faster than "wrong password".
        Some(c) => auth_svc::verify_password_async(req.password.clone(), c.password_hash.clone())
            .await
            .map_err(|e| internal_error(e).into_response())?,
        None => {
            auth_svc::verify_against_dummy_async(req.password.clone()).await;
            false
        }
    };

    // 3. Failure: count it, maybe lock, and answer exactly as for any other bad login.
    let Some(customer) = customer.filter(|_| valid) else {
        let locked = login_throttle::record_failure(&state.pool, &req.email)
            .await
            .map_err(|e| internal_error(e).into_response())?;
        audit_entry(audit::actions::USER_LOGIN, audit::AuditStatus::Failure, Some("invalid credentials"))
            .log_or_warn(&state.pool)
            .await;
        if let Some(secs) = locked {
            audit_entry(audit::actions::USER_LOGIN_LOCKED, audit::AuditStatus::Failure, None)
                .details(serde_json::json!({ "lock_seconds": secs }))
                .log_or_warn(&state.pool)
                .await;
        }
        return Err(err(StatusCode::UNAUTHORIZED, "Invalid email or password"));
    };

    // 4. Success.
    login_throttle::clear(&state.pool, &req.email)
        .await
        .map_err(|e| internal_error(e).into_response())?;

    let token = auth_svc::issue_token(
        customer.id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(|e| internal_error(e).into_response())?;

    audit_entry(audit::actions::USER_LOGIN, audit::AuditStatus::Success, None)
        .customer_id(customer.id)
        .resource_id(customer.id)
        .log_or_warn(&state.pool)
        .await;

    Ok((
        [(header::SET_COOKIE, session_cookie(&token, state.config.secure_cookies()).to_string())],
        Json(AuthResponse { customer: customer.into() }),
    ))
}

/// POST /api/v1/auth/logout
/// Ends the browser session by expiring the session cookie. Idempotent, and works whether or
/// not the caller is signed in. There is no server-side revocation: a token copied before
/// logout stays valid until it expires (24 hours).
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses((status = 204, description = "Session cookie cleared")),
)]
pub async fn logout(State(state): State<AppState>) -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cleared_session_cookie(state.config.secure_cookies()).to_string())],
    )
}
