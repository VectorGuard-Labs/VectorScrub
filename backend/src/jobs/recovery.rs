//! Scan job recovery.
//!
//! A scan job used to exist in two places that could disagree: a Redis list entry
//! (destroyed when a worker pops it) and a `scan_jobs` row. A worker that crashed after
//! popping left the row `queued` or `running` forever and the customer waiting. The row
//! is now the source of truth and this loop repairs it:
//!
//!   * a `running` job whose worker stopped sending heartbeats is put back on the queue;
//!   * a job that has been lost too many times is failed, and the customer is told;
//!   * a `queued` job with no queue entry (the enqueue failed, or the entry was lost) is
//!     dispatched again.
//!
//! Re-dispatch mints a fresh job-scoped identity token each time. Workers claim a job
//! atomically before processing it and fence every update with the attempt number, so a
//! duplicate queue entry, or a worker that was presumed dead and comes back, is harmless.
//! Safe to run on several backend instances at once (`FOR UPDATE SKIP LOCKED`).

use std::time::Duration;

use sqlx::PgPool;
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::crypto::MasterKey;
use crate::services::scan::RecoveryConfig;
use crate::services::{
    audit, login_throttle, notification as notification_svc, privacy_score as privacy_score_svc,
    removal as removal_svc, scan as scan_svc,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecoverySummary {
    pub requeued: usize,
    pub failed: usize,
    pub dispatched: usize,
    pub dispatch_errors: usize,
}

/// Run recovery forever, once per `interval`.
pub async fn run_scan_recovery(
    pool: PgPool,
    redis: redis::Client,
    master_key_b64: String,
    cfg: RecoveryConfig,
    interval: Duration,
) {
    info!(
        interval_secs = interval.as_secs(),
        stale_after_secs = cfg.stale_after_secs,
        max_attempts = cfg.max_attempts,
        "Scan job recovery starting"
    );
    loop {
        match recover_once(&pool, &redis, &master_key_b64, &cfg).await {
            Ok(s) if s != RecoverySummary::default() => info!(?s, "Scan recovery pass"),
            Ok(_) => {}
            Err(e) => error!(error = %e, "Scan recovery pass failed"),
        }
        sleep(interval).await;
    }
}

/// One recovery pass. Returns what it did.
pub async fn recover_once(
    pool: &PgPool,
    redis: &redis::Client,
    master_key_b64: &str,
    cfg: &RecoveryConfig,
) -> anyhow::Result<RecoverySummary> {
    let master = MasterKey::from_base64(master_key_b64)?;
    let report = scan_svc::recover_jobs(pool, cfg).await?;
    let mut summary = RecoverySummary {
        requeued: report.requeued.len(),
        failed: report.failed.len(),
        ..Default::default()
    };

    for (job_id, customer_id) in &report.requeued {
        warn!(job_id = %job_id, "Scan worker went silent; job requeued");
        audit::AuditEntry::builder(audit::actions::SCAN_RECOVERED, audit::resources::SCAN)
            .customer_id(*customer_id)
            .resource_id(*job_id)
            .log_or_warn(pool)
            .await;
    }

    for (job_id, customer_id) in &report.failed {
        error!(job_id = %job_id, "Scan job lost too many times; failed");
        audit::AuditEntry::builder(audit::actions::SCAN_FAILED, audit::resources::SCAN)
            .customer_id(*customer_id)
            .resource_id(*job_id)
            .status(audit::AuditStatus::Failure)
            .error_message("worker stopped responding; max attempts reached")
            .log_or_warn(pool)
            .await;
        if let Err(e) = notification_svc::send_alert(
            pool,
            *customer_id,
            "scan_failed",
            "Your scan could not be completed",
            "We were unable to finish your scan after several attempts. Please start a new scan from your dashboard.",
        )
        .await
        {
            warn!(job_id = %job_id, error = %e, "Could not alert the customer that their scan failed");
        }
    }

    for pending in &report.to_dispatch {
        match scan_svc::dispatch_job(pool, redis, &master, pending.job_id, pending.customer_id, &pending.tier).await {
            Ok(()) => summary.dispatched += 1,
            Err(e) => {
                summary.dispatch_errors += 1;
                error!(job_id = %pending.job_id, error = %e, "Could not dispatch scan job; will retry");
                // Make it eligible again on the next pass instead of waiting out the
                // re-dispatch interval.
                if let Err(e) = sqlx::query("UPDATE scan_jobs SET dispatched_at = NULL WHERE id = $1 AND status = 'queued'")
                    .bind(pending.job_id)
                    .execute(pool)
                    .await
                {
                    warn!(job_id = %pending.job_id, error = %e, "Could not reset dispatch time; the job waits out the re-dispatch interval");
                }
            }
        }
    }

    // Removal submissions that a worker claimed and never reported on (it crashed, or the
    // backend was unreachable for too long). They are ended as failed, never handed to
    // another worker, because the first one may already have sent the request.
    match removal_svc::fail_abandoned_claims(pool, removal_svc::SUBMISSION_CLAIM_STALE_AFTER_SECS).await {
        Ok(ended) => {
            for (request_id, _) in &ended {
                warn!(removal_request_id = %request_id, "Removal submission claim abandoned; request failed");
            }
            metrics::counter!("removal_claims_abandoned_total").increment(ended.len() as u64);
        }
        Err(e) => error!(error = ?e, "Could not fail abandoned removal claims"),
    }

    metrics::counter!("scan_jobs_requeued_total").increment(summary.requeued as u64);
    metrics::counter!("scan_jobs_failed_by_recovery_total").increment(summary.failed as u64);
    metrics::counter!("scan_jobs_redispatched_total").increment(summary.dispatched as u64);

    // Housekeeping that rides on the same timer. Deliberately limited to data that is
    // derived or transient: expired login-throttle rows and privacy-score snapshots older
    // than a year. Nothing here touches customer data or any audit log.
    if let Err(e) = login_throttle::purge_stale(pool).await {
        warn!(error = %e, "Could not purge stale login throttle records");
    }
    match privacy_score_svc::cleanup_old_records(pool).await {
        Ok(n) if n > 0 => info!(deleted = n, "Pruned privacy score history older than 1 year"),
        Ok(_) => {}
        Err(e) => warn!(error = %e, "Could not prune privacy score history"),
    }

    Ok(summary)
}
