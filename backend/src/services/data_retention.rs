use anyhow::Result;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use tracing::{info, warn};

/// Data retention configuration
pub struct RetentionConfig {
    /// Inactive customer warning threshold (days)
    pub inactive_warning_days: i64,
    /// Inactive customer deletion threshold (days)
    pub inactive_deletion_days: i64,
    /// Removed exposure retention (days)
    pub removed_exposure_days: i64,
    /// Scan history retention (days)
    pub scan_history_days: i64,
    /// Application log retention (days)
    pub application_log_days: i64,
    /// Audit log retention (days)
    pub audit_log_days: i64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            inactive_warning_days: 730, // 2 years
            inactive_deletion_days: 760, // 2 years + 1 month
            removed_exposure_days: 90,
            scan_history_days: 365,
            application_log_days: 90,
            audit_log_days: 2555, // 7 years
        }
    }
}

pub struct DataRetentionService {
    pool: PgPool,
    config: RetentionConfig,
}

impl DataRetentionService {
    pub fn new(pool: PgPool, config: RetentionConfig) -> Self {
        Self { pool, config }
    }

    /// Run all retention cleanup tasks
    pub async fn run_cleanup(&self) -> Result<CleanupStats> {
        info!("Starting data retention cleanup");

        let stats = CleanupStats {
            inactive_customers_warned: self.warn_inactive_customers().await?,
            inactive_customers_deleted: self.delete_inactive_customers().await?,
            exposures_deleted: self.delete_old_exposures().await?,
            scans_deleted: self.delete_old_scans().await?,
            audit_logs_deleted: self.delete_old_audit_logs().await?,
        };

        info!(
            "Data retention cleanup complete: {:?} customers warned, {:?} customers deleted, \
             {:?} exposures deleted, {:?} scans deleted, {:?} audit logs deleted",
            stats.inactive_customers_warned,
            stats.inactive_customers_deleted,
            stats.exposures_deleted,
            stats.scans_deleted,
            stats.audit_logs_deleted,
        );

        // Record metrics
        metrics::counter!("data_retention_cleanup_runs_total").increment(1);
        metrics::gauge!("data_retention_customers_deleted_last_run")
            .set(stats.inactive_customers_deleted as f64);
        metrics::gauge!("data_retention_exposures_deleted_last_run")
            .set(stats.exposures_deleted as f64);

        Ok(stats)
    }

    /// Warn customers who have been inactive for warning threshold
    async fn warn_inactive_customers(&self) -> Result<u64> {
        let warning_date =
            OffsetDateTime::now_utc() - Duration::days(self.config.inactive_warning_days);

        let result = sqlx::query!(
            r#"
            UPDATE customers
            SET updated_at = NOW()
            WHERE last_scan_at < $1
              AND is_active = true
              AND warned_inactive = false
            RETURNING id
            "#,
            warning_date
        )
        .fetch_all(&self.pool)
        .await?;

        let count = result.len() as u64;

        if count > 0 {
            info!(
                "Marked {} inactive customers for warning (no activity since {})",
                count,
                warning_date.date()
            );

            // TODO: Send warning emails
            // for customer in result {
            //     send_inactive_warning_email(customer.id).await?;
            // }
        }

        Ok(count)
    }

    /// Soft delete customers who have been inactive beyond deletion threshold
    async fn delete_inactive_customers(&self) -> Result<u64> {
        let deletion_date =
            OffsetDateTime::now_utc() - Duration::days(self.config.inactive_deletion_days);

        let result = sqlx::query!(
            r#"
            UPDATE customers
            SET is_active = false,
                updated_at = NOW()
            WHERE last_scan_at < $1
              AND is_active = true
            RETURNING id
            "#,
            deletion_date
        )
        .fetch_all(&self.pool)
        .await?;

        let count = result.len() as u64;

        if count > 0 {
            warn!(
                "Soft deleted {} inactive customers (no activity since {})",
                count,
                deletion_date.date()
            );

            // Cascade delete related data
            for customer in result {
                self.delete_customer_data(customer.id).await?;
            }
        }

        Ok(count)
    }

