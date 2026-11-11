use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Lifecycle state of a data removal request.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "removal_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
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
    /// Automated submission could not be completed (form changed, CAPTCHA,
    /// no confirmation observed, transport error). The customer may re-authorize.
    Failed,
}

impl RemovalStatus {
    /// Legal moves of the removal lifecycle:
    ///
    /// ```text
    ///   authorized -> submitted | failed
    ///   submitted  -> submitted (refresh check date) | acknowledged | verified_removed
    ///                 | rejected | unverifiable
    ///   acknowledged -> acknowledged | verified_removed | rejected | unverifiable
    ///   verified_removed -> reappeared
    ///   rejected, unverifiable, failed, reappeared -> (terminal: the customer
    ///                 authorizes a new request)
    /// ```
    ///
    /// Everything else is refused. In particular nothing can jump to `verified_removed`
    /// without first being `submitted`, so a removal cannot be "confirmed" that was never
    /// sent.
    pub fn can_transition_to(&self, next: &RemovalStatus) -> bool {
        use RemovalStatus::*;
        matches!(
            (self, next),
            (Authorized, Submitted | Failed)
                | (Submitted, Submitted | Acknowledged | VerifiedRemoved | Rejected | Unverifiable)
                | (Acknowledged, Acknowledged | VerifiedRemoved | Rejected | Unverifiable)
                | (VerifiedRemoved, Reappeared)
        )
    }
}

/// A single data removal request targeting one exposure on one source.
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
    #[serde(with = "time::serde::rfc3339")]
    pub customer_authorized_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub submitted_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub verified_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub next_check_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Audit log entry for every action taken on a removal request.
/// Append-only; never updated or deleted.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RemovalAuditEntry {
    pub id: Uuid,
    pub removal_request_id: Uuid,
    pub customer_id: Uuid,
    pub action: String,
    pub actor: String,
    pub details: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub recorded_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::RemovalStatus::{self, *};

    const ALL: [RemovalStatus; 8] = [
        Authorized, Submitted, Acknowledged, VerifiedRemoved, Rejected, Unverifiable, Reappeared, Failed,
    ];

    #[test]
    fn the_legal_transitions_are_exactly_these() {
        let legal = [
            (Authorized, Submitted), (Authorized, Failed),
            (Submitted, Submitted), (Submitted, Acknowledged), (Submitted, VerifiedRemoved),
            (Submitted, Rejected), (Submitted, Unverifiable),
            (Acknowledged, Acknowledged), (Acknowledged, VerifiedRemoved),
            (Acknowledged, Rejected), (Acknowledged, Unverifiable),
            (VerifiedRemoved, Reappeared),
        ];
        for from in &ALL {
            for to in &ALL {
                let expected = legal.iter().any(|(f, t)| f == from && t == to);
                assert_eq!(from.can_transition_to(to), expected, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn terminal_states_go_nowhere() {
        for terminal in [Rejected, Unverifiable, Failed, Reappeared] {
            for to in &ALL {
                assert!(!terminal.can_transition_to(to), "{terminal:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn nothing_is_verified_removed_without_being_submitted_first() {
        assert!(!Authorized.can_transition_to(&VerifiedRemoved));
        assert!(!Failed.can_transition_to(&VerifiedRemoved));
        assert!(Submitted.can_transition_to(&VerifiedRemoved));
    }
}
