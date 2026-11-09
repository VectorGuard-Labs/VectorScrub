use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::middleware::csrf::{create_csrf_cookie, generate_csrf_token};

/// GET /api/v1/csrf-token
/// Returns a CSRF token that the client can use for subsequent requests
pub async fn get_csrf_token() -> Response {
    let token = generate_csrf_token();
    let cookie = create_csrf_cookie(&token);

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie.to_string())],
        serde_json::json!({ "csrf_token": token }).to_string(),
    )
        .into_response()
}
