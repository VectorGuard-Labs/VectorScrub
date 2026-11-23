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
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::customer::Customer;
use crate::services::{
    audit, auth as auth_svc, customer as customer_svc, login_throttle, mfa as mfa_svc, password_policy,
    session as session_svc,
};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(email, length(max = 254))]
    #[schema(example = "user@example.com")]
    pub email: String,
    /// Capped far above anything a person types; it only stops megabyte bodies reaching the hasher.
    #[validate(length(max = 1024))]
    #[schema(example = "SecurePassword123!")]
    pub password: String,
    /// Six-digit code from the authenticator app, or a recovery code. Needed only when the
    /// account has two-factor authentication on; send it with the first request, or send the
    /// password alone, learn from the `mfa_required` answer, and send both again.
    #[validate(length(max = 64))]
    #[schema(example = "123456")]
    pub mfa_code: Option<String>,
}

/// Answer to a login whose password was right but whose account needs a second factor. No
/// session exists yet and no cookie is set.
#[derive(Debug, Serialize, ToSchema)]
pub struct MfaRequiredResponse {
    pub mfa_required: bool,
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
    if req.accepted_terms_version.as_deref() != Some(crate::models::customer::TERMS_VERSION) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "You must accept the current Terms of Service and Privacy Policy to create an account.".to_string(),
        ));
    }

    // Length is checked above; this refuses the passwords people pick when only a length is demanded.
    password_policy::check(&req.password, &req.email)
        .map_err(|r| (StatusCode::UNPROCESSABLE_ENTITY, r.message().to_string()))?;

    // Adults only (Terms: "Who can use VectorScrub"). A declaration, checked here so a client
    // that skips the form cannot skip the question. Nothing is stored for a refused attempt.
    if req.confirmed_adult != Some(true) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "VectorScrub is for people aged 18 or over. Please confirm that you are 18 or older to create an account.".to_string(),
        ));
    }

    // A small pilot stays small: stop at the configured number of accounts.
    if let Some(max) = state.config.max_accounts {
        let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM customers")
            .fetch_one(&state.pool)
            .await
            .map_err(internal_error)?;
        if existing >= max {
            return Err((
                StatusCode::FORBIDDEN,
                "The founding pilot is full, so we are not accepting new accounts right now.".to_string(),
            ));
        }
    }

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

    let session_id = session_svc::create(&state.pool, customer.id, user_agent(&headers).as_deref())
        .await
        .map_err(internal_error)?;
    let token = auth_svc::issue_token(
        customer.id,
        session_id,
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

fn login_audit(
    action: &'static str,
    email: &str,
    ip: Option<std::net::IpAddr>,
    headers: &HeaderMap,
    status: audit::AuditStatus,
    note: Option<&str>,
) -> audit::AuditEntryBuilder {
    let mut b = audit::AuditEntry::builder(action, audit::resources::CUSTOMER)
        .user_email(email)
        .status(status);
    if let Some(ip) = ip {
        b = b.ip_address(ip);
    }
    if let Some(ua) = user_agent(headers) {
        b = b.user_agent(ua);
    }
    if let Some(n) = note {
        b = b.error_message(n);
    }
    b
}

/// A failed login attempt (wrong password, or right password and wrong second factor): count
/// it toward the lockout, audit it, and answer the same way each time.
async fn login_failed(
    state: &AppState,
    req_email: &str,
    ip: Option<std::net::IpAddr>,
    headers: &HeaderMap,
    note: &str,
    message: &str,
) -> Response {
    let locked = match login_throttle::record_failure(&state.pool, req_email).await {
        Ok(l) => l,
        Err(e) => return internal_error(e).into_response(),
    };
    login_audit(audit::actions::USER_LOGIN, req_email, ip, headers, audit::AuditStatus::Failure, Some(note))
        .log_or_warn(&state.pool)
        .await;
    if let Some(secs) = locked {
        login_audit(audit::actions::USER_LOGIN_LOCKED, req_email, ip, headers, audit::AuditStatus::Failure, None)
            .details(serde_json::json!({ "lock_seconds": secs }))
            .log_or_warn(&state.pool)
            .await;
    }
    err(StatusCode::UNAUTHORIZED, message)
}

/// POST /api/v1/auth/login
#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful (AuthResponse, session cookie set), or the password was right and a second factor is needed (MfaRequiredResponse, no cookie)", body = AuthResponse),
        (status = 401, description = "Invalid credentials or code"),
        (status = 422, description = "Validation error"),
        (status = 429, description = "Too many failed attempts for this email, or too many requests from this address (see Retry-After)"),
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Response, Response> {
    req.validate()
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()))?;

    let audit_entry = |action: &'static str, status: audit::AuditStatus, note: Option<&str>| {
        login_audit(action, &req.email, ip, &headers, status, note)
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
        return Err(login_failed(&state, &req.email, ip, &headers, "invalid credentials", "Invalid email or password").await);
    };

    // 4. Second factor, for accounts that have one. The password alone never opens such an
    //    account. A wrong code counts toward the same lockout as a wrong password, so codes
    //    cannot be guessed. Asking for the code is not a failure and does not count.
    let mut mfa_method = None;
    if mfa_svc::is_enabled(&state.pool, customer.id)
        .await
        .map_err(|e| internal_error(e).into_response())?
    {
        let code = req.mfa_code.as_deref().map(str::trim).filter(|c| !c.is_empty());
        let Some(code) = code else {
            return Ok(Json(MfaRequiredResponse { mfa_required: true }).into_response());
        };
        match mfa_svc::verify(&state.pool, &state.keys, customer.id, code)
            .await
            .map_err(|e| internal_error(e).into_response())?
        {
            Some(method) => mfa_method = Some(method),
            None => {
                return Err(login_failed(&state, &req.email, ip, &headers, "invalid second factor", "Invalid verification code").await);
            }
        }
    }

    // 5. Success.
    login_throttle::clear(&state.pool, &req.email)
        .await
        .map_err(|e| internal_error(e).into_response())?;

    let session_id = session_svc::create(&state.pool, customer.id, user_agent(&headers).as_deref())
        .await
        .map_err(|e| internal_error(e).into_response())?;
    let token = auth_svc::issue_token(
        customer.id,
        session_id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(|e| internal_error(e).into_response())?;

    audit_entry(audit::actions::USER_LOGIN, audit::AuditStatus::Success, None)
        .customer_id(customer.id)
        .resource_id(customer.id)
        .details(serde_json::json!({
            "second_factor": match mfa_method {
                Some(mfa_svc::Verified::Totp) => "totp",
                Some(mfa_svc::Verified::RecoveryCode) => "recovery_code",
                None => "none",
            }
        }))
        .log_or_warn(&state.pool)
        .await;
    if mfa_method == Some(mfa_svc::Verified::RecoveryCode) {
        audit_entry(audit::actions::MFA_RECOVERY_USED, audit::AuditStatus::Success, None)
            .customer_id(customer.id)
            .resource_id(customer.id)
            .log_or_warn(&state.pool)
            .await;
    }

    Ok((
        [(header::SET_COOKIE, session_cookie(&token, state.config.secure_cookies()).to_string())],
        Json(AuthResponse { customer: customer.into() }),
    )
        .into_response())
}

