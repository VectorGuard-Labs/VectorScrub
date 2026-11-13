//! The hand-worked help queue behind `vectorscrub assistance ...` (see main.rs).
//!
//! Deliberately small: customers ask (removal::request_assistance), a person reads the queue,
//! writes to the customer at their account email, and records how many minutes it took so the
//! pilot can learn what a manual removal costs. Nothing here reads or releases identity details:
//! the person helps the customer do it, with the customer's own screen and inbox.

use anyhow::{bail, Result};
use sqlx::PgPool;
use uuid::Uuid;

/// One open item, with nothing identifying: the broker and how long it has waited.
#[derive(Debug, sqlx::FromRow)]
pub struct QueueItem {
    pub id: Uuid,
    pub source_name: String,
    pub assistance_status: String,
    pub waiting_hours: i64,
    pub failure_reason: Option<String>,
}

pub async fn open_items(pool: &PgPool) -> Result<Vec<QueueItem>> {
    Ok(sqlx::query_as(
        "SELECT r.id, r.source_name, r.assistance_status, \
                (EXTRACT(EPOCH FROM (NOW() - r.assistance_requested_at)) / 3600)::bigint AS waiting_hours, \
                (SELECT left(a.details->>'error', 160) FROM removal_audit_log a \
                  WHERE a.removal_request_id = r.id AND a.action = 'failed' \
                  ORDER BY a.recorded_at DESC LIMIT 1) AS failure_reason \
         FROM removal_requests r \
         WHERE r.assistance_status IN ('requested', 'in_progress') \
         ORDER BY r.assistance_requested_at",
    )
    .fetch_all(pool)
    .await?)
}

/// The account email of the customer who asked, so a person can write to them. Reading it is
/// recorded in the audit log.
pub async fn contact_email(pool: &PgPool, request_id: Uuid) -> Result<String> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT c.id, c.email FROM removal_requests r JOIN customers c ON c.id = r.customer_id \
         WHERE r.id = $1 AND r.assistance_status IN ('requested', 'in_progress')",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await?;
    let Some((customer_id, email)) = row else { bail!("no open help request with that id") };
    super::audit::AuditEntry::builder(super::audit::actions::ASSISTANCE_CONTACT_READ, super::audit::resources::CUSTOMER)
        .customer_id(customer_id)
        .resource_id(request_id)
        .log_or_warn(pool)
        .await;
    Ok(email)
}

/// Move an open item along. `minutes` is the time spent, recorded when finishing.
pub async fn set_status(pool: &PgPool, request_id: Uuid, status: &str, minutes: Option<i32>) -> Result<()> {
    if !matches!(status, "in_progress" | "completed" | "declined") {
        bail!("status must be in_progress, completed or declined");
    }
    let done = sqlx::query(
        "UPDATE removal_requests \
         SET assistance_status = $2, assistance_minutes = COALESCE($3, assistance_minutes), updated_at = NOW() \
         WHERE id = $1 AND assistance_status IN ('requested', 'in_progress')",
    )
    .bind(request_id)
    .bind(status)
    .bind(minutes)
    .execute(pool)
    .await?;
    if done.rows_affected() != 1 {
        bail!("no open help request with that id");
    }
    Ok(())
}

/// What the pilot has cost so far: finished items and their average minutes.
pub async fn summary(pool: &PgPool) -> Result<(i64, Option<f64>)> {
    Ok(sqlx::query_as(
        "SELECT count(*), avg(assistance_minutes)::float8 FROM removal_requests \
         WHERE assistance_status = 'completed' AND assistance_minutes IS NOT NULL",
    )
    .fetch_one(pool)
    .await?)
}
