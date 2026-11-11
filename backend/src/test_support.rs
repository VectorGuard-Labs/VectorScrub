//! Fixtures for database-backed tests (`#[sqlx::test]`).
//!
//! Each test gets its own freshly-migrated database, so fixtures insert rows directly
//! with runtime queries instead of going through the API.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::MasterKey;
use crate::models::customer::SubmitIdentityRequest;
use crate::services::customer as customer_svc;

pub fn master_key_b64() -> String {
    BASE64.encode([7u8; 32])
}

pub fn other_master_key_b64() -> String {
    BASE64.encode([9u8; 32])
}

/// Insert a customer. `tier` is free | personal | premium.
/// A customer with a specific email address.
pub async fn customer_with_email(pool: &PgPool, email: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO customers (id, email, password_hash) VALUES ($1, $2, 'x')")
        .bind(id)
        .bind(email)
        .execute(pool)
        .await
        .expect("insert customer");
    id
}

pub async fn customer(pool: &PgPool, tier: &str, monitoring_enabled: bool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO customers (id, email, password_hash, subscription_tier, monitoring_enabled) \
         VALUES ($1, $2, 'x', $3::text::subscription_tier, $4)",
    )
    .bind(id)
    .bind(format!("{id}@test.example"))
    .bind(tier)
    .bind(monitoring_enabled)
    .execute(pool)
    .await
    .expect("insert customer");
    id
}

/// Store an encrypted identity profile for the customer.
pub async fn identity(pool: &PgPool, master: &MasterKey, customer_id: Uuid, full_name: &str) {
    let req = SubmitIdentityRequest {
        full_name: full_name.to_string(),
        current_address: "123 Main St, Austin, TX 78701".to_string(),
        previous_addresses: vec![],
        phone_numbers: vec!["512-555-0147".to_string()],
        email_addresses: vec!["jane@example.com".to_string()],
        date_of_birth: Some("1984-03-09".to_string()),
    };
    customer_svc::save_identity(pool, master, &req, customer_id)
        .await
        .expect("save identity");
}

pub async fn scan_job(pool: &PgPool, customer_id: Uuid, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO scan_jobs (id, customer_id, status, dispatched_at) \
         VALUES ($1, $2, $3::text::scan_job_status, NOW())",
    )
    .bind(id)
    .bind(customer_id)
    .bind(status)
    .execute(pool)
    .await
    .expect("insert scan job");
    id
}

pub async fn set_job_status(pool: &PgPool, job_id: Uuid, status: &str) {
    sqlx::query("UPDATE scan_jobs SET status = $2::text::scan_job_status WHERE id = $1")
        .bind(job_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("set job status");
}

/// Insert an exposure on a fresh, unique source. `removed_days_ago` sets `removed_at`.
pub async fn exposure(
    pool: &PgPool,
    customer_id: Uuid,
    is_active: bool,
    removed_days_ago: Option<i32>,
) -> Uuid {
    let job = scan_job(pool, customer_id, "completed").await;
    let id = Uuid::new_v4();
    let source = format!("{id}.example");
    sqlx::query(
        "INSERT INTO exposures (id, customer_id, scan_job_id, source_name, source_url, \
             pii_category, severity, pii_summary, confidence, is_active, removed_at) \
         VALUES ($1, $2, $3, $4, $5, 'full_name', 'high', 'J*** S***', 0.9, $6, \
             CASE WHEN $7::int IS NULL THEN NULL ELSE NOW() - make_interval(days => $7::int) END)",
    )
    .bind(id)
    .bind(customer_id)
    .bind(job)
    .bind(&source)
    .bind(format!("https://{source}/p"))
    .bind(is_active)
    .bind(removed_days_ago)
    .execute(pool)
    .await
    .expect("insert exposure");
    id
}

/// Insert a removal request for an exposure. `next_check_in_days` is relative to now
/// (negative = already due). Submitted/acknowledged requests get an 8-day-old
/// `submitted_at`, and every request looks 8 days old (created and last updated).
pub async fn removal_request(
    pool: &PgPool,
    customer_id: Uuid,
    exposure_id: Uuid,
    status: &str,
    next_check_in_days: Option<i32>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO removal_requests (id, customer_id, exposure_id, source_name, source_url, status, \
             submitted_at, next_check_at, created_at, updated_at) \
         SELECT $1, $2, e.id, e.source_name, e.source_url, $4::text::removal_status, \
             CASE WHEN $4 IN ('submitted', 'acknowledged') THEN NOW() - interval '8 days' END, \
             CASE WHEN $5::int IS NULL THEN NULL ELSE NOW() + make_interval(days => $5::int) END, \
             NOW() - interval '8 days', NOW() - interval '8 days' \
         FROM exposures e WHERE e.id = $3",
    )
    .bind(id)
    .bind(customer_id)
    .bind(exposure_id)
    .bind(status)
    .bind(next_check_in_days)
    .execute(pool)
    .await
    .expect("insert removal request");
    id
}

/// A complete config for tests that need an `AppConfig`.
pub fn test_config() -> crate::services::config::AppConfig {
    use crate::services::config::{AppConfig, Environment};
    AppConfig {
        database_url: "postgres://unused".into(),
        redis_url: "redis://127.0.0.1:1".into(),
        jwt_secret: "t".repeat(40),
        master_key_b64: master_key_b64(),
        agent_api_key: "a".repeat(40),
        port: 0,
        db_pool_size: 1,
        agent_service_url: "http://127.0.0.1:1".into(),
        environment: Environment::Development,
        allowed_origins: vec!["http://localhost:3000".into()],
        otlp_endpoint: None,
        trust_proxy_headers: false,
        trusted_proxy_hops: 1,
        rate_limit_general_per_second: 20,
        rate_limit_general_burst: 60,
        rate_limit_auth_per_minute: 5,
        rate_limit_auth_burst: 5,
        scan_recovery_interval_secs: 60,
        scan_stale_after_secs: 600,
        scan_redispatch_after_secs: 600,
        scan_max_attempts: 3,
        stripe: None,
    }
}

/// An `AppState` whose connections are never opened. For tests of layers that do not
/// touch the database or Redis (rate limiting, routing). Must run inside a Tokio runtime.
pub fn dummy_state() -> crate::db::AppState {
    crate::db::AppState {
        pool: sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://u:p@127.0.0.1:1/none")
            .expect("lazy pool"),
        redis: redis::Client::open("redis://127.0.0.1:1").expect("redis client"),
        config: test_config(),
        metrics: metrics_exporter_prometheus::PrometheusBuilder::new()
            .build_recorder()
            .handle(),
    }
}