/// POST /api/v1/auth/logout
/// Ends the browser session: the session is revoked on the server (so a copy of the token stops
/// working at once) and the cookie is cleared. Idempotent, and harmless when not signed in.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses((status = 204, description = "Session ended and cookie cleared")),
)]
pub async fn logout(
    State(state): State<AppState>,
    auth: Result<AuthenticatedCustomer, (StatusCode, String)>,
) -> Response {
    let cleared = [(header::SET_COOKIE, cleared_session_cookie(state.config.secure_cookies()).to_string())];
    match auth {
        Ok(a) => {
            if let Err(e) = session_svc::revoke(&state.pool, a.id, a.session_id).await {
                // The cookie is cleared regardless, but the caller must not be told the session
                // is over when it may still be live.
                tracing::error!(error = %e, "Could not revoke session at logout");
                return (StatusCode::INTERNAL_SERVER_ERROR, cleared, "Internal server error").into_response();
            }
        }
        // Not signed in (or the session had already ended): nothing to revoke.
        Err((StatusCode::UNAUTHORIZED, _)) => {}
        Err(other) => return (other.0, cleared, other.1).into_response(),
    }
    (StatusCode::NO_CONTENT, cleared).into_response()
}

/// POST /api/v1/auth/logout-all
/// Ends every session of the signed-in customer, this one included.
#[utoipa::path(
    post,
    path = "/api/v1/auth/logout-all",
    tag = "auth",
    responses((status = 204, description = "All sessions ended and cookie cleared"), (status = 401, description = "Not signed in")),
)]
pub async fn logout_all(auth: AuthenticatedCustomer, State(state): State<AppState>) -> Result<Response, Response> {
    let ended = session_svc::revoke_all(&state.pool, auth.id, None)
        .await
        .map_err(|e| internal_error(e).into_response())?;
    audit::AuditEntry::builder(audit::actions::SESSION_REVOKE, audit::resources::CUSTOMER)
        .customer_id(auth.id)
        .resource_id(auth.id)
        .details(serde_json::json!({ "scope": "all", "ended": ended }))
        .log_or_warn(&state.pool)
        .await;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cleared_session_cookie(state.config.secure_cookies()).to_string())],
    )
        .into_response())
}

