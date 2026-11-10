//! Scoped release of decrypted customer identity to the agent layer.
//!
//! Problem this fixes: the agent key used to be a skeleton key. `GET
//! /internal/customers/:id/identity` returned any customer's decrypted identity to
//! anyone holding the key, and the `identity_token` sent on the scan endpoint was
//! generated but never checked.
//!
//! Now identity is released only against a concrete work item that is currently
//! actionable, and the customer is DERIVED from that item (never supplied by the
//! caller):
//!
//! * scan: the job must exist, be queued/running, and the caller must present the
//!   scoped token minted for exactly that job (see ScopedToken);
//! * removal: the removal request must be `authorized` (about to be submitted) or
//!   `submitted`/`acknowledged` and due for verification;
//! * monitoring: the removed exposure must be due for a reappearance check for an
//!   eligible customer.
//!
//! Every release is written to the audit log, and the release FAILS if that write
//! fails: identity is never handed out without a record. Denied attempts are logged
//! too. A holder of the agent key can therefore only obtain identities for customers
//! that currently have actionable work, one item at a time, with a trail.
//!
//! Not solved here: the agent key is still a single shared secret, and the released
//! identity still contains every field (name, address, phones, emails, DOB) because
//! redaction and form filling need them. Per-field minimization per purpose would be
//! a further step.

use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::crypto::{decrypt_str, derive_customer_key, encrypt_str, MasterKey};
use crate::models::customer::CleartextIdentity;
use crate::services::{audit, customer as customer_svc, monitoring};

/// How long a scan token stays valid. Must exceed the longest a job can sit in the
/// queue; the recovery loop mints a fresh token whenever it re-dispatches a job.
pub const SCAN_TOKEN_TTL_SECS: i64 = 24 * 60 * 60;
/// Tolerated clock difference when checking `iat`.
const CLOCK_SKEW_SECS: i64 = 60;

const TOKEN_VERSION: u8 = 1;
const PURPOSE_SCAN: &str = "scan";

/// Plaintext of a scan token. It is encrypted with the customer's own derived key
/// (AES-256-GCM), so it cannot be forged or edited without the master key, and a
/// token minted for one customer cannot even be decrypted under another's key.
#[derive(Debug, Serialize, Deserialize)]
struct ScopedToken {
    v: u8,
    purpose: String,
    customer_id: Uuid,
    job_id: Uuid,
    iat: i64,
    exp: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    /// Deliberately uninformative: do not tell a caller why access was refused.
    #[error("identity access denied")]
    Denied,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Scan tokens
// ---------------------------------------------------------------------------

/// Mint a token that authorizes fetching identity for exactly one scan job.
pub fn issue_scan_token(
    master: &MasterKey,
    customer_id: Uuid,
    job_id: Uuid,
) -> anyhow::Result<String> {
    issue_scan_token_at(master, customer_id, job_id, OffsetDateTime::now_utc().unix_timestamp())
}

fn issue_scan_token_at(
    master: &MasterKey,
    customer_id: Uuid,
    job_id: Uuid,
    now: i64,
) -> anyhow::Result<String> {
    let token = ScopedToken {
        v: TOKEN_VERSION,
        purpose: PURPOSE_SCAN.to_string(),
        customer_id,
        job_id,
        iat: now,
        exp: now + SCAN_TOKEN_TTL_SECS,
    };
    let key = derive_customer_key(master, customer_id);
    let blob = encrypt_str(&key, &serde_json::to_string(&token)?)?;
    Ok(BASE64.encode(blob))
}

fn verify_scan_token(
    master: &MasterKey,
    customer_id: Uuid,
    job_id: Uuid,
    token: &str,
    now: i64,
) -> Result<(), AccessError> {
    let blob = BASE64.decode(token.trim()).map_err(|_| AccessError::Denied)?;
    // Wrong customer => wrong derived key => authentication failure here.
    let json = decrypt_str(&derive_customer_key(master, customer_id), &blob)
        .map_err(|_| AccessError::Denied)?;
    let t: ScopedToken = serde_json::from_str(&json).map_err(|_| AccessError::Denied)?;

    let ok = t.v == TOKEN_VERSION
        && t.purpose == PURPOSE_SCAN
        && t.customer_id == customer_id
        && t.job_id == job_id
        && t.iat <= now + CLOCK_SKEW_SECS
        && t.exp > now;
    if ok {
        Ok(())
    } else {
        Err(AccessError::Denied)
    }
}

// ---------------------------------------------------------------------------
// Authorization checks (database state)
// ---------------------------------------------------------------------------

async fn authorize_scan(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    job_id: Uuid,
    token: &str,
    now: i64,
) -> Result<(), AccessError> {
    verify_scan_token(master, customer_id, job_id, token, now)?;

    // The token proves the job was issued; the row proves it is still wanted.
    let live: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT j.id FROM scan_jobs j
        JOIN customers c ON c.id = j.customer_id
        WHERE j.id = $1 AND j.customer_id = $2
          AND j.status IN ('queued', 'running')
          AND c.is_active = true
        "#,
    )
    .bind(job_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;
    live.map(|_| ()).ok_or(AccessError::Denied)
}

/// Customer for a removal request that is currently actionable, or Denied.
async fn authorize_removal(pool: &PgPool, request_id: Uuid) -> Result<Uuid, AccessError> {
    let sql = format!(
        r#"
        SELECT r.customer_id FROM removal_requests r
        JOIN customers c ON c.id = r.customer_id
        WHERE r.id = $1 AND c.is_active = true
          AND (r.status = 'authorized'
               OR (r.status IN ('submitted', 'acknowledged') AND {}))
        "#,
        monitoring::verification_due_predicate("r")
    );
    sqlx::query_scalar::<_, Uuid>(&sql)
        .bind(request_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AccessError::Denied)
}

/// Customer for a removed exposure that is currently due for a reappearance check.
async fn authorize_monitoring(pool: &PgPool, exposure_id: Uuid) -> Result<Uuid, AccessError> {
    let sql = format!(
        r#"
        SELECT e.customer_id FROM exposures e
        JOIN customers c ON c.id = e.customer_id
        WHERE e.id = $1 AND {}
        "#,
        monitoring::monitoring_due_predicate("$2", "$3")
    );
    sqlx::query_scalar::<_, Uuid>(&sql)
        .bind(exposure_id)
        .bind(monitoring::PREMIUM_INTERVAL_DAYS)
        .bind(monitoring::PERSONAL_INTERVAL_DAYS)
        .fetch_optional(pool)
        .await?
        .ok_or(AccessError::Denied)
}

// ---------------------------------------------------------------------------
// Release (authorize, decrypt, audit)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Purpose {
    Scan,
    Removal,
    Monitoring,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Purpose::Scan => "scan",
            Purpose::Removal => "removal",
            Purpose::Monitoring => "monitoring",
        }
    }
}

