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
    crypto::install_tls_provider();

    // `vectorscrub erase-account <email>`: erase an account on the customer's written request.
    // The operator must first confirm the request really comes from the account's owner (for
    // example by writing back to the account's own address). Same erasure as the self-service
    // button; it also refuses while a subscription would keep billing.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("erase-account") {
        return erase_account_cli(args.get(2).map(String::as_str)).await;
    }

    if args.get(1).map(String::as_str) == Some("kms") {
        return kms_cli(&args[2..]).await;
    }

    if args.get(1).map(String::as_str) == Some("verify-backup") {
        return verify_backup_cli(&args[2..]).await;
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

    // Where customers' data keys come from. With PyHSM, wait for it (it may start a moment after
    // this process) and check that the key-encryption key exists: without it nothing can be
    // decrypted, and silently making a new one would orphan every stored data key.
    let keys = std::sync::Arc::new(services::keys::KeyService::from_config(pool.clone(), &config)?);
    wait_for_key_service(&keys).await?;
    info!(provider = keys.provider(), "Key service ready");

    // Seal any listing links stored before they were encrypted (idempotent; a no-op once done).
    // Not fatal: unsealed rows still read correctly, but a failure is loud so it gets fixed.
    match services::url_vault::backfill(&pool, &keys).await {
        Ok(0) => {}
        Ok(n) => info!(sealed = n, "Encrypted legacy listing links"),
        Err(e) => tracing::error!(error = %e, "Could not encrypt legacy listing links; they remain readable but unencrypted"),
    }

    // Redis connection
    let redis_client = redis::Client::open(config.redis_url.as_str())
        .context("Failed to create Redis client")?;

    let state = AppState {
        pool: pool.clone(),
        redis: redis_client.clone(),
        config: config.clone(),
        metrics: metrics_handle,
        keys: keys.clone(),
    };

    // Build the password-hash timing-equalization value now, so the first login for an unknown
    // email does not pay for it (which would be a timing difference of its own).
    services::auth::warm_up().await;

    // Spawn scheduled rescan dispatcher as a background task
    let scheduler_pool = pool.clone();
    let scheduler_redis = redis_client.clone();
    let scheduler_keys = keys.clone();
    tokio::spawn(async move {
        jobs::scheduler::run_scheduled_rescans(
            scheduler_pool,
            scheduler_redis,
            scheduler_keys,
        )
        .await;
    });

    // Repair scan jobs lost to a crashed worker or a failed enqueue (see jobs/recovery.rs)
    let recovery_pool = pool.clone();
    let recovery_redis = redis_client.clone();
    let recovery_keys = keys.clone();
    let recovery_cfg = services::scan::RecoveryConfig {
        audit_identifier_retention_days: config.audit_identifier_retention_days,
        stale_after_secs: config.scan_stale_after_secs,
        redispatch_after_secs: config.scan_redispatch_after_secs,
        max_attempts: config.scan_max_attempts,
    };
    let recovery_interval = std::time::Duration::from_secs(config.scan_recovery_interval_secs.max(1));
    tokio::spawn(async move {
        jobs::recovery::run_scan_recovery(
            recovery_pool,
            recovery_redis,
            recovery_keys,
            recovery_cfg,
            recovery_interval,
        )
        .await;
    });

    // The interactive API documentation maps every endpoint for an attacker, so it is served only
    // outside production.
    let mut public = Router::new();
    if !config.is_production() {
        public = public.merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", docs::ApiDoc::openapi()));
    }

    // With INTERNAL_PORT set, the agent-only routes live on their own listener and the public
    // port does not have them at all; otherwise one port serves both (development).
    let (api_routes, internal_app) = match config.internal_port {
        Some(_) => (
            api::public_router(state.clone()),
            Some(middleware::apply(
                Router::new().nest("/api/v1", api::internal_router(state.clone())),
                &config,
            )),
        ),
        None => {
            if config.is_production() {
                tracing::warn!(
                    "INTERNAL_PORT is not set: the agent-only /internal routes are served on the public port. \
                     Block /api/v1/internal at the reverse proxy, or set INTERNAL_PORT"
                );
            }
            (api::router(state.clone()), None)
        }
    };
    let app = middleware::apply(public.nest("/api/v1", api_routes), &config);

    // One shutdown signal for every listener.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop_tx.send(true);
    });
    let wait_for_stop = |mut rx: tokio::sync::watch::Receiver<bool>| async move {
        let _ = rx.wait_for(|stopped| *stopped).await;
    };

    let mut internal_server = None;
    if let (Some(port), Some(internal_app)) = (config.internal_port, internal_app) {
        let internal_addr = SocketAddr::new(config.internal_bind, port);
        let listener = TcpListener::bind(internal_addr).await?;
        info!("VectorScrub internal (agent) API listening on {}", internal_addr);
        let rx = stop_rx.clone();
        internal_server = Some(tokio::spawn(async move {
            axum::serve(listener, internal_app.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(wait_for_stop(rx))
                .await
        }));
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = TcpListener::bind(addr).await?;

    info!("VectorScrub API listening on {}", addr);

    // `ConnectInfo` makes the client's socket address available to the rate limiter and the
    // audit log.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(wait_for_stop(stop_rx))
    .await?;

    if let Some(handle) = internal_server {
        handle.await??;
    }
    observability::shutdown_observability();

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

/// `vectorscrub verify-backup [--sample N]`: check a database restored from a backup (see
/// infra/scripts/restore_verify.sh). Reads DATABASE_URL (the RESTORED copy, never production)
/// and MASTER_ENCRYPTION_KEY. Prints counts and problems only; exits non-zero if the backup is
/// not usable.
async fn verify_backup_cli(args: &[String]) -> anyhow::Result<()> {
    use services::backup_check;
    let sample = match args {
        [] => backup_check::DEFAULT_SAMPLE,
        [flag, n] if flag == "--sample" => n.parse().context("--sample needs a number")?,
        _ => anyhow::bail!("usage: vectorscrub verify-backup [--sample N]"),
    };
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await.context("could not connect to DATABASE_URL")?;
    let keys = key_service_from_env(&pool)?;
    let report = backup_check::run(&pool, &keys, sample).await?;
    print!("{}", report.render());
    anyhow::ensure!(report.ok(), "the backup is not usable");
    Ok(())
}

/// The key service the operator commands use: the same `KEY_PROVIDER` settings as the service,
/// read from the environment, without needing the rest of the service's configuration.
fn key_service_from_env(pool: &sqlx::PgPool) -> anyhow::Result<services::keys::KeyService> {
    use services::config::{KeyProviderConfig, KeySettings};
    use services::keys::{EnvWrapper, KeyService, PyHsmClient, Wrapper};
    let settings = KeySettings::from_env()?;
    let wrapper = match &settings.provider {
        KeyProviderConfig::Env => {
            let master = settings.master_key_b64.as_deref().context("MASTER_ENCRYPTION_KEY is not set")?;
            Wrapper::Env(EnvWrapper::new(&crypto::MasterKey::from_base64(master)?))
        }
        KeyProviderConfig::PyHsm { socket, ipc_secret, key_id, timeout_ms } => {
            Wrapper::PyHsm(PyHsmClient::new(socket, ipc_secret.clone(), key_id, *timeout_ms))
        }
    };
    Ok(KeyService::new(pool.clone(), wrapper, settings.dek_cache_ttl_secs))
}

/// Wait up to a minute for the key service, and require the key-encryption key to exist.
async fn wait_for_key_service(keys: &services::keys::KeyService) -> anyhow::Result<()> {
    let Some(hsm) = keys.pyhsm() else { return Ok(()) };
    let started = std::time::Instant::now();
    loop {
        if hsm.health().await {
            break;
        }
        anyhow::ensure!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "the key service (PyHSM) did not answer within 60 seconds; check PYHSM_SOCKET and PYHSM_IPC_SECRET"
        );
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    anyhow::ensure!(
        hsm.has_key().await.map_err(|e| anyhow::anyhow!("could not ask the key service about its key: {e}"))?,
        "the key service has no key named '{}'. If this is a new installation run `vectorscrub kms init`; \
         if not, this is the wrong keystore and starting would make every stored customer key unreadable",
        hsm.key_id()
    );
    Ok(())
}

/// `vectorscrub kms init | status | rotate | rewrap | migrate-legacy`
/// Operator commands for the key service. None of them print key material or customer data.
async fn kms_cli(args: &[String]) -> anyhow::Result<()> {
    use services::{keys::EnvWrapper, kms_migrate};
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await.context("could not connect to DATABASE_URL")?;
    let keys = key_service_from_env(&pool)?;
    let legacy_master = || -> anyhow::Result<Option<crypto::MasterKey>> {
        match std::env::var("MASTER_ENCRYPTION_KEY").ok().filter(|v| !v.trim().is_empty()) {
            Some(v) => Ok(Some(crypto::MasterKey::from_base64(v.trim())?)),
            None => Ok(None),
        }
    };
    match args.first().map(String::as_str) {
        Some("init") => {
            let hsm = keys.pyhsm().context("`kms init` needs KEY_PROVIDER=pyhsm")?;
            anyhow::ensure!(hsm.health().await, "the key service is not answering");
            if hsm.has_key().await? {
                println!("Key '{}' already exists. Nothing changed.", hsm.key_id());
            } else {
                hsm.generate_key().await?;
                println!("Created key '{}'. Back up the PyHSM keystore now (see docs/runbooks/key-service.md): without it every customer's data is unreadable.", hsm.key_id());
            }
        }
        Some("status") => {
            println!("provider: {}", keys.provider());
            if let Some(hsm) = keys.pyhsm() {
                println!("key service answering: {}", hsm.health().await);
                println!("key '{}' exists: {}", hsm.key_id(), hsm.has_key().await.unwrap_or(false));
            }
            let rows = keys.status().await?;
            if rows.is_empty() {
                println!("customer data keys: none");
            }
            for (provider, version, count) in rows {
                println!("customer data keys wrapped by {provider} version {version}: {count}");
            }
            println!("values still in the old encryption format: {}", kms_migrate::count_legacy(&pool).await?);
        }
        Some("rotate") => {
            let hsm = keys.pyhsm().context("`kms rotate` needs KEY_PROVIDER=pyhsm")?;
            let version = hsm.rotate_key().await?;
            println!("The key-encryption key is now at version {version}.");
            let report = keys.rewrap_all(None).await?;
            println!("re-wrapped {} customer data keys ({} already current, {} failed)", report.rewrapped, report.already_current, report.failed);
            anyhow::ensure!(report.failed == 0, "some keys could not be re-wrapped; they still open under the old version. Fix the cause and run `kms rewrap`");
        }
        Some("rewrap") => {
            let master = legacy_master()?;
            let env = master.as_ref().map(EnvWrapper::new);
            let report = keys.rewrap_all(env.as_ref()).await?;
            println!("re-wrapped {} customer data keys ({} already current, {} failed)", report.rewrapped, report.already_current, report.failed);
            anyhow::ensure!(report.failed == 0, "some keys could not be re-wrapped");
        }
        Some("migrate-legacy") => {
            let master = legacy_master()?.context("`kms migrate-legacy` needs the OLD MASTER_ENCRYPTION_KEY in the environment")?;
            let report = kms_migrate::migrate_legacy(&pool, &keys, &master).await?;
            println!(
                "migrated {} customers: {} identity profiles, {} two-factor secrets, {} listing links ({} already current, {} failed)",
                report.customers, report.identity_profiles, report.mfa_secrets, report.links, report.already_current, report.failed_customers
            );
            let left = kms_migrate::count_legacy(&pool).await?;
            println!("values still in the old format: {left}");
            anyhow::ensure!(report.failed_customers == 0 && left == 0, "not everything was migrated; see the log for the customers that failed");
        }
        _ => anyhow::bail!("usage: vectorscrub kms init | status | rotate | rewrap | migrate-legacy"),
    }
    Ok(())
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