/// Prove that the person at the keyboard is the account's owner, for actions a stolen session
/// must not be able to take (erasing the account, changing two-factor settings). Needs the
/// password and, for an account with two-factor on, a current code.
///
/// Shares the login lockout: these endpoints must not become a way to guess the password or a
/// code. Asking for a missing code is not counted as a failure.
pub(super) async fn reauthenticate(
    state: &AppState,
    customer_id: uuid::Uuid,
    password: &str,
    mfa_code: Option<&str>,
) -> Result<Customer, Response> {
    let customer = customer_svc::find_by_id(&state.pool, customer_id)
        .await
        .map_err(|e| internal_error(e).into_response())?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Customer not found"))?;

    if let Some(wait) = login_throttle::locked_for(&state.pool, &customer.email)
        .await
        .map_err(|e| internal_error(e).into_response())?
    {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, wait.to_string())],
            format!("Too many failed attempts. Try again in {} minute(s).", (wait + 59) / 60),
        )
            .into_response());
    }
    let valid = auth_svc::verify_password_async(password.to_string(), customer.password_hash.clone())
        .await
        .map_err(|e| internal_error(e).into_response())?;
    if !valid {
        login_throttle::record_failure(&state.pool, &customer.email)
            .await
            .map_err(|e| internal_error(e).into_response())?;
        return Err(err(StatusCode::UNAUTHORIZED, "Incorrect password"));
    }

    if mfa_svc::is_enabled(&state.pool, customer.id)
        .await
        .map_err(|e| internal_error(e).into_response())?
    {
        let Some(code) = mfa_code.map(str::trim).filter(|c| !c.is_empty()) else {
            return Err(err(StatusCode::UNAUTHORIZED, "Enter a code from your authenticator app."));
        };
        let ok = mfa_svc::verify(&state.pool, &state.keys, customer.id, code)
            .await
            .map_err(|e| internal_error(e).into_response())?;
        if ok.is_none() {
            login_throttle::record_failure(&state.pool, &customer.email)
                .await
                .map_err(|e| internal_error(e).into_response())?;
            return Err(err(StatusCode::UNAUTHORIZED, "Invalid verification code"));
        }
    }
    Ok(customer)
}

/// Body of `DELETE /api/v1/me`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct DeleteAccountRequest {
    /// Re-authentication: a stolen session alone cannot erase an account.
    #[validate(length(max = 1024))]
    pub password: String,
    /// Required when the account has two-factor authentication on.
    #[validate(length(max = 64))]
    pub mfa_code: Option<String>,
}

/// DELETE /api/v1/me
/// Permanently erases the signed-in customer's account and personal data. Needs the password
/// again (and a two-factor code if the account uses one), and refuses while a subscription would keep billing. See services/account_deletion.rs
/// for exactly what is removed and what is kept.
#[utoipa::path(
    delete,
    path = "/api/v1/me",
    tag = "auth",
    request_body = DeleteAccountRequest,
    responses(
        (status = 204, description = "Account erased; session cookie cleared"),
        (status = 401, description = "Wrong password, or a missing or wrong two-factor code"),
        (status = 409, description = "A subscription is still billing"),
        (status = 429, description = "Too many failed attempts"),
    ),
)]
pub async fn delete_account(
    auth: crate::middleware::auth_extractor::AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<DeleteAccountRequest>,
) -> Result<([(header::HeaderName, String); 1], StatusCode), Response> {
    req.validate()
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()))?;

    let customer = reauthenticate(&state, auth.id, &req.password, req.mfa_code.as_deref()).await?;

    match crate::services::account_deletion::delete_account(&state.pool, auth.id).await {
        Ok(()) => {}
        Err(crate::services::account_deletion::DeleteError::SubscriptionActive) => {
            return Err(err(
                StatusCode::CONFLICT,
                "Cancel your subscription first (Billing page), then delete your account.",
            ));
        }
        Err(crate::services::account_deletion::DeleteError::NotFound) => {
            return Err(err(StatusCode::NOT_FOUND, "Customer not found"));
        }
        Err(e) => return Err(internal_error(e).into_response()),
    }
    login_throttle::clear(&state.pool, &customer.email).await.ok();

    // No customer, no email, no IP: only that an account was erased, and when.
    audit::AuditEntry::builder(audit::actions::ACCOUNT_DELETE, audit::resources::CUSTOMER)
        .resource_id(auth.id)
        .log_or_warn(&state.pool)
        .await;

    Ok((
        [(header::SET_COOKIE, cleared_session_cookie(state.config.secure_cookies()).to_string())],
        StatusCode::NO_CONTENT,
    ))
}
