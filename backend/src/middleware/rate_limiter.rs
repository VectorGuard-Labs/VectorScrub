//! Per-client-IP rate limiting, shared across backend instances through Redis.
//!
//! Two tiers, applied by `api::router`:
//! * general: customer-facing API, RATE_LIMIT_GENERAL_PER_SECOND sustained, burst
//!   RATE_LIMIT_GENERAL_BURST;
//! * auth: login and register, RATE_LIMIT_AUTH_PER_MINUTE sustained, burst
//!   RATE_LIMIT_AUTH_BURST. Slows credential stuffing and keeps one source from
//!   burning CPU on Argon2 hashes.
//!
//! NOT limited: the internal agent endpoints (they all share one IP and are protected
//! by the agent key) and the health probes.
//!
//! Algorithm: GCRA (a token bucket kept as one number, the "theoretical arrival time") run as
//! a single Lua script, so the check and the update are atomic and every instance sees the same
//! bucket. The script reads the clock from Redis (`TIME`), so instances with skewed clocks agree.
//! Keys are `<prefix>:rl:<tier>:<period_ms>:<burst>:<client>` and expire on their own.
//!
//! Client identity: the TCP peer address by default. With TRUST_PROXY_HEADERS=true the
//! `X-Forwarded-For` entry added by our own proxy is used instead (TRUSTED_PROXY_HOPS places
//! from the right; never the client-controlled left side), exactly as the audit trail does
//! (api/client_ip.rs). IPv6 clients are keyed by their /64, not their full address. A request
//! whose address cannot be determined shares one bucket with all others like it.
//!
//! When Redis is unreachable (or slow: calls are cut off after `REDIS_TIMEOUT`) the limiter
//! does not open the door and does not take the API down with it. It falls back to the same
//! algorithm in this process's memory, which is per instance (with N instances the effective
//! limit is up to N times higher). Failures are counted in `rate_limit_backend_errors_total`
//! and logged once per outage; Redis is retried every `RETRY_REDIS_AFTER`. Per-account login
//! lockout (services/login_throttle.rs) lives in Postgres and is exact either way.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Router,
};
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use tokio::sync::OnceCell;

use crate::api::client_ip::{client_ip, rate_limit_key};
use crate::db::AppState;
use crate::services::config::AppConfig;

/// A Redis call that takes longer than this counts as a failure.
const REDIS_TIMEOUT: Duration = Duration::from_millis(250);
/// After a Redis failure, use the local fallback for this long before trying Redis again.
const RETRY_REDIS_AFTER: Duration = Duration::from_secs(5);
/// Most entries the in-memory fallback keeps before it sweeps out finished ones.
const LOCAL_SWEEP_ABOVE: usize = 4096;

/// GCRA. KEYS[1] = bucket key; ARGV[1] = ms per token, ARGV[2] = burst.
/// Returns {allowed (1/0), retry_after_ms, remaining}.
const GCRA_SCRIPT: &str = r#"
local t = redis.call('TIME')
local now = t[1] * 1000 + math.floor(t[2] / 1000)
local period = tonumber(ARGV[1])
local burst = tonumber(ARGV[2])
local tat = tonumber(redis.call('GET', KEYS[1]))
if (not tat) or tat < now then tat = now end
local new_tat = tat + period
local allow_at = new_tat - burst * period
if now < allow_at then
  return {0, allow_at - now, 0}
end
redis.call('SET', KEYS[1], new_tat, 'PX', (new_tat - now) + 1000)
return {1, 0, math.floor((now + burst * period - new_tat) / period)}
"#;

/// Which client a request is rate-limited as. Uses the same address selection as the audit
/// trail, so the two always agree, and never trusts the client-controlled part of
/// `X-Forwarded-For`.
#[derive(Debug, Clone, Copy)]
pub struct ClientKeyExtractor {
    pub trust_proxy_headers: bool,
    pub trusted_hops: usize,
}

