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
async fn a_removal_submission_can_be_claimed_once_and_only_by_the_agent(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "personal", true).await;
    let e = ts::exposure(&pool, c, true, None).await;
    let r = ts::removal_request(&pool, c, e, "authorized", None).await;
    let uri = format!("/internal/removal-requests/{r}/claim");

    let no_key = send(&app, "POST", &uri, &bearer(&cfg, c, "personal"), None).await;
    assert_eq!(no_key.status, StatusCode::UNAUTHORIZED, "a customer token is not an agent key");
    assert_eq!(send(&app, "POST", &uri, &agent(&cfg), None).await.status, StatusCode::NO_CONTENT);
    assert_eq!(send(&app, "POST", &uri, &agent(&cfg), None).await.status, StatusCode::CONFLICT, "second worker is refused");
    let unknown = format!("/internal/removal-requests/{}/claim", Uuid::new_v4());
    assert_eq!(send(&app, "POST", &unknown, &agent(&cfg), None).await.status, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn authorizing_the_same_exposure_twice_is_a_conflict_not_an_error(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "personal", true).await;
    let e = ts::exposure(&pool, c, true, None).await;
    let uri = format!("/removal-requests/{e}/authorize");
    let first = send(&app, "POST", &uri, &bearer(&cfg, c, "personal"), None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.body);
    let second = send(&app, "POST", &uri, &bearer(&cfg, c, "personal"), None).await;
    assert_eq!(second.status, StatusCode::CONFLICT);
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

/// The raw `Set-Cookie` line for the session cookie, if the reply set one.
fn session_set_cookie(r: &Reply) -> Option<String> {
    r.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("vs_session="))
        .map(str::to_string)
}
/// Just the session token out of that cookie.
fn session_cookie_value(r: &Reply) -> String {
    let line = session_set_cookie(r).expect("a session cookie was set");
    line["vs_session=".len()..].split(';').next().unwrap().to_string()
}