    /// Delete all data for a customer (GDPR right to deletion)
    async fn delete_customer_data(&self, customer_id: uuid::Uuid) -> Result<()> {
        info!(customer_id = %customer_id, "Deleting all customer data");

        // Delete in order of dependencies
        sqlx::query!("DELETE FROM alerts WHERE customer_id = $1", customer_id)
            .execute(&self.pool)
            .await?;

        sqlx::query!(
            "DELETE FROM notification_preferences WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!(
            "DELETE FROM removal_audit_log WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!(
            "DELETE FROM removal_requests WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!(
            "DELETE FROM exposures WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!("DELETE FROM scan_jobs WHERE customer_id = $1", customer_id)
            .execute(&self.pool)
            .await?;

        sqlx::query!(
            "DELETE FROM privacy_score_history WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!(
            "DELETE FROM identity_profiles WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        // Note: Audit logs are preserved for compliance
        // They are pseudonymized by setting customer_id to NULL

        sqlx::query!(
            "UPDATE audit_log SET customer_id = NULL WHERE customer_id = $1",
            customer_id
        )
        .execute(&self.pool)
        .await?;

        // Finally delete customer record
        sqlx::query!("DELETE FROM customers WHERE id = $1", customer_id)
            .execute(&self.pool)
            .await?;

        info!(customer_id = %customer_id, "Successfully deleted all customer data");

        Ok(())
    }

    /// Delete old removed exposures
    async fn delete_old_exposures(&self) -> Result<u64> {
        let cutoff_date =
            OffsetDateTime::now_utc() - Duration::days(self.config.removed_exposure_days);

        let result = sqlx::query!(
            r#"
            DELETE FROM exposures
            WHERE is_active = false
              AND updated_at < $1
            "#,
            cutoff_date
        )
        .execute(&self.pool)
        .await?;

        let count = result.rows_affected();

        if count > 0 {
            info!(
                "Deleted {} old removed exposures (removed before {})",
                count,
                cutoff_date.date()
            );
        }

        Ok(count)
    }

    /// Delete old scan jobs
    async fn delete_old_scans(&self) -> Result<u64> {
        let cutoff_date =
            OffsetDateTime::now_utc() - Duration::days(self.config.scan_history_days);

        let result = sqlx::query!(
            r#"
            DELETE FROM scan_jobs
            WHERE created_at < $1
            "#,
            cutoff_date
        )
        .execute(&self.pool)
        .await?;

        let count = result.rows_affected();

        if count > 0 {
            info!(
                "Deleted {} old scan jobs (created before {})",
                count,
                cutoff_date.date()
            );
        }

        Ok(count)
    }

    /// Delete old audit logs (except required retention period)
    async fn delete_old_audit_logs(&self) -> Result<u64> {
        let cutoff_date =
            OffsetDateTime::now_utc() - Duration::days(self.config.audit_log_days);

        // Only delete logs that are not required for compliance
        // Keep all security-related logs for 7 years
        let result = sqlx::query!(
            r#"
            DELETE FROM audit_log
            WHERE timestamp < $1
              AND action NOT LIKE 'user.%'
              AND action NOT LIKE 'security.%'
            "#,
            cutoff_date
        )
        .execute(&self.pool)
        .await?;

        let count = result.rows_affected();

        if count > 0 {
            info!(
                "Deleted {} old audit logs (before {})",
                count,
                cutoff_date.date()
            );
        }

        Ok(count)
    }
}

/// Statistics from a cleanup run
#[derive(Debug, Clone)]
pub struct CleanupStats {
    pub inactive_customers_warned: u64,
    pub inactive_customers_deleted: u64,
    pub exposures_deleted: u64,
    pub scans_deleted: u64,
    pub audit_logs_deleted: u64,
}

/// Schedule periodic data retention cleanup
pub async fn schedule_retention_cleanup(
    pool: PgPool,
    config: RetentionConfig,
    interval_hours: u64,
) {
    let service = DataRetentionService::new(pool, config);

    info!(
        "Starting data retention scheduler (runs every {} hours)",
        interval_hours
    );

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        interval_hours * 3600,
    ));

    loop {
        interval.tick().await;

        match service.run_cleanup().await {
            Ok(stats) => {
                info!("Data retention cleanup completed: {:?}", stats);
            }
            Err(e) => {
                warn!("Data retention cleanup failed: {}", e);
                metrics::counter!("data_retention_cleanup_failures_total").increment(1);
            }
        }
    }
}
