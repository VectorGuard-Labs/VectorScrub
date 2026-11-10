//! Per-client-IP rate limiting (tower_governor, token bucket).
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
//! Client identity: the TCP peer address by default. With TRUST_PROXY_HEADERS=true the
//! forwarded headers are used instead; only enable that behind a proxy that overwrites
//! them, or a client can pick its own "IP" and walk around the limit.
//!
//! State is in-memory per backend instance: with N instances the effective limit is up
//! to N times higher. Per-account lockout (services/login_throttle.rs) is in Postgres
//! and is exact across instances.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    http::{header, StatusCode},
    response::Response,
    Router,
};
use tower_governor::{
    governor::GovernorConfigBuilder,
    key_extractor::{KeyExtractor, PeerIpKeyExtractor, SmartIpKeyExtractor},
    GovernorLayer,
};

use crate::db::AppState;
use crate::services::config::AppConfig;

/// Apply the general per-IP limit to `router`.
pub fn limit_general(router: Router<AppState>, cfg: &AppConfig) -> Router<AppState> {
    let per_second = cfg.rate_limit_general_per_second.max(1);
    let period = Duration::from_millis(1000 / per_second.min(1000));
    with_extractor(router, cfg, period, cfg.rate_limit_general_burst.max(1))
}

/// Apply the strict login/register per-IP limit to `router`.
pub fn limit_auth(router: Router<AppState>, cfg: &AppConfig) -> Router<AppState> {
    let per_minute = cfg.rate_limit_auth_per_minute.max(1);
    let period = Duration::from_millis(60_000 / per_minute.min(60_000));
    with_extractor(router, cfg, period, cfg.rate_limit_auth_burst.max(1))
}

fn with_extractor(
    router: Router<AppState>,
    cfg: &AppConfig,
    period: Duration,
    burst: u32,
) -> Router<AppState> {
    if cfg.trust_proxy_headers {
        apply(router, period, burst, SmartIpKeyExtractor)
    } else {
        apply(router, period, burst, PeerIpKeyExtractor)
    }
}

/// One token is replenished every `period`; `burst` is the bucket size.
pub(crate) fn apply<K>(
    router: Router<AppState>,
    period: Duration,
    burst: u32,
    extractor: K,
) -> Router<AppState>
where
    K: KeyExtractor + Send + Sync + 'static,
    K::Key: Send + Sync + 'static,
{
    let mut builder = GovernorConfigBuilder::default().key_extractor(extractor);
    builder.period(period).burst_size(burst);
    // `use_headers` adds x-ratelimit-* to every response and Retry-After to rejections, so
    // well-behaved clients can back off instead of hammering.
    let config = Arc::new(
        builder
            .use_headers()
            .finish()
            .expect("rate limit period and burst are non-zero"),
    );

    // Forget clients that have gone quiet so the table cannot grow without bound.
    let limiter = config.limiter().clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            limiter.retain_recent();
        }
    });

    // The mapper sits OUTSIDE the limiter so it sees the 429 the limiter produces.
    router
        .layer(GovernorLayer { config })
        .layer(axum::middleware::map_response(add_retry_after))
}

/// tower_governor reports the wait in the non-standard `x-ratelimit-after` header. Mirror it
/// into the standard `Retry-After` that HTTP clients, proxies and CDNs understand. A
/// `Retry-After` set by a handler (e.g. login lockout) is left alone.
async fn add_retry_after(mut res: Response) -> Response {
    if res.status() == StatusCode::TOO_MANY_REQUESTS && !res.headers().contains_key(header::RETRY_AFTER) {
        if let Some(wait) = res.headers().get("x-ratelimit-after").cloned() {
            res.headers_mut().insert(header::RETRY_AFTER, wait);
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        routing::get,
    };
    use std::net::SocketAddr;
    use tower::ServiceExt;

    fn app(period: Duration, burst: u32) -> Router {
        // State is irrelevant to the limiter; use a unit-state router for the test.
        let r: Router<AppState> = Router::new().route("/x", get(|| async { "ok" }));
        let r = apply(r, period, burst, PeerIpKeyExtractor);
        // `Router<AppState>` needs state to become a service; the handler ignores it, so
        // build the minimal AppState-free path by converting via with_state on a dummy.
        r.with_state(crate::test_support::dummy_state())
    }

    async fn hit(app: &Router, ip: &str) -> StatusCode {
        let mut req = Request::builder().uri("/x").body(Body::empty()).unwrap();
        let addr: SocketAddr = format!("{ip}:4000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn requests_beyond_the_burst_are_rejected_with_429() {
        let app = app(Duration::from_secs(3600), 3);
        for _ in 0..3 {
            assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::OK);
        }
        assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn each_client_ip_has_its_own_bucket() {
        let app = app(Duration::from_secs(3600), 1);
        assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::OK);
        assert_eq!(hit(&app, "198.51.100.1").await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(hit(&app, "198.51.100.2").await, StatusCode::OK, "a different client is unaffected");
    }

    #[tokio::test]
    async fn the_bucket_refills_over_time() {
        let app = app(Duration::from_millis(100), 1);
        assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::OK);
        assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::TOO_MANY_REQUESTS);
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert_eq!(hit(&app, "198.51.100.9").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn spoofed_forwarding_headers_do_not_change_the_bucket_by_default() {
        // PeerIpKeyExtractor ignores X-Forwarded-For, so rotating it cannot dodge the limit.
        let app = app(Duration::from_secs(3600), 1);
        let mut statuses = Vec::new();
        for i in 0..3 {
            let mut req = Request::builder()
                .uri("/x")
                .header("x-forwarded-for", format!("203.0.113.{i}"))
                .body(Body::empty())
                .unwrap();
            req.extensions_mut().insert(ConnectInfo("198.51.100.50:1".parse::<SocketAddr>().unwrap()));
            statuses.push(app.clone().oneshot(req).await.unwrap().status());
        }
        assert_eq!(statuses, vec![StatusCode::OK, StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS]);
    }
}
