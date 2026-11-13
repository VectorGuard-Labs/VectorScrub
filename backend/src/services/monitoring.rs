//! Continuous monitoring: removal verification and reappearance detection.
//!
//! The Python monitoring agent does the page checks. This module owns the state:
//! which records are due for a check, and what happens in the database when the
//! agent reports a result.
//!
//! Two loops, both driven by the agent through `X-Agent-Key` endpoints:
//!
//! * Verification: a removal request that was submitted is re-checked once its
//!   `next_check_at` passes. Only a positive "record is gone" result moves it to
//!   `verified_removed` (and deactivates the exposure). A request that never
//!   verifies ends as `rejected` / `unverifiable` after `VERIFICATION_TIMEOUT_DAYS`,
//!   and the customer is alerted.
//! * Reappearance: exposures that were verified removed are re-checked on the
//!   customer's tier interval (Premium weekly, Personal monthly). If the record is
//!   back, the exposure is reactivated, the removal request becomes `reappeared`,
//!   and the customer is alerted. Nothing is re-submitted automatically: removal
//!   always requires fresh customer authorization.

use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::removal_request::RemovalStatus;
use crate::services::{
    exposure as exposure_svc, notification as notification_svc, removal as removal_svc,
};

/// Re-check interval for removed exposures, by tier.
pub const PREMIUM_INTERVAL_DAYS: i32 = 7;
pub const PERSONAL_INTERVAL_DAYS: i32 = 30;

/// When a check could not reach a conclusion (blocked, timeout, empty page) retry
/// after this many days instead of waiting a full interval or hammering the source.
const INCONCLUSIVE_RETRY_DAYS: i32 = 1;

/// Stop verifying after this many days since submission and tell the customer.
pub const VERIFICATION_TIMEOUT_DAYS: i32 = 45;

/// Days since submission on which a submitted removal is re-checked. Brokers take anywhere from
/// hours to weeks, so a result is neither expected nor promised early: after each check that does
/// not settle things (still listed, or the page could not be read) the next one waits for the next
/// of these days. The last is the timeout, after which the request is closed and the customer told.
/// There are no per-broker adjustments: we have no measured processing times to base them on.
pub const VERIFICATION_SCHEDULE_DAYS: [i32; 5] = [3, 7, 14, 30, VERIFICATION_TIMEOUT_DAYS];

/// Days from now until the next scheduled check, given how many days ago the request was submitted.
pub fn days_until_next_verification(age_days: i32) -> i64 {
    VERIFICATION_SCHEDULE_DAYS
        .iter()
        .find(|d| **d > age_days)
        .map(|d| (*d - age_days) as i64)
        .unwrap_or(1)
}

/// Actor names written to the append-only audit log.
const ACTOR_MONITORING: &str = "monitoring_agent";
const ACTOR_VERIFICATION: &str = "verification_agent";

