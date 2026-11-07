use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Privacy score history snapshot for trend tracking.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PrivacyScoreHistory {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub score: f32,
    pub critical_count: i32,
    pub high_count: i32,
    pub medium_count: i32,
    pub low_count: i32,
    pub total_exposures: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub recorded_at: OffsetDateTime,
}

/// Request to retrieve privacy score history with optional time range.
#[derive(Debug, Deserialize)]
pub struct PrivacyScoreHistoryQuery {
    #[serde(default = "default_days")]
    pub days: i32,
}

fn default_days() -> i32 {
    30
}

/// Privacy score trend response with delta calculations.
#[derive(Debug, Serialize)]
pub struct PrivacyScoreTrend {
    pub current_score: f32,
    pub previous_score: Option<f32>,
    pub score_delta: Option<f32>,
    pub history: Vec<PrivacyScoreHistory>,
}
