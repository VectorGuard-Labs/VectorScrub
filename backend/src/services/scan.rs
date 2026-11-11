use anyhow::anyhow;
use redis::AsyncCommands;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::crypto::MasterKey;
use crate::models::customer::SubscriptionTier;
use crate::models::scan_job::{ScanJob, ScanJobPayload, ScanJobStatus, SourceCoverage};
use crate::services::{exposure as exposure_svc, identity_access};

pub const SCAN_QUEUE_KEY: &str = "vectorscrub:scan_jobs";

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The caller is over their plan's scan limit. The message is safe to show to them.
    #[error("{0}")]
    RateLimited(&'static str),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// The tier name as it appears in the queue payload ("free" | "personal" | "premium").
pub fn tier_name(tier: &SubscriptionTier) -> String {
    format!("{tier:?}").to_lowercase()
}

/// Create a scan job and put it on the queue for the agent orchestrator.
///
/// The database row is the source of truth; the Redis entry is only a delivery hint.
/// If the enqueue fails the job is NOT lost: it stays `queued` with no `dispatched_at`,
/// and the recovery loop (jobs/recovery.rs) dispatches it shortly.
pub async fn initiate_scan(
    pool: &PgPool,
    redis: &redis::Client,
    master: &MasterKey,
    customer_id: Uuid,
    tier: &SubscriptionTier,
) -> Result<ScanJob, ScanError> {
    // Per-tier scan limit. Development: 1 minute for testing, production: 7 days.
    // A scan that failed (for example the analysis service was down) does not count: the
    // customer got nothing from it and must be able to run it again.
    let rate_limit_interval = if cfg!(debug_assertions) { "1 minute" } else { "7 days" };
    let recent_scan: Option<(OffsetDateTime,)> = sqlx::query_as(&format!(
        "SELECT queued_at FROM scan_jobs WHERE customer_id = $1 AND status <> 'failed' \
         AND queued_at > NOW() - INTERVAL '{rate_limit_interval}' ORDER BY queued_at DESC LIMIT 1"
    ))
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;

    if recent_scan.is_some() && matches!(tier, SubscriptionTier::Free) {
        return Err(ScanError::RateLimited(if cfg!(debug_assertions) {
            "Free accounts can run one scan per minute (dev mode)"
        } else {
            "Free accounts can run one scan every 7 days"
        }));
    }

    let job = sqlx::query_as::<_, ScanJob>(
        r#"
        INSERT INTO scan_jobs (id, customer_id, status, sources_queued, sources_completed,
                               sources_failed, exposures_found, queued_at)
        VALUES ($1, $2, 'queued', 0, 0, 0, 0, NOW())
        RETURNING *
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    // Any scan, manual or scheduled, restarts the clock the rescan scheduler counts from.
    // Before this only the scheduler set it, so a customer who scanned by hand was scanned
    // again straight away, and `/me` never showed a last scan.
    sqlx::query("UPDATE customers SET last_scan_at = NOW() WHERE id = $1")
        .bind(customer_id)
        .execute(pool)
        .await?;

    if let Err(e) = dispatch_job(pool, redis, master, job.id, customer_id, &tier_name(tier)).await {
        tracing::error!(job_id = %job.id, error = %e,
            "Could not enqueue scan job; the recovery loop will dispatch it");
    } else {
        tracing::info!(job_id = %job.id, customer_id = %customer_id, "Scan job queued");
    }

    Ok(job)
}

/// Put a job on the Redis queue with a freshly minted, job-scoped identity token, and
/// record when. Safe to call more than once for the same job: workers claim a job
/// atomically before processing it, so a duplicate queue entry is harmless.
pub async fn dispatch_job(
    pool: &PgPool,
    redis: &redis::Client,
    master: &MasterKey,
    job_id: Uuid,
    customer_id: Uuid,
    tier: &str,
) -> anyhow::Result<()> {
    let payload = ScanJobPayload {
        job_id,
        customer_id,
        identity_token: identity_access::issue_scan_token(master, customer_id, job_id)?,
        tier: tier.to_string(),
    };
    let payload_json = serde_json::to_string(&payload)?;

    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| anyhow!("Redis connection failed: {e}"))?;
    conn.lpush::<_, _, ()>(SCAN_QUEUE_KEY, &payload_json)
        .await
        .map_err(|e| anyhow!("Failed to enqueue scan job: {e}"))?;

    sqlx::query("UPDATE scan_jobs SET dispatched_at = NOW() WHERE id = $1")
        .bind(job_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Fetch a scan job by ID, verifying it belongs to the requesting customer.
pub async fn get_scan_job(
    pool: &PgPool,
    job_id: Uuid,
    customer_id: Uuid,
) -> anyhow::Result<Option<ScanJob>> {
    Ok(sqlx::query_as::<_, ScanJob>(
        "SELECT * FROM scan_jobs WHERE id = $1 AND customer_id = $2",
    )
    .bind(job_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await?)
}

/// List recent scan jobs for a customer (most recent first).
pub async fn list_scan_jobs(
    pool: &PgPool,
    customer_id: Uuid,
    limit: i64,
) -> anyhow::Result<Vec<ScanJob>> {
    Ok(sqlx::query_as::<_, ScanJob>(
        "SELECT * FROM scan_jobs WHERE customer_id = $1 ORDER BY queued_at DESC LIMIT $2",
    )
    .bind(customer_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

// ---------------------------------------------------------------------------
// Worker protocol: claim -> heartbeat/progress -> complete|fail
// ---------------------------------------------------------------------------

/// Atomically take a queued job. Returns the attempt number (the fencing token the
/// worker must echo on every later call), or None if the job is not claimable
/// (already running, finished, or gone), in which case the worker just skips it.
pub async fn claim_scan_job(pool: &PgPool, job_id: Uuid) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        UPDATE scan_jobs
        SET status = 'running',
            started_at = COALESCE(started_at, NOW()),
            heartbeat_at = NOW(),
            attempts = attempts + 1,
            error_message = NULL
        WHERE id = $1 AND status = 'queued'
        RETURNING attempts
        "#,
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

/// Result of a status update from a worker.
#[derive(Debug, PartialEq, Eq)]
pub enum StatusOutcome {
    Applied,
    /// The job is not running under this attempt (finished, requeued, or taken over by
    /// another worker). The caller must stop working on it.
    Stale,
}

/// Store which sources the scan could and could not read. Applies only under the same
/// conditions as a status update (job running, same attempt), so a worker that lost the job
/// cannot overwrite the new attempt's report. Written BEFORE the final status, so a customer
/// who sees "completed" always sees its coverage too.
pub async fn record_coverage(
    pool: &PgPool,
    job_id: Uuid,
    attempt: Option<i32>,
    coverage: &[SourceCoverage],
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE scan_jobs SET coverage = $2, sources_queued = $3 \
         WHERE id = $1 AND status = 'running' AND ($4::int IS NULL OR attempts = $4)",
    )
    .bind(job_id)
    .bind(serde_json::to_value(coverage)?)
    .bind(i32::try_from(coverage.len())?)
    .bind(attempt)
    .execute(pool)
    .await?;
    Ok(())
}

/// Progress / completion report from the worker holding the job. Applies only if the
/// job is `running` and, when `attempt` is given, still on that attempt. This fences
/// out a worker that was presumed dead and comes back after a newer attempt took over.
/// A `Running` update is a progress report and doubles as a heartbeat.
#[allow(clippy::too_many_arguments)] // mirrors the columns of the report; a struct would just rename them
pub async fn update_scan_status(
    pool: &PgPool,
    job_id: Uuid,
    status: ScanJobStatus,
    sources_completed: Option<i32>,
    sources_failed: Option<i32>,
    exposures_found: Option<i32>,
    error_message: Option<String>,
    attempt: Option<i32>,
) -> anyhow::Result<StatusOutcome> {
    let customer_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE scan_jobs SET
            status = $1,
            sources_completed = COALESCE($2, sources_completed),
            sources_failed = COALESCE($3, sources_failed),
            exposures_found = COALESCE($4, exposures_found),
            error_message = $5,
            completed_at = CASE WHEN $1 IN ('completed', 'failed') THEN NOW() ELSE completed_at END,
            heartbeat_at = NOW()
        WHERE id = $6
          AND status = 'running'
          AND ($7::int IS NULL OR attempts = $7)
        RETURNING customer_id
        "#,
    )
    .bind(&status)
    .bind(sources_completed)
    .bind(sources_failed)
    .bind(exposures_found)
    .bind(error_message)
    .bind(job_id)
    .bind(attempt)
    .fetch_optional(pool)
    .await?;

    let Some(customer_id) = customer_id else {
        return Ok(StatusOutcome::Stale);
    };

    // Update privacy score and record snapshot when scan completes
    if status == ScanJobStatus::Completed {
        exposure_svc::update_privacy_score(pool, customer_id).await?;
    }
    Ok(StatusOutcome::Applied)
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// A running job with no heartbeat for this long is presumed lost.
    pub stale_after_secs: i64,
    /// A queued job not dispatched for this long gets (re-)dispatched.
    pub redispatch_after_secs: i64,
    /// A job lost this many times is failed instead of requeued again.
    pub max_attempts: i32,
}

#[derive(Debug)]
pub struct PendingDispatch {
    pub job_id: Uuid,
    pub customer_id: Uuid,
    pub tier: String,
}

#[derive(Debug, Default)]
pub struct RecoveryReport {
    /// Running jobs presumed lost and put back to `queued`: (job_id, customer_id).
    pub requeued: Vec<(Uuid, Uuid)>,
    /// Jobs that kept getting lost and were failed: (job_id, customer_id).
    pub failed: Vec<(Uuid, Uuid)>,
    /// Queued jobs that now need a Redis entry (stuck, never dispatched, or requeued).
    pub to_dispatch: Vec<PendingDispatch>,
}

/// Database half of recovery. Multi-instance safe (`FOR UPDATE SKIP LOCKED`). The
/// caller dispatches `to_dispatch` to Redis (see jobs/recovery.rs).
pub async fn recover_jobs(pool: &PgPool, cfg: &RecoveryConfig) -> Result<RecoveryReport, sqlx::Error> {
    // 1. Running jobs that went silent too many times: fail them.
    let failed: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        UPDATE scan_jobs
        SET status = 'failed', completed_at = NOW(),
            error_message = 'The scan did not complete: the worker stopped responding and the maximum number of attempts was reached'
        WHERE id IN (
            SELECT id FROM scan_jobs
            WHERE status = 'running'
              AND COALESCE(heartbeat_at, started_at, queued_at) < NOW() - make_interval(secs => $1::int)
              AND attempts >= $2
            FOR UPDATE SKIP LOCKED)
        RETURNING id, customer_id
        "#,
    )
    .bind(cfg.stale_after_secs)
    .bind(cfg.max_attempts)
    .fetch_all(pool)
    .await?;

    // 2. The rest of the silent running jobs: put them back on the queue.
    let requeued: Vec<(Uuid, Uuid)> = sqlx::query_as(
        r#"
        UPDATE scan_jobs
        SET status = 'queued', dispatched_at = NULL, heartbeat_at = NULL
        WHERE id IN (
            SELECT id FROM scan_jobs
            WHERE status = 'running'
              AND COALESCE(heartbeat_at, started_at, queued_at) < NOW() - make_interval(secs => $1::int)
              AND attempts < $2
            FOR UPDATE SKIP LOCKED)
        RETURNING id, customer_id
        "#,
    )
    .bind(cfg.stale_after_secs)
    .bind(cfg.max_attempts)
    .fetch_all(pool)
    .await?;

    // 3. Queued jobs that need a queue entry: never dispatched (enqueue failed, or just
    //    requeued above), or dispatched long ago and still unclaimed (entry may be lost).
    let to_dispatch: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"
        UPDATE scan_jobs j SET dispatched_at = NOW()
        FROM customers c
        WHERE c.id = j.customer_id AND c.is_active = true
          AND j.id IN (
            SELECT id FROM scan_jobs
            WHERE status = 'queued'
              AND (dispatched_at IS NULL
                   OR dispatched_at < NOW() - make_interval(secs => $1::int))
            FOR UPDATE SKIP LOCKED)
        RETURNING j.id, j.customer_id, c.subscription_tier::text
        "#,
    )
    .bind(cfg.redispatch_after_secs)
    .fetch_all(pool)
    .await?;

    Ok(RecoveryReport {
        requeued,
        failed,
        to_dispatch: to_dispatch
            .into_iter()
            .map(|(job_id, customer_id, tier)| PendingDispatch { job_id, customer_id, tier })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    fn cfg() -> RecoveryConfig {
        RecoveryConfig { stale_after_secs: 600, redispatch_after_secs: 600, max_attempts: 3 }
    }

    async fn row(pool: &PgPool, id: Uuid) -> (String, i32, Option<String>) {
        sqlx::query_as("SELECT status::text, attempts, error_message FROM scan_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }
    async fn age(pool: &PgPool, id: Uuid, column: &str, minutes: i32) {
        sqlx::query(&format!(
            "UPDATE scan_jobs SET {column} = NOW() - make_interval(mins => $2) WHERE id = $1"
        ))
        .bind(id)
        .bind(minutes)
        .execute(pool)
        .await
        .unwrap();
    }

    // ---- claim ----------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn exactly_one_worker_can_claim_a_job(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;

        assert_eq!(claim_scan_job(&pool, job).await.unwrap(), Some(1));
        assert_eq!(claim_scan_job(&pool, job).await.unwrap(), None, "already running");
        assert_eq!(row(&pool, job).await.0, "running");

        // concurrent claimers: still exactly one winner
        let c2 = ts::customer(&pool, "free", false).await;
        let job2 = ts::scan_job(&pool, c2, "queued").await;
        let results = futures_all(&pool, job2, 8).await;
        assert_eq!(results.iter().filter(|r| r.is_some()).count(), 1);
    }

    async fn futures_all(pool: &PgPool, job: Uuid, n: usize) -> Vec<Option<i32>> {
        let mut handles = Vec::new();
        for _ in 0..n {
            let p = pool.clone();
            handles.push(tokio::spawn(async move { claim_scan_job(&p, job).await.unwrap() }));
        }
        let mut out = Vec::new();
        for h in handles {
            out.push(h.await.unwrap());
        }
        out
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn finished_or_missing_jobs_cannot_be_claimed(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        for status in ["completed", "failed", "cancelled", "running"] {
            let job = ts::scan_job(&pool, c, status).await;
            assert_eq!(claim_scan_job(&pool, job).await.unwrap(), None, "{status}");
        }
        assert_eq!(claim_scan_job(&pool, Uuid::new_v4()).await.unwrap(), None);
    }

    // ---- status updates and fencing --------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn completion_requires_the_current_attempt(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        let attempt = claim_scan_job(&pool, job).await.unwrap().unwrap();

        // wrong attempt (a zombie from an earlier claim) is fenced out
        let stale = update_scan_status(&pool, job, ScanJobStatus::Completed, Some(5), None, Some(2), None, Some(attempt + 1)).await.unwrap();
        assert_eq!(stale, StatusOutcome::Stale);
        assert_eq!(row(&pool, job).await.0, "running");

        let ok = update_scan_status(&pool, job, ScanJobStatus::Completed, Some(5), Some(0), Some(2), None, Some(attempt)).await.unwrap();
        assert_eq!(ok, StatusOutcome::Applied);
        let (status, _, _) = row(&pool, job).await;
        assert_eq!(status, "completed");
        let completed_at: bool = sqlx::query_scalar("SELECT completed_at IS NOT NULL FROM scan_jobs WHERE id = $1")
            .bind(job).fetch_one(&pool).await.unwrap();
        assert!(completed_at);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_finished_job_cannot_be_overwritten(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        let attempt = claim_scan_job(&pool, job).await.unwrap().unwrap();
        update_scan_status(&pool, job, ScanJobStatus::Completed, None, None, Some(3), None, Some(attempt)).await.unwrap();
        // a late "failed" from a confused worker must not rewrite history
        let late = update_scan_status(&pool, job, ScanJobStatus::Failed, None, None, None, Some("boom".into()), Some(attempt)).await.unwrap();
        assert_eq!(late, StatusOutcome::Stale);
        assert_eq!(row(&pool, job).await.0, "completed");
        // and a queued (not yet claimed) job cannot be completed by an unauthenticated guess
        let queued = ts::scan_job(&pool, c, "queued").await;
        assert_eq!(
            update_scan_status(&pool, queued, ScanJobStatus::Completed, None, None, None, None, None).await.unwrap(),
            StatusOutcome::Stale
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn progress_reports_double_as_heartbeats(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        let attempt = claim_scan_job(&pool, job).await.unwrap().unwrap();
        age(&pool, job, "heartbeat_at", 30).await;
        update_scan_status(&pool, job, ScanJobStatus::Running, Some(2), None, None, None, Some(attempt)).await.unwrap();
        let fresh: bool = sqlx::query_scalar("SELECT heartbeat_at > NOW() - interval '1 minute' FROM scan_jobs WHERE id = $1")
            .bind(job).fetch_one(&pool).await.unwrap();
        assert!(fresh);
    }

    // ---- recovery ----------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn a_silent_running_job_is_requeued_and_redispatched(pool: PgPool) {
        let c = ts::customer(&pool, "premium", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        claim_scan_job(&pool, job).await.unwrap();
        age(&pool, job, "heartbeat_at", 15).await; // worker died 15 minutes ago

        let report = recover_jobs(&pool, &cfg()).await.unwrap();
        assert_eq!(report.requeued, vec![(job, c)]);
        assert!(report.failed.is_empty());
        assert_eq!(report.to_dispatch.len(), 1);
        assert_eq!(report.to_dispatch[0].job_id, job);
        assert_eq!(report.to_dispatch[0].customer_id, c);
        assert_eq!(report.to_dispatch[0].tier, "premium");
        assert_eq!(row(&pool, job).await.0, "queued");

        // the next claim is attempt 2, and the original worker's attempt 1 is fenced out
        assert_eq!(claim_scan_job(&pool, job).await.unwrap(), Some(2));
        let zombie = update_scan_status(&pool, job, ScanJobStatus::Completed, None, None, Some(9), None, Some(1)).await.unwrap();
        assert_eq!(zombie, StatusOutcome::Stale);
        assert_eq!(row(&pool, job).await.0, "running");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_healthy_running_job_is_left_alone(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        claim_scan_job(&pool, job).await.unwrap();
        age(&pool, job, "heartbeat_at", 2).await;
        let report = recover_jobs(&pool, &cfg()).await.unwrap();
        assert!(report.requeued.is_empty() && report.failed.is_empty() && report.to_dispatch.is_empty());
        assert_eq!(row(&pool, job).await.0, "running");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_job_that_keeps_dying_is_failed_not_retried_forever(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        for _ in 0..3 {
            assert!(claim_scan_job(&pool, job).await.unwrap().is_some());
            age(&pool, job, "heartbeat_at", 30).await;
            let report = recover_jobs(&pool, &cfg()).await.unwrap();
            if report.failed.is_empty() {
                assert_eq!(report.requeued, vec![(job, c)]);
            }
        }
        let (status, attempts, err) = row(&pool, job).await;
        assert_eq!(status, "failed");
        assert_eq!(attempts, 3);
        assert!(err.unwrap().contains("maximum number of attempts"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn queued_jobs_are_redispatched_only_when_the_queue_entry_may_be_lost(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let fresh = ts::scan_job(&pool, c, "queued").await; // dispatched just now
        let stuck = ts::scan_job(&pool, c, "queued").await;
        age(&pool, stuck, "dispatched_at", 20).await; // dispatched 20 min ago, never claimed
        let never = ts::scan_job(&pool, c, "queued").await;
        sqlx::query("UPDATE scan_jobs SET dispatched_at = NULL WHERE id = $1").bind(never).execute(&pool).await.unwrap(); // enqueue failed

        let report = recover_jobs(&pool, &cfg()).await.unwrap();
        let ids: Vec<Uuid> = report.to_dispatch.iter().map(|d| d.job_id).collect();
        assert!(ids.contains(&stuck) && ids.contains(&never));
        assert!(!ids.contains(&fresh));

        // dispatch is recorded, so a second pass does not pile up duplicates
        let again = recover_jobs(&pool, &cfg()).await.unwrap();
        assert!(again.to_dispatch.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn completed_failed_and_cancelled_jobs_are_never_touched(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        for status in ["completed", "failed", "cancelled"] {
            let job = ts::scan_job(&pool, c, status).await;
            age(&pool, job, "heartbeat_at", 600).await;
            age(&pool, job, "dispatched_at", 600).await;
        }
        let report = recover_jobs(&pool, &cfg()).await.unwrap();
        assert!(report.requeued.is_empty() && report.failed.is_empty() && report.to_dispatch.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn jobs_of_deactivated_customers_are_not_redispatched(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        age(&pool, job, "dispatched_at", 60).await;
        sqlx::query("UPDATE customers SET is_active = false WHERE id = $1").bind(c).execute(&pool).await.unwrap();
        assert!(recover_jobs(&pool, &cfg()).await.unwrap().to_dispatch.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_recovery_passes_do_not_double_requeue(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "queued").await;
        claim_scan_job(&pool, job).await.unwrap();
        age(&pool, job, "heartbeat_at", 30).await;

        let config = cfg();
        let (a, b) = tokio::join!(recover_jobs(&pool, &config), recover_jobs(&pool, &config));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.requeued.len() + b.requeued.len(), 1, "exactly one instance requeues it");
        assert_eq!(a.to_dispatch.len() + b.to_dispatch.len(), 1, "and exactly one dispatches it");
    }
}