#[derive(Debug, thiserror::Error)]
pub enum MonitoringError {
    /// No such target for this customer (also returned on customer mismatch so the
    /// endpoint does not reveal whether an id exists for someone else).
    #[error("target not found")]
    NotFound,
    /// The target exists but is not in a state that accepts this result.
    #[error("target is not in a state that accepts this result")]
    WrongState,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Shared "is it due?" predicates
// ---------------------------------------------------------------------------
//
// Defined once because two things must agree exactly: the queries that list work for
// the agent, and the identity-release check (services/identity_access.rs) that only
// hands out identity for work that is currently due.

/// SQL predicate over aliases `e` (exposures) and `c` (customers): this removed
/// exposure is due for a reappearance check for an eligible customer. The arguments are
/// the SQL placeholders (e.g. "$1") for the Premium and Personal intervals in days.
pub(crate) fn monitoring_due_predicate(premium_days: &str, personal_days: &str) -> String {
    format!(
        "e.is_active = false \
         AND c.is_active = true \
         AND c.monitoring_enabled = true \
         AND c.subscription_tier IN ('personal', 'premium') \
         AND (COALESCE(e.last_monitored_at, e.removed_at) IS NULL \
              OR COALESCE(e.last_monitored_at, e.removed_at) < \
                 NOW() - make_interval(days => CASE WHEN c.subscription_tier = 'premium' \
                                                    THEN {premium_days}::int ELSE {personal_days}::int END))"
    )
}

/// SQL predicate over alias `r` (removal_requests): this submitted request's check
/// time has passed. A request with no `next_check_at` counts as due 7 days after its
/// last update so it cannot be stranded in `submitted` forever.
pub(crate) fn verification_due_predicate(alias: &str) -> String {
    format!(
        "COALESCE({alias}.next_check_at, {alias}.updated_at + make_interval(days => 7)) <= NOW()"
    )
}

// ---------------------------------------------------------------------------
// Reappearance monitoring
// ---------------------------------------------------------------------------

/// A removed exposure that is due for a reappearance check.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MonitoringTarget {
    pub exposure_id: Uuid,
    pub customer_id: Uuid,
    pub source_name: String,
    pub source_url: String,
    pub subscription_tier: String,
}

/// What the agent concluded about a removed exposure.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringOutcome {
    /// Record is not on the page.
    Clear,
    /// Record is back.
    Reappeared,
    /// Could not tell (blocked, error, unusable page). Retry soon.
    Inconclusive,
}

