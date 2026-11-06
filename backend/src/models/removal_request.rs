use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Lifecycle state of a data removal request.
/// Phase 2 — used when removal workflow is wired up.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "removal_status", rename_all = "snake_case")]
pub enum RemovalStatus {
    /// Customer has authorized; request not yet submitted
    Authorized,
    /// Request submitted to the data source
    Submitted,
    /// Data source acknowledged receipt
    Acknowledged,
    /// VectorScrub verified the record is no longer present
    VerifiedRemoved,
    /// Data source rejected or denied the request
    Rejected,
    /// Unable to verify removal (source unresponsive or unverifiable)
    Unverifiable,
    /// Previously removed record has reappeared
    Reappeared,
}

/// A single data removal request targeting one exposure on one source.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RemovalRequest {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub exposure_id: Uuid,
    pub source_name: String,
    pub source_url: String,
    pub status: RemovalStatus,
    /// Which opt-out procedure was used (e.g., "web_form", "email", "drop_api")
    pub procedure_used: String,
    /// Confirmation or reference number returned by the source
    pub confirmation_ref: Option<String>,
    /// Evidence URL or screenshot hash proving removal
    pub evidence_hash: Option<String>,
    pub customer_authorized_at: OffsetDateTime,
    pub submitted_at: Option<OffsetDateTime>,
    pub verified_at: Option<OffsetDateTime>,
    pub next_check_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Audit log entry for every action taken on a removal request.
/// Append-only; never updated or deleted.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RemovalAuditEntry {
    pub id: Uuid,
    pub removal_request_id: Uuid,
    pub customer_id: Uuid,
    pub action: String,
    pub actor: String,
    pub details: serde_json::Value,
    pub recorded_at: OffsetDateTime,
}
