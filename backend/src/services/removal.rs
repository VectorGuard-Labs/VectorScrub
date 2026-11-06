use crate::models::removal_request::{RemovalAuditEntry, RemovalRequest, RemovalStatus};
use anyhow::{Context, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// Creates a removal request for a specific exposure with status=authorized.
/// Also creates the first audit log entry.
/// Returns error if removal already authorized for this exposure.
pub async fn authorize_removal(
    pool: &PgPool,
    customer_id: Uuid,
    exposure_id: Uuid,
) -> Result<RemovalRequest> {
    let mut tx = pool.begin().await.context("begin transaction")?;

    // Check if removal already exists for this exposure
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM removal_requests WHERE exposure_id = $1 AND customer_id = $2"
    )
    .bind(exposure_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("check existing removal")?;

    if existing.is_some() {
        anyhow::bail!("Removal already authorized for this exposure");
    }

    // Get exposure details for denormalization
    let (source_name, source_url): (String, String) = sqlx::query_as(
        "SELECT source_name, source_url FROM exposures WHERE id = $1"
    )
    .bind(exposure_id)
    .fetch_one(&mut *tx)
    .await
    .context("fetch exposure details")?;

    // Create removal request
    let request_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let request: RemovalRequest = sqlx::query_as(
        r#"
        INSERT INTO removal_requests (
            id, customer_id, exposure_id, source_name, source_url, status, customer_authorized_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
        RETURNING *
        "#,
    )
    .bind(request_id)
    .bind(customer_id)
    .bind(exposure_id)
    .bind(source_name)
    .bind(source_url)
    .bind(RemovalStatus::Authorized)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .context("insert removal request")?;

    // Create initial audit log entry
    sqlx::query(
        r#"
        INSERT INTO removal_audit_log (
            id, removal_request_id, customer_id, action, actor, details, recorded_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(customer_id)
    .bind("authorized")
    .bind("customer")
    .bind(serde_json::json!({}))
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("insert audit log entry")?;

    tx.commit().await.context("commit transaction")?;

    Ok(request)
}

/// Returns all removal requests for a customer, ordered by created_at DESC.
pub async fn list_removal_requests(
    pool: &PgPool,
    customer_id: Uuid,
) -> Result<Vec<RemovalRequest>> {
    let requests = sqlx::query_as(
        r#"
        SELECT * FROM removal_requests
        WHERE customer_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await
    .context("list removal requests")?;

    Ok(requests)
}

/// Returns all removal requests with a specific status.
/// Used by agents to poll for pending work.
pub async fn list_removal_requests_by_status(
    pool: &PgPool,
    status: RemovalStatus,
) -> Result<Vec<RemovalRequest>> {
    let requests = sqlx::query_as(
        r#"
        SELECT * FROM removal_requests
        WHERE status = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(status)
    .fetch_all(pool)
    .await
    .context("list removal requests by status")?;

    Ok(requests)
}

/// Returns a single removal request if it belongs to the specified customer.
pub async fn get_removal_request(
    pool: &PgPool,
    request_id: Uuid,
    customer_id: Uuid,
) -> Result<Option<RemovalRequest>> {
    let request = sqlx::query_as(
        "SELECT * FROM removal_requests WHERE id = $1 AND customer_id = $2",
    )
    .bind(request_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await
    .context("get removal request")?;

    Ok(request)
}

/// Updates removal request status and related metadata.
/// Sets submitted_at when status=Submitted, verified_at when status=VerifiedRemoved.
pub async fn update_removal_status(
    pool: &PgPool,
    request_id: Uuid,
    new_status: RemovalStatus,
    confirmation_ref: Option<String>,
    evidence_hash: Option<String>,
    next_check_at: Option<OffsetDateTime>,
) -> Result<RemovalRequest> {
    let now = OffsetDateTime::now_utc();

    let request: RemovalRequest = sqlx::query_as(
        r#"
        UPDATE removal_requests
        SET
            status = $2,
            confirmation_ref = COALESCE($3, confirmation_ref),
            evidence_hash = COALESCE($4, evidence_hash),
            next_check_at = COALESCE($5, next_check_at),
            submitted_at = CASE WHEN $2 = 'submitted' AND submitted_at IS NULL THEN $6 ELSE submitted_at END,
            verified_at = CASE WHEN $2 = 'verified_removed' AND verified_at IS NULL THEN $6 ELSE verified_at END,
            updated_at = $6
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(request_id)
    .bind(&new_status)
    .bind(confirmation_ref)
    .bind(evidence_hash)
    .bind(next_check_at)
    .bind(now)
    .fetch_one(pool)
    .await
    .context("update removal status")?;

    // If verified removed, mark the exposure as inactive
    if new_status == RemovalStatus::VerifiedRemoved {
        let exposure_id = request.exposure_id;
        sqlx::query(
            r#"
            UPDATE exposures
            SET is_active = false, updated_at = $2
            WHERE id = $1
            "#,
        )
        .bind(exposure_id)
        .bind(now)
        .execute(pool)
        .await
        .context("mark exposure inactive")?;
    }

    Ok(request)
}

/// Appends an entry to the removal audit log.
/// The audit log is immutable (enforced by DB rules).
pub async fn record_audit_entry(
    pool: &PgPool,
    removal_request_id: Uuid,
    customer_id: Uuid,
    action: String,
    actor: String,
    details: serde_json::Value,
) -> Result<RemovalAuditEntry> {
    let entry_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let entry: RemovalAuditEntry = sqlx::query_as(
        r#"
        INSERT INTO removal_audit_log (
            id, removal_request_id, customer_id, action, actor, details, recorded_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(entry_id)
    .bind(removal_request_id)
    .bind(customer_id)
    .bind(action)
    .bind(actor)
    .bind(details)
    .bind(now)
    .fetch_one(pool)
    .await
    .context("insert audit entry")?;

    Ok(entry)
}

/// Returns the full audit trail for a removal request.
pub async fn get_removal_audit_log(
    pool: &PgPool,
    removal_request_id: Uuid,
) -> Result<Vec<RemovalAuditEntry>> {
    let entries = sqlx::query_as(
        r#"
        SELECT * FROM removal_audit_log
        WHERE removal_request_id = $1
        ORDER BY recorded_at ASC
        "#,
    )
    .bind(removal_request_id)
    .fetch_all(pool)
    .await
    .context("get audit log")?;

    Ok(entries)
}