// ===========================================================================
// Browser session: HttpOnly cookie, not a token the page can read
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn login_and_register_put_the_session_in_an_httponly_cookie_and_not_in_the_body(pool: PgPool) {
    let (app, _) = app(pool);
    let reg = register(&app, "cookie@example.com").await;
    assert_eq!(reg.status, StatusCode::CREATED);
    let log = login(&app, "cookie@example.com", PASSWORD).await;
    assert_eq!(log.status, StatusCode::OK);

    for reply in [&reg, &log] {
        let cookie = session_set_cookie(reply).expect("session cookie");
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict") && cookie.contains("Path=/api/v1"), "{cookie}");
        let token = session_cookie_value(reply);
        assert!(!reply.body.contains(&token), "the token must not appear in the response body");
        assert!(reply.json().get("token").is_none());
        assert_eq!(reply.json()["customer"]["email"], "cookie@example.com");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_session_cookie_authenticates_requests(pool: PgPool) {
    let (app, _) = app(pool);
    register(&app, "me@example.com").await;
    let token = session_cookie_value(&login(&app, "me@example.com", PASSWORD).await);

    let me = send(&app, "GET", "/me", &[("cookie", format!("theme=dark; vs_session={token}; other=1"))], None).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert_eq!(me.json()["email"], "me@example.com");
}

#[sqlx::test(migrations = "./migrations")]
async fn requests_without_a_valid_session_are_refused_without_saying_why(pool: PgPool) {
    let (app, _) = app(pool);
    register(&app, "x@example.com").await;
    let token = session_cookie_value(&login(&app, "x@example.com", PASSWORD).await);

    let none = send(&app, "GET", "/me", &[], None).await;
    let junk = send(&app, "GET", "/me", &[("cookie", "vs_session=not.a.jwt".into())], None).await;
    let empty = send(&app, "GET", "/me", &[("cookie", "vs_session=".into())], None).await;
    let tampered = send(&app, "GET", "/me", &[("cookie", format!("vs_session={token}x"))], None).await;
    let lookalike = send(&app, "GET", "/me", &[("cookie", format!("xvs_session={token}"))], None).await;
    for r in [&none, &junk, &empty, &tampered, &lookalike] {
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.body);
    }
    assert_eq!(junk.body, "Invalid or expired session", "no library error text");
    assert_eq!(tampered.body, junk.body);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_bearer_header_still_works_and_a_bad_one_is_not_rescued_by_a_cookie(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "personal", true).await;
    let good = bearer(&cfg, c, "personal");
    assert_eq!(send(&app, "GET", "/me", &good, None).await.status, StatusCode::OK);

    // a malformed Authorization header does not silently fall back to the cookie
    let other = ts::customer(&pool, "personal", true).await;
    let cookie_token = auth_svc::issue_token(other, "personal", &cfg.jwt_secret).unwrap();
    let r = send(&app, "GET", "/me", &[
        ("authorization", "Bearer not-a-token".into()),
        ("cookie", format!("vs_session={cookie_token}")),
    ], None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let r = send(&app, "GET", "/me", &[("authorization", "Basic abc".into()), ("cookie", format!("vs_session={cookie_token}"))], None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_expires_the_session_cookie_and_needs_no_session(pool: PgPool) {
    let (app, _) = app(pool);
    let r = send(&app, "POST", "/auth/logout", &[], None).await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let cookie = session_set_cookie(&r).expect("the clearing cookie");
    assert!(cookie.starts_with("vs_session=;") && cookie.contains("Max-Age=0") && cookie.contains("Path=/api/v1"), "{cookie}");
}

#[sqlx::test(migrations = "./migrations")]
async fn production_sessions_are_marked_secure(pool: PgPool) {
    use crate::services::config::Environment;
    let mut cfg = relaxed();
    cfg.environment = Environment::Production;
    let (app, _) = app_with(pool, cfg, &redis_url());
    let reg = register(&app, "sec@example.com").await;
    assert!(session_set_cookie(&reg).unwrap().contains("Secure"));
    let out = send(&app, "POST", "/auth/logout", &[], None).await;
    assert!(session_set_cookie(&out).unwrap().contains("Secure"));
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
    assert!(session_cookie_value(&ok).len() > 20, "the session is in the cookie");
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
async fn email_addresses_are_one_account_regardless_of_case_or_padding(pool: PgPool) {
    let (app, _) = app(pool.clone());
    assert_eq!(register(&app, "Jane.Smith@Example.com").await.status, StatusCode::CREATED);

    // the same address in another case is the same account: a conflict, not a second account
    for variant in ["jane.smith@example.com", "JANE.SMITH@EXAMPLE.COM", "jAnE.sMiTh@eXaMpLe.CoM"] {
        assert_eq!(register(&app, variant).await.status, StatusCode::CONFLICT, "{variant:?}");
    }
    // padding is not part of an address: it is rejected as invalid rather than treated as a new one
    assert_eq!(register(&app, "  jane.smith@example.com ").await.status, StatusCode::UNPROCESSABLE_ENTITY);
    // and logging in works with any casing, because the lockout and the lookup agree
    for variant in ["Jane.Smith@Example.com", "jane.smith@example.com", "JANE.SMITH@EXAMPLE.COM"] {
        assert_eq!(login(&app, variant, PASSWORD).await.status, StatusCode::OK, "{variant:?}");
    }
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM customers").fetch_one(&pool).await.unwrap();
    assert_eq!(accounts, 1);
    // the address is kept as the customer typed it
    let stored: String = sqlx::query_scalar("SELECT email FROM customers").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, "Jane.Smith@Example.com");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_database_itself_refuses_two_accounts_that_differ_only_by_case(pool: PgPool) {
    ts::customer_with_email(&pool, "Dup@Example.com").await;
    let err = sqlx::query("INSERT INTO customers (id, email, password_hash, subscription_tier, is_verified, is_active, created_at, updated_at) \
                           VALUES (gen_random_uuid(), 'dup@example.com', 'x', 'free', false, true, NOW(), NOW())")
        .execute(&pool).await.unwrap_err();
    assert!(err.as_database_error().is_some_and(|e| e.is_unique_violation()), "{err}");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_login_lockout_applies_across_email_casing(pool: PgPool) {
    let (app, _) = app(pool);
    register(&app, "case@example.com").await;
    for variant in ["case@example.com", "CASE@example.com", "Case@Example.com", "cAsE@example.com", "case@EXAMPLE.com"] {
        assert_eq!(login(&app, variant, "wrong wrong wrong").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login(&app, "case@example.com", PASSWORD).await.status, StatusCode::TOO_MANY_REQUESTS,
        "five failures under five spellings are five failures for one account");
}

#[sqlx::test(migrations = "./migrations")]
async fn absurdly_long_passwords_are_rejected_before_they_reach_the_hasher(pool: PgPool) {
    let (app, _) = app(pool);
    let long = "p".repeat(129);
    let r = send(&app, "POST", "/auth/register", &[], Some(json!({ "email": "long@example.com", "password": long }))).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let r = send(&app, "POST", "/auth/login", &[], Some(json!({ "email": "long@example.com", "password": "p".repeat(5000) }))).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
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

// ===========================================================================
// Stripe billing
// ===========================================================================

mod billing {
    use super::*;
    use crate::services::config::{BillingInterval, PaidTier, StripeConfig, StripePlan};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    const SECRET: &str = "whsec_http_test_signing_secret";

    fn stripe_cfg() -> AppConfig {
        let mut c = relaxed();
        c.stripe = Some(StripeConfig {
            webhook_secret: SECRET.into(),
            plans: vec![StripePlan {
                tier: PaidTier::Premium,
                interval: BillingInterval::Month,
                price_id: "price_premium_m".into(),
                legacy_price_ids: vec![],
                link: Some("https://buy.stripe.com/test_rm".into()),
            }],
            portal_url: None,
        });
        c
    }

    /// POST a raw body (the signature covers the exact bytes, so no JSON re-encoding).
    async fn post_raw(app: &Router, body: &str, signature: Option<String>, cookies: Option<&str>) -> Reply {
        let mut req = Request::builder().method("POST").uri("/webhooks/stripe");
        if let Some(s) = signature {
            req = req.header("stripe-signature", s);
        }
        if let Some(c) = cookies {
            req = req.header("cookie", c);
        }
        let mut req = req.header("content-type", "application/json").body(Body::from(body.to_string())).unwrap();
        req.extensions_mut().insert(ConnectInfo("203.0.113.9:4000".parse::<SocketAddr>().unwrap()));
        let resp = app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let body = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).to_string();
        Reply { status, headers, body }
    }

    fn signed(body: &str) -> String {
        let t = time::OffsetDateTime::now_utc().unix_timestamp();
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(format!("{t}.{body}").as_bytes());
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    async fn tier_of(pool: &PgPool, c: Uuid) -> String {
        sqlx::query_scalar("SELECT subscription_tier::text FROM customers WHERE id = $1")
            .bind(c).fetch_one(pool).await.unwrap()
    }

    fn checkout_body(customer: Uuid) -> String {
        json!({"id":"evt_c","type":"checkout.session.completed","created":100,"data":{"object":{
            "mode":"subscription","customer":"cus_1","subscription":"sub_1",
            "client_reference_id": crate::services::billing::client_reference_id(SECRET, customer)
        }}}).to_string()
    }
    fn subscription_body(status: &str, created: i64) -> String {
        json!({"id":"evt_s","type":"customer.subscription.updated","created":created,"data":{"object":{
            "id":"sub_1","customer":"cus_1","status":status,
            "items":{"data":[{"price":{"id":"price_premium_m"}}]}
        }}}).to_string()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_webhook_does_not_exist_until_stripe_is_configured(pool: PgPool) {
        let (app, _) = app(pool);
        let body = "{}";
        assert_eq!(post_raw(&app, body, Some(signed(body)), None).await.status, StatusCode::NOT_FOUND);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unsigned_badly_signed_and_tampered_webhooks_change_nothing(pool: PgPool) {
        let (app, _) = app_with(pool.clone(), stripe_cfg(), &redis_url());
        let c = ts::customer(&pool, "free", false).await;
        let body = checkout_body(c);
        let sub = subscription_body("active", 101);

        assert_eq!(post_raw(&app, &body, None, None).await.status, StatusCode::BAD_REQUEST, "no signature");
        assert_eq!(post_raw(&app, &body, Some("t=1,v1=00".into()), None).await.status, StatusCode::BAD_REQUEST);
        // a valid signature for a different body
        assert_eq!(post_raw(&app, &sub, Some(signed(&body)), None).await.status, StatusCode::BAD_REQUEST, "tampered");
        assert_eq!(tier_of(&pool, c).await, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_signed_purchase_upgrades_and_a_signed_deletion_downgrades_without_a_csrf_token(pool: PgPool) {
        let (app, cfg) = app_with(pool.clone(), stripe_cfg(), &redis_url());
        let c = ts::customer(&pool, "free", false).await;

        for body in [checkout_body(c), subscription_body("active", 101)] {
            let r = post_raw(&app, &body, Some(signed(&body)), None).await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.body);
            assert_eq!(r.json()["received"], true);
        }
        assert_eq!(tier_of(&pool, c).await, "premium");

        // the customer's own billing page reflects it
        let page = send(&app, "GET", "/me/billing", &bearer(&cfg, c, "premium"), None).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        assert_eq!(page.json()["subscription"]["tier"], "premium");
        assert_eq!(page.json()["plans"].as_array().unwrap().len(), 1);

        let gone = json!({"id":"evt_d","type":"customer.subscription.deleted","created":200,"data":{"object":{
            "id":"sub_1","customer":"cus_1","status":"canceled","items":{"data":[{"price":{"id":"price_premium_m"}}]}
        }}}).to_string();
        assert_eq!(post_raw(&app, &gone, Some(signed(&gone)), None).await.status, StatusCode::OK);
        assert_eq!(tier_of(&pool, c).await, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_browser_cannot_upgrade_itself(pool: PgPool) {
        // The only route that changes a plan is the signed webhook: a signed-in customer
        // posting the same payload without Stripe's signature gets nowhere.
        let (app, cfg) = app_with(pool.clone(), stripe_cfg(), &redis_url());
        let c = ts::customer(&pool, "free", false).await;
        let t = auth_svc::issue_token(c, "free", &cfg.jwt_secret).unwrap();
        for body in [checkout_body(c), subscription_body("active", 101)] {
            let r = post_raw(&app, &body, None, Some(&format!("vs_session={t}"))).await;
            assert_eq!(r.status, StatusCode::BAD_REQUEST);
        }
        assert_eq!(tier_of(&pool, c).await, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_billing_page_needs_a_session(pool: PgPool) {
        let (app, _) = app_with(pool, stripe_cfg(), &redis_url());
        assert_eq!(send(&app, "GET", "/me/billing", &[], None).await.status, StatusCode::UNAUTHORIZED);
    }
}

// ===========================================================================
// Dates reach the browser as RFC 3339 strings
// ===========================================================================

/// `time::OffsetDateTime` serializes as a numeric array unless told otherwise, which the
/// frontend's `new Date(...)` cannot read: every date on the dashboard showed as "—".
#[sqlx::test(migrations = "./migrations")]
async fn every_date_in_a_response_is_an_rfc3339_string(pool: PgPool) {
    use time::format_description::well_known::Rfc3339;
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "free", false).await;
    sqlx::query("UPDATE customers SET last_scan_at = NOW() WHERE id = $1").bind(c).execute(&pool).await.unwrap();
    ts::exposure(&pool, c, true, None).await;
    let removed = ts::exposure(&pool, c, false, Some(3)).await;
    ts::removal_request(&pool, c, removed, "submitted", Some(2)).await;
    let who = bearer(&cfg, c, "free");

    let is_date = |v: &Value| v.as_str().map(|s| time::OffsetDateTime::parse(s, &Rfc3339).is_ok()).unwrap_or(false);

    let me = send(&app, "GET", "/me", &who, None).await.json();
    assert!(is_date(&me["created_at"]) && is_date(&me["last_scan_at"]), "{me}");

    let scans = send(&app, "GET", "/scans", &who, None).await.json();
    assert!(!scans["jobs"].as_array().unwrap().is_empty());
    for job in scans["jobs"].as_array().unwrap() {
        assert!(is_date(&job["queued_at"]), "{job}");
        assert!(job["started_at"].is_null() || is_date(&job["started_at"]), "{job}");
    }

    let exposures = send(&app, "GET", "/exposures", &who, None).await.json();
    for e in exposures["exposures"].as_array().unwrap() {
        assert!(is_date(&e["discovered_at"]) && is_date(&e["last_verified_at"]), "{e}");
        assert!(e["removed_at"].is_null() || is_date(&e["removed_at"]), "{e}");
    }

    let summary = send(&app, "GET", "/exposures/summary", &who, None).await.json();
    assert!(is_date(&summary["summary"]["last_scan_at"]), "{summary}");

    let removals = send(&app, "GET", "/removal-requests", &who, None).await.json();
    let reqs = removals["requests"].as_array().unwrap();
    assert!(!reqs.is_empty());
    for r in reqs {
        assert!(is_date(&r["customer_authorized_at"]) && is_date(&r["created_at"]), "{r}");
        assert!(r["next_check_at"].is_null() || is_date(&r["next_check_at"]), "{r}");
    }
}

// ===========================================================================
// Scan coverage: which sources could be read
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn the_coverage_a_worker_reports_reaches_the_customer_with_the_completed_scan(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "premium", false).await;
    let job = ts::scan_job(&pool, c, "queued").await;
    send(&app, "POST", &format!("/internal/scan/{job}/claim"), &agent(&cfg), None).await;

    let report = json!([
        { "source": "spokeo.com", "outcome": "checked" },
        { "source": "whitepages.com", "outcome": "blocked" },
        { "source": "peoplefinder.com", "outcome": "not_found" },
        { "source": "radaris.com", "outcome": "unreadable" },
        { "source": "nuwber.com", "outcome": "unavailable" },
        { "source": "intelius.com", "outcome": "not_supported" },
    ]);
    let done = send(&app, "PUT", "/internal/scan/status", &agent(&cfg), Some(json!({
        "job_id": job, "status": "completed", "attempt": 1,
        "sources_completed": 1, "sources_failed": 4, "exposures_found": 1, "coverage": report
    }))).await;
    assert_eq!(done.status, StatusCode::NO_CONTENT, "{}", done.body);

    let seen = send(&app, "GET", &format!("/scans/{job}"), &bearer(&cfg, c, "premium"), None).await.json();
    assert_eq!(seen["job"]["status"], "completed");
    assert_eq!(seen["job"]["coverage"], report);
    assert_eq!(seen["job"]["sources_queued"], 6);
    assert_eq!((seen["job"]["sources_completed"].clone(), seen["job"]["sources_failed"].clone()), (json!(1), json!(4)));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_malformed_coverage_report_is_refused_and_a_stale_worker_cannot_overwrite_it(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "free", false).await;
    let job = ts::scan_job(&pool, c, "queued").await;
    send(&app, "POST", &format!("/internal/scan/{job}/claim"), &agent(&cfg), None).await;
    let put = |coverage: Value, attempt: i32| json!({ "job_id": job, "status": "completed", "attempt": attempt, "coverage": coverage });

    for bad in [
        json!([{ "source": "x.com", "outcome": "mystery" }]),                       // unknown outcome
        json!([{ "source": "https://x.com/Jane-Smith", "outcome": "checked" }]),    // a URL, not a hostname
        json!([{ "source": "", "outcome": "checked" }]),
        json!([{ "source": "a b.com", "outcome": "checked" }]),
        json!((0..101).map(|i| json!({ "source": format!("s{i}.com"), "outcome": "checked" })).collect::<Vec<_>>()),
    ] {
        let r = send(&app, "PUT", "/internal/scan/status", &agent(&cfg), Some(put(bad.clone(), 1))).await;
        assert!(r.status == StatusCode::BAD_REQUEST || r.status == StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {}", r.status);
    }
    let still: String = sqlx::query_scalar("SELECT status::text FROM scan_jobs WHERE id = $1").bind(job).fetch_one(&pool).await.unwrap();
    assert_eq!(still, "running", "a refused report changes nothing");

    // a worker from an older attempt is fenced out of the coverage as well as the status
    let stale = send(&app, "PUT", "/internal/scan/status", &agent(&cfg),
        Some(put(json!([{ "source": "x.com", "outcome": "checked" }]), 2))).await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    let stored: Option<Value> = sqlx::query_scalar("SELECT coverage FROM scan_jobs WHERE id = $1").bind(job).fetch_one(&pool).await.unwrap();
    assert!(stored.is_none(), "nothing stored by the stale worker");

    // and coverage is optional: older workers and failed scans send none
    let ok = send(&app, "PUT", "/internal/scan/status", &agent(&cfg),
        Some(json!({ "job_id": job, "status": "completed", "attempt": 1 }))).await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
}

// ===========================================================================
// "Last scan"
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn starting_a_scan_by_hand_restarts_the_clock_the_rescan_scheduler_counts_from(pool: PgPool) {
    // Redis unreachable on purpose: the job is still created and recovery dispatches it later.
    let (app, cfg) = app_with(pool.clone(), relaxed(), "redis://127.0.0.1:1/0");
    let c = ts::customer(&pool, "premium", true).await;
    ts::identity(&pool, &master(), c, "Jane Smith").await;
    let before: Option<time::OffsetDateTime> = sqlx::query_scalar("SELECT last_scan_at FROM customers WHERE id = $1").bind(c).fetch_one(&pool).await.unwrap();
    assert!(before.is_none());

    let r = send(&app, "POST", "/scans", &bearer(&cfg, c, "premium"), None).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.body);

    let after: Option<time::OffsetDateTime> = sqlx::query_scalar("SELECT last_scan_at FROM customers WHERE id = $1").bind(c).fetch_one(&pool).await.unwrap();
    let stamp = after.expect("a manual scan records when the customer was last scanned");
    assert!((time::OffsetDateTime::now_utc() - stamp).whole_seconds().abs() < 60);

    // so the scheduler does not scan them again straight away
    let due: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM customers WHERE id = $1 AND subscription_tier = 'premium' \
         AND (last_scan_at IS NULL OR last_scan_at < NOW() - INTERVAL '7 days')",
    ).bind(c).fetch_one(&pool).await.unwrap();
    assert_eq!(due, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_last_scan_shown_is_when_the_scan_finished_not_when_it_was_queued(pool: PgPool) {
    use time::format_description::well_known::Rfc3339;
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "free", false).await;
    let job = ts::scan_job(&pool, c, "completed").await;
    sqlx::query("UPDATE scan_jobs SET queued_at = NOW() - interval '2 days', completed_at = NOW() - interval '1 hour' WHERE id = $1")
        .bind(job).execute(&pool).await.unwrap();
    // a newer scan that did not complete does not count as the last known scan
    let failed = ts::scan_job(&pool, c, "failed").await;
    sqlx::query("UPDATE scan_jobs SET queued_at = NOW(), completed_at = NOW() WHERE id = $1").bind(failed).execute(&pool).await.unwrap();

    let summary = send(&app, "GET", "/exposures/summary", &bearer(&cfg, c, "free"), None).await.json();
    let shown = time::OffsetDateTime::parse(summary["summary"]["last_scan_at"].as_str().unwrap(), &Rfc3339).unwrap();
    let age = (time::OffsetDateTime::now_utc() - shown).whole_minutes();
    assert!((55..=65).contains(&age), "about 1 hour ago (finished), not 2 days ago (queued): {age} minutes");
}

// ===========================================================================
// Plan differences that the pricing page promises
// ===========================================================================

mod plan_limits {
    use super::*;

    async fn authorize(app: &Router, cfg: &AppConfig, c: Uuid, tier: &str, exposure: Uuid) -> Reply {
        send(app, "POST", &format!("/removal-requests/{exposure}/authorize"), &bearer(cfg, c, tier), None).await
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_free_account_gets_three_opt_outs_then_is_asked_to_upgrade(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        for n in 1..=3 {
            let e = ts::exposure(&pool, c, true, None).await;
            let r = authorize(&app, &cfg, c, "free", e).await;
            assert_eq!(r.status, StatusCode::OK, "opt-out {n}: {}", r.body);
        }
        let e4 = ts::exposure(&pool, c, true, None).await;
        let r = authorize(&app, &cfg, c, "free", e4).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        // (plain text here; middleware::apply turns it into {"message": ...} in the running app)
        assert!(r.body.contains("Upgrade to Personal") && r.body.contains("3 opt-out"), "{}", r.body);
        let made: i64 = sqlx::query_scalar("SELECT count(*) FROM removal_requests WHERE customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(made, 3, "the refused one was not created");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_opt_out_does_not_use_up_the_free_allowance(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        for _ in 0..3 {
            let e = ts::exposure(&pool, c, true, None).await;
            ts::removal_request(&pool, c, e, "failed", None).await;
        }
        let e = ts::exposure(&pool, c, true, None).await;
        assert_eq!(authorize(&app, &cfg, c, "free", e).await.status, StatusCode::OK, "failed requests do not count");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn paid_accounts_have_no_opt_out_limit_and_other_accounts_do_not_share_one(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (paid, other_free) = (ts::customer(&pool, "personal", true).await, ts::customer(&pool, "free", false).await);
        for _ in 0..5 {
            let e = ts::exposure(&pool, paid, true, None).await;
            assert_eq!(authorize(&app, &cfg, paid, "personal", e).await.status, StatusCode::OK);
        }
        let e = ts::exposure(&pool, other_free, true, None).await;
        assert_eq!(authorize(&app, &cfg, other_free, "free", e).await.status, StatusCode::OK);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn simultaneous_requests_cannot_slip_past_the_free_limit(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        let mut exposures = vec![];
        for _ in 0..6 {
            exposures.push(ts::exposure(&pool, c, true, None).await);
        }
        let replies = futures_util_join(&app, &cfg, c, &exposures).await;
        let ok = replies.iter().filter(|s| **s == StatusCode::OK).count();
        assert_eq!(ok, 3, "exactly the allowance, however the requests interleave: {replies:?}");
    }

    async fn futures_util_join(app: &Router, cfg: &AppConfig, c: Uuid, exposures: &[Uuid]) -> Vec<StatusCode> {
        let mut handles = vec![];
        for e in exposures {
            let (app, cfg, e) = (app.clone(), cfg.clone(), *e);
            handles.push(tokio::spawn(async move { authorize(&app, &cfg, c, "free", e).await.status }));
        }
        let mut out = vec![];
        for h in handles {
            out.push(h.await.unwrap());
        }
        out
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn premium_requests_are_handed_to_the_agents_before_older_personal_and_free_ones(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let mut ids = vec![];
        // created oldest first: free, personal, premium
        for tier in ["free", "personal", "premium"] {
            let c = ts::customer(&pool, tier, tier != "free").await;
            let e = ts::exposure(&pool, c, true, None).await;
            ids.push((tier, ts::removal_request(&pool, c, e, "authorized", None).await));
            sqlx::query("UPDATE removal_requests SET created_at = NOW() - make_interval(secs => $2) WHERE id = $1")
                .bind(ids.last().unwrap().1).bind(match tier { "free" => 300.0_f64, "personal" => 200.0, _ => 100.0 })
                .execute(&pool).await.unwrap();
        }
        let r = send(&app, "GET", "/internal/removal-requests?status=authorized", &agent(&cfg), None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        let order: Vec<String> = r.json()["requests"].as_array().unwrap().iter()
            .map(|x| x["id"].as_str().unwrap().to_string()).collect();
        let want: Vec<String> = ["premium", "personal", "free"].iter()
            .map(|t| ids.iter().find(|(tier, _)| tier == t).unwrap().1.to_string()).collect();
        assert_eq!(order, want);
    }
}
