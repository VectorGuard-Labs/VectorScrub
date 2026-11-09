use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::{Engine as _, engine::general_purpose};
use rand::Rng;

const CSRF_COOKIE_NAME: &str = "csrf_token";
const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Generate a random CSRF token
pub fn generate_csrf_token() -> String {
    let mut rng = rand::thread_rng();
    let token: [u8; 32] = rng.gen();
    general_purpose::STANDARD.encode(token)
}

/// CSRF protection middleware using double-submit cookie pattern
///
/// This middleware:
/// 1. Sets a CSRF token cookie on GET requests
/// 2. Validates the token on state-changing requests (POST, PUT, DELETE)
/// 3. Compares the cookie value with the header value
pub async fn csrf_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path();

    // Skip CSRF check for:
    // - Safe methods (GET, HEAD, OPTIONS)
    // - Login/register endpoints (they don't have tokens yet)
    // - Internal agent endpoints (use API key auth instead)
    if method == "GET"
        || method == "HEAD"
        || method == "OPTIONS"
        || path.starts_with("/api/v1/auth/")
        || path.starts_with("/api/v1/internal/")
    {
        return next.run(request).await;
    }

    // For state-changing requests, validate CSRF token
    let cookie_token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|cookies| {
            cookies
                .split(';')
                .find(|c| c.trim().starts_with(&format!("{}=", CSRF_COOKIE_NAME)))
                .and_then(|c| c.split('=').nth(1))
        });

    let header_token = request
        .headers()
        .get(CSRF_HEADER_NAME)
        .and_then(|h| h.to_str().ok());

    match (cookie_token, header_token) {
        (Some(cookie), Some(header)) if cookie == header => {
            // Tokens match, proceed with request
            next.run(request).await
        }
        _ => {
            // Tokens don't match or are missing
            (
                StatusCode::FORBIDDEN,
                "CSRF token validation failed",
            )
                .into_response()
        }
    }
}

/// Helper to create a CSRF token cookie
pub fn create_csrf_cookie(token: &str) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE_NAME, token.to_string()))
        .path("/")
        .secure(true)  // Only send over HTTPS in production
        .same_site(SameSite::Strict)
        .http_only(false)  // JavaScript needs to read this for the header
        .build()
}
