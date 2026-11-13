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
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.body);
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
    send(app, "POST", "/auth/register", &[], Some(json!({ "email": email, "password": PASSWORD, "accepted_terms_version": crate::models::customer::TERMS_VERSION }))).await
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

#[sqlx::test(migrations = "./migrations")]
async fn registration_requires_accepting_the_current_terms_and_records_it(pool: PgPool) {
    let (app, _) = app(pool.clone());
    let body = |v: Option<&str>| match v {
        Some(v) => json!({ "email": "t@example.com", "password": PASSWORD, "accepted_terms_version": v }),
        None => json!({ "email": "t@example.com", "password": PASSWORD }),
    };
    for bad in [None, Some(""), Some("2020-01-old")] {
        let r = send(&app, "POST", "/auth/register", &[], Some(body(bad))).await;
        assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad:?}");
        assert!(r.body.contains("Terms of Service"), "{}", r.body);
    }
    let none: i64 = sqlx::query_scalar("SELECT count(*) FROM customers").fetch_one(&pool).await.unwrap();
    assert_eq!(none, 0, "a refused registration creates no account");

    let ok = send(&app, "POST", "/auth/register", &[], Some(body(Some(crate::models::customer::TERMS_VERSION)))).await;
    assert_eq!(ok.status, StatusCode::CREATED);
    let (v, at): (Option<String>, Option<time::OffsetDateTime>) =
        sqlx::query_as("SELECT terms_version, terms_accepted_at FROM customers").fetch_one(&pool).await.unwrap();
    assert_eq!(v.as_deref(), Some(crate::models::customer::TERMS_VERSION));
    assert!(at.is_some());
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
    assert_eq!(send(&app, "POST", "/auth/register", &[], Some(json!({ "email": "z@example.com", "password": PASSWORD, "accepted_terms_version": crate::models::customer::TERMS_VERSION }))).await.status, StatusCode::TOO_MANY_REQUESTS);
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

// ===========================================================================
// Public coverage figure (shown on the pricing page)
// ===========================================================================

mod public_coverage {
    use super::*;

    async fn scan_with(pool: &PgPool, c: Uuid, status: &str, days_ago: i32, coverage: Value) {
        let job = ts::scan_job(pool, c, status).await;
        sqlx::query("UPDATE scan_jobs SET coverage = $2, queued_at = NOW() - make_interval(days => $3) WHERE id = $1")
            .bind(job).bind(coverage).bind(days_ago).execute(pool).await.unwrap();
    }

    fn entry(source: &str, outcome: &str) -> Value { json!({ "source": source, "outcome": outcome }) }

    #[sqlx::test(migrations = "./migrations")]
    async fn with_no_scans_nothing_is_claimed(pool: PgPool) {
        let (app, _) = app(pool);
        let r = send(&app, "GET", "/public/coverage", &[], None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.json()["scans"], 0);
        assert_eq!(r.json()["sources"], json!([]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_summarises_recent_scans_across_customers_without_needing_a_session(pool: PgPool) {
        let (app, _) = app(pool.clone());
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "premium", true).await);
        scan_with(&pool, a, "completed", 1, json!([entry("spokeo.com", "checked"), entry("whitepages.com", "blocked"), entry("peoplefinder.com", "not_supported")])).await;
        scan_with(&pool, b, "completed", 2, json!([entry("spokeo.com", "checked"), entry("whitepages.com", "blocked"), entry("radaris.com", "checked")])).await;
        // a failed-analysis scan still read its pages; an old scan, a running one and one with no coverage do not count
        scan_with(&pool, a, "failed", 3, json!([entry("radaris.com", "checked")])).await;
        scan_with(&pool, a, "completed", 45, json!([entry("whitepages.com", "checked")])).await;
        scan_with(&pool, a, "running", 0, json!([entry("whitepages.com", "checked")])).await;
        scan_with(&pool, a, "completed", 1, Value::Null).await;

        let r = send(&app, "GET", "/public/coverage", &[], None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.headers.get("cache-control").unwrap(), "public, max-age=300");
        let j = r.json();
        assert_eq!((j["scans"].clone(), j["window_days"].clone()), (json!(3), json!(30)));
        let status: std::collections::BTreeMap<String, bool> = j["sources"].as_array().unwrap().iter()
            .map(|s| (s["source"].as_str().unwrap().to_string(), s["readable"].as_bool().unwrap())).collect();
        assert_eq!(status, [("peoplefinder.com", false), ("radaris.com", true), ("spokeo.com", true), ("whitepages.com", false)]
            .iter().map(|(k, v)| (k.to_string(), *v)).collect());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_source_is_readable_when_at_least_half_of_the_scans_that_tried_it_read_it(pool: PgPool) {
        let (app, _) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        for outcome in ["checked", "blocked", "checked", "blocked", "blocked"] {
            scan_with(&pool, c, "completed", 1, json!([entry("flaky.com", outcome)])).await;
        }
        let j = send(&app, "GET", "/public/coverage", &[], None).await.json();
        assert_eq!(j["sources"][0]["readable"], false, "2 of 5 is not enough");
        scan_with(&pool, c, "completed", 1, json!([entry("flaky.com", "checked")])).await;
        let j = send(&app, "GET", "/public/coverage", &[], None).await.json();
        assert_eq!(j["sources"][0]["readable"], true, "3 of 6 is exactly half, and half counts");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_response_identifies_no_customer_and_one_odd_row_cannot_break_it(pool: PgPool) {
        let (app, _) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        scan_with(&pool, c, "completed", 1, json!([entry("spokeo.com", "checked")])).await;
        scan_with(&pool, c, "completed", 1, json!({ "not": "an array" })).await;
        scan_with(&pool, c, "completed", 1, json!("scalar")).await;
        scan_with(&pool, c, "completed", 1, json!([{ "outcome": "checked" }])).await; // no source name
        let r = send(&app, "GET", "/public/coverage", &[], None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert!(!r.body.contains(&c.to_string()), "no customer id in a public response");
        assert_eq!(r.json()["sources"].as_array().unwrap().len(), 1);
    }
}

// ===========================================================================
// Opt-out page health (nightly check of brokers' public opt-out pages)
// ===========================================================================

mod optout_health {
    use super::*;

    fn check(source: &str, status: &str, http: Option<i32>) -> Value {
        json!({ "source": source, "status": status, "http_status": http })
    }

    async fn report(app: &Router, cfg: &AppConfig, checks: Vec<Value>) -> Reply {
        send(app, "PUT", "/internal/optout-health", &agent(cfg), Some(json!({ "checks": checks }))).await
    }

    async fn row(pool: &PgPool, source: &str) -> (String, Option<i32>, bool, i32) {
        sqlx::query_as("SELECT status, http_status, last_reachable_at IS NOT NULL, consecutive_failures FROM optout_health WHERE source = $1")
            .bind(source).fetch_one(pool).await.unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn only_an_agent_can_report(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let body = Some(json!({ "checks": [check("spokeo.com", "reachable", Some(200))] }));
        assert_eq!(send(&app, "PUT", "/internal/optout-health", &[], body.clone()).await.status, StatusCode::UNAUTHORIZED);
        let c = ts::customer(&pool, "premium", true).await;
        assert_eq!(send(&app, "PUT", "/internal/optout-health", &bearer(&cfg, c, "premium"), body).await.status,
            StatusCode::UNAUTHORIZED, "a customer session is not an agent key");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM optout_health").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_report_is_stored_and_anyone_can_read_it_without_a_session(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let r = report(&app, &cfg, vec![
            check("spokeo.com", "captcha", Some(200)), check("whitepages.com", "blocked", Some(403)),
            check("ownerly.com", "not_found", Some(404)), check("radaris.com", "reachable", Some(200)),
            check("nuwber.com", "unavailable", None),
        ]).await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);

        let page = send(&app, "GET", "/public/optout-health", &[], None).await;
        assert_eq!(page.status, StatusCode::OK);
        assert_eq!(page.headers.get("cache-control").unwrap(), "public, max-age=300");
        let j = page.json();
        let by: std::collections::BTreeMap<String, String> = j["checks"].as_array().unwrap().iter()
            .map(|c| (c["source"].as_str().unwrap().to_string(), c["status"].as_str().unwrap().to_string())).collect();
        assert_eq!(by["spokeo.com"], "captcha");
        assert_eq!(by["whitepages.com"], "blocked");
        assert_eq!(by["ownerly.com"], "not_found");
        assert_eq!(by["radaris.com"], "reachable");
        assert_eq!(by["nuwber.com"], "unavailable");
        assert!(j["checks"][0]["checked_at"].as_str().unwrap().contains('T'), "RFC 3339, like every other date");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_last_time_a_page_opened_survives_later_failures_and_failures_are_counted(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        report(&app, &cfg, vec![check("radaris.com", "reachable", Some(200))]).await;
        assert_eq!(row(&pool, "radaris.com").await, ("reachable".into(), Some(200), true, 0));

        for n in 1..=3 {
            report(&app, &cfg, vec![check("radaris.com", "not_found", Some(404))]).await;
            assert_eq!(row(&pool, "radaris.com").await, ("not_found".into(), Some(404), true, n),
                "still remembers it last opened; {n} failures in a row");
        }
        report(&app, &cfg, vec![check("radaris.com", "captcha", Some(200))]).await;
        assert_eq!(row(&pool, "radaris.com").await.3, 0, "a page that opens (even with a CAPTCHA) resets the count");

        // a source that has never opened has no last-reachable time
        report(&app, &cfg, vec![check("dead.example", "blocked", Some(403))]).await;
        assert_eq!(row(&pool, "dead.example").await, ("blocked".into(), Some(403), false, 1));
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM optout_health WHERE source = 'radaris.com'").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 1, "one row per source, overwritten");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_bad_report_is_refused_whole_and_stores_nothing(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        for bad in [
            vec![check("good.com", "reachable", Some(200)), check("https://x.com/opt-out", "reachable", None)],  // a URL, not a host
            vec![check("good.com", "reachable", Some(200)), check("", "reachable", None)],
            vec![check("good.com", "reachable", Some(200)), check("a b.com", "reachable", None)],
            vec![check("good.com", "reachable", Some(99))],
            vec![check("good.com", "reachable", Some(600))],
            (0..201).map(|i| check(&format!("s{i}.com"), "reachable", Some(200))).collect(),
        ] {
            let r = report(&app, &cfg, bad).await;
            assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.body);
        }
        // an unknown status never gets as far as the handler
        let r = report(&app, &cfg, vec![check("good.com", "working_great", Some(200))]).await;
        assert!(r.status == StatusCode::BAD_REQUEST || r.status == StatusCode::UNPROCESSABLE_ENTITY, "{}", r.status);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM optout_health").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0, "nothing from any refused report, including its valid entries");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn with_nothing_reported_the_public_list_is_empty(pool: PgPool) {
        let (app, _) = app(pool);
        let r = send(&app, "GET", "/public/optout-health", &[], None).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.json()["checks"], json!([]));
    }
}

