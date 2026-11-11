use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Category of exposed personal information.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "pii_category", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum PiiCategory {
    FullName,
    HomeAddress,
    PhoneNumber,
    EmailAddress,
    DateOfBirth,
    FamilyAssociation,
    EmploymentHistory,
    SocialMediaHandle,
    VehicleRecord,
    CourtRecord,
    Other,
}

/// Severity of the exposure finding.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, PartialOrd)]
#[sqlx(type_name = "exposure_severity", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ExposureSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// A single discovered exposure of a customer's personal information.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Exposure {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub scan_job_id: Uuid,
    /// The data source where PII was found (e.g., "spokeo.com")
    pub source_name: String,
    pub source_url: String,
    pub pii_category: PiiCategory,
    pub severity: ExposureSeverity,
    /// Masked/truncated representation for UI display — never the full raw value
    pub pii_summary: String,
    /// Confidence score from the detection agent (0.0 - 1.0)
    pub confidence: f32,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub discovered_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_verified_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub removed_at: Option<OffsetDateTime>,
}

/// Aggregated exposure summary for the customer dashboard.
#[derive(Debug, Serialize)]
pub struct ExposureSummary {
    pub customer_id: Uuid,
    pub total_exposures: i64,
    pub active_exposures: i64,
    pub removed_exposures: i64,
    pub critical_count: i64,
    pub high_count: i64,
    pub medium_count: i64,
    pub low_count: i64,
    pub privacy_score: f32,
    pub sources_affected: Vec<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_scan_at: Option<OffsetDateTime>,
}
