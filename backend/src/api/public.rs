use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use super::error::internal_error;
use crate::db::AppState;
use crate::services::optout_health::{self, PageKind, PublicOptOutHealth};
use crate::services::scan as scan_svc;

/// GET /api/v1/public/coverage
/// Which data brokers our scans could read lately. Unauthenticated, because the pricing page
/// shows it to visitors; it is an aggregate over all customers' recent scans (site names and a
/// yes/no) and identifies no one. Cacheable for five minutes.
#[utoipa::path(
    get,
    path = "/api/v1/public/coverage",
    tag = "public",
    responses((status = 200, description = "Per-source readability over recent scans", body = crate::services::scan::PublicCoverage)),
)]
pub async fn coverage(State(state): State<AppState>) -> Result<Response, (StatusCode, String)> {
    let body = scan_svc::public_coverage(&state.pool).await.map_err(internal_error)?;
    Ok(([(header::CACHE_CONTROL, "public, max-age=300")], Json(body)).into_response())
}

#[derive(serde::Deserialize)]
pub struct OptOutHealthQuery {
    /// `optout` (default): the brokers VectorScrub handles. `ca_rights`: the California
    /// registry companies' privacy-request pages.
    kind: Option<PageKind>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct OptOutHealthResponse {
    pub checks: Vec<PublicOptOutHealth>,
}

/// GET /api/v1/public/optout-health
/// Whether each broker's public opt-out page loaded at its last nightly check. "Reachable"
/// means the page opened, not that an automated opt-out works there. Cacheable for five minutes.
#[utoipa::path(
    get,
    path = "/api/v1/public/optout-health",
    tag = "public",
    params(("kind" = Option<PageKind>, Query, description = "optout (default) or ca_rights")),
    responses((status = 200, description = "Last check of each broker's opt-out page", body = OptOutHealthResponse)),
)]
pub async fn optout_health(
    State(state): State<AppState>,
    Query(q): Query<OptOutHealthQuery>,
) -> Result<Response, (StatusCode, String)> {
    let checks = optout_health::list_public(&state.pool, q.kind.unwrap_or_default())
        .await
        .map_err(internal_error)?;
    Ok(([(header::CACHE_CONTROL, "public, max-age=300")], Json(OptOutHealthResponse { checks })).into_response())
}
