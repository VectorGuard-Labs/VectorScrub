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

    // `vectorscrub erase-account <email>`: erase an account on the customer's written request.
    // The operator must first confirm the request really comes from the account's owner (for
    // example by writing back to the account's own address). Same erasure as the self-service
    // button; it also refuses while a subscription would keep billing.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("erase-account") {
        return erase_account_cli(args.get(2).map(String::as_str)).await;
    }

    if args.get(1).map(String::as_str) == Some("pilot") {
        return pilot_cli(&args[2..]).await;
    }
    if args.get(1).map(String::as_str) == Some("assistance") {
        return assistance_cli(&args[2..]).await;
    }

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

    // The catalog of deletion-request channels is data. A malformed or evidence-free entry must
    // stop the deploy here, not fail when the first customer uses it.
    let channels = services::broker_channels::check_embedded()
        .map_err(|e| anyhow::anyhow!(e))
        .context("Invalid deletion-request channel catalog")?;
    info!(channels, "Deletion-request channel catalog is valid");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations applied successfully");

    // Seal any listing links stored before they were encrypted (idempotent; a no-op once done).
    // Not fatal: unsealed rows still read correctly, but a failure is loud so it gets fixed.
    match crypto::MasterKey::from_base64(&config.master_key_b64) {
        Ok(master) => match services::url_vault::backfill(&pool, &master).await {
            Ok(0) => {}
            Ok(n) => info!(sealed = n, "Encrypted legacy listing links"),
            Err(e) => tracing::error!(error = %e, "Could not encrypt legacy listing links; they remain readable but unencrypted"),
        },
        Err(e) => tracing::error!(error = %e, "Master key unusable; listing links not encrypted"),
    }

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

async fn erase_account_cli(email: Option<&str>) -> anyhow::Result<()> {
    use services::{account_deletion, audit};
    let email = email.context("usage: vectorscrub erase-account <email>")?;
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await?;
    let id: Option<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM customers WHERE lower(email) = lower($1)")
        .bind(email.trim())
        .fetch_optional(&pool)
        .await?;
    let id = id.context("no account with that email")?;
    account_deletion::delete_account(&pool, id).await?;
    audit::AuditEntry::builder(audit::actions::ACCOUNT_DELETE, audit::resources::CUSTOMER)
        .resource_id(id)
        .details(serde_json::json!({ "by": "operator" }))
        .log_or_warn(&pool)
        .await;
    println!("Account erased.");
    Ok(())
}

/// `vectorscrub assistance list | contact <id> | start <id> | finish <id> <minutes> | decline <id> | summary`
/// The pilot's hand-worked help queue. `list` shows no personal details; `contact` prints the
/// customer's account email (and is audited) so a person can write to them.
async fn assistance_cli(args: &[String]) -> anyhow::Result<()> {
    use services::assistance;
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await?;
    let id = |i: usize| -> anyhow::Result<uuid::Uuid> {
        args.get(i).context("missing request id")?.parse().context("not a valid request id")
    };
    match args.first().map(String::as_str) {
        Some("list") => {
            let items = assistance::open_items(&pool).await?;
            if items.is_empty() {
                println!("No open help requests.");
            }
            for i in items {
                println!(
                    "{}  {:<22} {:<12} waiting {}h  {}",
                    i.id, i.source_name, i.assistance_status, i.waiting_hours,
                    i.failure_reason.unwrap_or_default()
                );
            }
        }
        Some("contact") => println!("{}", assistance::contact_email(&pool, id(1)?).await?),
        Some("start") => assistance::set_status(&pool, id(1)?, "in_progress", None).await?,
        Some("decline") => assistance::set_status(&pool, id(1)?, "declined", None).await?,
        Some("finish") => {
            let minutes: i32 = args.get(2).context("usage: finish <id> <minutes>")?.parse().context("minutes must be a number")?;
            assistance::set_status(&pool, id(1)?, "completed", Some(minutes)).await?;
        }
        Some("summary") => {
            let (n, avg) = assistance::summary(&pool).await?;
            println!("{n} finished; average {} minutes", avg.map(|a| format!("{a:.1}")).unwrap_or_else(|| "n/a".into()));
        }
        _ => bail_usage()?,
    }
    Ok(())
}

fn bail_usage() -> anyhow::Result<()> {
    anyhow::bail!("usage: vectorscrub assistance list | contact <id> | start <id> | finish <id> <minutes> | decline <id> | summary")
}

/// `vectorscrub pilot scorecard | cohort <email> <a|b|c> | log-time <email> <kind> <minutes> | case <email>`
/// Operator tools for running the pilot. None of them print identity details, listing links or emails.
async fn pilot_cli(args: &[String]) -> anyhow::Result<()> {
    use services::pilot;
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await?;
    let arg = |i: usize, what: &str| -> anyhow::Result<String> {
        args.get(i).cloned().with_context(|| format!("missing {what}"))
    };
    match args.first().map(String::as_str) {
        Some("scorecard") => print!("{}", pilot::render_scorecard(&pilot::scorecard(&pool).await?)),
        Some("cohort") => {
            let id = pilot::customer_by_email(&pool, &arg(1, "email")?).await?;
            pilot::set_cohort(&pool, id, &arg(2, "cohort (a, b or c)")?).await?;
            println!("Tagged.");
        }
        Some("log-time") => {
            let id = pilot::customer_by_email(&pool, &arg(1, "email")?).await?;
            let minutes: i32 = arg(3, "minutes")?.parse().context("minutes must be a number")?;
            pilot::log_time(&pool, id, &arg(2, "kind")?, minutes).await?;
            println!("Logged.");
        }
        Some("case") => {
            let id = pilot::customer_by_email(&pool, &arg(1, "email")?).await?;
            print!("{}", pilot::case_view(&pool, id).await?);
        }
        _ => anyhow::bail!(
            "usage: vectorscrub pilot scorecard | cohort <email> <a|b|c> | log-time <email> <{}> <minutes> | case <email>",
            pilot::TIME_KINDS.join("|")
        ),
    }
    Ok(())
}
