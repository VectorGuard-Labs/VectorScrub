use std::sync::OnceLock;

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
use crate::models::customer::{CreateAccountRequest, CustomerProfile};
use crate::services::{audit, auth as auth_svc, customer as customer_svc, login_throttle};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email)]
    #[schema(example = "user@example.com")]
    pub email: String,
    #[schema(example = "SecurePassword123!")]
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    #[schema(example = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...")]
    pub token: String,
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

/// A hash of a throwaway password, used to spend the same time on an unknown email as on
/// a real account. Without it, "no such account" answers measurably faster than "wrong
/// password", which reveals which emails are registered.
fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        auth_svc::hash_password("timing-equalization-only").expect("hash a constant password")
    })
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
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
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
    entry.log(&state.pool).await.ok(); // Don't fail the request if audit logging fails

    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            token,
            customer: customer.into(),
        }),
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
) -> Result<Json<AuthResponse>, Response> {
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
            .log(&state.pool)
            .await
            .ok();
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
        Some(c) => auth_svc::verify_password(&req.password, &c.password_hash)
            .map_err(|e| internal_error(e).into_response())?,
        None => {
            let _ = auth_svc::verify_password(&req.password, dummy_hash());
            false
        }
    };

    // 3. Failure: count it, maybe lock, and answer exactly as for any other bad login.
    let Some(customer) = customer.filter(|_| valid) else {
        let locked = login_throttle::record_failure(&state.pool, &req.email)
            .await
            .map_err(|e| internal_error(e).into_response())?;
        audit_entry(audit::actions::USER_LOGIN, audit::AuditStatus::Failure, Some("invalid credentials"))
            .log(&state.pool)
            .await
            .ok();
        if let Some(secs) = locked {
            audit_entry(audit::actions::USER_LOGIN_LOCKED, audit::AuditStatus::Failure, None)
                .details(serde_json::json!({ "lock_seconds": secs }))
                .log(&state.pool)
                .await
                .ok();
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
        .log(&state.pool)
        .await
        .ok();

    Ok(Json(AuthResponse {
        token,
        customer: customer.into(),
    }))
}
