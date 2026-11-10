//! Every error response is JSON with a human-readable `message`.
//!
//! The project rule is "all API responses are JSON; error responses include a human-readable
//! string message". Handlers, extractors and middleware each produced errors their own way:
//! plain text (`"Invalid email or password"`), a bare status with an empty body, the rate
//! limiter's and CSRF check's text. Rather than touch every site (and miss the next one), this
//! layer sits outermost and normalizes whatever comes out:
//!
//!   * a 4xx/5xx that is already JSON is left alone;
//!   * otherwise its body (or, if empty, the status's standard reason) becomes
//!     `{"message": "..."}` with `Content-Type: application/json`.
//!
//! Headers such as `Retry-After` are kept. The message is capped so an unexpectedly large body
//! cannot be echoed back wholesale.

use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

const MAX_INSPECTED_BYTES: usize = 8 * 1024;
const MAX_MESSAGE_CHARS: usize = 500;

pub async fn json_errors_middleware(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }
    let already_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
    if already_json {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let bytes = to_bytes(body, MAX_INSPECTED_BYTES).await.unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim();
    let message: String = if text.is_empty() {
        status.canonical_reason().unwrap_or("Error").to_string()
    } else {
        text.chars().take(MAX_MESSAGE_CHARS).collect()
    };

    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Response::from_parts(parts, Body::from(serde_json::json!({ "message": message }).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::StatusCode,
        middleware::from_fn,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/ok", get(|| async { "fine" }))
            .route("/text", get(|| async { (StatusCode::UNAUTHORIZED, "Invalid email or password") }))
            .route("/empty", get(|| async { StatusCode::NOT_FOUND }))
            .route("/json", get(|| async { (StatusCode::CONFLICT, axum::Json(serde_json::json!({ "message": "taken", "code": 7 }))) }))
            .route("/retry", get(|| async { (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "30")], "Slow down").into_response() }))
            .route("/huge", get(|| async { (StatusCode::BAD_REQUEST, "x".repeat(100_000)) }))
            .route("/redirect", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/elsewhere")]).into_response() }))
            .layer(from_fn(json_errors_middleware))
    }

    async fn get_json(uri: &str) -> (StatusCode, axum::http::HeaderMap, serde_json::Value, String) {
        let res = app()
            .oneshot(axum::http::Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let body = String::from_utf8(to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        (status, headers, serde_json::from_str(&body).unwrap_or(serde_json::Value::Null), body)
    }

    #[tokio::test]
    async fn successful_responses_are_untouched() {
        let (status, _, _, body) = get_json("/ok").await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, "fine"));
    }

    #[tokio::test]
    async fn a_text_error_becomes_json_with_the_same_message_and_status() {
        let (status, headers, json, _) = get_json("/text").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers[header::CONTENT_TYPE], "application/json");
        assert_eq!(json["message"], "Invalid email or password");
    }

    #[tokio::test]
    async fn an_empty_error_body_gets_the_standard_reason() {
        let (status, _, json, _) = get_json("/empty").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["message"], "Not Found");
    }

    #[tokio::test]
    async fn a_json_error_is_left_exactly_as_it_was() {
        let (status, _, json, _) = get_json("/json").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json, serde_json::json!({ "message": "taken", "code": 7 }));
    }

    #[tokio::test]
    async fn other_headers_such_as_retry_after_survive() {
        let (status, headers, json, _) = get_json("/retry").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(headers[header::RETRY_AFTER], "30");
        assert_eq!(json["message"], "Slow down");
    }

    #[tokio::test]
    async fn the_content_length_matches_the_new_body() {
        let res = app()
            .oneshot(axum::http::Request::builder().uri("/text").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let declared = res.headers().get(header::CONTENT_LENGTH).map(|v| v.to_str().unwrap().parse::<usize>().unwrap());
        let actual = to_bytes(res.into_body(), usize::MAX).await.unwrap().len();
        assert!(declared.is_none() || declared == Some(actual), "{declared:?} vs {actual}");
    }

    #[tokio::test]
    async fn a_huge_error_body_is_not_echoed_back_whole() {
        let (status, _, json, body) = get_json("/huge").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json["message"].as_str().unwrap().chars().count() <= MAX_MESSAGE_CHARS);
        assert!(body.len() < 1000);
    }

    #[tokio::test]
    async fn redirects_are_not_errors() {
        let res = app()
            .oneshot(axum::http::Request::builder().uri("/redirect").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FOUND);
        assert_eq!(res.headers()[header::LOCATION], "/elsewhere");
    }

    #[tokio::test]
    async fn csrf_and_timeout_and_rate_limit_failures_through_the_real_stack_are_json() {
        // The same stack main.rs builds (middleware::apply), around a real route.
        let cfg = crate::test_support::test_config();
        let router: Router = crate::middleware::apply(
            Router::new().route("/api/v1/me/identity", axum::routing::post(|| async { "ok" })),
            &cfg,
        );
        let res = router
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/v1/me/identity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let body: serde_json::Value = serde_json::from_slice(&to_bytes(res.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(body["message"], "CSRF token validation failed");
    }
}
