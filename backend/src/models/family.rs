use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Family member association for family plan subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FamilyMember {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub member_id: Uuid,
    pub role: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub invited_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub accepted_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Request to add a family member.
#[derive(Debug, Deserialize)]
pub struct AddFamilyMemberRequest {
    pub member_email: String,
}

/// Response with family member details.
#[derive(Debug, Serialize)]
pub struct FamilyMemberResponse {
    pub id: Uuid,
    pub member_email: String,
    pub member_name: Option<String>,
    pub privacy_score: Option<f32>,
    pub role: String,
    pub invited_at: Option<OffsetDateTime>,
    pub accepted_at: Option<OffsetDateTime>,
}

/// Aggregate family statistics.
#[derive(Debug, Serialize)]
pub struct FamilyStats {
    pub total_members: usize,
    pub average_privacy_score: f32,
    pub total_exposures: i64,
    pub members: Vec<FamilyMemberResponse>,
}
