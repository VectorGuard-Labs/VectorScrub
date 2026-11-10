use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};

/// Middleware that adds security headers to all responses
///
/// Headers added:
/// - X-Frame-Options: Prevents clickjacking attacks
/// - X-Content-Type-Options: Prevents MIME type sniffing
/// - X-XSS-Protection: Legacy XSS protection (browser-dependent)
/// - Strict-Transport-Security: Forces HTTPS in production
/// - Content-Security-Policy: Restricts resource loading to prevent XSS
/// - Referrer-Policy: Controls referrer information sent with requests
/// - Permissions-Policy: Controls browser features
pub async fn security_headers_middleware(
    State(hsts): State<bool>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // Prevent clickjacking by disallowing iframe embedding
    headers.insert(
        "X-Frame-Options",
        HeaderValue::from_static("DENY"),
    );

    // Prevent MIME type sniffing
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );

    // Enable browser's XSS filter (legacy, but doesn't hurt)
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block"),
    );

    // Strict Transport Security (HSTS) - Force HTTPS for 1 year.
    // `hsts` comes from AppConfig::hsts_enabled(), the same validated ENVIRONMENT the
    // cookies use. (It used to re-read the raw variable on every request, so a value the
    // config did not recognise could disagree with it.)
    if hsts {
        headers.insert(
            "Strict-Transport-Security",
            HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
        );
    }

    // Content Security Policy - Restrict resource loading
    // This is a strict policy; adjust based on your needs
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: https:; \
             font-src 'self'; \
             connect-src 'self'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self'",
        ),
    );

    // Referrer Policy - Limit referrer information
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    // Permissions Policy - Disable unnecessary browser features
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static(
            "geolocation=(), \
             microphone=(), \
             camera=(), \
             payment=(), \
             usb=(), \
             magnetometer=(), \
             gyroscope=(), \
             accelerometer=()",
        ),
    );

    // Remove server information header (if present)
    headers.remove("Server");

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, middleware::from_fn_with_state, routing::get, Router};
    use tower::ServiceExt;

    async fn headers(hsts: bool) -> axum::http::HeaderMap {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(from_fn_with_state(hsts, security_headers_middleware));
        app.oneshot(HttpRequest::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .headers()
            .clone()
    }

    #[tokio::test]
    async fn hsts_follows_the_flag_it_is_given() {
        assert!(headers(true).await.contains_key("strict-transport-security"));
        assert!(!headers(false).await.contains_key("strict-transport-security"));
    }

    #[tokio::test]
    async fn the_other_headers_are_always_sent() {
        for hsts in [true, false] {
            let h = headers(hsts).await;
            for name in ["x-frame-options", "x-content-type-options", "content-security-policy", "referrer-policy"] {
                assert!(h.contains_key(name), "{name} (hsts={hsts})");
            }
        }
    }
}
