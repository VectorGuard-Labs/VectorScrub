use axum::{extract::Request, middleware::Next, response::Response};
use metrics::{counter, histogram};
use std::time::Instant;

/// Middleware that records HTTP request metrics
///
/// Metrics recorded:
/// - http_requests_total: Counter of total requests by method, path, and status
/// - http_request_duration_seconds: Histogram of request duration
/// - http_requests_errors_total: Counter of error responses (4xx, 5xx)
pub async fn metrics_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let start = Instant::now();

    // Process request
    let response = next.run(request).await;

    // Record metrics
    let duration = start.elapsed();
    let status = response.status();

    // Increment total requests counter
    counter!(
        "http_requests_total",
        "method" => method.as_str().to_string(),
        "path" => normalize_path(&path),
        "status" => status.as_u16().to_string(),
    )
    .increment(1);

    // Record request duration
    histogram!(
        "http_request_duration_seconds",
        "method" => method.as_str().to_string(),
        "path" => normalize_path(&path),
    )
    .record(duration.as_secs_f64());

    // Record errors
    if status.is_client_error() || status.is_server_error() {
        counter!(
            "http_requests_errors_total",
            "method" => method.as_str().to_string(),
            "path" => normalize_path(&path),
            "status" => status.as_u16().to_string(),
        )
        .increment(1);
    }

    response
}

/// Normalize path by replacing UUIDs and numeric IDs with placeholders
/// This prevents high cardinality in metrics labels
fn normalize_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let normalized: Vec<String> = parts
        .iter()
        .map(|part| {
            // Replace UUIDs with :id
            if part.len() == 36 && part.contains('-') {
                ":id".to_string()
            }
            // Replace numeric IDs with :id
            else if part.parse::<u64>().is_ok() {
                ":id".to_string()
            } else {
                (*part).to_string()
            }
        })
        .collect();

    normalized.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path("/api/v1/scans/123e4567-e89b-12d3-a456-426614174000"),
            "/api/v1/scans/:id"
        );
        assert_eq!(normalize_path("/api/v1/exposures/123"), "/api/v1/exposures/:id");
        assert_eq!(normalize_path("/api/v1/exposures"), "/api/v1/exposures");
    }
}
