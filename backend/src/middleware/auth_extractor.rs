use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    RequestPartsExt,
};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use uuid::Uuid;

use crate::db::AppState;
use crate::services::auth::{validate_token, Claims};

/// Extractor that validates a Bearer JWT and provides the customer's UUID
/// and subscription tier to route handlers.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AuthenticatedCustomer {
    pub id: Uuid,
    pub tier: String,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedCustomer {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| {
                (
                    StatusCode::UNAUTHORIZED,
                    "Missing or malformed Authorization header".into(),
                )
            })?;

        let claims: Claims = validate_token(bearer.token(), &state.config.jwt_secret)
            .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

        let id = Uuid::parse_str(&claims.sub)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token subject".into()))?;

        Ok(AuthenticatedCustomer { id, tier: claims.tier })
    }
}

/// Extractor for internal agent-to-API calls authenticated by a static API key.
pub struct AgentAuth;

#[async_trait]
impl FromRequestParts<AppState> for AgentAuth {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let key = parts
            .headers
            .get("X-Agent-Key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        // The agent API key is derived from the JWT secret — in production
        // use a separate secret stored in the environment.
        let expected = format!("agent-{}", &state.config.jwt_secret[..16]);
        if key != expected {
            return Err((StatusCode::UNAUTHORIZED, "Invalid agent key".into()));
        }
        Ok(AgentAuth)
    }
}
