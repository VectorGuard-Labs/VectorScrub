use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// The current state of a scan job in its lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "scan_job_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ScanJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// What happened when a scan tried to read one source.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoverageOutcome {
    /// The page was read. Whether it listed the customer is a separate question.
    Checked,
    /// The site refused automated access (HTTP 401/403, a CAPTCHA or "verify you are human" page).
    Blocked,
    /// The search address no longer works (HTTP 404/410 and similar).
    NotFound,
    /// The site was down, rate-limited us, or could not be reached.
    Unavailable,
    /// The page loaded but showed no readable results (for example it is built by JavaScript).
    Unreadable,
    /// The site can only be searched through an interactive form, so it is not checked
    /// automatically. The customer has to search it themselves.
    NotSupported,
}

/// One source's outcome in a scan. `source` is the public site name, never a URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceCoverage {
    pub source: String,
    pub outcome: CoverageOutcome,
}

/// Most sources a single scan can report. The highest tier has 19; this leaves headroom.
pub const MAX_COVERAGE_ENTRIES: usize = 100;

impl SourceCoverage {
    /// A source name is a hostname: letters, digits, dots and hyphens, 1 to 100 characters.
    pub fn is_valid(&self) -> bool {
        !self.source.is_empty()
            && self.source.len() <= 100
            && self.source.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    }
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
    /// Per-source outcomes (a JSON array of `SourceCoverage`), or null for older scans.
    pub coverage: Option<serde_json::Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub queued_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
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