impl ClientKeyExtractor {
    /// The key for this request, or `None` when no address can be determined.
    fn extract(&self, req: &Request) -> Option<IpAddr> {
        let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        client_ip(req.headers(), peer, self.trust_proxy_headers, self.trusted_hops).map(rate_limit_key)
    }
}

/// The outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Allowed { remaining: u32 },
    Limited { retry_after_ms: u64 },
}

/// The same algorithm as the Lua script, over a map in this process. The fallback when Redis is
/// unavailable, and the only store when no Redis is reachable at all.
struct LocalBuckets {
    start: Instant,
    state: Mutex<LocalState>,
}

#[derive(Default)]
struct LocalState {
    /// Theoretical arrival time per client, in ms since `start`.
    tat: HashMap<IpAddr, u64>,
    last_sweep_ms: u64,
}

impl LocalBuckets {
    fn new() -> Self {
        Self { start: Instant::now(), state: Mutex::new(LocalState::default()) }
    }

    fn check(&self, client: IpAddr, period_ms: u64, burst: u32) -> Decision {
        let now = self.start.elapsed().as_millis() as u64;
        // A poisoned lock only means another request panicked mid-update; the map is still usable.
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());

        // Forget clients whose bucket has refilled, at most once a second, so the table cannot
        // grow without bound.
        if st.tat.len() > LOCAL_SWEEP_ABOVE && now.saturating_sub(st.last_sweep_ms) >= 1000 {
            st.tat.retain(|_, tat| *tat > now);
            st.last_sweep_ms = now;
        }

        let tat = st.tat.get(&client).copied().unwrap_or(0).max(now);
        let new_tat = tat + period_ms;
        let window = burst as u64 * period_ms;
        if new_tat > now + window {
            return Decision::Limited { retry_after_ms: new_tat - window - now };
        }
        st.tat.insert(client, new_tat);
        Decision::Allowed { remaining: ((now + window - new_tat) / period_ms) as u32 }
    }
}

/// One rate limit (a tier) for a set of routes.
pub struct Limiter {
    tier: &'static str,
    key_prefix: String,
    period_ms: u64,
    burst: u32,
    extractor: ClientKeyExtractor,
    redis: redis::Client,
    conn: OnceCell<ConnectionManager>,
    script: redis::Script,
    local: LocalBuckets,
    /// Milliseconds since `local.start` before which Redis is not tried again (0 = no outage).
    redis_down_until_ms: AtomicU64,
    degraded: AtomicBool,
}

impl Limiter {
    pub fn new(
        tier: &'static str,
        cfg: &AppConfig,
        redis: redis::Client,
        period: Duration,
        burst: u32,
    ) -> Self {
        Self {
            tier,
            key_prefix: cfg.rate_limit_key_prefix.clone(),
            period_ms: (period.as_millis() as u64).max(1),
            burst: burst.max(1),
            extractor: ClientKeyExtractor {
                trust_proxy_headers: cfg.trust_proxy_headers,
                trusted_hops: cfg.trusted_proxy_hops,
            },
            redis,
            conn: OnceCell::new(),
            script: redis::Script::new(GCRA_SCRIPT),
            local: LocalBuckets::new(),
            redis_down_until_ms: AtomicU64::new(0),
            degraded: AtomicBool::new(false),
        }
    }

    /// Bucket key. The limits are part of it, so changing a limit starts a fresh bucket instead
    /// of reinterpreting an old one.
    fn redis_key(&self, client: Option<IpAddr>) -> String {
        let who = client.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
        format!("{}:rl:{}:{}:{}:{}", self.key_prefix, self.tier, self.period_ms, self.burst, who)
    }

    async fn connection(&self) -> redis::RedisResult<ConnectionManager> {
        let conn = self
            .conn
            .get_or_try_init(|| async {
                let cfg = ConnectionManagerConfig::new()
                    .set_connection_timeout(REDIS_TIMEOUT)
                    .set_response_timeout(REDIS_TIMEOUT)
                    .set_number_of_retries(1);
                ConnectionManager::new_with_config(self.redis.clone(), cfg).await
            })
            .await?;
        Ok(conn.clone())
    }

