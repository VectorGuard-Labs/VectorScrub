use sqlx::PgPool;
use tracing::info;

/// Placeholder for the scheduled rescan dispatcher.
/// Phase 3 — wired up when continuous monitoring is implemented.
#[allow(dead_code)]
pub async fn run_scheduled_rescans(pool: &PgPool) {
    info!("Scheduled rescan dispatcher starting");
    // TODO: query customers due for rescan and call scan_svc::initiate_scan
    let _ = pool;
}
