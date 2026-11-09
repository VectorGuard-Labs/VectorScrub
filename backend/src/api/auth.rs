use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use validator::Validate;

use crate::db::AppState;
use crate::models::customer::{CreateAccountRequest, CustomerProfile};
use crate::services::{audit, auth as auth_svc, customer as customer_svc};

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
    ),
)]
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let customer = customer_svc::create_account(&state.pool, &req)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    let token = auth_svc::issue_token(
        customer.id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Audit log: successful registration
    audit::AuditEntry::builder(audit::actions::USER_REGISTER, audit::resources::CUSTOMER)
        .customer_id(customer.id)
        .user_email(&customer.email)
        .resource_id(customer.id)
        .details(serde_json::json!({
            "subscription_tier": format!("{:?}", customer.subscription_tier),
        }))
        .log(&state.pool)
        .await
        .ok(); // Don't fail request if audit logging fails

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
    ),
)]
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    req.validate()
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let customer = customer_svc::find_by_email(&state.pool, &req.email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid email or password".into()))?;

    let valid = auth_svc::verify_password(&req.password, &customer.password_hash)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !valid {
        // Audit log: failed login attempt
        audit::AuditEntry::builder(audit::actions::USER_LOGIN, audit::resources::CUSTOMER)
            .customer_id(customer.id)
            .user_email(&customer.email)
            .status(audit::AuditStatus::Failure)
            .error_message("Invalid password")
            .log(&state.pool)
            .await
            .ok();

        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".into()));
    }

    let token = auth_svc::issue_token(
        customer.id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Audit log: successful login
    audit::AuditEntry::builder(audit::actions::USER_LOGIN, audit::resources::CUSTOMER)
        .customer_id(customer.id)
        .user_email(&customer.email)
        .resource_id(customer.id)
        .log(&state.pool)
        .await
        .ok(); // Don't fail request if audit logging fails

    Ok(Json(AuthResponse {
        token,
        customer: customer.into(),
    }))
}
