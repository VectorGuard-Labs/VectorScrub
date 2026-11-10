//! Client IP, for audit records and for rate limiting. One implementation serves both, so the
//! address that is rate-limited is the address that is recorded.
//!
//! By default this is the TCP peer. Only with `TRUST_PROXY_HEADERS=true` (the service sits
//! behind a reverse proxy) are forwarded headers read, and then NOT from the left.
//!
//! Why not the leftmost `X-Forwarded-For` entry: each proxy APPENDS the address it received
//! the request from, so the list reads `client-supplied..., real-client, proxy1, proxy2`. The
//! left side is whatever the client chose to send; only the entries our own proxies added, on
//! the right, can be trusted. `TRUSTED_PROXY_HOPS` says how many proxies of ours are in front
//! of this service (default 1); the client address is the entry that many places from the
//! right. With one proxy that is simply the last entry, which is also correct when the proxy
//! overwrites the header instead of appending (the list then has a single entry).
//!
//! If the header is missing, malformed, or shorter than the configured hop count, the TCP
//! peer is used: a misconfiguration degrades to "every client looks like the proxy", which is
//! safe, instead of to "clients choose their own address".
//!
//! `X-Real-IP` is consulted only when there is no `X-Forwarded-For` at all. The `Forwarded`
//! header is not read.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts},
    http::{request::Parts, HeaderMap},
};

use crate::db::AppState;

pub struct ClientIp(pub Option<IpAddr>);

/// The address forwarded by the trusted proxy chain, or None if the headers do not say.
fn forwarded_ip(headers: &HeaderMap, hops: usize) -> Option<Option<IpAddr>> {
    // Outer Option: were there forwarding headers at all? Inner: did they yield a usable address?
    let entries: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if entries.is_empty() {
        return headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().parse().ok());
    }
    let hops = hops.max(1);
    Some(
        entries
            .len()
            .checked_sub(hops)
            .and_then(|i| entries.get(i))
            .and_then(|s| s.parse().ok()),
    )
}

/// The client address for a request. See the module documentation.
pub fn client_ip(
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    trust_proxy_headers: bool,
    trusted_hops: usize,
) -> Option<IpAddr> {
    if trust_proxy_headers {
        if let Some(Some(ip)) = forwarded_ip(headers, trusted_hops) {
            return Some(ip);
        }
    }
    peer
}

/// The key a client is rate-limited under. IPv4 addresses are used as they are. An IPv6 user
/// is typically handed a whole /64, so keying by the full address would give one person
/// 2^64 separate buckets; they are keyed by the /64 instead.
pub fn rate_limit_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            let mapped = v6.to_ipv4_mapped();
            match mapped {
                Some(v4) => IpAddr::V4(v4),
                None => {
                    let s = v6.segments();
                    IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
                }
            }
        }
    }
}

pub fn peer_ip(parts: &Parts) -> Option<IpAddr> {
    parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip())
}

#[async_trait]
impl FromRequestParts<AppState> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(client_ip(
            &parts.headers,
            peer_ip(parts),
            state.config.trust_proxy_headers,
            state.config.trusted_proxy_hops,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, v.parse().unwrap());
        }
        h
    }
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    const PEER: &str = "10.0.0.1";

    fn resolve(h: &HeaderMap, trust: bool, hops: usize) -> IpAddr {
        client_ip(h, Some(ip(PEER)), trust, hops).unwrap()
    }

    #[test]
    fn without_trust_the_peer_is_used_and_headers_are_ignored() {
        let h = headers(&[("x-forwarded-for", "203.0.113.7"), ("x-real-ip", "203.0.113.8")]);
        assert_eq!(resolve(&h, false, 1), ip(PEER));
    }

    #[test]
    fn a_client_cannot_choose_its_address_by_prepending_to_x_forwarded_for() {
        // The client sends "6.6.6.6"; our proxy appends the address it actually saw.
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 203.0.113.7")]);
        assert_eq!(resolve(&h, true, 1), ip("203.0.113.7"), "the entry our proxy added, not the client's");
    }

    #[test]
    fn a_proxy_that_overwrites_the_header_works_too() {
        assert_eq!(resolve(&headers(&[("x-forwarded-for", "203.0.113.7")]), true, 1), ip("203.0.113.7"));
    }

    #[test]
    fn the_hop_count_is_counted_from_the_right() {
        // client -> CDN -> load balancer -> us:  "spoof, client, cdn"
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 198.51.100.9, 192.0.2.50")]);
        assert_eq!(resolve(&h, true, 1), ip("192.0.2.50"));
        assert_eq!(resolve(&h, true, 2), ip("198.51.100.9"));
        assert_eq!(resolve(&h, true, 3), ip("6.6.6.6"));
    }

    #[test]
    fn too_few_entries_for_the_hop_count_falls_back_to_the_peer() {
        let h = headers(&[("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(resolve(&h, true, 2), ip(PEER), "never reaches into client-controlled entries");
    }

    #[test]
    fn repeated_headers_are_treated_as_one_list() {
        let h = headers(&[("x-forwarded-for", "6.6.6.6"), ("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(resolve(&h, true, 1), ip("203.0.113.7"));
    }

    #[test]
    fn garbage_or_empty_headers_fall_back_to_the_peer() {
        assert_eq!(resolve(&headers(&[("x-forwarded-for", "not-an-ip")]), true, 1), ip(PEER));
        assert_eq!(resolve(&headers(&[("x-forwarded-for", "203.0.113.7, unknown")]), true, 1), ip(PEER));
        assert_eq!(resolve(&headers(&[("x-forwarded-for", " , ")]), true, 1), ip(PEER));
        assert_eq!(resolve(&HeaderMap::new(), true, 1), ip(PEER));
    }

    #[test]
    fn x_real_ip_is_used_only_when_there_is_no_x_forwarded_for() {
        assert_eq!(resolve(&headers(&[("x-real-ip", "198.51.100.2")]), true, 1), ip("198.51.100.2"));
        let both = headers(&[("x-forwarded-for", "203.0.113.7"), ("x-real-ip", "198.51.100.2")]);
        assert_eq!(resolve(&both, true, 1), ip("203.0.113.7"));
        assert_eq!(resolve(&headers(&[("x-real-ip", "junk")]), true, 1), ip(PEER));
    }

    #[test]
    fn the_forwarded_header_is_not_trusted() {
        assert_eq!(resolve(&headers(&[("forwarded", "for=6.6.6.6")]), true, 1), ip(PEER));
    }

    #[test]
    fn ipv6_clients_share_one_rate_limit_bucket_per_slash_64() {
        let a = rate_limit_key(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd"));
        let b = rate_limit_key(ip("2001:db8:1:2:1111:2222:3333:4444"));
        let other = rate_limit_key(ip("2001:db8:1:3::1"));
        assert_eq!(a, b);
        assert_ne!(a, other);
        assert_eq!(rate_limit_key(ip("203.0.113.7")), ip("203.0.113.7"));
        assert_eq!(rate_limit_key(ip("::ffff:203.0.113.7")), ip("203.0.113.7"), "mapped addresses key as IPv4");
    }
}
