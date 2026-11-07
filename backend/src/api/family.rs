use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::family::{AddFamilyMemberRequest, FamilyMemberResponse, FamilyStats};
use crate::services::family as family_svc;

/// POST /api/v1/family/members
/// Add a family member to the authenticated customer's family plan.
pub async fn add_member(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<AddFamilyMemberRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let member = family_svc::add_family_member(&state.pool, auth.id, &req.member_email)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": member.id,
            "member_id": member.member_id,
            "role": member.role,
            "invited_at": member.invited_at,
            "message": "Family member added successfully"
        })),
    ))
}

/// DELETE /api/v1/family/members/:member_id
/// Remove a family member from the authenticated customer's family plan.
pub async fn remove_member(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(member_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    family_svc::remove_family_member(&state.pool, auth.id, member_id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/family/members
/// List all family members for the authenticated customer.
pub async fn list_members(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<FamilyMemberResponse>>, (StatusCode, String)> {
    let members = family_svc::list_family_members(&state.pool, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(members))
}

/// GET /api/v1/family/stats
/// Get aggregate family statistics including all members' privacy scores.
pub async fn get_stats(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<FamilyStats>, (StatusCode, String)> {
    let stats = family_svc::get_family_stats(&state.pool, auth.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(stats))
}
