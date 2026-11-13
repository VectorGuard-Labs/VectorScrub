//! Listings the customer found themselves.
//!
//! Most brokers cannot be scanned, so a customer who finds their own listing can add its link and
//! have it tracked: it becomes an exposure (`origin = 'customer'`) with a removal request already
//! in the "manual action required" state. They do the opt-out by hand, report it, and the usual
//! re-checks follow. Nothing here claims the listing is theirs beyond their word; the re-check
//! only decides whether the page still shows them.
//!
//! The link is sealed with the customer's key like any other (services/url_vault.rs), and later
//! fetched only through the agents' public-address-only fetcher confined to the link's own site.

use anyhow::{Context, Result};
use sqlx::PgPool;
use url::{Host, Url};
use uuid::Uuid;

use crate::models::exposure::Exposure;

pub const MAX_OWN_LISTINGS_FREE: i64 = 5;
pub const MAX_OWN_LISTINGS_PAID: i64 = 25;

/// Shown in the removal's failure reason, so the dashboard explains why it is a manual one.
pub const SELF_ADDED_NOTE: &str = "You added this listing yourself, so the opt-out is done by hand.";

#[derive(Debug, thiserror::Error)]
pub enum ListingError {
    #[error("Enter the full https:// link to your listing on the site.")]
    BadUrl,
    #[error("That is the site's home page. Open the page about you and copy its address.")]
    NotAListing,
    #[error("You are already tracking a listing on this site.")]
    AlreadyTracked,
    #[error("You can track up to {0} listings you found yourself. Remove one or upgrade.")]
    TooMany(i64),
}

/// A link we are willing to track: https, a real site name (no address, no credentials, no
/// odd port), at most 2000 characters. Returns the site name (lowercase, without a leading "www.").
pub fn site_of(raw: &str) -> Result<(String, String), ListingError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2000 {
        return Err(ListingError::BadUrl);
    }
    let url = Url::parse(raw).map_err(|_| ListingError::BadUrl)?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(ListingError::BadUrl);
    }
    if url.port().is_some() {
        return Err(ListingError::BadUrl);
    }
    let Some(Host::Domain(host)) = url.host() else {
        return Err(ListingError::BadUrl); // an IP address, or no host
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    // a real public name has a dot and a TLD of letters (rules out "localhost", "intranet", "x.local")
    let tld = host.rsplit('.').next().unwrap_or("");
    if !host.contains('.') || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) || host.ends_with(".local") || host.ends_with(".internal") {
        return Err(ListingError::BadUrl);
    }
    // A home page never names anyone, so tracking one would look "removed" at the first check.
    // Insist on a page: some path beyond "/" or a query.
    if url.path().trim_matches('/').is_empty() && url.query().is_none_or(str::is_empty) {
        return Err(ListingError::NotAListing);
    }
    Ok((host, url.to_string()))
}

/// Track a listing the customer found. `sealed_url` is the link already sealed for this customer;
/// `site` comes from `site_of`. Creates the exposure and a failed (= manual action required)
/// removal request in one transaction.
pub async fn add_listing(pool: &PgPool, customer_id: Uuid, tier: &str, site: &str, sealed_url: &str) -> Result<Exposure> {
    let mut tx = pool.begin().await.context("begin")?;

    // Serialize this customer's additions so the cap and the duplicate check hold.
    sqlx::query("SELECT id FROM customers WHERE id = $1 FOR UPDATE").bind(customer_id).fetch_one(&mut *tx).await.context("lock customer")?;

    let cap = if tier == "free" { MAX_OWN_LISTINGS_FREE } else { MAX_OWN_LISTINGS_PAID };
    let open: i64 = sqlx::query_scalar("SELECT count(*) FROM exposures WHERE customer_id = $1 AND origin = 'customer' AND is_active")
        .bind(customer_id).fetch_one(&mut *tx).await.context("count")?;
    if open >= cap {
        return Err(ListingError::TooMany(cap).into());
    }
    let dup: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM exposures WHERE customer_id = $1 AND source_name = $2 AND pii_category = 'full_name')")
        .bind(customer_id).bind(site).fetch_one(&mut *tx).await.context("duplicate check")?;
    if dup {
        return Err(ListingError::AlreadyTracked.into());
    }

    let exposure: Exposure = sqlx::query_as(
        r#"
        INSERT INTO exposures (id, customer_id, scan_job_id, origin, source_name, source_url,
                               pii_category, severity, pii_summary, confidence, is_active)
        VALUES ($1, $2, NULL, 'customer', $3, $4, 'full_name', 'medium', 'Added by you', 1.0, true)
        RETURNING *
        "#,
    )
    .bind(Uuid::new_v4()).bind(customer_id).bind(site).bind(sealed_url)
    .fetch_one(&mut *tx).await.context("insert exposure")?;

    let request_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO removal_requests (id, customer_id, exposure_id, source_name, source_url, status,
                                      customer_authorized_at, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, 'failed', NOW(), NOW(), NOW())
        "#,
    )
    .bind(request_id).bind(customer_id).bind(exposure.id).bind(site).bind(sealed_url)
    .execute(&mut *tx).await.context("insert request")?;
    sqlx::query(
        "INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, 'failed', 'system', $3)",
    )
    .bind(request_id).bind(customer_id).bind(serde_json::json!({ "error": SELF_ADDED_NOTE }))
    .execute(&mut *tx).await.context("audit")?;

    tx.commit().await.context("commit")?;
    Ok(exposure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_links_to_real_sites_are_accepted() {
        let ok = site_of(" https://www.Spokeo.com/First-Last?q=a+b ").unwrap();
        assert_eq!(ok.0, "spokeo.com");
        for bad in [
            "", "spokeo.com/x", "http://spokeo.com/x", "ftp://spokeo.com", "javascript:alert(1)",
            "https://user:pw@spokeo.com/x", "https://spokeo.com:8443/x", "https://127.0.0.1/x",
            "https://[::1]/x", "https://localhost/x", "https://intranet/x", "https://printer.local/x",
            "https://x.internal/p", "https://a.b1/p", "https:///x",
        ] {
            assert!(site_of(bad).is_err(), "{bad}");
        }
        assert!(site_of(&format!("https://spokeo.com/{}", "a".repeat(2000))).is_err());
        for home in ["https://www.spokeo.com", "https://www.spokeo.com/", "https://spokeo.com//", "https://spokeo.com/?"] {
            assert!(matches!(site_of(home), Err(ListingError::NotAListing)), "{home}");
        }
        assert!(site_of("https://www.spokeo.com/search?q=a").is_ok());
        assert!(site_of("https://thatsthem.com/name/Jane-Smith").is_ok());
    }
}