// ===========================================================================
// Residence (state only) for the DROP card
// ===========================================================================

mod residence {
    use super::*;
    use crate::models::customer::SubmitIdentityRequest;
    use crate::services::customer as customer_svc;

    async fn save(pool: &PgPool, c: Uuid, address: &str) {
        let req = SubmitIdentityRequest {
            full_name: "Jane Smith".into(), current_address: address.into(), previous_addresses: vec![],
            phone_numbers: vec![], email_addresses: vec!["jane@example.com".into()], date_of_birth: None,
        };
        customer_svc::save_identity(pool, &master(), &req, c).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_returns_only_the_state_of_the_saved_address(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        let who = bearer(&cfg, c, "free");

        assert_eq!(send(&app, "GET", "/me/residence", &who, None).await.json()["state"], Value::Null, "no identity yet");

        save(&pool, c, "742 Evergreen Terrace, Springfield, CA 90210").await;
        let r = send(&app, "GET", "/me/residence", &who, None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.json(), json!({ "state": "CA" }));
        for leaked in ["Evergreen", "Springfield", "90210", "Terrace"] {
            assert!(!r.body.contains(leaked), "the address must not appear: {}", r.body);
        }

        save(&pool, c, "1 Main St, Austin, TX 78701").await;
        assert_eq!(send(&app, "GET", "/me/residence", &who, None).await.json()["state"], "TX");

        save(&pool, c, "12 Rue de Rivoli, Paris, France").await;
        assert_eq!(send(&app, "GET", "/me/residence", &who, None).await.json()["state"], Value::Null, "cannot tell: says so");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_needs_a_session_and_only_ever_reads_the_callers_own_address(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (alice, bob) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        save(&pool, alice, "1 Main St, Los Angeles, CA 90001").await;
        save(&pool, bob, "1 Main St, Austin, TX 78701").await;
        assert_eq!(send(&app, "GET", "/me/residence", &[], None).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, "GET", "/me/residence", &bearer(&cfg, alice, "free"), None).await.json()["state"], "CA");
        assert_eq!(send(&app, "GET", "/me/residence", &bearer(&cfg, bob, "free"), None).await.json()["state"], "TX");
        // there is no way to name another customer
        let r = send(&app, "GET", &format!("/me/residence?customer_id={bob}"), &bearer(&cfg, alice, "free"), None).await;
        assert_eq!(r.json()["state"], "CA", "a query parameter changes nothing");
    }
}

mod optout_health_kinds {
    use super::*;

    fn check(kind: Option<&str>, source: &str, status: &str) -> Value {
        match kind {
            Some(k) => json!({ "kind": k, "source": source, "status": status, "http_status": 200 }),
            None => json!({ "source": source, "status": status, "http_status": 200 }),
        }
    }

    async fn list(app: &Router, query: &str) -> Vec<(String, String)> {
        let r = send(app, "GET", &format!("/public/optout-health{query}"), &[], None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        r.json()["checks"].as_array().unwrap().iter()
            .map(|c| (c["source"].as_str().unwrap().to_string(), c["status"].as_str().unwrap().to_string())).collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_two_kinds_of_page_never_overwrite_or_leak_into_each_other(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        // a registry company's id can equal nothing of ours, but a name could: same source, different kind
        let r = send(&app, "PUT", "/internal/optout-health", &agent(&cfg), Some(json!({ "checks": [
            check(None, "spokeo.com", "reachable"),                 // an older worker: no kind means optout
            check(Some("optout"), "radaris.com", "captcha"),
            check(Some("ca_rights"), "1461", "not_found"),
            check(Some("ca_rights"), "spokeo.com", "blocked"),      // same name, other kind
        ] }))).await;
        assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.body);

        let mut own = list(&app, "").await;
        own.sort();
        assert_eq!(own, vec![("radaris.com".into(), "captcha".into()), ("spokeo.com".into(), "reachable".into())],
            "the default view is the brokers' own pages, untouched by the registry's same-named entry");
        assert_eq!(list(&app, "?kind=optout").await.len(), 2);
        let mut ca = list(&app, "?kind=ca_rights").await;
        ca.sort();
        assert_eq!(ca, vec![("1461".into(), "not_found".into()), ("spokeo.com".into(), "blocked".into())]);

        // a second report updates in place, per kind
        send(&app, "PUT", "/internal/optout-health", &agent(&cfg), Some(json!({ "checks": [check(Some("ca_rights"), "1461", "reachable")] }))).await;
        assert!(list(&app, "?kind=ca_rights").await.contains(&("1461".into(), "reachable".into())));
        assert_eq!(list(&app, "").await.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_kind_is_refused(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let r = send(&app, "PUT", "/internal/optout-health", &agent(&cfg), Some(json!({ "checks": [check(Some("everything"), "x.com", "reachable")] }))).await;
        assert!(r.status == StatusCode::BAD_REQUEST || r.status == StatusCode::UNPROCESSABLE_ENTITY, "{}", r.status);
        let r = send(&app, "GET", "/public/optout-health?kind=everything", &[], None).await;
        assert!(r.status.is_client_error(), "{}", r.status);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM optout_health").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }
}

// ===========================================================================
// Deletion requests sent to companies on the customer's behalf
// ===========================================================================

mod deletion_requests {
    use super::*;
    use crate::models::customer::SubmitIdentityRequest;
    use crate::services::{customer as customer_svc, deletion};

    fn enabled_app(pool: PgPool) -> (Router, AppConfig) {
        let mut c = relaxed();
        c.deletion_requests_enabled = true;
        app_with(pool, c, &redis_url())
    }

    async fn save_identity(pool: &PgPool, c: Uuid, address: &str) {
        let req = SubmitIdentityRequest {
            full_name: "Jane Marie Smith".into(), current_address: address.into(),
            previous_addresses: vec!["45 Oak Ave, Dallas, TX 75201".into()],
            phone_numbers: vec!["512-555-0147".into()],
            email_addresses: vec!["jane.smith@example.com".into(), "second@example.com".into()],
            date_of_birth: Some("1984-03-09".into()),
        };
        customer_svc::save_identity(pool, &master(), &req, c).await.unwrap();
    }

    /// A paid customer who lives in California.
    async fn californian(pool: &PgPool) -> Uuid {
        let c = ts::customer(pool, "personal", true).await;
        save_identity(pool, c, "742 Evergreen Terrace, Springfield, CA 90210").await;
        c
    }

    fn body(ids: &[&str], address: bool) -> Value {
        json!({ "channel_ids": ids, "include_address": address, "consent": true,
                "authorization_version": deletion::AUTHORIZATION_VERSION })
    }

    async fn campaign(app: &Router, cfg: &AppConfig, c: Uuid, b: Value) -> Reply {
        send(app, "POST", "/deletion-campaigns", &bearer(cfg, c, "personal"), Some(b)).await
    }

    async fn status_of(pool: &PgPool, id: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM deletion_requests WHERE id = $1").bind(id).fetch_one(pool).await.unwrap()
    }

    async fn only_request(pool: &PgPool, c: Uuid) -> Uuid {
        sqlx::query_scalar("SELECT id FROM deletion_requests WHERE customer_id = $1").bind(c).fetch_one(pool).await.unwrap()
    }

    // ---- off by default ----------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn everything_is_off_until_it_is_switched_on(pool: PgPool) {
        let (app, cfg) = app(pool.clone());           // the default config
        let c = californian(&pool).await;
        let who = bearer(&cfg, c, "personal");
        for (method, uri) in [("GET", "/deletion-requests/info"), ("GET", "/deletion-requests")] {
            assert_eq!(send(&app, method, uri, &who, None).await.status, StatusCode::NOT_FOUND, "{uri}");
        }
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::NOT_FOUND);
        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await;
        assert_eq!((q.status, q.json()["requests"].clone()), (StatusCode::OK, json!([])), "the worker just sees nothing");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deletion_requests").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_needs_a_session_or_the_agent_key(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        assert_eq!(send(&app, "GET", "/deletion-requests/info", &[], None).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, "POST", "/deletion-campaigns", &[], Some(body(&["whitepages"], false))).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, "GET", "/internal/deletion-requests", &[], None).await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, "GET", "/internal/deletion-requests", &bearer(&cfg, c, "personal"), None).await.status,
            StatusCode::UNAUTHORIZED, "a customer session is not an agent key");
    }

    // ---- what the customer is shown ----------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn the_customer_sees_the_text_the_recipient_and_the_evidence_and_whether_they_qualify(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let ca = californian(&pool).await;
        let tx = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, tx, "1 Main St, Austin, TX 78701").await;
        let nowhere = ts::customer(&pool, "free", false).await;

        let j = send(&app, "GET", "/deletion-requests/info", &bearer(&cfg, ca, "personal"), None).await.json();
        assert_eq!(j["authorization"]["version"], deletion::AUTHORIZATION_VERSION);
        assert!(j["authorization"]["text"].as_str().unwrap().contains("will not share my phone numbers or date of birth"));
        assert_eq!(j["plan_ok"], true);
        let w = &j["channels"][0];
        assert_eq!((w["id"].clone(), w["address"].clone(), w["eligible"].clone()), (json!("whitepages"), json!("privacyrequest@whitepages.com"), json!(true)));
        assert!(w["evidence"].as_str().unwrap().contains("privacyrequest@whitepages.com"));
        assert!(w["source_url"].as_str().unwrap().starts_with("https://www.whitepages.com/"));

