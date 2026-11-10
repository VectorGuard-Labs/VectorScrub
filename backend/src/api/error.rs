//! Shared API error helpers.

use axum::http::StatusCode;

/// Map an unexpected internal failure to a generic 500.
///
/// The detail (SQL errors, file paths, library messages) is logged server-side and
/// never returned to the client: it helps an attacker map the system and can leak
/// data. Use this for failures that are not the caller's fault; use a specific 4xx
/// with a purpose-written message for failures that are.
pub fn internal_error<E: std::fmt::Display>(err: E) -> (StatusCode, String) {
    tracing::error!(error = %err, "Internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}
