use anyhow::{anyhow, Result};
use redis::AsyncCommands;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::customer::SubscriptionTier;
use crate::models::scan_job::{ScanJob, ScanJobPayload, ScanJobStatus};

const SCAN_QUEUE_KEY: &str = "vectorscrub:scan_jobs";

/// Create a new scan job record and enqueue it for the Python agent orchestrator.
pub async fn initiate_scan(
    pool: &PgPool,
    redis: &redis::Client,
    customer_id: Uuid,
    tier: &SubscriptionTier,
    identity_token: String,
) -> Result<ScanJob> {
    // Enforce per-tier scan rate limits (simplified — production should use sliding window)
    // Development: 1 minute for testing, production: 24 hours
    let rate_limit_interval = if cfg!(debug_assertions) { "1 minute" } else { "24 hours" };
    let recent_scan: Option<(OffsetDateTime,)> = sqlx::query_as(
        &format!("SELECT queued_at FROM scan_jobs WHERE customer_id = $1 AND queued_at > NOW() - INTERVAL '{}' ORDER BY queued_at DESC LIMIT 1", rate_limit_interval)
    )
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;

    if recent_scan.is_some() {
        match tier {
            SubscriptionTier::Free => {
                let msg = if cfg!(debug_assertions) {
                    "Free accounts can run one scan per minute (dev mode)"
                } else {
                    "Free accounts can run one scan per 24 hours"
                };
                return Err(anyhow!(msg))
            }
            _ => {} // Paid tiers allow multiple scans per day
        }
    }

    let id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let job = sqlx::query_as::<_, ScanJob>(
        r#"
        INSERT INTO scan_jobs (id, customer_id, status, sources_queued, sources_completed,
                               sources_failed, exposures_found, queued_at)
        VALUES ($1, $2, 'queued', 0, 0, 0, 0, $3)
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(customer_id)
    .bind(now)
    .fetch_one(pool)
    .await?;

    // Dispatch to Python agent orchestrator via Redis queue
    let payload = ScanJobPayload {
        job_id: id,
        customer_id,
        identity_token,
        tier: format!("{:?}", tier).to_lowercase(),
    };
    let payload_json = serde_json::to_string(&payload)?;

    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| anyhow!("Redis connection failed: {e}"))?;

    conn.lpush::<_, _, ()>(SCAN_QUEUE_KEY, &payload_json)
        .await
        .map_err(|e| anyhow!("Failed to enqueue scan job: {e}"))?;

    tracing::info!(
        job_id = %id,
        customer_id = %customer_id,
        "Scan job queued"
    );

    Ok(job)
}

/// Fetch a scan job by ID, verifying it belongs to the requesting customer.
pub async fn get_scan_job(
    pool: &PgPool,
    job_id: Uuid,
    customer_id: Uuid,
) -> Result<Option<ScanJob>> {
    Ok(sqlx::query_as::<_, ScanJob>(
        "SELECT * FROM scan_jobs WHERE id = $1 AND customer_id = $2",
    )
    .bind(job_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await?)
}

/// List recent scan jobs for a customer (most recent first).
pub async fn list_scan_jobs(pool: &PgPool, customer_id: Uuid, limit: i64) -> Result<Vec<ScanJob>> {
    Ok(sqlx::query_as::<_, ScanJob>(
        "SELECT * FROM scan_jobs WHERE customer_id = $1 ORDER BY queued_at DESC LIMIT $2",
    )
    .bind(customer_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// Update a scan job's status — called by the agent callback endpoint.
pub async fn update_scan_status(
    pool: &PgPool,
    job_id: Uuid,
    status: ScanJobStatus,
    sources_completed: Option<i32>,
    sources_failed: Option<i32>,
    exposures_found: Option<i32>,
    error_message: Option<String>,
) -> Result<()> {
    let completed_at = matches!(status, ScanJobStatus::Completed | ScanJobStatus::Failed)
        .then(OffsetDateTime::now_utc);

    sqlx::query(
        r#"
        UPDATE scan_jobs SET
            status = $1,
            sources_completed = COALESCE($2, sources_completed),
            sources_failed = COALESCE($3, sources_failed),
            exposures_found = COALESCE($4, exposures_found),
            error_message = $5,
            completed_at = COALESCE($6, completed_at),
            started_at = CASE WHEN $1 = 'running' AND started_at IS NULL THEN NOW() ELSE started_at END
        WHERE id = $7
        "#,
    )
    .bind(&status)
    .bind(sources_completed)
    .bind(sources_failed)
    .bind(exposures_found)
    .bind(error_message)
    .bind(completed_at)
    .bind(job_id)
    .execute(pool)
    .await?;

    Ok(())
}