/// Removed exposures due for a re-check. Free-tier and monitoring-disabled
/// customers are never returned. The interval is measured from the last check, or
/// from removal if it has never been checked.
pub async fn list_exposures_due_for_monitoring(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<MonitoringTarget>, sqlx::Error> {
    let sql = format!(
        r#"
        SELECT e.id AS exposure_id,
               e.customer_id,
               e.source_name,
               e.source_url,
               c.subscription_tier::text AS subscription_tier
        FROM exposures e
        JOIN customers c ON c.id = e.customer_id
        WHERE {}
        ORDER BY COALESCE(e.last_monitored_at, e.removed_at) ASC NULLS FIRST
        LIMIT $3
        "#,
        monitoring_due_predicate("$1", "$2")
    );
    sqlx::query_as::<_, MonitoringTarget>(&sql)
        .bind(PREMIUM_INTERVAL_DAYS)
        .bind(PERSONAL_INTERVAL_DAYS)
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Apply the agent's reappearance-check result to a removed exposure.
pub async fn record_monitoring_result(
    pool: &PgPool,
    exposure_id: Uuid,
    customer_id: Uuid,
    outcome: MonitoringOutcome,
) -> Result<(), MonitoringError> {
    let mut tx = pool.begin().await?;

    // Lock the row so a repeated report cannot double-process.
    let row: Option<(Uuid, bool, String)> = sqlx::query_as(
        r#"
        SELECT e.customer_id, e.is_active, e.source_name
        FROM exposures e
        WHERE e.id = $1
        FOR UPDATE OF e
        "#,
    )
    .bind(exposure_id)
    .fetch_optional(&mut *tx)
    .await?;

    let source_name = match row {
        None => return Err(MonitoringError::NotFound),
        Some((owner, _, _)) if owner != customer_id => return Err(MonitoringError::NotFound),
        Some((_, true, _)) => return Err(MonitoringError::WrongState),
        Some((_, false, name)) => name,
    };

    match outcome {
        MonitoringOutcome::Clear => {
            sqlx::query("UPDATE exposures SET last_monitored_at = NOW() WHERE id = $1")
                .bind(exposure_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }

        MonitoringOutcome::Inconclusive => {
            // Make the exposure due again in INCONCLUSIVE_RETRY_DAYS rather than a
            // full interval from now.
            sqlx::query(
                r#"
                UPDATE exposures e
                SET last_monitored_at =
                    NOW()
                    - make_interval(days => CASE WHEN c.subscription_tier = 'premium'
                                                 THEN $2::int ELSE $3::int END)
                    + make_interval(days => $4::int)
                FROM customers c
                WHERE e.id = $1 AND c.id = e.customer_id
                "#,
            )
            .bind(exposure_id)
            .bind(PREMIUM_INTERVAL_DAYS)
            .bind(PERSONAL_INTERVAL_DAYS)
            .bind(INCONCLUSIVE_RETRY_DAYS)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }

        MonitoringOutcome::Reappeared => {
            sqlx::query(
                r#"
                UPDATE exposures
                SET is_active = true,
                    removed_at = NULL,
                    last_verified_at = NOW(),
                    last_monitored_at = NOW()
                WHERE id = $1
                "#,
            )
            .bind(exposure_id)
            .execute(&mut *tx)
            .await?;

            // Flip the most recent verified removal to `reappeared` and log it.
            let request_id: Option<Uuid> = sqlx::query_scalar(
                r#"
                SELECT id FROM removal_requests
                WHERE exposure_id = $1 AND status = 'verified_removed'
                ORDER BY created_at DESC
                LIMIT 1
                FOR UPDATE
                "#,
            )
            .bind(exposure_id)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(request_id) = request_id {
                sqlx::query(
                    "UPDATE removal_requests SET status = 'reappeared', updated_at = NOW() WHERE id = $1",
                )
                .bind(request_id)
                .execute(&mut *tx)
                .await?;

                sqlx::query(
                    r#"
                    INSERT INTO removal_audit_log
                        (id, removal_request_id, customer_id, action, actor, details, recorded_at)
                    VALUES ($1, $2, $3, 'reappeared', $4, $5, NOW())
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(request_id)
                .bind(customer_id)
                .bind(ACTOR_MONITORING)
                .bind(json!({ "source_name": source_name }))
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;

            // Post-commit side effects are best-effort: the state change above is
            // what matters, and a retry from the agent would (correctly) be rejected
            // as WrongState.
            if let Err(e) = exposure_svc::update_privacy_score(pool, customer_id).await {
                tracing::warn!(customer_id = %customer_id, error = %e,
                    "Failed to refresh privacy score after reappearance");
            }
            if let Err(e) =
                notification_svc::alert_reappearance(pool, customer_id, &source_name).await
            {
                tracing::warn!(customer_id = %customer_id, error = %e,
                    "Failed to queue reappearance alert");
            }
            tracing::warn!(
                customer_id = %customer_id,
                exposure_id = %exposure_id,
                source = %source_name,
                "Removed record reappeared"
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Removal verification
// ---------------------------------------------------------------------------

/// A submitted removal request that is due to be checked.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VerificationTarget {
    pub removal_request_id: Uuid,
    pub customer_id: Uuid,
    pub exposure_id: Uuid,
    pub source_name: String,
    pub source_url: String,
    pub status: String,
    pub days_since_submission: i32,
}

/// What the agent concluded about a submitted removal.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    /// Record is gone: the removal worked.
    Removed,
    /// Record is still live.
    StillPresent,
    /// Could not tell. Retry soon.
    Inconclusive,
}

/// Submitted / acknowledged removal requests whose check time has passed.
/// Requests without a `next_check_at` are treated as due 7 days after last update
/// so they cannot be stranded in `submitted` forever.
pub async fn list_removals_due_for_verification(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<VerificationTarget>, sqlx::Error> {
    let sql = format!(
        r#"
        SELECT r.id AS removal_request_id,
               r.customer_id,
               r.exposure_id,
               r.source_name,
               r.source_url,
               r.status::text AS status,
               (CURRENT_DATE - COALESCE(r.submitted_at, r.created_at)::date)::int4
                   AS days_since_submission
        FROM removal_requests r
        JOIN customers c ON c.id = r.customer_id
        WHERE r.status IN ('submitted', 'acknowledged')
          AND c.is_active = true
          AND {due}
        ORDER BY COALESCE(r.next_check_at, r.updated_at + make_interval(days => 7)) ASC
        LIMIT $1
        "#,
        due = verification_due_predicate("r")
    );
    sqlx::query_as::<_, VerificationTarget>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Apply the agent's verification result to a submitted removal request.
pub async fn record_verification_result(
    pool: &PgPool,
    removal_request_id: Uuid,
    customer_id: Uuid,
    outcome: VerificationOutcome,
) -> Result<(), MonitoringError> {
    let row: Option<(RemovalStatus, i32)> = sqlx::query_as(
        r#"
        SELECT status,
               (CURRENT_DATE - COALESCE(submitted_at, created_at)::date)::int4
        FROM removal_requests
        WHERE id = $1 AND customer_id = $2
        "#,
    )
    .bind(removal_request_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;

    let (current, age_days) = match row {
        None => return Err(MonitoringError::NotFound),
        Some((s, age)) if s == RemovalStatus::Submitted || s == RemovalStatus::Acknowledged => {
            (s, age)
        }
        Some(_) => return Err(MonitoringError::WrongState),
    };

    let timed_out = age_days >= VERIFICATION_TIMEOUT_DAYS;

    let (audit_action, audit_details) = match outcome {
        VerificationOutcome::Removed => {
            removal_svc::update_removal_status(
                pool,
                removal_request_id,
                RemovalStatus::VerifiedRemoved,
                None,
                None,
                None,
            )
            .await?;
            ("verified_removed", json!({ "days_since_submission": age_days }))
        }

        VerificationOutcome::StillPresent if timed_out => {
            removal_svc::update_removal_status(
                pool,
                removal_request_id,
                RemovalStatus::Rejected,
                None,
                None,
                None,
            )
            .await?;
            (
                "still_present_after_timeout",
                json!({ "days_since_submission": age_days, "timeout_days": VERIFICATION_TIMEOUT_DAYS }),
            )
        }

        VerificationOutcome::StillPresent => {
            let recheck_in = days_until_next_verification(age_days);
            let next = OffsetDateTime::now_utc() + time::Duration::days(recheck_in);
            removal_svc::update_removal_status(
                pool,
                removal_request_id,
                current,
                None,
                None,
                Some(next),
            )
            .await?;
            (
                "still_present",
                json!({ "days_since_submission": age_days, "recheck_in_days": recheck_in }),
            )
        }

        VerificationOutcome::Inconclusive if timed_out => {
            removal_svc::update_removal_status(
                pool,
                removal_request_id,
                RemovalStatus::Unverifiable,
                None,
                None,
                None,
            )
            .await?;
            (
                "unverifiable_after_timeout",
                json!({ "days_since_submission": age_days, "timeout_days": VERIFICATION_TIMEOUT_DAYS }),
            )
        }

        VerificationOutcome::Inconclusive => {
            let retry_in = days_until_next_verification(age_days);
            let next = OffsetDateTime::now_utc() + time::Duration::days(retry_in);
            removal_svc::update_removal_status(
                pool,
                removal_request_id,
                current,
                None,
                None,
                Some(next),
            )
            .await?;
            (
                "verification_inconclusive",
                json!({ "days_since_submission": age_days, "retry_in_days": retry_in }),
            )
        }
    };

    removal_svc::record_audit_entry(
        pool,
        removal_request_id,
        customer_id,
        audit_action.to_string(),
        ACTOR_VERIFICATION.to_string(),
        audit_details,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use sqlx::PgPool;

    async fn scalar_bool(pool: &PgPool, sql: &str, id: Uuid) -> bool {
        sqlx::query_scalar::<_, bool>(sql).bind(id).fetch_one(pool).await.unwrap()
    }
    async fn status_of(pool: &PgPool, id: Uuid) -> String {
        sqlx::query_scalar("SELECT status::text FROM removal_requests WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }
    async fn due_exposures(pool: &PgPool) -> Vec<Uuid> {
        list_exposures_due_for_monitoring(pool, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.exposure_id)
            .collect()
    }

    // ---- which exposures are due ---------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn due_selection_follows_tier_interval_and_eligibility(pool: PgPool) {
        let premium = ts::customer(&pool, "premium", true).await;
        let personal = ts::customer(&pool, "personal", true).await;
        let free = ts::customer(&pool, "free", true).await;
        let opted_out = ts::customer(&pool, "premium", false).await;

        let premium_due = ts::exposure(&pool, premium, false, Some(8)).await; // > 7 days
        let premium_fresh = ts::exposure(&pool, premium, false, Some(3)).await; // < 7 days
        let personal_due = ts::exposure(&pool, personal, false, Some(31)).await; // > 30 days
        let personal_fresh = ts::exposure(&pool, personal, false, Some(10)).await; // < 30 days
        let free_old = ts::exposure(&pool, free, false, Some(365)).await;
        let opted_out_old = ts::exposure(&pool, opted_out, false, Some(365)).await;
        let still_active = ts::exposure(&pool, premium, true, None).await;

        let due = due_exposures(&pool).await;
        assert!(due.contains(&premium_due));
        assert!(due.contains(&personal_due));
        for not_due in [premium_fresh, personal_fresh, free_old, opted_out_old, still_active] {
            assert!(!due.contains(&not_due), "{not_due} must not be due");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn interval_counts_from_last_check_not_from_removal(pool: PgPool) {
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(100)).await;
        assert!(due_exposures(&pool).await.contains(&e));
        sqlx::query("UPDATE exposures SET last_monitored_at = NOW() - interval '2 days' WHERE id = $1")
            .bind(e)
            .execute(&pool)
            .await
            .unwrap();
        assert!(!due_exposures(&pool).await.contains(&e), "checked 2 days ago, weekly tier");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deactivated_customers_are_never_monitored(pool: PgPool) {
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(100)).await;
        sqlx::query("UPDATE customers SET is_active = false WHERE id = $1").bind(c).execute(&pool).await.unwrap();
        assert!(!due_exposures(&pool).await.contains(&e));
    }

    // ---- recording a monitoring result ------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn clear_result_just_records_the_check(pool: PgPool) {
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(8)).await;
        record_monitoring_result(&pool, e, c, MonitoringOutcome::Clear).await.unwrap();
        assert!(!scalar_bool(&pool, "SELECT is_active FROM exposures WHERE id = $1", e).await);
        assert!(scalar_bool(&pool, "SELECT last_monitored_at IS NOT NULL FROM exposures WHERE id = $1", e).await);
        assert!(!due_exposures(&pool).await.contains(&e), "no longer due");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn inconclusive_result_retries_in_a_day_not_a_full_interval(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, false, Some(40)).await;
        record_monitoring_result(&pool, e, c, MonitoringOutcome::Inconclusive).await.unwrap();
        assert!(!due_exposures(&pool).await.contains(&e), "not due immediately");
        // Pretend a day passed: due again, long before the 30-day interval.
        sqlx::query("UPDATE exposures SET last_monitored_at = last_monitored_at - interval '25 hours' WHERE id = $1")
            .bind(e).execute(&pool).await.unwrap();
        assert!(due_exposures(&pool).await.contains(&e));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reappearance_reactivates_flags_the_request_audits_and_alerts(pool: PgPool) {
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(8)).await;
        let r = ts::removal_request(&pool, c, e, "verified_removed", None).await;

        record_monitoring_result(&pool, e, c, MonitoringOutcome::Reappeared).await.unwrap();

        assert!(scalar_bool(&pool, "SELECT is_active FROM exposures WHERE id = $1", e).await);
        assert!(scalar_bool(&pool, "SELECT removed_at IS NULL FROM exposures WHERE id = $1", e).await);
        assert_eq!(status_of(&pool, r).await, "reappeared");

        let audit: (String, String) = sqlx::query_as(
            "SELECT action, actor FROM removal_audit_log WHERE removal_request_id = $1",
        ).bind(r).fetch_one(&pool).await.unwrap();
        assert_eq!(audit, ("reappeared".into(), "monitoring_agent".into()));

        let alerts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM alerts WHERE customer_id = $1 AND alert_type = 'reappearance'",
        ).bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(alerts, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reappearance_never_submits_anything_automatically(pool: PgPool) {
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(8)).await;
        let r = ts::removal_request(&pool, c, e, "verified_removed", None).await;
        record_monitoring_result(&pool, e, c, MonitoringOutcome::Reappeared).await.unwrap();
        let authorized: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM removal_requests WHERE exposure_id = $1 AND status = 'authorized'",
        ).bind(e).fetch_one(&pool).await.unwrap();
        assert_eq!(authorized, 0, "re-removal needs fresh customer authorization");
        assert_eq!(status_of(&pool, r).await, "reappeared");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn results_are_idempotent_and_tenant_isolated(pool: PgPool) {
        let (alice, mallory) = (ts::customer(&pool, "premium", true).await, ts::customer(&pool, "premium", true).await);
        let e = ts::exposure(&pool, alice, false, Some(8)).await;

        // another customer's id cannot touch it, and learns nothing about it
        assert!(matches!(
            record_monitoring_result(&pool, e, mallory, MonitoringOutcome::Reappeared).await,
            Err(MonitoringError::NotFound)
        ));
        assert!(!scalar_bool(&pool, "SELECT is_active FROM exposures WHERE id = $1", e).await);

        record_monitoring_result(&pool, e, alice, MonitoringOutcome::Reappeared).await.unwrap();
        // replaying the report must not double-process (exposure is active now)
        assert!(matches!(
            record_monitoring_result(&pool, e, alice, MonitoringOutcome::Reappeared).await,
            Err(MonitoringError::WrongState)
        ));
        assert!(matches!(
            record_monitoring_result(&pool, Uuid::new_v4(), alice, MonitoringOutcome::Clear).await,
            Err(MonitoringError::NotFound)
        ));
        let alerts: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts WHERE customer_id = $1")
            .bind(alice).fetch_one(&pool).await.unwrap();
        assert_eq!(alerts, 1, "exactly one alert despite the replay");
    }

    // ---- verification -----------------------------------------------------------

    async fn due_removals(pool: &PgPool) -> Vec<Uuid> {
        list_removals_due_for_verification(pool, 100)
            .await.unwrap().into_iter().map(|t| t.removal_request_id).collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn verification_due_selection(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let mk = |status: &'static str, days: Option<i32>| {
            let pool = pool.clone();
            async move {
                let e = ts::exposure(&pool, c, true, None).await;
                ts::removal_request(&pool, c, e, status, days).await
            }
        };
        let due = mk("submitted", Some(-1)).await;
        let due_ack = mk("acknowledged", Some(-1)).await;
        let future = mk("submitted", Some(5)).await;
        let authorized = mk("authorized", Some(-1)).await;
        let done = mk("verified_removed", Some(-1)).await;
        let no_date = mk("submitted", None).await; // created 8 days ago => counts as due

        let listed = due_removals(&pool).await;
        for expected in [due, due_ack, no_date] {
            assert!(listed.contains(&expected), "{expected} should be due");
        }
        for unexpected in [future, authorized, done] {
            assert!(!listed.contains(&unexpected), "{unexpected} should not be due");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn removed_verdict_completes_the_removal(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(-1)).await;

        record_verification_result(&pool, r, c, VerificationOutcome::Removed).await.unwrap();

        assert_eq!(status_of(&pool, r).await, "verified_removed");
        assert!(scalar_bool(&pool, "SELECT verified_at IS NOT NULL FROM removal_requests WHERE id = $1", r).await);
        assert!(!scalar_bool(&pool, "SELECT is_active FROM exposures WHERE id = $1", e).await);
        assert!(scalar_bool(&pool, "SELECT removed_at IS NOT NULL FROM exposures WHERE id = $1", e).await);
        let audit: String = sqlx::query_scalar(
            "SELECT action FROM removal_audit_log WHERE removal_request_id = $1 AND actor = 'verification_agent'",
        ).bind(r).fetch_one(&pool).await.unwrap();
        assert_eq!(audit, "verified_removed");
        // terminal now: a replay is refused
        assert!(matches!(
            record_verification_result(&pool, r, c, VerificationOutcome::Removed).await,
            Err(MonitoringError::WrongState)
        ));
    }

    #[test]
    fn checks_follow_the_published_schedule_and_end_at_the_timeout() {
        assert_eq!(VERIFICATION_SCHEDULE_DAYS, [3, 7, 14, 30, 45]);
        // (days since submission) -> (days until the next check)
        for (age, wait) in [(0, 3), (3, 4), (7, 7), (8, 6), (14, 16), (29, 1), (30, 15), (44, 1)] {
            assert_eq!(days_until_next_verification(age), wait, "age {age}");
        }
        assert_eq!(days_until_next_verification(45), 1, "past the end: the timeout handles it, never a gap");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unsettled_check_waits_for_the_next_scheduled_day_not_the_next_day(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(-1)).await; // submitted 8 days ago
        record_verification_result(&pool, r, c, VerificationOutcome::Inconclusive).await.unwrap();
        // day 8 -> next check on day 14: about 6 days out, not tomorrow
        assert!(scalar_bool(&pool,
            "SELECT next_check_at > NOW() + interval '5 days' AND next_check_at < NOW() + interval '7 days' FROM removal_requests WHERE id = $1", r).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn still_present_keeps_waiting_then_gives_up_as_rejected(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(-1)).await;

        record_verification_result(&pool, r, c, VerificationOutcome::StillPresent).await.unwrap();
        assert_eq!(status_of(&pool, r).await, "submitted", "young request: keep waiting");
        assert!(!due_removals(&pool).await.contains(&r), "rechecked in ~7 days");

        // age past the timeout
        sqlx::query("UPDATE removal_requests SET submitted_at = NOW() - interval '50 days' WHERE id = $1")
            .bind(r).execute(&pool).await.unwrap();
        record_verification_result(&pool, r, c, VerificationOutcome::StillPresent).await.unwrap();
        assert_eq!(status_of(&pool, r).await, "rejected");
        assert!(scalar_bool(&pool, "SELECT is_active FROM exposures WHERE id = $1", e).await, "exposure stays active");
        let alerts: i64 = sqlx::query_scalar("SELECT count(*) FROM alerts WHERE customer_id = $1 AND alert_type = 'removal_failed'")
            .bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(alerts, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn inconclusive_retries_daily_then_ends_unverifiable(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(-1)).await;

        record_verification_result(&pool, r, c, VerificationOutcome::Inconclusive).await.unwrap();
        assert_eq!(status_of(&pool, r).await, "submitted");

        sqlx::query("UPDATE removal_requests SET submitted_at = NOW() - interval '50 days' WHERE id = $1")
            .bind(r).execute(&pool).await.unwrap();
        record_verification_result(&pool, r, c, VerificationOutcome::Inconclusive).await.unwrap();
        assert_eq!(status_of(&pool, r).await, "unverifiable");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn verification_results_are_tenant_isolated(pool: PgPool) {
        let (alice, mallory) = (ts::customer(&pool, "personal", true).await, ts::customer(&pool, "personal", true).await);
        let e = ts::exposure(&pool, alice, true, None).await;
        let r = ts::removal_request(&pool, alice, e, "submitted", Some(-1)).await;
        assert!(matches!(
            record_verification_result(&pool, r, mallory, VerificationOutcome::Removed).await,
            Err(MonitoringError::NotFound)
        ));
        assert_eq!(status_of(&pool, r).await, "submitted");
    }
}