async fn record_denied(pool: &PgPool, purpose: Purpose, scope_id: Uuid) {
    let _ = audit::AuditEntry::builder(audit::actions::IDENTITY_RELEASE, audit::resources::IDENTITY)
        .resource_id(scope_id)
        .status(audit::AuditStatus::Failure)
        .error_message("access denied")
        .details(serde_json::json!({ "purpose": purpose.as_str() }))
        .log(pool)
        .await;
}

/// Decrypt and return the customer's identity, but only after the release has been
/// written to the audit log. If the audit write fails, nothing is returned.
async fn decrypt_and_audit(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    purpose: Purpose,
    scope_id: Uuid,
) -> Result<CleartextIdentity, AccessError> {
    let identity = customer_svc::load_cleartext_identity(pool, master, customer_id)
        .await
        .context("load identity")?;

    audit::AuditEntry::builder(audit::actions::IDENTITY_RELEASE, audit::resources::IDENTITY)
        .customer_id(customer_id)
        .resource_id(scope_id)
        .details(serde_json::json!({ "purpose": purpose.as_str() }))
        .log(pool)
        .await?;

    Ok(identity)
}

/// Identity for a scan job, against the token minted for that job.
pub async fn release_for_scan(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    job_id: Uuid,
    token: &str,
) -> Result<CleartextIdentity, AccessError> {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if let Err(e) = authorize_scan(pool, master, customer_id, job_id, token, now).await {
        if matches!(e, AccessError::Denied) {
            record_denied(pool, Purpose::Scan, job_id).await;
        }
        return Err(e);
    }
    decrypt_and_audit(pool, master, customer_id, Purpose::Scan, job_id).await
}

/// Identity for submitting or verifying a removal request. The customer comes from
/// the request itself.
pub async fn release_for_removal(
    pool: &PgPool,
    master: &MasterKey,
    request_id: Uuid,
) -> Result<CleartextIdentity, AccessError> {
    let customer_id = match authorize_removal(pool, request_id).await {
        Ok(id) => id,
        Err(e) => {
            if matches!(e, AccessError::Denied) {
                record_denied(pool, Purpose::Removal, request_id).await;
            }
            return Err(e);
        }
    };
    decrypt_and_audit(pool, master, customer_id, Purpose::Removal, request_id).await
}

