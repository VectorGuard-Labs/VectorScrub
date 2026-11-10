use anyhow::Context;
use axum::Router;
use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod api;
mod crypto;
mod db;
mod docs;
mod jobs;
mod middleware;
mod models;
mod observability;
mod services;
#[cfg(test)]
mod test_support;

pub use db::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    let config = services::config::AppConfig::from_env()
        .context("Failed to load application configuration")?;

    // Initialize observability (tracing, metrics, and optional OTLP)
    let metrics_handle = observability::init_observability(
        "vectorscrub-backend",
        config.otlp_endpoint.as_deref(),
    );

    // Database pool
    let pool = PgPoolOptions::new()
        .max_connections(config.db_pool_size)
        .connect(&config.database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations applied successfully");

    // Redis connection
    let redis_client = redis::Client::open(config.redis_url.as_str())
        .context("Failed to create Redis client")?;

    let state = AppState {
        pool: pool.clone(),
        redis: redis_client.clone(),
        config: config.clone(),
        metrics: metrics_handle,
    };

    // Build the password-hash timing-equalization value now, so the first login for an unknown
    // email does not pay for it (which would be a timing difference of its own).
    services::auth::warm_up().await;

    // Spawn scheduled rescan dispatcher as a background task
    let scheduler_pool = pool.clone();
    let scheduler_redis = redis_client.clone();
    let scheduler_master_key_b64 = config.master_key_b64.clone();
    tokio::spawn(async move {
        jobs::scheduler::run_scheduled_rescans(
            scheduler_pool,
            scheduler_redis,
            scheduler_master_key_b64,
        )
        .await;
    });

    // Repair scan jobs lost to a crashed worker or a failed enqueue (see jobs/recovery.rs)
    let recovery_pool = pool.clone();
    let recovery_redis = redis_client.clone();
    let recovery_master_key_b64 = config.master_key_b64.clone();
    let recovery_cfg = services::scan::RecoveryConfig {
        stale_after_secs: config.scan_stale_after_secs,
        redispatch_after_secs: config.scan_redispatch_after_secs,
        max_attempts: config.scan_max_attempts,
    };
    let recovery_interval = std::time::Duration::from_secs(config.scan_recovery_interval_secs.max(1));
    tokio::spawn(async move {
        jobs::recovery::run_scan_recovery(
            recovery_pool,
            recovery_redis,
            recovery_master_key_b64,
            recovery_cfg,
            recovery_interval,
        )
        .await;
    });

    let app = Router::new()
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", docs::ApiDoc::openapi()))
        .nest("/api/v1", api::router(state.clone()));
    let app = middleware::apply(app, &config);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;

    info!("VectorScrub API listening on {}", addr);

    // `ConnectInfo` makes the client's socket address available to the rate limiter and the
    // audit log.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutdown signal received — draining connections");
    observability::shutdown_observability();
}
