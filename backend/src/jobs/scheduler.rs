use sqlx::PgPool;
use tracing::info;

/// Placeholder for the scheduled rescan dispatcher.
/// In production this would run as a separate Tokio task, wake on a cron schedule,
/// and re-queue scans for active paid customers whose last scan is older than N days.
pub async fn run_scheduled_rescans(pool: &PgPool) {
    info!("Scheduled rescan dispatcher starting");
    // TODO: query customers due for rescan and call scan_svc::initiate_scan
    let _ = pool;
}
