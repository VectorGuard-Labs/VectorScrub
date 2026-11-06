use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// The current state of a scan job in its lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "scan_job_status", rename_all = "snake_case")]
pub enum ScanJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Represents a single scan pass for a customer.
/// Created when a customer initiates a scan; updated as the AI agents
/// process sources and report back findings.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ScanJob {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub status: ScanJobStatus,
    pub sources_queued: i32,
    pub sources_completed: i32,
    pub sources_failed: i32,
    pub exposures_found: i32,
    pub error_message: Option<String>,
    pub queued_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
}

/// Payload dispatched to the Python agent orchestrator via Redis.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScanJobPayload {
    pub job_id: Uuid,
    pub customer_id: Uuid,
    /// Encrypted identity token — the Python agent calls back to the
    /// Rust API to retrieve decrypted fields on a need-to-know basis.
    pub identity_token: String,
    pub tier: String,
}
