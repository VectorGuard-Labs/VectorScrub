use anyhow::Context;
use axum::Router;
use dotenvy::dotenv;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod crypto;
mod db;
mod jobs;
mod middleware;
mod models;
mod services;

pub use db::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();

    // Structured logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vectorscrub=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let config = services::config::AppConfig::from_env()
        .context("Failed to load application configuration")?;

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
    };

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

    let app = Router::new()
        .nest("/api/v1", api::router(state.clone()));
    let app = middleware::apply(app, &config);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;

    info!("VectorScrub API listening on {}", addr);

    axum::serve(listener, app)
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
}
