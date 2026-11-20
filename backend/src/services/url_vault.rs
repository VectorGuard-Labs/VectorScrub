//! Listing links, encrypted at rest.
//!
//! Some brokers put the person's name in the address of their listing (a search URL such as
//! `/first-last?q=first+last`), so `exposures.source_url` and `removal_requests.source_url` are
//! personal data. They are stored sealed with the customer's own key (the same per-customer
//! AES-256-GCM key as the identity profile) and opened only at the API edge, when a response
//! carries the link to the customer or to an authenticated agent that must fetch it.
//!
//! Storage format: `enc2:` + base64(envelope blob, see `crypto::envelope`), in the same TEXT
//! column. `enc1:` is the older format (key derived from the master key); it is read only by the
//! migration (`vectorscrub kms migrate-legacy`), and `open` refuses it with a message saying so.
//! A value with neither prefix is a legacy plaintext row from before sealing existed; it still
//! reads (so nothing breaks mid-rollout) and `backfill` seals it at startup. Services in between only move
//! the opaque string: authorizing a removal copies the sealed value from the exposure, which is
//! valid because both belong to the same customer.
//!
//! Never log a value from here, sealed or open: log the host at most.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::envelope::purpose;
use crate::services::keys::KeyService;
use crate::models::exposure::Exposure;
use crate::models::removal_request::RemovalRequest;
use crate::services::monitoring::{MonitoringTarget, VerificationTarget};

pub const PREFIX: &str = "enc2:";
/// The previous sealed format, written before envelope encryption.
pub const LEGACY_PREFIX: &str = "enc1:";

pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(PREFIX) || stored.starts_with(LEGACY_PREFIX)
}

/// Seal a link for storage. Already-sealed input is returned unchanged, so sealing twice is harmless.
pub async fn seal(keys: &KeyService, customer_id: Uuid, url: &str) -> Result<String> {
    if is_sealed(url) {
        return Ok(url.to_string());
    }
    let ck = keys.customer(customer_id).await?;
    let blob = ck.seal_str(purpose::LISTING_LINK, url)?;
    Ok(format!("{PREFIX}{}", BASE64.encode(blob)))
}

/// Open a stored link. A legacy plaintext value is returned as it is.
pub async fn open(keys: &KeyService, customer_id: Uuid, stored: &str) -> Result<String> {
    if stored.starts_with(LEGACY_PREFIX) {
        anyhow::bail!("a listing link is in the old sealed format; run `vectorscrub kms migrate-legacy`");
    }
    let Some(encoded) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let blob = BASE64.decode(encoded).context("sealed link is not valid base64")?;
    let ck = keys.customer(customer_id).await?;
    ck.open_str(purpose::LISTING_LINK, &blob)
}

/// A response row that carries a stored link and the customer it belongs to.
pub trait CarriesLink {
    fn owner(&self) -> Uuid;
    fn link_mut(&mut self) -> &mut String;
}

impl CarriesLink for Exposure {
    fn owner(&self) -> Uuid { self.customer_id }
    fn link_mut(&mut self) -> &mut String { &mut self.source_url }
}
impl CarriesLink for RemovalRequest {
    fn owner(&self) -> Uuid { self.customer_id }
    fn link_mut(&mut self) -> &mut String { &mut self.source_url }
}
impl CarriesLink for MonitoringTarget {
    fn owner(&self) -> Uuid { self.customer_id }
    fn link_mut(&mut self) -> &mut String { &mut self.source_url }
}
impl CarriesLink for VerificationTarget {
    fn owner(&self) -> Uuid { self.customer_id }
    fn link_mut(&mut self) -> &mut String { &mut self.source_url }
}

/// Open the link on each row, in place. Fails the whole call if any link cannot be opened: a
/// blank or garbled link must never reach an agent that would try to fetch it.
pub async fn open_all<T: CarriesLink>(keys: &KeyService, rows: &mut [T]) -> Result<()> {
    for row in rows {
        let owner = row.owner();
        let opened = open(keys, owner, row.link_mut()).await?;
        *row.link_mut() = opened;
    }
    Ok(())
}

const BACKFILL_BATCH: i64 = 500;

/// Seal every legacy plaintext link in both tables. Idempotent and safe to run on every start:
/// each row is updated only if it still holds the plaintext that was read, so a concurrent write
/// is never overwritten. Returns how many links were sealed.
pub async fn backfill(pool: &PgPool, keys: &KeyService) -> Result<u64> {
    let mut sealed = 0u64;
    for table in ["exposures", "removal_requests"] {
        loop {
            let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(&format!(
                "SELECT id, customer_id, source_url FROM {table} WHERE source_url NOT LIKE 'enc2:%' AND source_url NOT LIKE 'enc1:%' LIMIT $1"
            ))
            .bind(BACKFILL_BATCH)
            .fetch_all(pool)
            .await
            .with_context(|| format!("read legacy links in {table}"))?;
            if rows.is_empty() {
                break;
            }
            for (id, customer_id, plain) in &rows {
                let sealed_value = seal(keys, *customer_id, plain).await?;
                let done = sqlx::query(&format!(
                    "UPDATE {table} SET source_url = $2 WHERE id = $1 AND source_url = $3"
                ))
                .bind(id)
                .bind(sealed_value)
                .bind(plain)
                .execute(pool)
                .await
                .with_context(|| format!("seal a link in {table}"))?;
                sealed += done.rows_affected();
            }
        }
    }
    Ok(sealed)
}
