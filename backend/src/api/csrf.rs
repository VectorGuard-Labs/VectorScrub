use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::db::AppState;
use crate::middleware::csrf::{create_csrf_cookie, generate_csrf_token};

/// GET /api/v1/csrf-token
/// Returns a CSRF token that the client can use for subsequent requests.
/// The same value is set as the `csrf_token` cookie (double-submit pattern); the
/// client echoes it back in the `X-CSRF-Token` header on POST/PUT/DELETE.
pub async fn get_csrf_token(State(state): State<AppState>) -> Response {
    let token = generate_csrf_token();
    let cookie = create_csrf_cookie(&token, state.config.is_production());

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie.to_string())],
        serde_json::json!({ "csrf_token": token }).to_string(),
    )
        .into_response()
}
