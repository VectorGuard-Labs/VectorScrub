//! HTTP-level tests: the real router, real database, and (where noted) real Redis.
//!
//! Redis-backed tests use `TEST_REDIS_URL` (default `redis://127.0.0.1:6379/14`); they
//! clear only the scan queue key in that database.

use std::net::SocketAddr;

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::crypto::MasterKey;
use crate::db::AppState;
use crate::services::{auth as auth_svc, config::AppConfig, identity_access, scan as scan_svc};
use crate::test_support as ts;

fn redis_url() -> String {
    std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/14".into())
}

/// Config with limits high enough to stay out of the way of non-limiter tests.
fn relaxed() -> AppConfig {
    let mut c = ts::test_config();
    c.rate_limit_general_per_second = 1000;
    c.rate_limit_general_burst = 100_000;
    c.rate_limit_auth_per_minute = 60_000;
    c.rate_limit_auth_burst = 100_000;
    c
}

fn app_with(pool: PgPool, config: AppConfig, redis: &str) -> (Router, AppConfig) {
    let state = AppState {
        pool,
        redis: redis::Client::open(redis).expect("redis client"),
        config: config.clone(),
        metrics: metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder().handle(),
    };
    (super::router(state), config)
}

fn app(pool: PgPool) -> (Router, AppConfig) {
    app_with(pool, relaxed(), &redis_url())
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}
impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Reply {
    send_from(app, "127.0.0.1:5000", method, uri, headers, body).await
}

