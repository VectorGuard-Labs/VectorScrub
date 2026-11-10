//! Client IP for audit records.
//!
//! Uses the TCP peer address. Only when `TRUST_PROXY_HEADERS=true` (the service sits
//! behind a reverse proxy that overwrites these headers) does it prefer the forwarded
//! address, taking the same value the rate limiter keys on so the two always agree.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts},
    http::{request::Parts, HeaderMap},
};

use crate::db::AppState;

pub struct ClientIp(pub Option<IpAddr>);

/// Leftmost X-Forwarded-For entry, else X-Real-IP.
fn forwarded_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let from_xff = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse().ok());
    from_xff.or_else(|| {
        headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok())
    })
}

#[async_trait]
impl FromRequestParts<AppState> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.trust_proxy_headers {
            if let Some(ip) = forwarded_ip(&parts.headers) {
                return Ok(ClientIp(Some(ip)));
            }
        }
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        Ok(ClientIp(peer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn takes_the_leftmost_forwarded_address() {
        let h = headers(&[("x-forwarded-for", "203.0.113.7, 10.0.0.1")]);
        assert_eq!(forwarded_ip(&h), Some("203.0.113.7".parse().unwrap()));
    }

    #[test]
    fn falls_back_to_x_real_ip_and_ignores_garbage() {
        assert_eq!(
            forwarded_ip(&headers(&[("x-real-ip", "198.51.100.2")])),
            Some("198.51.100.2".parse().unwrap())
        );
        assert_eq!(forwarded_ip(&headers(&[("x-forwarded-for", "not-an-ip")])), None);
        assert_eq!(forwarded_ip(&HeaderMap::new()), None);
    }
}
