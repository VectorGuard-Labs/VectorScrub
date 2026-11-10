use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
};
use uuid::Uuid;

use super::session::SESSION_COOKIE;
use crate::db::AppState;
use crate::services::auth::{validate_token, Claims};

/// The session token on a request: an `Authorization: Bearer` header if there is one
/// (non-browser clients), otherwise the `HttpOnly` session cookie (the browser frontend).
fn request_token(parts: &Parts) -> Option<String> {
    if let Some(value) = parts.headers.get(header::AUTHORIZATION) {
        // A present-but-malformed header is not silently ignored in favour of a cookie.
        let value = value.to_str().ok()?;
        let (scheme, token) = value.split_once(' ')?;
        return (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty())
            .then(|| token.trim().to_string());
    }
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == SESSION_COOKIE && !value.is_empty()).then(|| value.to_string())
        })
}

/// Extractor that validates the session JWT (Bearer header or session cookie) and provides
/// the customer's UUID and subscription tier to route handlers.
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
        let token = request_token(parts)
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Not signed in".to_string()))?;

        // The library's error text (which says why a token failed) is not sent to the caller.
        let claims: Claims = validate_token(&token, &state.config.jwt_secret)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid or expired session".to_string()))?;

        let id = Uuid::parse_str(&claims.sub)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid session".to_string()))?;

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
            .map(|v| v.as_bytes())
            .unwrap_or(b"");

        // Constant-time (and length-hiding) comparison: `!=` on strings short-circuits at
        // the first differing byte, which leaks how much of a guess was correct.
        if !crate::crypto::secrets_equal(key, state.config.agent_api_key.as_bytes()) {
            return Err((StatusCode::UNAUTHORIZED, "Invalid agent key".into()));
        }
        Ok(AgentAuth)
    }
}