/// Identity for a reappearance check. The customer comes from the exposure itself.
pub async fn release_for_monitoring(
    pool: &PgPool,
    master: &MasterKey,
    exposure_id: Uuid,
) -> Result<CleartextIdentity, AccessError> {
    let customer_id = match authorize_monitoring(pool, exposure_id).await {
        Ok(id) => id,
        Err(e) => {
            if matches!(e, AccessError::Denied) {
                record_denied(pool, Purpose::Monitoring, exposure_id).await;
            }
            return Err(e);
        }
    };
    decrypt_and_audit(pool, master, customer_id, Purpose::Monitoring, exposure_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use sqlx::PgPool;

    fn master() -> MasterKey {
        MasterKey::from_base64(&ts::master_key_b64()).unwrap()
    }

    // ---- token ------------------------------------------------------------

    #[test]
    fn token_round_trips_for_the_job_it_was_minted_for() {
        let (m, c, j) = (master(), Uuid::new_v4(), Uuid::new_v4());
        let t = issue_scan_token_at(&m, c, j, 1_000).unwrap();
        assert!(verify_scan_token(&m, c, j, &t, 1_000).is_ok());
        assert!(verify_scan_token(&m, c, j, &t, 1_000 + SCAN_TOKEN_TTL_SECS - 1).is_ok());
    }

    #[test]
    fn token_for_one_job_is_rejected_for_another() {
        let (m, c) = (master(), Uuid::new_v4());
        let t = issue_scan_token_at(&m, c, Uuid::new_v4(), 1_000).unwrap();
        assert!(matches!(
            verify_scan_token(&m, c, Uuid::new_v4(), &t, 1_000),
            Err(AccessError::Denied)
        ));
    }

    #[test]
    fn token_for_one_customer_is_rejected_for_another() {
        let (m, j) = (master(), Uuid::new_v4());
        let t = issue_scan_token_at(&m, Uuid::new_v4(), j, 1_000).unwrap();
        assert!(matches!(
            verify_scan_token(&m, Uuid::new_v4(), j, &t, 1_000),
            Err(AccessError::Denied)
        ));
    }

    #[test]
    fn expired_and_future_dated_tokens_are_rejected() {
        let (m, c, j) = (master(), Uuid::new_v4(), Uuid::new_v4());
        let t = issue_scan_token_at(&m, c, j, 1_000).unwrap();
        assert!(verify_scan_token(&m, c, j, &t, 1_000 + SCAN_TOKEN_TTL_SECS).is_err(), "expired");
        assert!(verify_scan_token(&m, c, j, &t, 1_000 - CLOCK_SKEW_SECS - 10).is_err(), "issued in the future");
    }

    #[test]
    fn garbage_and_tampered_tokens_are_rejected() {
        let (m, c, j) = (master(), Uuid::new_v4(), Uuid::new_v4());
        for bad in ["", "not base64!!", "AAAA", &BASE64.encode(b"short")] {
            assert!(verify_scan_token(&m, c, j, bad, 1_000).is_err(), "{bad:?}");
        }
        let mut blob = BASE64.decode(issue_scan_token_at(&m, c, j, 1_000).unwrap()).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert!(verify_scan_token(&m, c, j, &BASE64.encode(blob), 1_000).is_err());
    }

    #[test]
    fn a_token_minted_under_a_different_master_key_is_rejected() {
        let (c, j) = (Uuid::new_v4(), Uuid::new_v4());
        let other = MasterKey::from_base64(&ts::other_master_key_b64()).unwrap();
        let t = issue_scan_token_at(&other, c, j, 1_000).unwrap();
        assert!(verify_scan_token(&master(), c, j, &t, 1_000).is_err());
    }

    // ---- scan release ------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn scan_release_requires_token_and_live_job(pool: PgPool) {
        let m = master();
        let cust = ts::customer(&pool, "premium", true).await;
        ts::identity(&pool, &m, cust, "Jane Smith").await;
        let job = ts::scan_job(&pool, cust, "queued").await;
        let token = issue_scan_token(&m, cust, job).unwrap();

        let id = release_for_scan(&pool, &m, cust, job, &token).await.expect("released");
        assert_eq!(id.full_name, "Jane Smith");

        // wrong token / other job's token / no token
        let other_job = ts::scan_job(&pool, cust, "queued").await;
        let other_token = issue_scan_token(&m, cust, other_job).unwrap();
        for bad in [other_token.as_str(), "", "garbage"] {
            assert!(matches!(
                release_for_scan(&pool, &m, cust, job, bad).await,
                Err(AccessError::Denied)
            ));
        }

        // a finished job no longer qualifies even with its own valid token
        ts::set_job_status(&pool, job, "completed").await;
        assert!(matches!(
            release_for_scan(&pool, &m, cust, job, &token).await,
            Err(AccessError::Denied)
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn another_customers_job_cannot_be_used_to_read_identity(pool: PgPool) {
        let m = master();
        let (alice, bob) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        ts::identity(&pool, &m, alice, "Alice A").await;
        ts::identity(&pool, &m, bob, "Bob B").await;
        let alice_job = ts::scan_job(&pool, alice, "queued").await;
        let alice_token = issue_scan_token(&m, alice, alice_job).unwrap();

        // claim to be Bob while presenting Alice's job and token
        assert!(matches!(
            release_for_scan(&pool, &m, bob, alice_job, &alice_token).await,
            Err(AccessError::Denied)
        ));
    }

    // ---- removal release ---------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn removal_release_is_limited_to_actionable_requests(pool: PgPool) {
        let m = master();
        let cust = ts::customer(&pool, "personal", true).await;
        ts::identity(&pool, &m, cust, "Jane Smith").await;
        let exp = ts::exposure(&pool, cust, true, None).await;

        // authorized -> about to be submitted: allowed
        let authorized = ts::removal_request(&pool, cust, exp, "authorized", None).await;
        assert_eq!(release_for_removal(&pool, &m, authorized).await.unwrap().full_name, "Jane Smith");

        // submitted but not yet due for verification: denied
        let not_due = ts::removal_request(&pool, cust, ts::exposure(&pool, cust, true, None).await, "submitted", Some(5)).await;
        assert!(matches!(release_for_removal(&pool, &m, not_due).await, Err(AccessError::Denied)));

        // submitted and due: allowed
        let due = ts::removal_request(&pool, cust, ts::exposure(&pool, cust, true, None).await, "submitted", Some(-1)).await;
        assert!(release_for_removal(&pool, &m, due).await.is_ok());

        // terminal states: denied
        for status in ["verified_removed", "rejected", "unverifiable", "failed", "reappeared"] {
            let e = ts::exposure(&pool, cust, true, None).await;
            let r = ts::removal_request(&pool, cust, e, status, Some(-1)).await;
            assert!(matches!(release_for_removal(&pool, &m, r).await, Err(AccessError::Denied)), "{status}");
        }

        // unknown id: denied
        assert!(matches!(release_for_removal(&pool, &m, Uuid::new_v4()).await, Err(AccessError::Denied)));
    }

    // ---- monitoring release -------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn monitoring_release_is_limited_to_due_eligible_exposures(pool: PgPool) {
        let m = master();
        let premium = ts::customer(&pool, "premium", true).await;
        let free = ts::customer(&pool, "free", true).await;
        let disabled = ts::customer(&pool, "premium", false).await;
        for c in [premium, free, disabled] {
            ts::identity(&pool, &m, c, "Jane Smith").await;
        }

        let due = ts::exposure(&pool, premium, false, Some(8)).await;
        let not_due = ts::exposure(&pool, premium, false, Some(3)).await;
        let active = ts::exposure(&pool, premium, true, None).await;
        let free_exp = ts::exposure(&pool, free, false, Some(100)).await;
        let disabled_exp = ts::exposure(&pool, disabled, false, Some(100)).await;

        assert!(release_for_monitoring(&pool, &m, due).await.is_ok());
        for denied in [not_due, active, free_exp, disabled_exp, Uuid::new_v4()] {
            assert!(matches!(release_for_monitoring(&pool, &m, denied).await, Err(AccessError::Denied)));
        }
    }

    // ---- audit ---------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn every_release_and_every_denial_is_audited(pool: PgPool) {
        let m = master();
        let cust = ts::customer(&pool, "personal", true).await;
        ts::identity(&pool, &m, cust, "Jane Smith").await;
        let exp = ts::exposure(&pool, cust, true, None).await;
        let req = ts::removal_request(&pool, cust, exp, "authorized", None).await;

        release_for_removal(&pool, &m, req).await.unwrap();
        let _ = release_for_removal(&pool, &m, Uuid::new_v4()).await;

        let rows: Vec<(String, String, Option<Uuid>)> = sqlx::query_as(
            "SELECT status, details->>'purpose', customer_id FROM audit_log WHERE action = 'identity.release' ORDER BY timestamp",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("success".into(), "removal".into(), Some(cust)));
        assert_eq!(rows[1].0, "failure");
        assert_eq!(rows[1].1, "removal");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn if_the_audit_write_fails_the_identity_is_not_released(pool: PgPool) {
        let m = master();
        let cust = ts::customer(&pool, "personal", true).await;
        ts::identity(&pool, &m, cust, "Jane Smith").await;
        let exp = ts::exposure(&pool, cust, true, None).await;
        let req = ts::removal_request(&pool, cust, exp, "authorized", None).await;

        // Make audit inserts fail without touching anything else.
        sqlx::query("ALTER TABLE audit_log RENAME TO audit_log_gone").execute(&pool).await.unwrap();

        assert!(release_for_removal(&pool, &m, req).await.is_err(), "no record, no release");
    }
}
