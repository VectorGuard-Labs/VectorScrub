use anyhow::Result;
use sqlx::PgPool;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::crypto::MasterKey;
use crate::models::customer::Customer;
use crate::services::scan as scan_svc;

/// Scheduled rescan dispatcher — continuously checks for customers due for rescans.
/// Rescan frequency by tier (requires `monitoring_enabled`):
/// - Personal: monthly
/// - Premium: weekly
/// - Free: no automatic rescans
pub async fn run_scheduled_rescans(
    pool: PgPool,
    redis: redis::Client,
    master_key_b64: String,
) {
    info!("Scheduled rescan dispatcher starting");

    loop {
        if let Err(e) = dispatch_due_rescans(&pool, &redis, &master_key_b64).await {
            error!(error = %e, "Failed to dispatch scheduled rescans");
        }

        // Check every 6 hours
        sleep(Duration::from_secs(6 * 60 * 60)).await;
    }
}

/// Query customers due for rescans and initiate scan jobs.
async fn dispatch_due_rescans(
    pool: &PgPool,
    redis: &redis::Client,
    master_key_b64: &str,
) -> Result<()> {
    // Query customers with monitoring enabled and due for rescan
    // Personal: monthly (30 days), Premium: weekly (7 days)
    let customers: Vec<Customer> = sqlx::query_as::<_, Customer>(
        r#"
        SELECT * FROM customers
        WHERE monitoring_enabled = true
        AND is_active = true
        AND EXISTS (SELECT 1 FROM identity_profiles ip WHERE ip.customer_id = customers.id)
        AND subscription_tier IN ('personal', 'premium')
        AND (
            (subscription_tier = 'personal' AND (last_scan_at IS NULL OR last_scan_at < NOW() - INTERVAL '30 days'))
            OR
            (subscription_tier IN ('premium') AND (last_scan_at IS NULL OR last_scan_at < NOW() - INTERVAL '7 days'))
        )
        "#
    )
    .fetch_all(pool)
    .await?;

    if customers.is_empty() {
        info!("No customers due for scheduled rescan");
        return Ok(());
    }

    info!(count = customers.len(), "Customers due for scheduled rescan");

    for customer in customers {
        match initiate_scheduled_scan(pool, redis, master_key_b64, &customer).await {
            Ok(job_id) => {
                info!(
                    customer_id = %customer.id,
                    job_id = %job_id,
                    tier = ?customer.subscription_tier,
                    "Scheduled rescan initiated"
                );
            }
            Err(e) => {
                warn!(
                    customer_id = %customer.id,
                    error = %e,
                    "Failed to initiate scheduled rescan"
                );
            }
        }
    }

    Ok(())
}

/// Initiate a scan for a single customer as part of scheduled monitoring. Goes through the
/// same path as a customer-initiated scan, so the job gets its own scoped identity token
/// and is covered by recovery.
async fn initiate_scheduled_scan(
    pool: &PgPool,
    redis: &redis::Client,
    master_key_b64: &str,
    customer: &Customer,
) -> Result<Uuid> {
    let master = MasterKey::from_base64(master_key_b64)?;
    let job = scan_svc::initiate_scan(pool, redis, &master, customer.id, &customer.subscription_tier).await?;

    sqlx::query("UPDATE customers SET last_scan_at = NOW() WHERE id = $1")
        .bind(customer.id)
        .execute(pool)
        .await?;

    Ok(job.id)
}
