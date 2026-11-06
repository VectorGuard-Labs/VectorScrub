use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::db::AppState;
use crate::models::customer::{CreateAccountRequest, CustomerProfile};
use crate::services::{auth as auth_svc, customer as customer_svc};

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub customer: CustomerProfile,
}

/// POST /api/v1/auth/register
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

    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            token,
            customer: customer.into(),
        }),
    ))
}

/// POST /api/v1/auth/login
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
        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".into()));
    }

    let token = auth_svc::issue_token(
        customer.id,
        &format!("{:?}", customer.subscription_tier).to_lowercase(),
        &state.config.jwt_secret,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AuthResponse {
        token,
        customer: customer.into(),
    }))
}
