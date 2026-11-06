use anyhow::Result;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::exposure::{Exposure, ExposureSeverity, ExposureSummary, PiiCategory};

/// Insert a new exposure discovered by the detection agent.
pub async fn record_exposure(
    pool: &PgPool,
    customer_id: Uuid,
    scan_job_id: Uuid,
    source_name: &str,
    source_url: &str,
    pii_category: PiiCategory,
    severity: ExposureSeverity,
    pii_summary: &str,
    confidence: f32,
) -> Result<Exposure> {
    let id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let exposure = sqlx::query_as::<_, Exposure>(
        r#"
        INSERT INTO exposures (id, customer_id, scan_job_id, source_name, source_url,
            pii_category, severity, pii_summary, confidence, is_active,
            discovered_at, last_verified_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,true,$10,$10)
        ON CONFLICT (customer_id, source_name, pii_category) DO UPDATE SET
            is_active = true,
            confidence = EXCLUDED.confidence,
            pii_summary = EXCLUDED.pii_summary,
            last_verified_at = EXCLUDED.last_verified_at
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(customer_id)
    .bind(scan_job_id)
    .bind(source_name)
    .bind(source_url)
    .bind(&pii_category)
    .bind(&severity)
    .bind(pii_summary)
    .bind(confidence)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(exposure)
}

/// Retrieve a single exposure by ID.
pub async fn get_exposure(pool: &PgPool, exposure_id: Uuid) -> Result<Option<Exposure>> {
    Ok(sqlx::query_as::<_, Exposure>("SELECT * FROM exposures WHERE id = $1")
        .bind(exposure_id)
        .fetch_optional(pool)
        .await?)
}

/// Retrieve all active exposures for a customer.
pub async fn list_exposures(pool: &PgPool, customer_id: Uuid) -> Result<Vec<Exposure>> {
    Ok(sqlx::query_as::<_, Exposure>(
        "SELECT * FROM exposures WHERE customer_id = $1 ORDER BY severity DESC, discovered_at DESC",
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await?)
}

/// Compute and return an exposure summary with privacy score calculation.
/// Privacy score: 100 points minus weighted deductions per exposure.
pub async fn get_exposure_summary(pool: &PgPool, customer_id: Uuid) -> Result<ExposureSummary> {
    let exposures = list_exposures(pool, customer_id).await?;

    let total = exposures.len() as i64;
    let active: Vec<&Exposure> = exposures.iter().filter(|e| e.is_active).collect();
    let removed = total - active.len() as i64;

    let critical_count = active.iter().filter(|e| e.severity == ExposureSeverity::Critical).count() as i64;
    let high_count = active.iter().filter(|e| e.severity == ExposureSeverity::High).count() as i64;
    let medium_count = active.iter().filter(|e| e.severity == ExposureSeverity::Medium).count() as i64;
    let low_count = active.iter().filter(|e| e.severity == ExposureSeverity::Low).count() as i64;

    // Weighted deductions: critical=25, high=10, medium=5, low=2 (floor at 0)
    let deduction = (critical_count * 25 + high_count * 10 + medium_count * 5 + low_count * 2)
        .min(100) as f32;
    let privacy_score = (100.0_f32 - deduction).max(0.0);

    let sources_affected: Vec<String> = {
        let mut sources: Vec<String> = active.iter().map(|e| e.source_name.clone()).collect();
        sources.sort();
        sources.dedup();
        sources
    };

    let last_scan_at = sqlx::query_scalar::<_, Option<OffsetDateTime>>(
        "SELECT MAX(queued_at) FROM scan_jobs WHERE customer_id = $1 AND status = 'completed'",
    )
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok(ExposureSummary {
        customer_id,
        total_exposures: total,
        active_exposures: active.len() as i64,
        removed_exposures: removed,
        critical_count,
        high_count,
        medium_count,
        low_count,
        privacy_score,
        sources_affected,
        last_scan_at,
    })
}

/// Mark an exposure as removed (called when removal is verified).
/// Phase 2 — wired up when removal workflow is complete.
#[allow(dead_code)]
pub async fn mark_removed(pool: &PgPool, exposure_id: Uuid, customer_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE exposures SET is_active = false, removed_at = $1 WHERE id = $2 AND customer_id = $3"
    )
    .bind(OffsetDateTime::now_utc())
    .bind(exposure_id)
    .bind(customer_id)
    .execute(pool)
    .await?;
    Ok(())
}