async fn send_from(
    app: &Router,
    peer: &str,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> Reply {
    // `main` nests this router under /api/v1; the router under test is un-nested.
    let mut req = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    let mut req = match body {
        Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string();
    Reply { status, headers, body }
}

fn agent(cfg: &AppConfig) -> Vec<(&'static str, String)> {
    vec![("x-agent-key", cfg.agent_api_key.clone())]
}
fn bearer(cfg: &AppConfig, customer: Uuid, tier: &str) -> Vec<(&'static str, String)> {
    let t = auth_svc::issue_token(customer, tier, &cfg.jwt_secret).unwrap();
    vec![("authorization", format!("Bearer {t}"))]
}
fn master() -> MasterKey {
    MasterKey::from_base64(&ts::master_key_b64()).unwrap()
}

// ===========================================================================
// Agent key
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn internal_endpoints_need_the_exact_agent_key(pool: PgPool) {
    let (app, cfg) = app(pool);
    let uri = "/internal/exposures/due-for-monitoring";
    assert_eq!(send(&app, "GET", uri, &[], None).await.status, StatusCode::UNAUTHORIZED);
    for wrong in ["", "wrong", &cfg.agent_api_key[..cfg.agent_api_key.len() - 1], &format!("{}x", cfg.agent_api_key)] {
        let r = send(&app, "GET", uri, &[("x-agent-key", wrong.to_string())], None).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{wrong:?}");
    }
    assert_eq!(send(&app, "GET", uri, &agent(&cfg), None).await.status, StatusCode::OK);
    // a customer JWT is not an agent key
    let jwt = bearer(&cfg, Uuid::new_v4(), "premium");
    assert_eq!(send(&app, "GET", uri, &jwt, None).await.status, StatusCode::UNAUTHORIZED);
}

// ===========================================================================
// The skeleton key is gone: identity only against actionable work
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn there_is_no_identity_by_customer_id_route_any_more(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "premium", true).await;
    ts::identity(&pool, &master(), c, "Jane Smith").await;
    // Control: a route that does exist answers (403 = reached the handler and was refused).
    // Without this, a 404 below could just mean "the whole router is mis-wired".
    let control = send(&app, "POST", &format!("/internal/exposures/{}/identity", Uuid::new_v4()), &agent(&cfg), None).await;
    assert_eq!(control.status, StatusCode::FORBIDDEN);

    for method in ["GET", "POST"] {
        let r = send(&app, method, &format!("/internal/customers/{c}/identity"), &agent(&cfg), None).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{method}");
        assert!(!r.body.contains("Jane"));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn scan_identity_needs_the_token_minted_for_that_job(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let m = master();
    let c = ts::customer(&pool, "premium", true).await;
    ts::identity(&pool, &m, c, "Jane Smith").await;
    let job = ts::scan_job(&pool, c, "queued").await;
    let token = identity_access::issue_scan_token(&m, c, job).unwrap();

    let ok = send(&app, "POST", "/internal/identity", &agent(&cfg),
        Some(json!({ "customer_id": c, "job_id": job, "identity_token": token }))).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.json()["full_name"], "Jane Smith");

    // knowing the customer id and holding the agent key is NOT enough
    for bad in [json!({ "customer_id": c, "job_id": job, "identity_token": "forged" }),
                json!({ "customer_id": c, "job_id": Uuid::new_v4(), "identity_token": token }),
                json!({ "customer_id": Uuid::new_v4(), "job_id": job, "identity_token": token })] {
        let r = send(&app, "POST", "/internal/identity", &agent(&cfg), Some(bad)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        assert_eq!(r.body, "Identity access denied", "no hint about why");
        assert!(!r.body.contains("Jane"));
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn removal_and_monitoring_identity_only_for_actionable_items(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let m = master();
    let c = ts::customer(&pool, "premium", true).await;
    ts::identity(&pool, &m, c, "Jane Smith").await;

    // a random id (the old skeleton-key probing pattern) gets nothing
    for uri in [format!("/internal/removal-requests/{}/identity", Uuid::new_v4()),
                format!("/internal/exposures/{}/identity", Uuid::new_v4())] {
        assert_eq!(send(&app, "POST", &uri, &agent(&cfg), None).await.status, StatusCode::FORBIDDEN, "{uri}");
    }

    let e = ts::exposure(&pool, c, true, None).await;
    let authorized = ts::removal_request(&pool, c, e, "authorized", None).await;
    let r = send(&app, "POST", &format!("/internal/removal-requests/{authorized}/identity"), &agent(&cfg), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.json()["full_name"], "Jane Smith");

    let due = ts::exposure(&pool, c, false, Some(8)).await;
    let r = send(&app, "POST", &format!("/internal/exposures/{due}/identity"), &agent(&cfg), None).await;
    assert_eq!(r.status, StatusCode::OK);

    // and every one of those was recorded
    let released: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'identity.release' AND status = 'success'")
        .fetch_one(&pool).await.unwrap();
    let denied: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'identity.release' AND status = 'failure'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!((released, denied), (2, 2));
}

// ===========================================================================
// Scan worker protocol over HTTP
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn claim_then_update_is_fenced_by_attempt(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "free", false).await;
    let job = ts::scan_job(&pool, c, "queued").await;

    let first = send(&app, "POST", &format!("/internal/scan/{job}/claim"), &agent(&cfg), None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.json()["attempt"], 1);
    let second = send(&app, "POST", &format!("/internal/scan/{job}/claim"), &agent(&cfg), None).await;
    assert_eq!(second.status, StatusCode::CONFLICT, "only one worker may hold the job");

    let stale = send(&app, "PUT", "/internal/scan/status", &agent(&cfg),
        Some(json!({ "job_id": job, "status": "completed", "attempt": 2, "exposures_found": 1 }))).await;
    assert_eq!(stale.status, StatusCode::CONFLICT);

    let queued_status = send(&app, "PUT", "/internal/scan/status", &agent(&cfg),
        Some(json!({ "job_id": job, "status": "queued", "attempt": 1 }))).await;
    assert_eq!(queued_status.status, StatusCode::BAD_REQUEST, "workers cannot set queued/cancelled");

    let done = send(&app, "PUT", "/internal/scan/status", &agent(&cfg),
        Some(json!({ "job_id": job, "status": "completed", "attempt": 1, "sources_completed": 4, "exposures_found": 0 }))).await;
    assert_eq!(done.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrations = "./migrations")]
async fn exposures_can_only_be_recorded_against_a_running_job_of_that_customer(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let (alice, bob) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
    let running = ts::scan_job(&pool, alice, "running").await;
    let queued = ts::scan_job(&pool, alice, "queued").await;
    let body = |customer: Uuid, job: Uuid| json!({
        "customer_id": customer, "scan_job_id": job, "source_name": "spokeo.com",
        "source_url": "https://www.spokeo.com/x", "pii_category": "full_name", "severity": "high",
        "pii_summary": "J*** S***", "confidence": 0.9
    });

    let ok = send(&app, "POST", "/internal/exposures", &agent(&cfg), Some(body(alice, running))).await;
    assert_eq!(ok.status, StatusCode::CREATED);
    for (who, job, why) in [(bob, running, "another customer's job"), (alice, queued, "job not running"), (alice, Uuid::new_v4(), "no such job")] {
        let r = send(&app, "POST", "/internal/exposures", &agent(&cfg), Some(body(who, job))).await;
        assert_eq!(r.status, StatusCode::CONFLICT, "{why}");
    }
    let bobs: i64 = sqlx::query_scalar("SELECT count(*) FROM exposures WHERE customer_id = $1").bind(bob).fetch_one(&pool).await.unwrap();
    assert_eq!(bobs, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_removal_status_endpoint_enforces_the_lifecycle(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "personal", true).await;
    let e = ts::exposure(&pool, c, true, None).await;
    let r = ts::removal_request(&pool, c, e, "authorized", None).await;
    let uri = format!("/internal/removal-requests/{r}/status");

    // an authorized request cannot be declared removed without being submitted
    let forged = send(&app, "PUT", &uri, &agent(&cfg), Some(json!({ "status": "verified_removed" }))).await;
    assert_eq!(forged.status, StatusCode::CONFLICT);
    let ok = send(&app, "PUT", &uri, &agent(&cfg), Some(json!({ "status": "submitted", "next_check_days": 7 }))).await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
    let missing = send(&app, "PUT", &format!("/internal/removal-requests/{}/status", Uuid::new_v4()), &agent(&cfg),
        Some(json!({ "status": "submitted" }))).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    // the agent can fail a request it could not submit, which now leaves 'authorized'
    let r2 = ts::removal_request(&pool, c, ts::exposure(&pool, c, true, None).await, "authorized", None).await;
    let failed = send(&app, "PUT", &format!("/internal/removal-requests/{r2}/status"), &agent(&cfg), Some(json!({ "status": "failed" }))).await;
    assert_eq!(failed.status, StatusCode::NO_CONTENT);
}

// ===========================================================================
// Scan initiation, job-scoped token, enqueue failure, real Redis
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn starting_a_scan_requires_an_identity_profile(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "premium", false).await;
    let r = send(&app, "POST", "/scans", &bearer(&cfg, c, "premium"), None).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(r.body.contains("identity"));
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM scan_jobs WHERE customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
    assert_eq!(jobs, 0, "nothing queued");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_started_scan_puts_a_job_scoped_token_on_the_queue(pool: PgPool) {
    // Real Redis.
    let (app, cfg) = app(pool.clone());
    let m = master();
    let c = ts::customer(&pool, "premium", false).await;
    ts::identity(&pool, &m, c, "Jane Smith").await;

    let client = redis::Client::open(redis_url().as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.expect("Redis must be running for this test (set TEST_REDIS_URL)");
    redis::cmd("DEL").arg(scan_svc::SCAN_QUEUE_KEY).query_async::<()>(&mut conn).await.unwrap();

    let r = send(&app, "POST", "/scans", &bearer(&cfg, c, "premium"), None).await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    let job_id: Uuid = r.json()["job_id"].as_str().unwrap().parse().unwrap();

    let queued: Vec<String> = redis::cmd("LRANGE").arg(scan_svc::SCAN_QUEUE_KEY).arg(0).arg(-1).query_async(&mut conn).await.unwrap();
    assert_eq!(queued.len(), 1);
    let payload: Value = serde_json::from_str(&queued[0]).unwrap();
    assert_eq!(payload["job_id"], job_id.to_string());
    assert_eq!(payload["tier"], "premium");
    let token = payload["identity_token"].as_str().unwrap();

    // The queued token unlocks exactly this job's identity, via the real endpoint...
    let ok = send(&app, "POST", "/internal/identity", &agent(&cfg),
        Some(json!({ "customer_id": c, "job_id": job_id, "identity_token": token }))).await;
    assert_eq!(ok.status, StatusCode::OK);
    // ...and nothing else.
    let other_job = ts::scan_job(&pool, c, "queued").await;
    let reuse = send(&app, "POST", "/internal/identity", &agent(&cfg),
        Some(json!({ "customer_id": c, "job_id": other_job, "identity_token": token }))).await;
    assert_eq!(reuse.status, StatusCode::FORBIDDEN);

    let dispatched: bool = sqlx::query_scalar("SELECT dispatched_at IS NOT NULL FROM scan_jobs WHERE id = $1").bind(job_id).fetch_one(&pool).await.unwrap();
    assert!(dispatched);
    redis::cmd("DEL").arg(scan_svc::SCAN_QUEUE_KEY).query_async::<()>(&mut conn).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn if_redis_is_down_the_scan_is_accepted_and_recovered_not_lost(pool: PgPool) {
    let (app, cfg) = app_with(pool.clone(), relaxed(), "redis://127.0.0.1:1/0");
    let c = ts::customer(&pool, "premium", false).await;
    ts::identity(&pool, &master(), c, "Jane Smith").await;

    let r = send(&app, "POST", "/scans", &bearer(&cfg, c, "premium"), None).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "the customer is not told it failed");
    let job_id: Uuid = r.json()["job_id"].as_str().unwrap().parse().unwrap();

    let (status, dispatched): (String, bool) = sqlx::query_as("SELECT status::text, dispatched_at IS NOT NULL FROM scan_jobs WHERE id = $1")
        .bind(job_id).fetch_one(&pool).await.unwrap();
    assert_eq!((status.as_str(), dispatched), ("queued", false));

    // the recovery pass picks it up
    let report = scan_svc::recover_jobs(&pool, &scan_svc::RecoveryConfig { stale_after_secs: 600, redispatch_after_secs: 600, max_attempts: 3 }).await.unwrap();
    assert!(report.to_dispatch.iter().any(|d| d.job_id == job_id));
}

#[sqlx::test(migrations = "./migrations")]
async fn free_accounts_get_a_429_with_a_clear_message_when_over_their_scan_limit(pool: PgPool) {
    let (app, cfg) = app_with(pool.clone(), relaxed(), "redis://127.0.0.1:1/0");
    let c = ts::customer(&pool, "free", false).await;
    ts::identity(&pool, &master(), c, "Jane Smith").await;
    assert_eq!(send(&app, "POST", "/scans", &bearer(&cfg, c, "free"), None).await.status, StatusCode::ACCEPTED);
    let again = send(&app, "POST", "/scans", &bearer(&cfg, c, "free"), None).await;
    assert_eq!(again.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(again.body.contains("Free accounts"));
}

// ===========================================================================
// Login lockout
// ===========================================================================

const PASSWORD: &str = "correct horse battery staple";

async fn register(app: &Router, email: &str) -> Reply {
    send(app, "POST", "/auth/register", &[], Some(json!({ "email": email, "password": PASSWORD }))).await
}
async fn login(app: &Router, email: &str, password: &str) -> Reply {
    send(app, "POST", "/auth/login", &[("user-agent", "test-agent/1.0".into())], Some(json!({ "email": email, "password": password }))).await
}

#[sqlx::test(migrations = "./migrations")]
async fn five_bad_passwords_lock_the_account_even_against_the_right_password(pool: PgPool) {
    let (app, _) = app(pool.clone());
    assert_eq!(register(&app, "lock@example.com").await.status, StatusCode::CREATED);

    let mut bodies = Vec::new();
    for _ in 0..5 {
        let r = login(&app, "lock@example.com", "wrong password here").await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        bodies.push(r.body);
    }
    assert!(bodies.iter().all(|b| b == "Invalid email or password"));

    let locked = login(&app, "lock@example.com", PASSWORD).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS, "right password is refused while locked");
    let retry: i64 = locked.headers["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((800..=900).contains(&retry), "Retry-After {retry}");
    assert!(locked.body.contains("Too many failed login attempts"));

    // forensic trail: failures, the lock event, with the client address
    let failures: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'user.login' AND status = 'failure'").fetch_one(&pool).await.unwrap();
    let lock_events: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'user.login_locked'").fetch_one(&pool).await.unwrap();
    let ip: Option<String> = sqlx::query_scalar("SELECT ip_address FROM audit_log WHERE action = 'user.login_locked'").fetch_one(&pool).await.unwrap();
    assert_eq!(failures, 6, "5 bad passwords + the refused attempt");
    assert_eq!(lock_events, 1);
    assert_eq!(ip.as_deref(), Some("127.0.0.1"));

    // another account is unaffected
    assert_eq!(register(&app, "other@example.com").await.status, StatusCode::CREATED);
    assert_eq!(login(&app, "other@example.com", PASSWORD).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_works_the_same_for_emails_that_do_not_exist(pool: PgPool) {
    let (app, _) = app(pool.clone());
    assert_eq!(register(&app, "real@example.com").await.status, StatusCode::CREATED);

    // identical response for "wrong password" and "no such account"
    let wrong_pw = login(&app, "real@example.com", "not the password").await;
    let no_account = login(&app, "ghost@example.com", "not the password").await;
    assert_eq!((wrong_pw.status, no_account.status), (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED));
    assert_eq!(wrong_pw.body, no_account.body, "no account enumeration through the message");

    for _ in 0..4 {
        login(&app, "ghost@example.com", "not the password").await;
    }
    let locked = login(&app, "ghost@example.com", "not the password").await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS, "a ghost account locks like a real one");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_successful_login_resets_the_failure_count(pool: PgPool) {
    let (app, _) = app(pool.clone());
    register(&app, "reset@example.com").await;
    for _ in 0..4 {
        login(&app, "reset@example.com", "nope nope nope nope").await;
    }
    let ok = login(&app, "reset@example.com", PASSWORD).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert!(ok.json()["token"].as_str().unwrap().len() > 20);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM login_throttle").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 0, "record cleared");
    // four more failures still do not lock (the count started over)
    for _ in 0..4 {
        assert_eq!(login(&app, "reset@example.com", "nope nope nope nope").await.status, StatusCode::UNAUTHORIZED);
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_email_still_costs_a_password_hash(pool: PgPool) {
    // Without timing equalization "no such account" returns in microseconds while "wrong
    // password" takes a full Argon2 verification, which reveals which emails exist.
    let (app, _) = app(pool.clone());
    register(&app, "timing@example.com").await;
    let _warm = login(&app, "warmup@example.com", "x1234567890123").await; // initializes the dummy hash

    let time = |email: &'static str| {
        let app = app.clone();
        async move {
            let start = std::time::Instant::now();
            // distinct emails keep each attempt below the lockout threshold
            login(&app, email, "not the password").await;
            start.elapsed()
        }
    };
    let known = time("timing@example.com").await;
    let unknown = time("nobody-here@example.com").await;
    assert!(
        unknown * 2 >= known,
        "unknown email answered in {unknown:?} vs {known:?} for a real account: it skipped the password hash"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn registering_a_taken_email_is_a_clean_conflict(pool: PgPool) {
    let (app, _) = app(pool);
    assert_eq!(register(&app, "dup@example.com").await.status, StatusCode::CREATED);
    let dup = register(&app, "dup@example.com").await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.body, "An account with this email address already exists");
    let weak = send(&app, "POST", "/auth/register", &[], Some(json!({ "email": "w@example.com", "password": "short" }))).await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ===========================================================================
// Rate limiting
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn login_and_register_share_a_strict_per_ip_limit(pool: PgPool) {
    let mut cfg = relaxed();
    cfg.rate_limit_auth_burst = 3;
    cfg.rate_limit_auth_per_minute = 1;
    let (app, _) = app_with(pool, cfg, &redis_url());

    let mut statuses = Vec::new();
    for i in 0..5 {
        let r = send(&app, "POST", "/auth/login", &[], Some(json!({ "email": format!("u{i}@example.com"), "password": "whatever-it-is" }))).await;
        statuses.push(r.status);
    }
    assert!(statuses[..3].iter().all(|s| *s != StatusCode::TOO_MANY_REQUESTS), "{statuses:?}");
    assert_eq!(&statuses[3..], &[StatusCode::TOO_MANY_REQUESTS; 2]);

    // the same address cannot dodge it through register
    assert_eq!(send(&app, "POST", "/auth/register", &[], Some(json!({ "email": "z@example.com", "password": PASSWORD }))).await.status, StatusCode::TOO_MANY_REQUESTS);
    // a different client address has its own allowance
    let other = send_from(&app, "198.51.100.77:9", "POST", "/auth/login", &[], Some(json!({ "email": "a@example.com", "password": "whatever-it-is" }))).await;
    assert_ne!(other.status, StatusCode::TOO_MANY_REQUESTS);
    // and the 429 tells the client when to retry
    let limited = send(&app, "POST", "/auth/login", &[], Some(json!({}))).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers.contains_key("retry-after"));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_customer_api_has_a_general_limit_but_agents_and_probes_do_not(pool: PgPool) {
    let mut cfg = relaxed();
    cfg.rate_limit_general_burst = 3;
    cfg.rate_limit_general_per_second = 1;
    let (app, cfg) = app_with(pool.clone(), cfg, &redis_url());
    let c = ts::customer(&pool, "free", false).await;
    let jwt = bearer(&cfg, c, "free");

    let mut statuses = Vec::new();
    for _ in 0..6 {
        statuses.push(send(&app, "GET", "/me", &jwt, None).await.status);
    }
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count(), 3, "{statuses:?}");

    // all agents share one address; they must never be throttled by IP
    for _ in 0..40 {
        let r = send(&app, "GET", "/internal/exposures/due-for-monitoring", &agent(&cfg), None).await;
        assert_eq!(r.status, StatusCode::OK);
    }
    for _ in 0..40 {
        assert_eq!(send(&app, "GET", "/health/live", &[], None).await.status, StatusCode::OK);
    }
}

// ===========================================================================
// Error hygiene and probes
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn internal_failures_do_not_leak_details_to_clients(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "free", false).await;
    pool.close().await; // every query from here on fails with a driver error

    let r = send(&app, "GET", "/exposures", &bearer(&cfg, c, "free"), None).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(r.body, "Internal server error");
    for leak in ["pool", "closed", "sqlx", "postgres", "connection"] {
        assert!(!r.body.to_lowercase().contains(leak), "leaked {leak:?}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn health_probes(pool: PgPool) {
    let (app, _) = app(pool.clone());
    assert_eq!(send(&app, "GET", "/health/live", &[], None).await.json()["status"], "ok");

    let ready = send(&app, "GET", "/health/ready", &[], None).await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.body);
    assert_eq!(ready.json()["database"], true);

    // Redis unreachable: not ready, names the component, hides the error
    let (down, _) = app_with(pool, relaxed(), "redis://127.0.0.1:1/0");
    let r = send(&down, "GET", "/health/ready", &[], None).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!((r.json()["database"].clone(), r.json()["redis"].clone()), (json!(true), json!(false)));
    assert!(!r.body.contains("refused") && !r.body.contains("127.0.0.1"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_customer_cannot_read_another_customers_scan(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let (alice, mallory) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
    let job = ts::scan_job(&pool, alice, "completed").await;
    let own = send(&app, "GET", &format!("/scans/{job}"), &bearer(&cfg, alice, "free"), None).await;
    assert_eq!(own.status, StatusCode::OK);
    let theirs = send(&app, "GET", &format!("/scans/{job}"), &bearer(&cfg, mallory, "free"), None).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let none = send(&app, "GET", &format!("/scans/{job}"), &[], None).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
}
