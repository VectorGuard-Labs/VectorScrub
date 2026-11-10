use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::privacy_score::{PrivacyScoreHistoryQuery, PrivacyScoreTrend};
use crate::services::privacy_score as score_svc;

/// GET /api/v1/privacy-score/history
/// Retrieve privacy score history for the authenticated customer.
pub async fn get_history(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Query(params): Query<PrivacyScoreHistoryQuery>,
) -> Result<Json<PrivacyScoreTrend>, (StatusCode, String)> {
    let trend = score_svc::get_trend(&state.pool, auth.id, params.days)
        .await
        .map_err(super::error::internal_error)?;

    Ok(Json(trend))
}