    async fn check_redis(&self, client: Option<IpAddr>) -> Result<Decision, String> {
        let attempt = async {
            let mut conn = self.connection().await?;
            let (allowed, retry_ms, remaining): (i64, i64, i64) = self
                .script
                .key(self.redis_key(client))
                .arg(self.period_ms)
                .arg(self.burst)
                .invoke_async(&mut conn)
                .await?;
            Ok::<_, redis::RedisError>(if allowed == 1 {
                Decision::Allowed { remaining: remaining.max(0) as u32 }
            } else {
                Decision::Limited { retry_after_ms: retry_ms.max(1) as u64 }
            })
        };
        match tokio::time::timeout(REDIS_TIMEOUT * 2, attempt).await {
            Ok(Ok(d)) => Ok(d),
            // Only the error kind is reported: a Redis error can echo connection details.
            Ok(Err(e)) => Err(format!("{:?}", e.kind())),
            Err(_) => Err("timeout".into()),
        }
    }

    async fn check(&self, client: Option<IpAddr>) -> Decision {
        let now_ms = self.local.start.elapsed().as_millis() as u64;
        if now_ms >= self.redis_down_until_ms.load(Ordering::Relaxed) {
            match self.check_redis(client).await {
                Ok(d) => {
                    if self.degraded.swap(false, Ordering::Relaxed) {
                        tracing::info!(tier = self.tier, "Rate limiter is using Redis again");
                    }
                    return d;
                }
                Err(kind) => {
                    metrics::counter!("rate_limit_backend_errors_total", "tier" => self.tier).increment(1);
                    let until = now_ms + RETRY_REDIS_AFTER.as_millis() as u64;
                    self.redis_down_until_ms.store(until, Ordering::Relaxed);
                    if !self.degraded.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            tier = self.tier,
                            error_kind = %kind,
                            "Redis unavailable for rate limiting; limiting per instance in memory until it returns"
                        );
                    }
                }
            }
        }
        // Local fallback. An unknown client shares the unspecified address's bucket.
        self.local.check(client.unwrap_or(IpAddr::from([0u8, 0, 0, 0])), self.period_ms, self.burst)
    }
}

async fn enforce(State(limiter): State<Arc<Limiter>>, req: Request, next: Next) -> Response {
    let client = limiter.extractor.extract(&req);
    match limiter.check(client).await {
        Decision::Allowed { remaining } => {
            let mut res = next.run(req).await;
            let h = res.headers_mut();
            h.insert(HeaderName::from_static("x-ratelimit-limit"), HeaderValue::from(limiter.burst));
            h.insert(HeaderName::from_static("x-ratelimit-remaining"), HeaderValue::from(remaining));
            res
        }
        Decision::Limited { retry_after_ms } => {
            let secs = retry_after_ms.div_ceil(1000).max(1);
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    (header::RETRY_AFTER, secs.to_string()),
                    (HeaderName::from_static("x-ratelimit-after"), secs.to_string()),
                ],
                format!("Too many requests. Try again in {secs} second(s)."),
            )
                .into_response()
        }
    }
}

/// Apply the general per-IP limit to `router`.
pub fn limit_general(router: Router<AppState>, cfg: &AppConfig, redis: &redis::Client) -> Router<AppState> {
    let per_second = cfg.rate_limit_general_per_second.max(1);
    let period = Duration::from_millis((1000 / per_second.min(1000)).max(1));
    apply(router, Limiter::new("general", cfg, redis.clone(), period, cfg.rate_limit_general_burst))
}

/// Apply the strict login/register per-IP limit to `router`.
pub fn limit_auth(router: Router<AppState>, cfg: &AppConfig, redis: &redis::Client) -> Router<AppState> {
    let per_minute = cfg.rate_limit_auth_per_minute.max(1);
    let period = Duration::from_millis((60_000 / per_minute.min(60_000)).max(1));
    apply(router, Limiter::new("auth", cfg, redis.clone(), period, cfg.rate_limit_auth_burst))
}