        let j = send(&app, "GET", "/deletion-requests/info", &bearer(&cfg, tx, "personal"), None).await.json();
        assert_eq!(j["channels"][0]["eligible"], false);
        assert!(j["channels"][0]["ineligible_reason"].as_str().unwrap().contains("other states"));

        let j = send(&app, "GET", "/deletion-requests/info", &bearer(&cfg, nowhere, "free"), None).await.json();
        assert_eq!((j["plan_ok"].clone(), j["channels"][0]["eligible"].clone()), (json!(false), json!(false)));
        assert!(j["channels"][0]["ineligible_reason"].as_str().unwrap().contains("could not tell your state"));
    }

    // ---- authorizing a campaign --------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn an_authorized_campaign_queues_one_request_per_company_and_keeps_the_exact_text_accepted(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        let r = campaign(&app, &cfg, c, body(&["whitepages", "whitepages"], true)).await;      // listed twice
        assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.body);
        assert_eq!(r.json()["requests"].as_array().unwrap().len(), 1, "a company is never queued twice");

        let (version, text, sha, addr): (String, String, String, bool) = sqlx::query_as(
            "SELECT authorization_version, authorization_text, authorization_sha256, include_address FROM deletion_campaigns WHERE customer_id = $1")
            .bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!((version.as_str(), text.as_str(), addr), (deletion::AUTHORIZATION_VERSION, deletion::AUTHORIZATION_TEXT, true));
        assert_eq!(sha, deletion::authorization_sha256());

        let audit: Value = sqlx::query_scalar("SELECT details FROM audit_log WHERE action = 'deletion.authorize' AND customer_id = $1")
            .bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(audit["channels"], json!(["whitepages", "whitepages"]));
        assert!(!audit.to_string().contains("Jane") && !audit.to_string().contains("Evergreen"), "no personal data in the audit trail");

        let list = send(&app, "GET", "/deletion-requests", &bearer(&cfg, c, "personal"), None).await.json();
        assert_eq!((list["requests"][0]["status"].clone(), list["requests"][0]["company"].clone()), (json!("queued"), json!("Whitepages, Inc")));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn every_refusal_leaves_nothing_behind(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        let free = ts::customer(&pool, "free", false).await;
        save_identity(&pool, free, "1 Main St, Los Angeles, CA 90001").await;
        let tx = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, tx, "1 Main St, Austin, TX 78701").await;
        let unknown = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, unknown, "12 Rue de Rivoli, Paris, France").await;

        let mut no_consent = body(&["whitepages"], false);
        no_consent["consent"] = json!(false);
        let mut old_text = body(&["whitepages"], false);
        old_text["authorization_version"] = json!("2020-01-old");

        for (what, who, tier, b, want) in [
            ("no consent", c, "personal", no_consent, StatusCode::UNPROCESSABLE_ENTITY),
            ("an older authorization text", c, "personal", old_text, StatusCode::CONFLICT),
            ("nothing selected", c, "personal", body(&[], false), StatusCode::UNPROCESSABLE_ENTITY),
            ("a company that is not in the catalog", c, "personal", body(&["whitepages", "evil-corp"], false), StatusCode::UNPROCESSABLE_ENTITY),
            ("a free plan", free, "free", body(&["whitepages"], false), StatusCode::FORBIDDEN),
            ("a state the page does not offer it to", tx, "personal", body(&["whitepages"], false), StatusCode::FORBIDDEN),
            ("a state that cannot be told", unknown, "personal", body(&["whitepages"], false), StatusCode::FORBIDDEN),
            ("too many companies", c, "personal", json!({ "channel_ids": (0..51).map(|i| format!("c{i}")).collect::<Vec<_>>(),
                "include_address": false, "consent": true, "authorization_version": deletion::AUTHORIZATION_VERSION }), StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            let r = send(&app, "POST", "/deletion-campaigns", &bearer(&cfg, who, tier), Some(b)).await;
            assert_eq!(r.status, want, "{what}: {}", r.body);
        }
        let n: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM deletion_requests) + (SELECT count(*) FROM deletion_campaigns)")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0, "no refused attempt created anything");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_company_is_not_asked_twice_within_ninety_days_but_a_plain_failure_does_not_count(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::ACCEPTED);
        let again = campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        assert_eq!(again.status, StatusCode::CONFLICT, "still queued");
        assert!(again.body.contains("90 days"));
        let id = only_request(&pool, c).await;

        sqlx::query("UPDATE deletion_requests SET status = 'sent', sent_at = NOW() WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::CONFLICT, "sent");

        sqlx::query("UPDATE deletion_requests SET status = 'failed', error = 'SMTP is not configured' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::ACCEPTED, "it plainly never left");

        sqlx::query("UPDATE deletion_requests SET status = 'failed', error = $2 WHERE campaign_id = (SELECT campaign_id FROM deletion_requests ORDER BY queued_at LIMIT 1) AND id = $1")
            .bind(id).bind(format!("{} sending was interrupted", deletion::ABANDONED_MARKER)).execute(&pool).await.unwrap();
        sqlx::query("UPDATE deletion_requests SET status = 'failed', error = 'x' WHERE id <> $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::CONFLICT,
            "an interrupted send may have gone out, so the cooldown holds");

        sqlx::query("UPDATE deletion_requests SET queued_at = NOW() - interval '91 days'").execute(&pool).await.unwrap();
        assert_eq!(campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status, StatusCode::ACCEPTED, "after the cooldown");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_campaigns_at_once_cannot_both_get_through(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        let mut handles = vec![];
        for _ in 0..6 {
            let (app, cfg) = (app.clone(), cfg.clone());
            handles.push(tokio::spawn(async move { campaign(&app, &cfg, c, body(&["whitepages"], false)).await.status }));
        }
        let mut accepted = 0;
        for h in handles {
            if h.await.unwrap() == StatusCode::ACCEPTED { accepted += 1; }
        }
        assert_eq!(accepted, 1);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deletion_requests").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 1);
    }

    // ---- the companies found on their own pages -----------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn general_channels_are_offered_to_everyone_and_california_only_ones_only_to_californians(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let ca = californian(&pool).await;
        let tx = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, tx, "1 Main St, Austin, TX 78701").await;
        let unknown = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, unknown, "12 Rue de Rivoli, Paris, France").await;

        let eligible_for = |who: Uuid| {
            let (app, cfg) = (app.clone(), cfg.clone());
            async move {
                let j = send(&app, "GET", "/deletion-requests/info", &bearer(&cfg, who, "personal"), None).await.json();
                let mut ids: Vec<String> = j["channels"].as_array().unwrap().iter()
                    .filter(|c| c["eligible"] == true).map(|c| c["id"].as_str().unwrap().to_string()).collect();
                ids.sort();
                ids
            }
        };
        let general = vec!["dealmachine", "realsource", "stirista"];
        let california = vec!["dealmachine", "realsource", "recruitbot", "snovio", "stirista", "whitepages"];
        assert_eq!(eligible_for(ca).await, california);
        assert_eq!(eligible_for(tx).await, general, "the pages offer the other routes to California residents");
        assert_eq!(eligible_for(unknown).await, general, "an unknown state is never assumed to be California");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_company_that_needs_the_address_is_not_sent_without_it(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        let info = send(&app, "GET", "/deletion-requests/info", &bearer(&cfg, c, "personal"), None).await.json();
        let needs: Vec<&str> = info["channels"].as_array().unwrap().iter()
            .filter(|ch| ch["requires_address"] == true).map(|ch| ch["id"].as_str().unwrap()).collect();
        assert_eq!(needs, vec!["stirista"]);

        let no = campaign(&app, &cfg, c, body(&["realsource", "stirista"], false)).await;
        assert_eq!(no.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", no.body);
        assert!(no.body.contains("needs your address"));
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM deletion_requests").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0, "nothing was queued, not even the company that did not need it");
        assert_eq!(campaign(&app, &cfg, c, body(&["realsource"], false)).await.status, StatusCode::ACCEPTED, "one that does not need it is fine");
        assert_eq!(campaign(&app, &cfg, c, body(&["stirista"], true)).await.status, StatusCode::ACCEPTED);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_queue_carries_each_companys_own_address_and_subject_line(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["dealmachine", "realsource", "stirista", "recruitbot", "snovio", "whitepages"], true)).await;
        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json();
        let by: std::collections::BTreeMap<String, (String, Value)> = q["requests"].as_array().unwrap().iter()
            .map(|r| (r["channel_id"].as_str().unwrap().to_string(), (r["to_address"].as_str().unwrap().to_string(), r["subject"].clone()))).collect();
        assert_eq!(by.len(), 6);
        assert_eq!(by["dealmachine"], ("privacy@dealmachine.com".into(), json!("Privacy Rights Request")));
        assert_eq!(by["stirista"], ("privacy@stirista.com".into(), Value::Null));
        assert_eq!(by["recruitbot"].0, "ccpa@recruitbot.com");
        assert_eq!(by["snovio"].0, "help@snov.io");
        assert_eq!(by["realsource"].0, "privacy@realsourcedata.com");
        assert_eq!(by["whitepages"].0, "privacyrequest@whitepages.com");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_texan_cannot_be_sent_to_the_california_only_companies_even_alongside_general_ones(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let tx = ts::customer(&pool, "personal", true).await;
        save_identity(&pool, tx, "1 Main St, Austin, TX 78701").await;
        let r = campaign(&app, &cfg, tx, body(&["realsource", "recruitbot"], false)).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.body);
        assert_eq!(campaign(&app, &cfg, tx, body(&["realsource", "dealmachine"], false)).await.status, StatusCode::ACCEPTED);
    }

    // ---- the worker ------------------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn the_worker_gets_the_address_from_the_catalog_and_claims_each_request_once(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], true)).await;
        let id = only_request(&pool, c).await;

        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json();
        let item = &q["requests"][0];
        assert_eq!(item["id"], id.to_string());
        assert_eq!(item["to_address"], "privacyrequest@whitepages.com");
        assert_eq!(item["include_address"], true);
        assert_eq!(item["authorization_text"], deletion::AUTHORIZATION_TEXT);
        assert!(!q.to_string().contains("Jane") && !q.to_string().contains("Evergreen"), "the queue carries no personal data");

        let claim = format!("/internal/deletion-requests/{id}/claim");
        assert_eq!(send(&app, "POST", &claim, &agent(&cfg), None).await.status, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, "POST", &claim, &agent(&cfg), None).await.status, StatusCode::CONFLICT, "a second worker is turned away");
        assert_eq!(status_of(&pool, id).await, "sending");
        assert_eq!(send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json()["requests"], json!([]), "no longer queued");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn identity_is_released_only_for_a_claimed_request_and_only_what_the_customer_agreed_to_share(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;                // address NOT shared
        let id = only_request(&pool, c).await;
        let ident = format!("/internal/deletion-requests/{id}/identity");

        assert_eq!(send(&app, "POST", &ident, &agent(&cfg), None).await.status, StatusCode::FORBIDDEN, "not claimed yet");
        send(&app, "POST", &format!("/internal/deletion-requests/{id}/claim"), &agent(&cfg), None).await;
        let r = send(&app, "POST", &ident, &agent(&cfg), None).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        let j = r.json();
        assert_eq!(j["full_name"], "Jane Marie Smith");
        assert_eq!(j["email_addresses"], json!(["jane.smith@example.com"]), "only the first email address");
        assert_eq!(j["current_address"], "", "the customer did not agree to share their address");
        for never in ["512-555-0147", "1984-03-09", "Dallas", "second@example.com", "Evergreen", "90210"] {
            assert!(!r.body.contains(never), "must not be released: {never}");
        }
        let released: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'identity.release' AND status = 'success' AND details->>'purpose' = 'deletion'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(released, 1, "the release is audited with its purpose");
        let denied: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'identity.release' AND status = 'failure'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(denied, 1, "so is the refusal");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_address_goes_out_only_when_the_customer_ticked_the_box(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], true)).await;
        let id = only_request(&pool, c).await;
        send(&app, "POST", &format!("/internal/deletion-requests/{id}/claim"), &agent(&cfg), None).await;
        let j = send(&app, "POST", &format!("/internal/deletion-requests/{id}/identity"), &agent(&cfg), None).await.json();
        assert_eq!(j["current_address"], "742 Evergreen Terrace, Springfield, CA 90210");
        assert!(j.get("phone_numbers").is_none() && j.get("date_of_birth").is_none() && j.get("previous_addresses").is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_result_can_only_be_recorded_once_for_a_request_being_sent_and_a_send_is_audited(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        let id = only_request(&pool, c).await;
        let result = format!("/internal/deletion-requests/{id}/result");
        let ok = json!({ "sent": true, "message_id": "<abc@vectorscrub.example>" });

        assert_eq!(send(&app, "PUT", &result, &agent(&cfg), Some(ok.clone())).await.status, StatusCode::CONFLICT, "never claimed");
        send(&app, "POST", &format!("/internal/deletion-requests/{id}/claim"), &agent(&cfg), None).await;
        assert_eq!(send(&app, "PUT", &result, &agent(&cfg), Some(ok.clone())).await.status, StatusCode::NO_CONTENT);
        assert_eq!(status_of(&pool, id).await, "sent");
        assert_eq!(send(&app, "PUT", &result, &agent(&cfg), Some(json!({ "sent": false, "error": "x" }))).await.status,
            StatusCode::CONFLICT, "a request that was sent cannot then be marked failed");
        let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'deletion.sent' AND customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(audits, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_request_the_worker_died_on_is_failed_not_sent_again(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        let id = only_request(&pool, c).await;
        send(&app, "POST", &format!("/internal/deletion-requests/{id}/claim"), &agent(&cfg), None).await;
        sqlx::query("UPDATE deletion_requests SET claimed_at = NOW() - interval '2 hours' WHERE id = $1").bind(id).execute(&pool).await.unwrap();

        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json();
        assert_eq!(q["requests"], json!([]), "it is not picked up again");
        assert_eq!(status_of(&pool, id).await, "failed");
        let err: String = sqlx::query_scalar("SELECT error FROM deletion_requests WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert!(err.starts_with(deletion::ABANDONED_MARKER) && err.contains("not known whether"), "{err}");
        let ident = send(&app, "POST", &format!("/internal/deletion-requests/{id}/identity"), &agent(&cfg), None).await;
        assert_eq!(ident.status, StatusCode::FORBIDDEN, "and its identity can no longer be released");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_customer_who_downgrades_or_is_closed_after_authorizing_is_not_sent_to(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let (downgraded, closed) = (californian(&pool).await, californian(&pool).await);
        campaign(&app, &cfg, downgraded, body(&["whitepages"], false)).await;
        campaign(&app, &cfg, closed, body(&["whitepages"], false)).await;
        sqlx::query("UPDATE customers SET subscription_tier = 'free' WHERE id = $1").bind(downgraded).execute(&pool).await.unwrap();
        sqlx::query("UPDATE customers SET is_active = false WHERE id = $1").bind(closed).execute(&pool).await.unwrap();
        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json();
        assert_eq!(q["requests"], json!([]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_company_that_left_the_catalog_is_failed_instead_of_sent_to(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        sqlx::query("UPDATE deletion_requests SET channel_id = 'retired-company'").execute(&pool).await.unwrap();
        let q = send(&app, "GET", "/internal/deletion-requests", &agent(&cfg), None).await.json();
        assert_eq!(q["requests"], json!([]));
        assert_eq!(status_of(&pool, only_request(&pool, c).await).await, "failed");
    }

    // ---- what the company answered (reported by the customer) -----------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn the_customer_reports_what_the_company_answered_and_only_forwards(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        let other = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        let id = only_request(&pool, c).await;
        let outcome = |who: Uuid, what: &str| {
            let (app, cfg, what) = (app.clone(), cfg.clone(), what.to_string());
            async move { send(&app, "POST", &format!("/deletion-requests/{id}/outcome"), &bearer(&cfg, who, "personal"), Some(json!({ "outcome": what }))).await.status }
        };

        assert_eq!(outcome(c, "removed").await, StatusCode::CONFLICT, "nothing was sent yet, so nothing can have been answered");
        sqlx::query("UPDATE deletion_requests SET status = 'sent', sent_at = NOW() WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(outcome(other, "removed").await, StatusCode::NOT_FOUND, "someone else's request");
        assert_eq!(outcome(c, "sold").await, StatusCode::CONFLICT, "not a recognised outcome");
        assert_eq!(outcome(c, "replied").await, StatusCode::NO_CONTENT);
        assert_eq!(outcome(c, "replied").await, StatusCode::CONFLICT, "already replied");
        assert_eq!(outcome(c, "removed").await, StatusCode::NO_CONTENT);
        assert_eq!(status_of(&pool, id).await, "removed");
        assert_eq!(outcome(c, "denied").await, StatusCode::CONFLICT, "a final outcome stays final");
        let logged: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'deletion.outcome' AND customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(logged, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_dashboard_says_when_a_company_is_overdue_and_shows_a_failure_in_plain_words(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let c = californian(&pool).await;
        campaign(&app, &cfg, c, body(&["whitepages"], false)).await;
        let id = only_request(&pool, c).await;
        let who = bearer(&cfg, c, "personal");
        sqlx::query("UPDATE deletion_requests SET status = 'sent', sent_at = NOW() - interval '30 days' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(send(&app, "GET", "/deletion-requests", &who, None).await.json()["requests"][0]["follow_up_due"], false);
        sqlx::query("UPDATE deletion_requests SET sent_at = NOW() - interval '46 days' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(send(&app, "GET", "/deletion-requests", &who, None).await.json()["requests"][0]["follow_up_due"], true);
        sqlx::query("UPDATE deletion_requests SET status = 'removed', outcome_at = NOW() WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(send(&app, "GET", "/deletion-requests", &who, None).await.json()["requests"][0]["follow_up_due"], false, "answered: not overdue");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_customer_sees_only_their_own_requests(pool: PgPool) {
        let (app, cfg) = enabled_app(pool.clone());
        let (a, b) = (californian(&pool).await, californian(&pool).await);
        campaign(&app, &cfg, a, body(&["whitepages"], false)).await;
        assert_eq!(send(&app, "GET", "/deletion-requests", &bearer(&cfg, a, "personal"), None).await.json()["requests"].as_array().unwrap().len(), 1);
        assert_eq!(send(&app, "GET", "/deletion-requests", &bearer(&cfg, b, "personal"), None).await.json()["requests"], json!([]));
    }
}

// ===========================================================================
// Why a removal failed reaches the customer
// ===========================================================================

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_removal_shows_the_reason_the_executor_recorded(pool: PgPool) {
    let (app, cfg) = app(pool.clone());
    let c = ts::customer(&pool, "personal", true).await;
    let (blocked, plain, other) = (ts::exposure(&pool, c, true, None).await, ts::exposure(&pool, c, true, None).await, ts::exposure(&pool, c, true, None).await);
    let r_blocked = ts::removal_request(&pool, c, blocked, "failed", None).await;
    let r_plain = ts::removal_request(&pool, c, plain, "authorized", None).await;
    let r_two = ts::removal_request(&pool, c, other, "failed", None).await;

    let audit = |req: Uuid, action: &'static str, details: Value| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, $3, 'removal_executor', $4)")
                .bind(req).bind(c).bind(action).bind(details).execute(&pool).await.unwrap();
        }
    };
    audit(r_blocked, "failed", json!({ "error": "The site blocked our automated browser (HTTP 403), so this opt-out cannot be done automatically." })).await;
    audit(r_plain, "authorized", json!({})).await;
    // two failures: the most recent reason is the one shown
    audit(r_two, "failed", json!({ "error": "The opt-out page timed out" })).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    audit(r_two, "failed", json!({ "error": "The site's opt-out page no longer exists at the address we have (HTTP 404)." })).await;

    let list = send(&app, "GET", "/removal-requests", &bearer(&cfg, c, "personal"), None).await.json();
    let by: std::collections::HashMap<String, Value> = list["requests"].as_array().unwrap().iter()
        .map(|r| (r["id"].as_str().unwrap().to_string(), r["failure_reason"].clone())).collect();
    assert!(by[&r_blocked.to_string()].as_str().unwrap().contains("blocked our automated browser"));
    assert_eq!(by[&r_plain.to_string()], Value::Null, "a request that has not failed has no failure reason");
    assert!(by[&r_two.to_string()].as_str().unwrap().contains("no longer exists"), "the latest reason wins");

    // another customer never sees it
    let stranger = ts::customer(&pool, "personal", true).await;
    let theirs = send(&app, "GET", "/removal-requests", &bearer(&cfg, stranger, "personal"), None).await.json();
    assert_eq!(theirs["requests"], json!([]));
}

// ===========================================================================
// "I did it by hand": a customer-reported opt-out that is then checked independently
// ===========================================================================

mod done_by_hand {
    use super::*;

    async fn failed_request(pool: &PgPool, c: Uuid) -> (Uuid, Uuid) {
        let e = ts::exposure(pool, c, true, None).await;
        (ts::removal_request(pool, c, e, "failed", None).await, e)
    }

    async fn mark(app: &Router, cfg: &AppConfig, c: Uuid, id: Uuid) -> Reply {
        mark_with(app, cfg, c, id, json!({ "email_confirmation": "not_required" })).await
    }

    async fn mark_with(app: &Router, cfg: &AppConfig, c: Uuid, id: Uuid, body: Value) -> Reply {
        send(app, "POST", &format!("/removal-requests/{id}/done-by-hand"), &bearer(cfg, c, "personal"), Some(body)).await
    }

    async fn row(pool: &PgPool, id: Uuid) -> (String, String, bool, bool, bool) {
        sqlx::query_as(
            "SELECT status::text, procedure_used, submitted_at IS NOT NULL, \
                    COALESCE(next_check_at > NOW() + interval '2 days' AND next_check_at < NOW() + interval '4 days', false), \
                    verified_at IS NOT NULL FROM removal_requests WHERE id = $1")
            .bind(id).fetch_one(pool).await.unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_report_is_recorded_as_the_customers_claim_and_must_be_sensible(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let (id, _) = failed_request(&pool, c).await;
        let today = time::OffsetDateTime::now_utc().date();
        let fmt = |d: time::Date| d.to_string();

        // no answer about the email, an unknown answer, a future date, a stale date: all refused
        for body in [
            json!({}),
            json!({ "email_confirmation": "not_yet" }),
            json!({ "email_confirmation": "confirmed", "submitted_on": fmt(today + time::Duration::days(1)) }),
            json!({ "email_confirmation": "confirmed", "submitted_on": fmt(today - time::Duration::days(90)) }),
            json!({ "email_confirmation": "confirmed", "submitted_on": "yesterday" }),
        ] {
            let r = mark_with(&app, &cfg, c, id, body.clone()).await;
            assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {}", r.body);
        }
        assert_eq!(row(&pool, id).await.0, "failed", "nothing changed");

        // a submission from 5 days ago is due for its first look straight away, and dated as reported
        let five = today - time::Duration::days(5);
        let r = mark_with(&app, &cfg, c, id, json!({ "email_confirmation": "confirmed", "submitted_on": fmt(five) })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        let j = r.json();
        assert_eq!(j["request"]["manual_email_confirmation"], "confirmed");
        assert_eq!(j["request"]["manual_submitted_on"], fmt(five));
        assert!(j["request"]["manual_reported_at"].is_string());
        assert_eq!(j["request"]["status"], "submitted", "customer-reported, never verified_removed");
        let due: bool = sqlx::query_scalar("SELECT next_check_at < NOW() + interval '1 day' FROM removal_requests WHERE id = $1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert!(due);
        let detail: Value = sqlx::query_scalar("SELECT details FROM removal_audit_log WHERE removal_request_id = $1 AND action = 'submitted_manually'")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(detail["reported_by"], "customer");
        assert_eq!(detail["email_confirmation"], "confirmed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_exact_profile_link_replaces_the_search_link_for_re_checks_on_the_same_site_only(pool: PgPool) {
        use crate::services::url_vault;
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let site: String = sqlx::query_scalar("SELECT source_name FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        let search = format!("https://www.{site}/Jane-Smith");
        let sealed_search = url_vault::seal(&master(), c, &search).unwrap();
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1").bind(e).bind(&sealed_search).execute(&pool).await.unwrap();
        let id = ts::removal_request(&pool, c, e, "failed", None).await;
        sqlx::query("UPDATE removal_requests SET source_url = $2 WHERE id = $1").bind(id).bind(&sealed_search).execute(&pool).await.unwrap();

        // another site, or junk, is refused and changes nothing
        for bad in [json!("https://www.othersite.example/p1"), json!("not a link"), json!("http://www.example.com/x")] {
            let r = mark_with(&app, &cfg, c, id, json!({ "email_confirmation": "confirmed", "listing_url": bad })).await;
            assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.body);
        }
        assert_eq!(row(&pool, id).await.0, "failed");

        let profile = format!("https://www.{site}/Smith-Jane/California/Town/p152826921");
        let r = mark_with(&app, &cfg, c, id, json!({ "email_confirmation": "confirmed", "listing_url": profile })).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.json()["request"]["source_url"], profile.as_str());

        let (se, sr): (String, String) = sqlx::query_as("SELECT e.source_url, r.source_url FROM exposures e JOIN removal_requests r ON r.exposure_id = e.id WHERE r.id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert!(se.starts_with("enc1:") && sr.starts_with("enc1:"), "still sealed");
        assert_eq!(url_vault::open(&master(), c, &se).unwrap(), profile);
        assert_eq!(url_vault::open(&master(), c, &sr).unwrap(), profile);

        sqlx::query("UPDATE removal_requests SET next_check_at = NOW() - interval '1 minute' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        let due = send(&app, "GET", "/internal/removal-requests/due-for-verification", &agent(&cfg), None).await.json();
        assert_eq!(due["requests"][0]["source_url"], profile.as_str(), "the re-check visits the profile page");
        let detail: Value = sqlx::query_scalar("SELECT details FROM removal_audit_log WHERE removal_request_id = $1 AND action = 'submitted_manually'").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(detail["listing_link_updated"], true);
        assert!(!detail.to_string().contains("p152826921"), "the link itself is never written to the audit trail");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_link_can_be_corrected_after_reporting_but_not_once_closed(pool: PgPool) {
        use crate::services::url_vault;
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let site: String = sqlx::query_scalar("SELECT source_name FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        let id = ts::removal_request(&pool, c, e, "submitted", Some(3)).await;
        let who = bearer(&cfg, c, "personal");
        let post = |url: String| { let (app, who) = (app.clone(), who.clone()); async move {
            send(&app, "POST", &format!("/removal-requests/{id}/listing-link"), &who, Some(json!({ "listing_url": url }))).await
        } };

        assert_eq!(post("https://elsewhere.example/p1".into()).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        let profile = format!("https://www.{site}/Smith-Jane/CA/Town/p1");
        let ok = post(profile.clone()).await;
        assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
        assert_eq!(ok.json()["request"]["status"], "submitted", "the status is untouched");
        let se: String = sqlx::query_scalar("SELECT source_url FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        assert_eq!(url_vault::open(&master(), c, &se).unwrap(), profile);
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM removal_audit_log WHERE removal_request_id = $1 AND action = 'listing_link_updated' AND details = '{}'::jsonb").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(audited, 1);

        let other = ts::customer(&pool, "personal", true).await;
        let r = send(&app, "POST", &format!("/removal-requests/{id}/listing-link"), &bearer(&cfg, other, "personal"), Some(json!({ "listing_url": profile }))).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "not someone else's request");

        sqlx::query("UPDATE removal_requests SET status = 'verified_removed' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        assert_eq!(post(profile).await.status, StatusCode::CONFLICT, "closed requests keep their evidence");
    }

    #[test]
    fn a_subdomain_of_the_site_counts_as_the_same_site() {
        use crate::services::removal::same_site;
        assert!(same_site("spokeo.com", "spokeo.com"));
        assert!(same_site("m.spokeo.com", "www.spokeo.com"));
        assert!(!same_site("notspokeo.com", "spokeo.com"));
        assert!(!same_site("spokeo.com.evil.example", "spokeo.com"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_list_shows_the_last_check_so_the_customer_sees_inconclusive_for_what_it_is(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(1)).await;
        for (action, secs) in [("still_present", 30), ("verification_inconclusive", 10)] {
            sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details, recorded_at) VALUES ($1, $2, $3, 'verification_agent', '{}', NOW() - make_interval(secs => $4))")
                .bind(r).bind(c).bind(action).bind(secs as f64).execute(&pool).await.unwrap();
        }
        let list = send(&app, "GET", "/removal-requests", &bearer(&cfg, c, "personal"), None).await.json();
        let item = &list["requests"][0];
        assert_eq!(item["last_check_result"], "inconclusive", "the most recent check wins");
        assert!(item["last_check_at"].is_string());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn asking_for_help_is_limited_to_manual_action_and_is_idempotent(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let who = bearer(&cfg, c, "personal");
        let ask = |id: Uuid| {
            let (app, who) = (app.clone(), who.clone());
            async move { send(&app, "POST", &format!("/removal-requests/{id}/request-assistance"), &who, None).await }
        };

        let e = ts::exposure(&pool, c, true, None).await;
        let waiting = ts::removal_request(&pool, c, e, "submitted", Some(1)).await;
        assert_eq!(ask(waiting).await.status, StatusCode::CONFLICT, "only a request that needs manual action");

        let (failed, _) = failed_request(&pool, c).await;
        let first = ask(failed).await;
        assert_eq!(first.status, StatusCode::OK, "{}", first.body);
        assert_eq!(first.json()["request"]["assistance_status"], "requested");
        assert!(first.body.contains("never ask for your email password"));
        let at: time::OffsetDateTime = sqlx::query_scalar("SELECT assistance_requested_at FROM removal_requests WHERE id = $1").bind(failed).fetch_one(&pool).await.unwrap();
        assert_eq!(ask(failed).await.status, StatusCode::OK);
        let again: time::OffsetDateTime = sqlx::query_scalar("SELECT assistance_requested_at FROM removal_requests WHERE id = $1").bind(failed).fetch_one(&pool).await.unwrap();
        assert_eq!(at, again, "asking twice does not jump the queue or reset the clock");

        // someone else's request is not theirs to ask about
        let other = ts::customer(&pool, "personal", true).await;
        let r = send(&app, "POST", &format!("/removal-requests/{failed}/request-assistance"), &bearer(&cfg, other, "personal"), None).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND);

        // at most three open at once
        for _ in 0..2 { let (f, _) = failed_request(&pool, c).await; assert_eq!(ask(f).await.status, StatusCode::OK); }
        let (fourth, _) = failed_request(&pool, c).await;
        assert_eq!(ask(fourth).await.status, StatusCode::CONFLICT);

        // the operator queue shows it without personal details, and reading the email is audited
        let items = crate::services::assistance::open_items(&pool).await.unwrap();
        assert_eq!(items.len(), 3);
        let email = crate::services::assistance::contact_email(&pool, failed).await.unwrap();
        assert!(email.ends_with("@test.example"));
        let reads: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'assistance.contact_read' AND customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(reads, 1);
        crate::services::assistance::set_status(&pool, failed, "completed", Some(25)).await.unwrap();
        assert_eq!(crate::services::assistance::summary(&pool).await.unwrap(), (1, Some(25.0)));
        assert!(crate::services::assistance::set_status(&pool, failed, "completed", Some(5)).await.is_err(), "a closed item cannot be reopened by accident");
        assert!(crate::services::assistance::contact_email(&pool, failed).await.is_err(), "no contact lookups for closed items");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_request_becomes_submitted_with_its_first_check_about_three_days_out_and_is_audited(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let (id, _) = failed_request(&pool, c).await;

        let r = mark(&app, &cfg, c, id).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);
        assert_eq!(r.json()["request"]["status"], "submitted");
        assert!(r.json()["message"].as_str().unwrap().contains("3, 7, 14 and 30"));
        assert_eq!(row(&pool, id).await, ("submitted".into(), "manual".into(), true, true, false),
            "submitted by hand, first check in about three days, and NOT verified");

        let own: String = sqlx::query_scalar("SELECT actor FROM removal_audit_log WHERE removal_request_id = $1 AND action = 'submitted_manually'")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(own, "customer");
        let general: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'removal.manual_submit' AND customer_id = $1")
            .bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(general, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_customers_word_is_never_enough_only_the_independent_check_can_mark_it_removed(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let (id, exposure) = failed_request(&pool, c).await;
        mark(&app, &cfg, c, id).await;

        let due = |app: &Router, cfg: &AppConfig| {
            let (app, cfg) = (app.clone(), cfg.clone());
            async move { send(&app, "GET", "/internal/removal-requests/due-for-verification", &agent(&cfg), None).await.json() }
        };
        assert_eq!(due(&app, &cfg).await["requests"], json!([]), "not due yet: the site gets two days");

        // two days later the worker re-fetches the listing...
        sqlx::query("UPDATE removal_requests SET next_check_at = NOW() - interval '1 minute' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        let listed = due(&app, &cfg).await;
        assert_eq!(listed["requests"][0]["removal_request_id"], id.to_string());

        // ...and the listing is still there: nothing is marked removed, it is checked again later
        let still = send(&app, "POST", &format!("/internal/removal-requests/{id}/verification-result"), &agent(&cfg),
            Some(json!({ "customer_id": c, "outcome": "still_present" }))).await;
        assert_eq!(still.status, StatusCode::NO_CONTENT, "{}", still.body);
        assert_eq!(row(&pool, id).await.0, "submitted");
        let active: bool = sqlx::query_scalar("SELECT is_active FROM exposures WHERE id = $1").bind(exposure).fetch_one(&pool).await.unwrap();
        assert!(active, "the exposure stays active while the listing is still up");

        // only when the listing is actually gone does it count
        sqlx::query("UPDATE removal_requests SET next_check_at = NOW() - interval '1 minute' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        let gone = send(&app, "POST", &format!("/internal/removal-requests/{id}/verification-result"), &agent(&cfg),
            Some(json!({ "customer_id": c, "outcome": "removed" }))).await;
        assert_eq!(gone.status, StatusCode::NO_CONTENT);
        assert_eq!(row(&pool, id).await.0, "verified_removed");
        let active: bool = sqlx::query_scalar("SELECT is_active FROM exposures WHERE id = $1").bind(exposure).fetch_one(&pool).await.unwrap();
        assert!(!active, "now the exposure is closed, on evidence");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_claim_that_the_listing_is_gone_is_not_believed_when_it_is_still_there_after_the_timeout(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let (id, _) = failed_request(&pool, c).await;
        mark(&app, &cfg, c, id).await;
        sqlx::query("UPDATE removal_requests SET submitted_at = NOW() - interval '46 days', next_check_at = NOW() - interval '1 minute' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
        send(&app, "POST", &format!("/internal/removal-requests/{id}/verification-result"), &agent(&cfg),
            Some(json!({ "customer_id": c, "outcome": "still_present" }))).await;
        assert_eq!(row(&pool, id).await.0, "rejected", "said done, listing still up after 45 days: not removed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn only_a_failed_request_can_be_marked_and_only_once(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        for status in ["authorized", "submitted", "acknowledged", "verified_removed", "rejected", "unverifiable", "reappeared"] {
            let e = ts::exposure(&pool, c, true, None).await;
            let id = ts::removal_request(&pool, c, e, status, Some(1)).await;
            let r = mark(&app, &cfg, c, id).await;
            assert_eq!(r.status, StatusCode::CONFLICT, "{status}: {}", r.body);
            assert!(r.body.contains("Only a request that failed"), "{}", r.body);
        }
        let (id, _) = failed_request(&pool, c).await;
        assert_eq!(mark(&app, &cfg, c, id).await.status, StatusCode::OK);
        assert_eq!(mark(&app, &cfg, c, id).await.status, StatusCode::CONFLICT, "a second tap does nothing");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_is_the_customers_own_request_and_needs_a_session(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (owner, stranger) = (ts::customer(&pool, "personal", true).await, ts::customer(&pool, "personal", true).await);
        let (id, _) = failed_request(&pool, owner).await;
        assert_eq!(mark(&app, &cfg, stranger, id).await.status, StatusCode::NOT_FOUND, "someone else's request");
        assert_eq!(mark(&app, &cfg, owner, Uuid::new_v4()).await.status, StatusCode::NOT_FOUND, "no such request");
        let anon = send(&app, "POST", &format!("/removal-requests/{id}/done-by-hand"), &[], None).await;
        assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
        assert_eq!(row(&pool, id).await.0, "failed", "nothing changed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_cannot_create_a_second_request_in_flight_for_the_same_listing(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let first = ts::removal_request(&pool, c, e, "failed", None).await;
        let second = ts::removal_request(&pool, c, e, "failed", None).await;       // the three failed Spokeo attempts look like this
        assert_eq!(mark(&app, &cfg, c, first).await.status, StatusCode::OK);
        let r = mark(&app, &cfg, c, second).await;
        assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
        assert!(r.body.contains("already in progress"));
        assert_eq!(row(&pool, second).await.0, "failed");

        // and one authorized again in the meantime blocks it too
        let e2 = ts::exposure(&pool, c, true, None).await;
        let old = ts::removal_request(&pool, c, e2, "failed", None).await;
        ts::removal_request(&pool, c, e2, "authorized", None).await;
        assert_eq!(mark(&app, &cfg, c, old).await.status, StatusCode::CONFLICT);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_listing_that_is_already_closed_has_nothing_left_to_remove(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, false, Some(3)).await;                       // already removed
        let id = ts::removal_request(&pool, c, e, "failed", None).await;
        let r = mark(&app, &cfg, c, id).await;
        assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
        assert!(r.body.contains("no longer active"));
        assert_eq!(row(&pool, id).await.0, "failed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn simultaneous_taps_produce_exactly_one_submission(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let (id, _) = failed_request(&pool, c).await;
        let mut handles = vec![];
        for _ in 0..6 {
            let (app, cfg) = (app.clone(), cfg.clone());
            handles.push(tokio::spawn(async move { mark(&app, &cfg, c, id).await.status }));
        }
        let mut ok = 0;
        for h in handles {
            if h.await.unwrap() == StatusCode::OK { ok += 1; }
        }
        assert_eq!(ok, 1);
        let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM removal_audit_log WHERE removal_request_id = $1 AND action = 'submitted_manually'")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(entries, 1);
    }
}

// ===========================================================================
// Account erasure
// ===========================================================================

mod account_erasure {
    use super::*;

    async fn signed_up(app: &Router, pool: &PgPool, cfg: &AppConfig, email: &str) -> (Uuid, Vec<(&'static str, String)>) {
        assert_eq!(register(app, email).await.status, StatusCode::CREATED);
        let id: Uuid = sqlx::query_scalar("SELECT id FROM customers WHERE email = $1").bind(email).fetch_one(pool).await.unwrap();
        (id, bearer(cfg, id, "free"))
    }

    async fn delete(app: &Router, who: &[(&'static str, String)], password: &str) -> Reply {
        send(app, "DELETE", "/me", who, Some(json!({ "password": password }))).await
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasing_removes_everything_owned_and_pseudonymizes_the_trail(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (id, who) = signed_up(&app, &pool, &cfg, "erase@example.com").await;
        let other = ts::customer(&pool, "free", false).await;
        let e = ts::exposure(&pool, id, true, None).await;
        let r = ts::removal_request(&pool, id, e, "failed", None).await;
        let other_e = ts::exposure(&pool, other, true, None).await;
        let other_r = ts::removal_request(&pool, other, other_e, "failed", None).await;
        for (req, cust) in [(r, id), (other_r, other)] {
            sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, 'failed', 'removal_executor', $3)")
                .bind(req).bind(cust).bind(json!({ "error": "free text that could name someone" })).execute(&pool).await.unwrap();
        }
        // a failed login carries only the email
        sqlx::query("INSERT INTO audit_log (user_email, action, resource_type, ip_address, user_agent, status) VALUES ('Erase@Example.com', 'user.login', 'customer', '9.9.9.9', 'ua', 'failure')")
            .execute(&pool).await.unwrap();

        // the wrong password changes nothing
        assert_eq!(delete(&app, &who, "not the password").await.status, StatusCode::UNAUTHORIZED);
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM customers WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap(), 1);

        let ok = delete(&app, &who, PASSWORD).await;
        assert_eq!(ok.status, StatusCode::NO_CONTENT);
        assert!(session_set_cookie(&ok).expect("cookie cleared").contains("Max-Age=0"));

        // nothing left in any table that has a customer_id column, for this customer
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.columns WHERE table_schema = 'public' AND column_name = 'customer_id'",
        ).fetch_all(&pool).await.unwrap();
        assert!(tables.len() > 5, "{tables:?}");
        for t in &tables {
            let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {t} WHERE customer_id = $1")).bind(id).fetch_one(&pool).await.unwrap();
            assert_eq!(n, 0, "{t} still holds the customer's rows");
        }
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM removal_requests WHERE id = $1").bind(r).fetch_one(&pool).await.unwrap(), 0);

        // the trail survives, with nothing that identifies the person
        let (n, linked, text): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*), count(customer_id) + count(removal_request_id), count(*) FILTER (WHERE details <> '{}'::jsonb) \
             FROM removal_audit_log WHERE action = 'failed' AND customer_id IS NULL",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!((n, linked, text), (1, 0, 0));
        let (email_rows, ip_rows): (i64, i64) = sqlx::query_as(
            "SELECT count(user_email), count(ip_address) + count(user_agent) FROM audit_log WHERE lower(coalesce(user_email,'')) = 'erase@example.com' OR customer_id = $1",
        ).bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!((email_rows, ip_rows), (0, 0));
        assert!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log WHERE action = 'user.register'").fetch_one(&pool).await.unwrap() >= 1, "history kept");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log WHERE action = 'account.delete' AND customer_id IS NULL AND user_email IS NULL").fetch_one(&pool).await.unwrap(), 1);

        // the other customer is untouched
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM removal_audit_log WHERE customer_id = $1 AND details <> '{}'::jsonb").bind(other).fetch_one(&pool).await.unwrap(), 1);
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM removal_requests WHERE customer_id = $1").bind(other).fetch_one(&pool).await.unwrap(), 1);
        // the account cannot be used again
        assert_eq!(login(&app, "erase@example.com", PASSWORD).await.status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_trail_is_still_tamper_proof_after_the_erasure_rules_changed(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "failed", None).await;
        sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, 'failed', 'x', '{\"a\":1}')")
            .bind(r).bind(c).execute(&pool).await.unwrap();
        for sql in [
            "UPDATE removal_audit_log SET action = 'forged'",
            "UPDATE removal_audit_log SET details = '{}'::jsonb",              // blanking without unlinking
            "UPDATE removal_audit_log SET customer_id = NULL, details = '{\"b\":2}'::jsonb",
            "UPDATE removal_audit_log SET customer_id = gen_random_uuid()",
            "UPDATE removal_audit_log SET actor = 'someone'",
            "DELETE FROM removal_audit_log",
            "TRUNCATE removal_audit_log",
            "UPDATE audit_log SET ip_address = '1.2.3.4'",
        ] {
            sqlx::query("INSERT INTO audit_log (action, resource_type) VALUES ('t', 'customer') ON CONFLICT DO NOTHING").execute(&pool).await.unwrap();
            let e = sqlx::query(sql).execute(&pool).await;
            assert!(e.is_err() || sql.starts_with("UPDATE audit_log"), "{sql} must be rejected");
            if sql.starts_with("UPDATE audit_log") {
                // a row with NULL ip may be "updated" to a value only if it is rejected; zero rows is fine only for empty tables
                assert!(e.is_err(), "{sql}");
            }
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_subscription_that_keeps_billing_blocks_erasure_but_one_ending_does_not(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (id, who) = signed_up(&app, &pool, &cfg, "paid@example.com").await;
        sqlx::query("INSERT INTO stripe_subscriptions (subscription_id, stripe_customer_id, customer_id, status, price_id, cancel_at_period_end) VALUES ('sub_1', 'cus_1', $1, 'active', 'p', false)")
            .bind(id).execute(&pool).await.unwrap();
        let blocked = delete(&app, &who, PASSWORD).await;
        assert_eq!(blocked.status, StatusCode::CONFLICT);
        assert!(blocked.body.contains("Cancel your subscription"), "{}", blocked.body);
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM customers WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap(), 1);

        sqlx::query("UPDATE stripe_subscriptions SET cancel_at_period_end = true").execute(&pool).await.unwrap();
        assert_eq!(delete(&app, &who, PASSWORD).await.status, StatusCode::NO_CONTENT);
        let (sub, linked): (i64, i64) = sqlx::query_as("SELECT count(*), count(customer_id) FROM stripe_subscriptions").fetch_one(&pool).await.unwrap();
        assert_eq!((sub, linked), (1, 0), "payment identifiers are kept, unlinked from any person");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasure_needs_a_session_and_repeated_wrong_passwords_lock_out(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let (id, who) = signed_up(&app, &pool, &cfg, "lock@example.com").await;
        assert_eq!(send(&app, "DELETE", "/me", &[], Some(json!({ "password": PASSWORD }))).await.status, StatusCode::UNAUTHORIZED);
        let mut last = StatusCode::OK;
        for _ in 0..12 {
            last = delete(&app, &who, "wrong wrong wrong").await.status;
        }
        assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(delete(&app, &who, PASSWORD).await.status, StatusCode::TOO_MANY_REQUESTS, "locked even for the right password");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM customers WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap(), 1);
    }
}

// ===========================================================================
// Listing links are encrypted at rest
// ===========================================================================

mod sealed_links {
    use super::*;
    use crate::services::url_vault;

    const URL: &str = "https://www.spokeo.com/jane-smith?q=jane+smith";

    #[test]
    fn a_link_round_trips_only_for_its_own_customer_and_hides_the_name() {
        let (m, a, b) = (master(), Uuid::new_v4(), Uuid::new_v4());
        let sealed = url_vault::seal(&m, a, URL).unwrap();
        assert!(url_vault::is_sealed(&sealed));
        assert!(!sealed.to_lowercase().contains("jane") && !sealed.contains("spokeo"), "{sealed}");
        assert_eq!(url_vault::open(&m, a, &sealed).unwrap(), URL);
        assert!(url_vault::open(&m, b, &sealed).is_err(), "another customer's key must not open it");
        assert_ne!(sealed, url_vault::seal(&m, a, URL).unwrap(), "fresh nonce each time");
        assert_eq!(url_vault::seal(&m, a, &sealed).unwrap(), sealed, "sealing twice does not double-wrap");
        // legacy plaintext still reads
        assert_eq!(url_vault::open(&m, a, URL).unwrap(), URL);
        // a damaged value fails loudly instead of yielding a wrong link
        let mut bad = sealed.clone();
        bad.push_str("AA");
        assert!(url_vault::open(&m, a, &bad).is_err());
        assert!(url_vault::open(&m, a, "enc1:!!!not base64").is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_recorded_link_is_stored_sealed_and_opened_for_the_customer_and_agents(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        let job = ts::scan_job(&pool, c, "running").await;
        let r = send(&app, "POST", "/internal/exposures", &agent(&cfg), Some(json!({
            "customer_id": c, "scan_job_id": job, "source_name": "spokeo.com", "source_url": URL,
            "pii_category": "full_name", "severity": "high", "pii_summary": "J*** S***", "confidence": 0.9
        }))).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);

        let stored: String = sqlx::query_scalar("SELECT source_url FROM exposures WHERE customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert!(stored.starts_with("enc1:") && !stored.to_lowercase().contains("jane"), "{stored}");

        let who = bearer(&cfg, c, "free");
        let list = send(&app, "GET", "/exposures", &who, None).await.json();
        assert_eq!(list["exposures"][0]["source_url"], URL);
        let exposure_id = list["exposures"][0]["id"].as_str().unwrap().to_string();

        // authorizing copies the sealed value; the customer and the agent both get the open link
        let auth = send(&app, "POST", &format!("/removal-requests/{exposure_id}/authorize"), &who, None).await;
        assert_eq!(auth.status, StatusCode::OK, "{}", auth.body);
        assert_eq!(auth.json()["request"]["source_url"], URL);
        let req_stored: String = sqlx::query_scalar("SELECT source_url FROM removal_requests WHERE customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert!(req_stored.starts_with("enc1:"), "{req_stored}");
        let mine = send(&app, "GET", "/removal-requests", &who, None).await.json();
        assert_eq!(mine["requests"][0]["source_url"], URL);
        let id = mine["requests"][0]["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, "GET", &format!("/removal-requests/{id}"), &who, None).await.json()["request"]["source_url"], URL);
        let pending = send(&app, "GET", "/internal/removal-requests?status=authorized", &agent(&cfg), None).await.json();
        assert_eq!(pending["requests"][0]["source_url"], URL);

        sqlx::query("UPDATE removal_requests SET status = 'submitted', submitted_at = NOW(), next_check_at = NOW() - interval '1 hour'").execute(&pool).await.unwrap();
        let due = send(&app, "GET", "/internal/removal-requests/due-for-verification", &agent(&cfg), None).await.json();
        assert_eq!(due["requests"][0]["source_url"], URL);

        sqlx::query("UPDATE exposures SET is_active = false, removed_at = NOW() - interval '40 days'").execute(&pool).await.unwrap();
        let monitored = send(&app, "GET", "/internal/exposures/due-for-monitoring", &agent(&cfg), None).await;
        // free customers are never monitored, so nothing is due, and nothing sealed leaks either
        assert!(!monitored.body.contains("enc1:"), "{}", monitored.body);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn monitoring_targets_get_the_open_link(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "premium", true).await;
        let e = ts::exposure(&pool, c, false, Some(40)).await;
        let sealed = url_vault::seal(&master(), c, URL).unwrap();
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1").bind(e).bind(sealed).execute(&pool).await.unwrap();
        let due = send(&app, "GET", "/internal/exposures/due-for-monitoring", &agent(&cfg), None).await.json();
        assert_eq!(due["exposures"][0]["source_url"], URL);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_backfill_seals_legacy_rows_once_and_changes_nothing_the_customer_sees(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "failed", None).await;
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1").bind(e).bind(URL).execute(&pool).await.unwrap();
        sqlx::query("UPDATE removal_requests SET source_url = $2 WHERE id = $1").bind(r).bind(URL).execute(&pool).await.unwrap();
        let sealed_already = url_vault::seal(&master(), c, "https://x.example/p").unwrap();
        let e2 = ts::exposure(&pool, c, true, None).await;
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1").bind(e2).bind(&sealed_already).execute(&pool).await.unwrap();

        assert_eq!(url_vault::backfill(&pool, &master()).await.unwrap(), 2, "the two legacy rows, not the sealed one");
        assert_eq!(url_vault::backfill(&pool, &master()).await.unwrap(), 0, "idempotent");
        let leftover: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM exposures WHERE source_url NOT LIKE 'enc1:%') + (SELECT count(*) FROM removal_requests WHERE source_url NOT LIKE 'enc1:%')",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(leftover, 0);
        let kept: String = sqlx::query_scalar("SELECT source_url FROM exposures WHERE id = $1").bind(e2).fetch_one(&pool).await.unwrap();
        assert_eq!(kept, sealed_already, "an already-sealed row is left alone");

        let who = bearer(&cfg, c, "free");
        let list = send(&app, "GET", "/removal-requests", &who, None).await.json();
        assert_eq!(list["requests"][0]["source_url"], URL);
    }
}

// ===========================================================================
// Pilot: account cap and separate consents
// ===========================================================================

mod pilot_registration {
    use super::*;

    fn body(email: &str, interview: Option<bool>, story: Option<bool>) -> Value {
        let mut b = json!({ "email": email, "password": PASSWORD, "accepted_terms_version": crate::models::customer::TERMS_VERSION });
        if let Some(i) = interview { b["pilot_interview_ok"] = json!(i); }
        if let Some(s) = story { b["pilot_story_ok"] = json!(s); }
        b
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn registration_stops_at_the_configured_number_of_accounts(pool: PgPool) {
        let mut c = relaxed();
        c.max_accounts = Some(2);
        let (app, _) = app_with(pool.clone(), c, &redis_url());
        assert_eq!(register(&app, "one@example.com").await.status, StatusCode::CREATED);
        assert_eq!(register(&app, "two@example.com").await.status, StatusCode::CREATED);
        let full = register(&app, "three@example.com").await;
        assert_eq!(full.status, StatusCode::FORBIDDEN);
        assert!(full.body.contains("pilot is full"), "{}", full.body);
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM customers").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 2);
        // signing in is unaffected by the cap
        assert_eq!(login(&app, "one@example.com", PASSWORD).await.status, StatusCode::OK);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn no_cap_means_no_limit(pool: PgPool) {
        let (app, _) = app(pool);
        for i in 0..4 { assert_eq!(register(&app, &format!("u{i}@example.com")).await.status, StatusCode::CREATED); }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn interview_and_story_consent_are_separate_and_off_unless_ticked(pool: PgPool) {
        let (app, _) = app(pool.clone());
        let post = |b: Value| { let app = app.clone(); async move { send(&app, "POST", "/auth/register", &[], Some(b)).await } };
        assert_eq!(post(body("none@example.com", None, None)).await.status, StatusCode::CREATED);
        assert_eq!(post(body("int@example.com", Some(true), None)).await.status, StatusCode::CREATED);
        assert_eq!(post(body("both@example.com", Some(true), Some(true))).await.status, StatusCode::CREATED);
        let flags = |email: &'static str| { let pool = pool.clone(); async move {
            sqlx::query_as::<_, (bool, bool)>("SELECT pilot_interview_ok, pilot_story_ok FROM customers WHERE email = $1").bind(email).fetch_one(&pool).await.unwrap()
        } };
        assert_eq!(flags("none@example.com").await, (false, false));
        assert_eq!(flags("int@example.com").await, (true, false), "agreeing to an interview is not agreeing to be quoted");
        assert_eq!(flags("both@example.com").await, (true, true));
    }
}

// ===========================================================================
// Listings the customer adds themselves
// ===========================================================================

mod own_listings {
    use super::*;

    const LINK: &str = "https://www.radaris.com/p/Jane/Smith/";

    async fn add(app: &Router, cfg: &AppConfig, c: Uuid, tier: &str, url: &str) -> Reply {
        send(app, "POST", "/me/listings", &bearer(cfg, c, tier), Some(json!({ "source_url": url }))).await
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_added_listing_is_tracked_as_manual_action_sealed_and_reported_like_any_other(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        let r = add(&app, &cfg, c, "personal", LINK).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        let e = &r.json()["exposure"];
        assert_eq!(e["source_name"], "radaris.com");
        assert_eq!(e["origin"], "customer");
        assert_eq!(e["source_url"], LINK, "the customer gets the open link back");
        assert!(e["scan_job_id"].is_null());

        let (stored_e, stored_r): (String, String) = sqlx::query_as(
            "SELECT e.source_url, r.source_url FROM exposures e JOIN removal_requests r ON r.exposure_id = e.id WHERE e.customer_id = $1",
        ).bind(c).fetch_one(&pool).await.unwrap();
        assert!(stored_e.starts_with("enc1:") && stored_r.starts_with("enc1:"), "sealed at rest");
        assert!(!stored_e.to_lowercase().contains("smith"));

        let list = send(&app, "GET", "/removal-requests", &bearer(&cfg, c, "personal"), None).await.json();
        let req = &list["requests"][0];
        assert_eq!(req["status"], "failed", "= manual action required");
        assert!(req["failure_reason"].as_str().unwrap().contains("added this listing yourself"));
        let id = req["id"].as_str().unwrap();

        // the usual self-report, then the usual verification
        let rep = send(&app, "POST", &format!("/removal-requests/{id}/done-by-hand"), &bearer(&cfg, c, "personal"), Some(json!({ "email_confirmation": "not_required" }))).await;
        assert_eq!(rep.status, StatusCode::OK, "{}", rep.body);
        sqlx::query("UPDATE removal_requests SET next_check_at = NOW() - interval '1 minute'").execute(&pool).await.unwrap();
        let due = send(&app, "GET", "/internal/removal-requests/due-for-verification", &agent(&cfg), None).await.json();
        assert_eq!(due["requests"][0]["source_url"], LINK, "the verifier gets the open link to re-check");

        let audit: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'listing.add' AND customer_id = $1 AND details->>'site' = 'radaris.com'").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(audit, 1);
        let leak: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE details::text ILIKE '%smith%'").fetch_one(&pool).await.unwrap();
        assert_eq!(leak, 0, "the link is never written to the audit log");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bad_links_duplicates_and_too_many_are_refused(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        for bad in ["radaris.com/p", "http://radaris.com/p", "https://10.0.0.5/p", "https://localhost/p", "https://u:p@radaris.com/p"] {
            assert_eq!(add(&app, &cfg, c, "free", bad).await.status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        }
        assert_eq!(add(&app, &cfg, c, "free", LINK).await.status, StatusCode::CREATED);
        assert_eq!(add(&app, &cfg, c, "free", "https://radaris.com/other").await.status, StatusCode::CONFLICT, "one per site");
        for i in 1..crate::services::listing::MAX_OWN_LISTINGS_FREE {
            let r = add(&app, &cfg, c, "free", &format!("https://site{i}.example.com/p")).await;
            assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        }
        let over = add(&app, &cfg, c, "free", "https://one-more.example.com/p").await;
        assert_eq!(over.status, StatusCode::CONFLICT);
        assert!(over.body.contains("up to 5"), "{}", over.body);
        assert_eq!(send(&app, "POST", "/me/listings", &[], Some(json!({ "source_url": LINK }))).await.status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn by_hand_opt_outs_do_not_use_up_the_free_allowance(pool: PgPool) {
        let (app, cfg) = app(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        let who = bearer(&cfg, c, "free");
        // one by-hand removal in progress, which used to count towards the three
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(3)).await;
        sqlx::query("UPDATE removal_requests SET procedure_used = 'manual' WHERE id = $1").bind(r).execute(&pool).await.unwrap();
        for _ in 0..crate::services::removal::FREE_REMOVAL_LIMIT {
            let e = ts::exposure(&pool, c, true, None).await;
            assert_eq!(send(&app, "POST", &format!("/removal-requests/{e}/authorize"), &who, None).await.status, StatusCode::OK);
        }
        let e = ts::exposure(&pool, c, true, None).await;
        assert_eq!(send(&app, "POST", &format!("/removal-requests/{e}/authorize"), &who, None).await.status, StatusCode::FORBIDDEN, "the three automated ones are still the limit");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn public_verified_sources_count_only_scan_found_listings(pool: PgPool) {
        let (app, _) = app(pool.clone());
        let none = send(&app, "GET", "/public/coverage", &[], None).await.json();
        assert_eq!(none["verified_removals"], 0);
        assert_eq!(none["verified_sources"], json!([]));

        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, false, Some(1)).await;
        let name: String = sqlx::query_scalar("SELECT source_name FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        ts::removal_request(&pool, c, e, "verified_removed", None).await;
        let own = ts::exposure(&pool, c, false, Some(1)).await;
        sqlx::query("UPDATE exposures SET origin = 'customer' WHERE id = $1").bind(own).execute(&pool).await.unwrap();
        ts::removal_request(&pool, c, own, "verified_removed", None).await;

        let body = send(&app, "GET", "/public/coverage", &[], None).await;
        let j = body.json();
        assert_eq!(j["verified_removals"], 1, "a customer-added listing is not counted");
        assert_eq!(j["verified_sources"], json!([name]));
        assert!(!body.body.contains(&c.to_string()));
    }
}
