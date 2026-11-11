use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::{Engine as _, engine::general_purpose};
use rand::Rng;

const CSRF_COOKIE_NAME: &str = "csrf_token";
const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Generate a random CSRF token (URL-safe base64, no padding, so the value contains
/// no `=`, `+` or `/` and survives naive cookie/header parsing).
pub fn generate_csrf_token() -> String {
    let mut rng = rand::thread_rng();
    let token: [u8; 32] = rng.gen();
    general_purpose::URL_SAFE_NO_PAD.encode(token)
}

/// Value of the cookie called `name` in a `Cookie:` header, or None.
/// Splits each pair on the FIRST `=` only: cookie values may themselves contain `=`.
fn cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    cookie_header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

/// Constant-time equality for secrets of any length.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
    // - Webhooks from other services (Stripe has no cookies; the payload is authenticated by
    //   its signature instead)
    if method == "GET"
        || method == "HEAD"
        || method == "OPTIONS"
        || path.starts_with("/api/v1/auth/")
        || path.starts_with("/api/v1/internal/")
        || path.starts_with("/api/v1/webhooks/")
    {
        return next.run(request).await;
    }

    // For state-changing requests, validate CSRF token (double-submit cookie)
    let cookie_token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|cookies| cookie_value(cookies, CSRF_COOKIE_NAME));

    let header_token = request
        .headers()
        .get(CSRF_HEADER_NAME)
        .and_then(|h| h.to_str().ok());

    match (cookie_token, header_token) {
        (Some(cookie), Some(header))
            if !cookie.is_empty() && constant_time_eq(cookie.as_bytes(), header.as_bytes()) =>
        {
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

/// Helper to create a CSRF token cookie.
/// `secure` should be true in production (HTTPS). In development over plain HTTP a
/// `Secure` cookie is silently dropped by some browsers (Safari), which would make
/// every state-changing request fail.
pub fn create_csrf_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((CSRF_COOKIE_NAME, token.to_string()))
        .path("/")
        .secure(secure)
        .same_site(SameSite::Strict)
        .http_only(false)  // JavaScript needs to read this for the header
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware::from_fn,
        routing::{any, get, post},
        Router,
    };
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/api/v1/me/identity", any(|| async { "ok" }))
            .route("/api/v1/me", get(|| async { "ok" }))
            .route("/api/v1/auth/login", post(|| async { "ok" }))
            .route("/api/v1/internal/scan/status", post(|| async { "ok" }))
            .route("/api/v1/webhooks/stripe", post(|| async { "ok" }))
            .layer(from_fn(csrf_middleware))
    }

    async fn send(method: &str, uri: &str, cookie: Option<&str>, header: Option<&str>) -> StatusCode {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(c) = cookie {
            req = req.header("cookie", c);
        }
        if let Some(h) = header {
            req = req.header("x-csrf-token", h);
        }
        app().oneshot(req.body(Body::empty()).unwrap()).await.unwrap().status()
    }

    const URI: &str = "/api/v1/me/identity";

    #[test]
    fn issued_tokens_are_url_safe_unpadded_and_unique() {
        let (a, b) = (generate_csrf_token(), generate_csrf_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43, "32 bytes, base64url without padding");
        assert!(!a.contains(['=', '+', '/']));
    }

    #[test]
    fn cookie_values_may_contain_equals_signs() {
        // The original bug: splitting on every '=' cut "abc=" down to "abc".
        assert_eq!(cookie_value("csrf_token=abc=", "csrf_token"), Some("abc="));
        assert_eq!(cookie_value("a=1; csrf_token=x==y; b=2", "csrf_token"), Some("x==y"));
        assert_eq!(cookie_value("xcsrf_token=nope", "csrf_token"), None, "exact name only");
        assert_eq!(cookie_value("csrf_token", "csrf_token"), None, "no value");
        assert_eq!(cookie_value("", "csrf_token"), None);
    }

    #[test]
    fn constant_time_eq_is_correct() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    #[tokio::test]
    async fn matching_cookie_and_header_pass() {
        let t = generate_csrf_token();
        assert_eq!(send("POST", URI, Some(&format!("csrf_token={t}")), Some(&t)).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_padded_base64_token_now_works_end_to_end() {
        // Regression: tokens used to end in '=' and could never match their own cookie.
        assert_eq!(send("POST", URI, Some("csrf_token=aGVsbG8="), Some("aGVsbG8=")).await, StatusCode::OK);
        assert_eq!(
            send("POST", URI, Some("session=a=b; csrf_token=aGVsbG8=; other=1"), Some("aGVsbG8=")).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn missing_mismatched_or_empty_tokens_are_rejected() {
        let t = generate_csrf_token();
        let cookie = format!("csrf_token={t}");
        assert_eq!(send("POST", URI, Some(&cookie), None).await, StatusCode::FORBIDDEN, "header missing");
        assert_eq!(send("POST", URI, None, Some(&t)).await, StatusCode::FORBIDDEN, "cookie missing");
        assert_eq!(send("POST", URI, Some(&cookie), Some("different")).await, StatusCode::FORBIDDEN, "mismatch");
        assert_eq!(send("POST", URI, Some(&cookie), Some(&format!("{t}x"))).await, StatusCode::FORBIDDEN, "longer");
        assert_eq!(send("POST", URI, Some("csrf_token="), Some("")).await, StatusCode::FORBIDDEN, "both empty");
        assert_eq!(send("POST", URI, Some(&format!("xcsrf_token={t}")), Some(&t)).await, StatusCode::FORBIDDEN, "look-alike cookie name");
    }

    #[tokio::test]
    async fn every_state_changing_method_is_protected() {
        let t = generate_csrf_token();
        let cookie = format!("csrf_token={t}");
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            assert_eq!(send(method, URI, None, None).await, StatusCode::FORBIDDEN, "{method} without a token");
            assert_eq!(send(method, URI, Some(&cookie), Some(&t)).await, StatusCode::OK, "{method} with a token");
        }
    }

    #[tokio::test]
    async fn safe_methods_auth_and_agent_endpoints_are_exempt() {
        assert_eq!(send("GET", "/api/v1/me", None, None).await, StatusCode::OK);
        assert_eq!(send("POST", "/api/v1/auth/login", None, None).await, StatusCode::OK, "no token exists yet at login");
        assert_eq!(send("POST", "/api/v1/internal/scan/status", None, None).await, StatusCode::OK, "agents use the agent key instead");
        assert_eq!(send("POST", "/api/v1/webhooks/stripe", None, None).await, StatusCode::OK, "Stripe has no cookie; it signs the payload instead");
    }

    #[test]
    fn the_cookie_is_secure_only_in_production() {
        let dev = create_csrf_cookie("t", false).to_string();
        let prod = create_csrf_cookie("t", true).to_string();
        assert!(!dev.contains("Secure"), "{dev}");
        assert!(prod.contains("Secure"), "{prod}");
        for c in [&dev, &prod] {
            assert!(c.contains("SameSite=Strict") && c.contains("Path=/"));
            assert!(!c.contains("HttpOnly"), "the client must be able to read it to echo it");
        }
    }
}