pub(crate) fn apply(router: Router<AppState>, limiter: Limiter) -> Router<AppState> {
    router.layer(axum::middleware::from_fn_with_state(Arc::new(limiter), enforce))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt;

    fn redis_url() -> String {
        std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/14".into())
    }

    /// A config whose Redis keys no other test (or earlier run) shares.
    fn cfg(trust_proxy: bool, hops: usize) -> AppConfig {
        let mut c = crate::test_support::test_config();
        c.rate_limit_key_prefix = format!("t{}", uuid::Uuid::new_v4().simple());
        c.trust_proxy_headers = trust_proxy;
        c.trusted_proxy_hops = hops;
        c
    }

    fn dead_redis() -> redis::Client {
        redis::Client::open("redis://127.0.0.1:1").unwrap()
    }

    fn live_redis() -> redis::Client {
        redis::Client::open(redis_url()).unwrap()
    }

    fn app_with(cfg: &AppConfig, redis: redis::Client, period: Duration, burst: u32) -> Router {
        let r: Router<AppState> = Router::new().route("/x", get(|| async { "ok" }));
        apply(r, Limiter::new("t", cfg, redis, period, burst)).with_state(crate::test_support::dummy_state())
    }

    async fn hit_via(app: &Router, peer: &str, xff: Option<&str>) -> Response {
        let mut req = Request::builder().uri("/x");
        if let Some(x) = xff {
            req = req.header("x-forwarded-for", x);
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(format!("{peer}:4000").parse::<SocketAddr>().unwrap()));
        app.clone().oneshot(req).await.unwrap()
    }

    async fn hit(app: &Router, ip: &str) -> StatusCode {
        hit_via(app, ip, None).await.status()
    }

    async fn status_via(app: &Router, peer: &str, xff: Option<&str>) -> StatusCode {
        hit_via(app, peer, xff).await.status()
    }

    // The behaviour below must hold with Redis (the normal case) and without it (the fallback),
    // so each test runs against both.
    fn backends() -> [(&'static str, redis::Client); 2] {
        [("redis", live_redis()), ("local fallback", dead_redis())]
    }

    #[tokio::test]
    async fn requests_beyond_the_burst_are_rejected_with_429_and_retry_after() {
        for (name, r) in backends() {
            let app = app_with(&cfg(false, 1), r, Duration::from_secs(3600), 3);
            for _ in 0..3 {
                assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::OK, "{name}");
            }
            let res = hit_via(&app, "198.51.100.1", None).await;
            assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "{name}");
            let wait: u64 = res.headers()[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
            assert!((1..=3600).contains(&wait), "{name}: {wait}");
        }
    }

    #[tokio::test]
    async fn successful_responses_report_the_remaining_allowance() {
        for (name, r) in backends() {
            let app = app_with(&cfg(false, 1), r, Duration::from_secs(3600), 3);
            let first = hit_via(&app, "198.51.100.3", None).await;
            assert_eq!(first.headers()["x-ratelimit-limit"], "3", "{name}");
            assert_eq!(first.headers()["x-ratelimit-remaining"], "2", "{name}");
            let second = hit_via(&app, "198.51.100.3", None).await;
            assert_eq!(second.headers()["x-ratelimit-remaining"], "1", "{name}");
        }
    }

    #[tokio::test]
    async fn each_client_ip_has_its_own_bucket() {
        for (name, r) in backends() {
            let app = app_with(&cfg(false, 1), r, Duration::from_secs(3600), 1);
            assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::OK, "{name}");
            assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::TOO_MANY_REQUESTS, "{name}");
            assert_eq!(hit(&app, "198.51.100.2").await, StatusCode::OK, "{name}: a different client is unaffected");
        }
    }

    #[tokio::test]
    async fn the_bucket_refills_over_time() {
        for (name, r) in backends() {
            let app = app_with(&cfg(false, 1), r, Duration::from_millis(200), 1);
            assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::OK, "{name}");
            assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::TOO_MANY_REQUESTS, "{name}");
            tokio::time::sleep(Duration::from_millis(450)).await;
            assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::OK, "{name}");
        }
    }

    #[tokio::test]
    async fn spoofed_forwarding_headers_do_not_change_the_bucket_by_default() {
        for (name, r) in backends() {
            let app = app_with(&cfg(false, 1), r, Duration::from_secs(3600), 1);
            let mut statuses = Vec::new();
            for i in 0..3 {
                statuses.push(status_via(&app, "198.51.100.50", Some(&format!("203.0.113.{i}"))).await);
            }
            assert_eq!(
                statuses,
                vec![StatusCode::OK, StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS],
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn behind_a_trusted_proxy_a_client_cannot_dodge_the_limit_by_prepending_addresses() {
        for (name, r) in backends() {
            // The proxy appends the real client address on the right; the left is client-chosen.
            let app = app_with(&cfg(true, 1), r, Duration::from_secs(3600), 1);
            assert_eq!(status_via(&app, "10.0.0.1", Some("1.1.1.1, 203.0.113.7")).await, StatusCode::OK, "{name}");
            for spoof in ["2.2.2.2", "3.3.3.3", "4.4.4.4"] {
                let xff = format!("{spoof}, 203.0.113.7");
                assert_eq!(
                    status_via(&app, "10.0.0.1", Some(&xff)).await,
                    StatusCode::TOO_MANY_REQUESTS,
                    "{name}: still the same client: {spoof}"
                );
            }
        }
    }

    #[tokio::test]
    async fn behind_a_trusted_proxy_different_real_clients_get_their_own_buckets() {
        for (name, r) in backends() {
            let app = app_with(&cfg(true, 1), r, Duration::from_secs(3600), 1);
            assert_eq!(status_via(&app, "10.0.0.1", Some("203.0.113.7")).await, StatusCode::OK, "{name}");
            assert_eq!(status_via(&app, "10.0.0.1", Some("203.0.113.8")).await, StatusCode::OK, "{name}");
            assert_eq!(status_via(&app, "10.0.0.1", Some("203.0.113.7")).await, StatusCode::TOO_MANY_REQUESTS, "{name}");
        }
    }

    #[tokio::test]
    async fn one_ipv6_network_is_one_client_not_billions() {
        for (name, r) in backends() {
            let app = app_with(&cfg(true, 1), r, Duration::from_secs(3600), 1);
            assert_eq!(status_via(&app, "10.0.0.1", Some("2001:db8:1:2::1")).await, StatusCode::OK, "{name}");
            assert_eq!(
                status_via(&app, "10.0.0.1", Some("2001:db8:1:2:ffff:eeee:dddd:cccc")).await,
                StatusCode::TOO_MANY_REQUESTS,
                "{name}"
            );
            assert_eq!(status_via(&app, "10.0.0.1", Some("2001:db8:1:3::1")).await, StatusCode::OK, "{name}: a different /64");
        }
    }

    #[tokio::test]
    async fn a_missing_header_falls_back_to_the_peer_rather_than_failing_open() {
        for (name, r) in backends() {
            let app = app_with(&cfg(true, 1), r, Duration::from_secs(3600), 1);
            assert_eq!(status_via(&app, "10.0.0.1", None).await, StatusCode::OK, "{name}");
            assert_eq!(status_via(&app, "10.0.0.1", None).await, StatusCode::TOO_MANY_REQUESTS, "{name}");
        }
    }

    #[tokio::test]
    async fn requests_with_no_determinable_address_share_one_bucket() {
        let app = app_with(&cfg(false, 1), dead_redis(), Duration::from_secs(3600), 1);
        let bare = || Request::builder().uri("/x").body(Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(bare()).await.unwrap().status(), StatusCode::OK);
        assert_eq!(app.clone().oneshot(bare()).await.unwrap().status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// The reason this limiter exists: two instances behind a load balancer must count together.
    #[tokio::test]
    async fn two_instances_sharing_redis_share_one_limit() {
        let c = cfg(false, 1);
        let a = app_with(&c, live_redis(), Duration::from_secs(3600), 3);
        let b = app_with(&c, live_redis(), Duration::from_secs(3600), 3);
        assert_eq!(hit(&a, "198.51.100.20").await, StatusCode::OK);
        assert_eq!(hit(&b, "198.51.100.20").await, StatusCode::OK);
        assert_eq!(hit(&a, "198.51.100.20").await, StatusCode::OK);
        // The fourth request overall is refused, whichever instance gets it.
        assert_eq!(hit(&b, "198.51.100.20").await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(hit(&a, "198.51.100.20").await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn instances_with_different_key_prefixes_do_not_share_a_limit() {
        let a = app_with(&cfg(false, 1), live_redis(), Duration::from_secs(3600), 1);
        let b = app_with(&cfg(false, 1), live_redis(), Duration::from_secs(3600), 1);
        assert_eq!(hit(&a, "198.51.100.21").await, StatusCode::OK);
        assert_eq!(hit(&b, "198.51.100.21").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn burst_requests_racing_across_instances_never_exceed_the_burst() {
        let c = cfg(false, 1);
        let apps: Vec<Router> =
            (0..4).map(|_| app_with(&c, live_redis(), Duration::from_secs(3600), 10)).collect();
        let mut tasks = Vec::new();
        for i in 0..40 {
            let app = apps[i % apps.len()].clone();
            tasks.push(tokio::spawn(async move { hit(&app, "198.51.100.22").await }));
        }
        let mut ok = 0;
        for t in tasks {
            if t.await.unwrap() == StatusCode::OK {
                ok += 1;
            }
        }
        assert_eq!(ok, 10, "the script is atomic, so exactly the burst gets through");
    }

    #[tokio::test]
    async fn a_redis_outage_degrades_to_per_instance_limiting_instead_of_failing_open_or_closed() {
        let app = app_with(&cfg(false, 1), dead_redis(), Duration::from_secs(3600), 2);
        let started = Instant::now();
        assert_eq!(hit(&app, "198.51.100.30").await, StatusCode::OK);
        assert_eq!(hit(&app, "198.51.100.30").await, StatusCode::OK);
        assert_eq!(hit(&app, "198.51.100.30").await, StatusCode::TOO_MANY_REQUESTS);
        // After the first failure Redis is not retried on every request, so there is no added delay.
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    }

    #[test]
    fn the_local_buckets_forget_finished_clients() {
        let b = LocalBuckets::new();
        for i in 0..(LOCAL_SWEEP_ABOVE as u32 + 10) {
            b.check(IpAddr::from(i.to_be_bytes()), 1, 1);
        }
        std::thread::sleep(Duration::from_millis(1100));
        b.check(IpAddr::from([255u8, 255, 255, 255]), 1, 1);
        let size = b.state.lock().unwrap().tat.len();
        assert!(size < 10, "finished buckets should have been swept, {size} remain");
    }

    #[test]
    fn the_local_algorithm_matches_a_token_bucket() {
        let b = LocalBuckets::new();
        let ip = IpAddr::from([1u8, 2, 3, 4]);
        assert_eq!(b.check(ip, 10_000, 3), Decision::Allowed { remaining: 2 });
        assert_eq!(b.check(ip, 10_000, 3), Decision::Allowed { remaining: 1 });
        assert_eq!(b.check(ip, 10_000, 3), Decision::Allowed { remaining: 0 });
        match b.check(ip, 10_000, 3) {
            Decision::Limited { retry_after_ms } => assert!((1..=10_000).contains(&retry_after_ms)),
            other => panic!("{other:?}"),
        }
    }
}
