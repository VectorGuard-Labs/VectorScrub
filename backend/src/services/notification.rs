use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::notification::{NotificationPreferences, UpdateNotificationPreferencesRequest};

/// Get or create notification preferences for a customer.
pub async fn get_preferences(pool: &PgPool, customer_id: Uuid) -> Result<NotificationPreferences> {
    // Try to get existing preferences
    let existing: Option<NotificationPreferences> = sqlx::query_as(
        "SELECT * FROM notification_preferences WHERE customer_id = $1"
    )
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;

    if let Some(prefs) = existing {
        return Ok(prefs);
    }

    // Create default preferences
    let prefs = sqlx::query_as::<_, NotificationPreferences>(
        r#"
        INSERT INTO notification_preferences
        (customer_id, email_enabled, sms_enabled, alert_new_exposures, alert_removal_failures, alert_reappearances)
        VALUES ($1, true, false, true, true, true)
        RETURNING *
        "#
    )
    .bind(customer_id)
    .fetch_one(pool)
    .await?;

    Ok(prefs)
}

/// Update notification preferences for a customer.
pub async fn update_preferences(
    pool: &PgPool,
    customer_id: Uuid,
    req: UpdateNotificationPreferencesRequest,
) -> Result<NotificationPreferences> {
    // Ensure preferences exist
    get_preferences(pool, customer_id).await?;

    // Build dynamic update query
    let prefs = sqlx::query_as::<_, NotificationPreferences>(
        r#"
        UPDATE notification_preferences SET
            email_enabled = COALESCE($2, email_enabled),
            sms_enabled = COALESCE($3, sms_enabled),
            phone_number = COALESCE($4, phone_number),
            alert_new_exposures = COALESCE($5, alert_new_exposures),
            alert_removal_failures = COALESCE($6, alert_removal_failures),
            alert_reappearances = COALESCE($7, alert_reappearances),
            updated_at = NOW()
        WHERE customer_id = $1
        RETURNING *
        "#
    )
    .bind(customer_id)
    .bind(req.email_enabled)
    .bind(req.sms_enabled)
    .bind(req.phone_number.as_deref())
    .bind(req.alert_new_exposures)
    .bind(req.alert_removal_failures)
    .bind(req.alert_reappearances)
    .fetch_one(pool)
    .await?;

    tracing::info!(customer_id = %customer_id, "Notification preferences updated");

    Ok(prefs)
}

/// Send an alert notification to a customer based on their preferences.
/// This queues the alert to the alerts table and returns immediately.
/// Actual email/SMS sending happens asynchronously.
pub async fn send_alert(
    pool: &PgPool,
    customer_id: Uuid,
    alert_type: &str,
    title: &str,
    message: &str,
) -> Result<()> {
    // Check if customer has this alert type enabled
    let prefs = get_preferences(pool, customer_id).await?;

    let should_send = match alert_type {
        "new_exposure" => prefs.alert_new_exposures,
        "removal_failed" => prefs.alert_removal_failures,
        "reappearance" => prefs.alert_reappearances,
        _ => true, // Unknown types always send
    };

    if !should_send {
        tracing::debug!(
            customer_id = %customer_id,
            alert_type = alert_type,
            "Alert skipped due to preferences"
        );
        return Ok(());
    }

    // Insert into alerts table
    sqlx::query(
        r#"
        INSERT INTO alerts (customer_id, alert_type, title, message, is_read, created_at)
        VALUES ($1, $2, $3, $4, false, NOW())
        "#
    )
    .bind(customer_id)
    .bind(alert_type)
    .bind(title)
    .bind(message)
    .execute(pool)
    .await?;

    tracing::info!(
        customer_id = %customer_id,
        alert_type = alert_type,
        "Alert queued"
    );

    // TODO: Queue background job to send email/SMS
    // For now, alerts are stored in DB and can be retrieved via API
    // Future: Use a job queue (Redis, Postgres LISTEN/NOTIFY, etc.)

    Ok(())
}

/// Send a new exposure alert.
pub async fn alert_new_exposure(
    pool: &PgPool,
    customer_id: Uuid,
    source_name: &str,
    severity: &str,
) -> Result<()> {
    let title = format!("New {} exposure found", severity);
    let message = format!(
        "VectorScrub found new {} severity personal information on {}. Review and authorize removal from your dashboard.",
        severity, source_name
    );

    send_alert(pool, customer_id, "new_exposure", &title, &message).await
}

/// Send a removal failure alert.
pub async fn alert_removal_failed(
    pool: &PgPool,
    customer_id: Uuid,
    source_name: &str,
    error: &str,
) -> Result<()> {
    let title = format!("Removal request failed: {}", source_name);
    let message = format!(
        "Your removal request to {} could not be completed. Error: {}. Our team will investigate.",
        source_name, error
    );

    send_alert(pool, customer_id, "removal_failed", &title, &message).await
}

/// Send a reappearance alert.
pub async fn alert_reappearance(
    pool: &PgPool,
    customer_id: Uuid,
    source_name: &str,
) -> Result<()> {
    let title = format!("Information reappeared: {}", source_name);
    let message = format!(
        "Previously removed information has reappeared on {}. A new removal request has been prepared for your review.",
        source_name
    );

    send_alert(pool, customer_id, "reappearance", &title, &message).await
}

/// Get unread alerts for a customer.
pub async fn get_unread_alerts(pool: &PgPool, customer_id: Uuid, limit: i64) -> Result<Vec<crate::models::alert::Alert>> {
    let alerts = sqlx::query_as(
        r#"
        SELECT * FROM alerts
        WHERE customer_id = $1 AND is_read = false
        ORDER BY created_at DESC
        LIMIT $2
        "#
    )
    .bind(customer_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(alerts)
}

/// Mark an alert as read.
pub async fn mark_alert_read(pool: &PgPool, alert_id: Uuid, customer_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE alerts SET is_read = true WHERE id = $1 AND customer_id = $2"
    )
    .bind(alert_id)
    .bind(customer_id)
    .execute(pool)
    .await?;

    Ok(())
}
