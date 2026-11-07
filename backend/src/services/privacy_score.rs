use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::privacy_score::{PrivacyScoreHistory, PrivacyScoreTrend};

/// Record a privacy score snapshot for a customer.
/// Called after scan completion or exposure changes.
pub async fn record_snapshot(
    pool: &PgPool,
    customer_id: Uuid,
    score: f32,
    critical_count: i32,
    high_count: i32,
    medium_count: i32,
    low_count: i32,
) -> Result<PrivacyScoreHistory> {
    let total_exposures = critical_count + high_count + medium_count + low_count;

    let snapshot = sqlx::query_as::<_, PrivacyScoreHistory>(
        r#"
        INSERT INTO privacy_score_history
        (customer_id, score, critical_count, high_count, medium_count, low_count, total_exposures, recorded_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        RETURNING *
        "#,
    )
    .bind(customer_id)
    .bind(score)
    .bind(critical_count)
    .bind(high_count)
    .bind(medium_count)
    .bind(low_count)
    .bind(total_exposures)
    .fetch_one(pool)
    .await?;

    tracing::debug!(
        customer_id = %customer_id,
        score = score,
        total = total_exposures,
        "Privacy score snapshot recorded"
    );

    Ok(snapshot)
}

/// Retrieve privacy score history for a customer over a given time period.
pub async fn get_history(
    pool: &PgPool,
    customer_id: Uuid,
    days: i32,
) -> Result<Vec<PrivacyScoreHistory>> {
    let history = sqlx::query_as::<_, PrivacyScoreHistory>(
        r#"
        SELECT * FROM privacy_score_history
        WHERE customer_id = $1
        AND recorded_at > NOW() - ($2 || ' days')::INTERVAL
        ORDER BY recorded_at ASC
        "#,
    )
    .bind(customer_id)
    .bind(days)
    .fetch_all(pool)
    .await?;

    Ok(history)
}

/// Get privacy score trend with delta calculations.
pub async fn get_trend(
    pool: &PgPool,
    customer_id: Uuid,
    days: i32,
) -> Result<PrivacyScoreTrend> {
    let history = get_history(pool, customer_id, days).await?;

    // Get current score from customer record
    let current_score: Option<f32> = sqlx::query_scalar(
        "SELECT privacy_score FROM customers WHERE id = $1"
    )
    .bind(customer_id)
    .fetch_optional(pool)
    .await?
    .flatten();

    let current_score = current_score.unwrap_or(100.0);

    // Calculate delta from earliest to latest
    let (previous_score, score_delta) = if history.len() >= 2 {
        let first = history.first().unwrap().score;
        let delta = current_score - first;
        (Some(first), Some(delta))
    } else {
        (None, None)
    };

    Ok(PrivacyScoreTrend {
        current_score,
        previous_score,
        score_delta,
        history,
    })
}

/// Clean up old privacy score history records (retention policy: 1 year).
pub async fn cleanup_old_records(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query(
        "DELETE FROM privacy_score_history WHERE recorded_at < NOW() - INTERVAL '1 year'"
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
