//! One-time move of data written before envelope encryption (format 1) to format 2.
//!
//! Format 1 derived every customer's key from one master key held in the environment, with no
//! associated data. This reads each such value with that master key and writes it back under the
//! customer's own data key (`crypto::envelope`), field by field, one customer per transaction.
//!
//! What it touches: the six encrypted columns of `identity_profiles`, `customer_mfa.secret_enc`,
//! and the sealed listing links (`enc1:` to `enc2:`) in `exposures` and `removal_requests`.
//! What it does not: Redis. Scan tokens queued under the old scheme stop validating; the scan
//! recovery loop puts those jobs back on the queue with fresh tokens.
//!
//! Safe to run twice, and safe to stop halfway: a value already in format 2 is left alone, and a
//! customer's rows change together or not at all. Run `vectorscrub verify-backup` afterwards, then
//! take the master key out of the environment.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::envelope::{self, purpose};
use crate::crypto::{self, MasterKey};
use crate::services::keys::KeyService;
use crate::services::url_vault::{LEGACY_PREFIX, PREFIX};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub customers: u64,
    pub identity_profiles: u64,
    pub mfa_secrets: u64,
    pub links: u64,
    pub already_current: u64,
    pub failed_customers: u64,
}

/// Re-encrypt `old` (format 1, under `master`) as `purpose` of this customer. A value already in
/// format 2 is returned as `None` (nothing to do).
fn convert(
    master: &MasterKey,
    ck: &envelope::CustomerKeys,
    purpose: &str,
    old: &[u8],
) -> Result<Option<Vec<u8>>> {
    if envelope::is_v2(old) {
        return Ok(None);
    }
    let legacy_key = crypto::derive_customer_key(master, ck.customer_id());
    let plain = crypto::decrypt(&legacy_key, old).context("a value does not open with the old master key")?;
    Ok(Some(ck.seal(purpose, &plain)?))
}

pub async fn migrate_legacy(pool: &PgPool, keys: &KeyService, master: &MasterKey) -> Result<MigrationReport> {
    let mut report = MigrationReport::default();
    let customers: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM customers WHERE \
            EXISTS (SELECT 1 FROM identity_profiles WHERE customer_id = customers.id) \
         OR EXISTS (SELECT 1 FROM customer_mfa WHERE customer_id = customers.id) \
         OR EXISTS (SELECT 1 FROM exposures WHERE customer_id = customers.id AND source_url LIKE 'enc1:%') \
         OR EXISTS (SELECT 1 FROM removal_requests WHERE customer_id = customers.id AND source_url LIKE 'enc1:%') \
         ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    for customer in customers {
        match migrate_customer(pool, keys, master, customer).await {
            Ok(part) => {
                report.customers += 1;
                report.identity_profiles += part.identity_profiles;
                report.mfa_secrets += part.mfa_secrets;
                report.links += part.links;
                report.already_current += part.already_current;
            }
            Err(e) => {
                report.failed_customers += 1;
                // The customer id, never a value from their data.
                tracing::error!(customer_id = %customer, error = %e, "Could not migrate a customer's data; it is unchanged");
            }
        }
    }
    Ok(report)
}

async fn migrate_customer(pool: &PgPool, keys: &KeyService, master: &MasterKey, customer: Uuid) -> Result<MigrationReport> {
    let ck = keys.customer_patient(customer).await?;
    let mut part = MigrationReport::default();
    let mut tx = pool.begin().await?;

    // Identity profile.
    type Profile = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>);
    let profile: Option<Profile> = sqlx::query_as(
        "SELECT full_name_enc, address_enc, previous_addresses_enc, phone_numbers_enc, email_addresses_enc, date_of_birth_enc \
         FROM identity_profiles WHERE customer_id = $1 FOR UPDATE",
    )
    .bind(customer)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((name, addr, prev, phones, emails, dob)) = profile {
        let converted = [
            convert(master, &ck, purpose::IDENTITY_FULL_NAME, &name)?,
            convert(master, &ck, purpose::IDENTITY_ADDRESS, &addr)?,
            convert(master, &ck, purpose::IDENTITY_PREVIOUS_ADDRESSES, &prev)?,
            convert(master, &ck, purpose::IDENTITY_PHONE_NUMBERS, &phones)?,
            convert(master, &ck, purpose::IDENTITY_EMAIL_ADDRESSES, &emails)?,
        ];
        let dob_new = match &dob {
            Some(b) => convert(master, &ck, purpose::IDENTITY_DATE_OF_BIRTH, b)?,
            None => None,
        };
        if converted.iter().any(Option::is_some) || dob_new.is_some() {
            let pick = |new: &Option<Vec<u8>>, old: &Vec<u8>| new.clone().unwrap_or_else(|| old.clone());
            sqlx::query(
                "UPDATE identity_profiles SET full_name_enc = $2, address_enc = $3, previous_addresses_enc = $4, \
                 phone_numbers_enc = $5, email_addresses_enc = $6, date_of_birth_enc = $7, updated_at = NOW() WHERE customer_id = $1",
            )
            .bind(customer)
            .bind(pick(&converted[0], &name))
            .bind(pick(&converted[1], &addr))
            .bind(pick(&converted[2], &prev))
            .bind(pick(&converted[3], &phones))
            .bind(pick(&converted[4], &emails))
            .bind(dob_new.or(dob))
            .execute(&mut *tx)
            .await?;
            part.identity_profiles += 1;
        } else {
            part.already_current += 1;
        }
    }

    // Two-factor secret.
    let mfa: Option<Vec<u8>> = sqlx::query_scalar("SELECT secret_enc FROM customer_mfa WHERE customer_id = $1 FOR UPDATE")
        .bind(customer)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(old) = mfa {
        match convert(master, &ck, purpose::MFA_SECRET, &old)? {
            Some(new) => {
                sqlx::query("UPDATE customer_mfa SET secret_enc = $2 WHERE customer_id = $1")
                    .bind(customer)
                    .bind(new)
                    .execute(&mut *tx)
                    .await?;
                part.mfa_secrets += 1;
            }
            None => part.already_current += 1,
        }
    }

    // Sealed listing links.
    for table in ["exposures", "removal_requests"] {
        let rows: Vec<(Uuid, String)> = sqlx::query_as(&format!(
            "SELECT id, source_url FROM {table} WHERE customer_id = $1 AND source_url LIKE 'enc1:%' FOR UPDATE"
        ))
        .bind(customer)
        .fetch_all(&mut *tx)
        .await?;
        for (id, stored) in rows {
            let old = BASE64
                .decode(stored.strip_prefix(LEGACY_PREFIX).unwrap_or(&stored))
                .context("a sealed link is not valid base64")?;
            if let Some(new) = convert(master, &ck, purpose::LISTING_LINK, &old)? {
                sqlx::query(&format!("UPDATE {table} SET source_url = $2 WHERE id = $1"))
                    .bind(id)
                    .bind(format!("{PREFIX}{}", BASE64.encode(new)))
                    .execute(&mut *tx)
                    .await?;
                part.links += 1;
            }
        }
    }

    tx.commit().await?;
    part.customers = 1;
    Ok(part)
}

/// How many stored values are still in the old format. Zero means the master key is no longer
/// needed to read anything.
pub async fn count_legacy(pool: &PgPool) -> Result<i64> {
    // A format-2 blob starts with the three bytes 56 53 02; anything else in these columns is old.
    let identity: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_profiles WHERE substring(full_name_enc from 1 for 3) <> '\\x565302'::bytea \
            OR substring(address_enc from 1 for 3) <> '\\x565302'::bytea",
    )
    .fetch_one(pool)
    .await?;
    let mfa: i64 = sqlx::query_scalar("SELECT count(*) FROM customer_mfa WHERE substring(secret_enc from 1 for 3) <> '\\x565302'::bytea")
        .fetch_one(pool)
        .await?;
    let links: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM exposures WHERE source_url LIKE 'enc1:%') + (SELECT count(*) FROM removal_requests WHERE source_url LIKE 'enc1:%')",
    )
    .fetch_one(pool)
    .await?;
    Ok(identity + mfa + links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    fn master() -> MasterKey {
        MasterKey::from_base64(&ts::master_key_b64()).unwrap()
    }

    /// Write what the old code wrote: key derived from the master key, no AAD, no prefix.
    async fn write_legacy(pool: &PgPool, customer: Uuid, with_dob: bool) {
        let key = crypto::derive_customer_key(&master(), customer);
        let enc = |s: &str| crypto::encrypt_str(&key, s).unwrap();
        sqlx::query(
            "INSERT INTO identity_profiles (customer_id, full_name_enc, address_enc, previous_addresses_enc, phone_numbers_enc, email_addresses_enc, date_of_birth_enc) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(customer)
        .bind(enc("Jane Smith"))
        .bind(enc("123 Main St, Austin, TX 78701"))
        .bind(enc("[]"))
        .bind(enc("[\"512-555-0147\"]"))
        .bind(enc("[\"jane@example.com\"]"))
        .bind(with_dob.then(|| enc("1984-03-09")))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn write_legacy_link(pool: &PgPool, customer: Uuid, url: &str) -> Uuid {
        let key = crypto::derive_customer_key(&master(), customer);
        let sealed = format!("enc1:{}", BASE64.encode(crypto::encrypt_str(&key, url).unwrap()));
        let exposure = ts::exposure(pool, customer, true, None).await;
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1").bind(exposure).bind(sealed).execute(pool).await.unwrap();
        exposure
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn old_data_is_read_again_after_migration_and_the_master_key_is_no_longer_needed(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let c = ts::customer(&pool, "personal", true).await;
        write_legacy(&pool, c, true).await;
        let exposure = write_legacy_link(&pool, c, "https://www.spokeo.com/jane-smith").await;
        assert!(count_legacy(&pool).await.unwrap() > 0);

        let report = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!(report, MigrationReport { customers: 1, identity_profiles: 1, mfa_secrets: 0, links: 1, already_current: 0, failed_customers: 0 });
        assert_eq!(count_legacy(&pool).await.unwrap(), 0);

        // Read it the normal way, with no master key in sight.
        let id = crate::services::customer::load_cleartext_identity(&pool, &keys, c).await.unwrap();
        assert_eq!((id.full_name.as_str(), id.current_address.as_str()), ("Jane Smith", "123 Main St, Austin, TX 78701"));
        assert_eq!(id.date_of_birth.as_deref(), Some("1984-03-09"));
        assert_eq!(id.phone_numbers, vec!["512-555-0147"]);
        assert_eq!(id.email_addresses, vec!["jane@example.com"]);
        let stored: String = sqlx::query_scalar("SELECT source_url FROM exposures WHERE id = $1").bind(exposure).fetch_one(&pool).await.unwrap();
        assert!(stored.starts_with("enc2:"), "{stored}");
        assert_eq!(crate::services::url_vault::open(&keys, c, &stored).await.unwrap(), "https://www.spokeo.com/jane-smith");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn it_is_safe_to_run_again_and_to_run_on_data_that_is_already_new(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let old = ts::customer(&pool, "free", false).await;
        write_legacy(&pool, old, false).await;
        let fresh = ts::customer(&pool, "free", false).await;
        ts::identity(&pool, &keys, fresh, "New Person").await;

        let first = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!((first.customers, first.identity_profiles, first.already_current, first.failed_customers), (2, 1, 1, 0));
        let second = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!((second.identity_profiles, second.already_current, second.failed_customers), (0, 2, 0));
        let id = crate::services::customer::load_cleartext_identity(&pool, &keys, fresh).await.unwrap();
        assert_eq!(id.full_name, "New Person");
        assert_eq!(crate::services::customer::load_cleartext_identity(&pool, &keys, old).await.unwrap().date_of_birth, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_wrong_master_key_changes_nothing_and_reports_the_customer_not_the_data(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        write_legacy(&pool, c, true).await;
        let before: Vec<u8> = sqlx::query_scalar("SELECT full_name_enc FROM identity_profiles").fetch_one(&pool).await.unwrap();
        let wrong = MasterKey::from_base64(&ts::other_master_key_b64()).unwrap();
        let report = migrate_legacy(&pool, &keys, &wrong).await.unwrap();
        assert_eq!((report.failed_customers, report.customers), (1, 0));
        let after: Vec<u8> = sqlx::query_scalar("SELECT full_name_enc FROM identity_profiles").fetch_one(&pool).await.unwrap();
        assert_eq!(before, after);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn one_customers_failure_does_not_undo_or_block_another(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let (good, bad) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        write_legacy(&pool, good, false).await;
        write_legacy(&pool, bad, false).await;
        // Corrupt the bad customer's address so its whole transaction must roll back.
        sqlx::query("UPDATE identity_profiles SET address_enc = '\\x00112233445566778899aabbccddeeff00112233445566778899aabb'::bytea WHERE customer_id = $1")
            .bind(bad).execute(&pool).await.unwrap();
        let name_before: Vec<u8> = sqlx::query_scalar("SELECT full_name_enc FROM identity_profiles WHERE customer_id = $1").bind(bad).fetch_one(&pool).await.unwrap();
        let report = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!((report.customers, report.failed_customers), (1, 1));
        let name_after: Vec<u8> = sqlx::query_scalar("SELECT full_name_enc FROM identity_profiles WHERE customer_id = $1").bind(bad).fetch_one(&pool).await.unwrap();
        assert_eq!(name_before, name_after, "all or nothing for a customer");
        assert!(crate::services::customer::load_cleartext_identity(&pool, &keys, good).await.is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_factor_secrets_are_migrated_and_still_verify(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        // A secret the way the old code stored it.
        let secret = [7u8; 20];
        let key = crypto::derive_customer_key(&master(), c);
        sqlx::query("INSERT INTO customer_mfa (customer_id, secret_enc, enabled_at) VALUES ($1, $2, NOW())")
            .bind(c).bind(crypto::encrypt(&key, &secret).unwrap()).execute(&pool).await.unwrap();
        let report = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!(report.mfa_secrets, 1);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let code = crate::services::totp::code_for_step(&secret, crate::services::totp::step_at(now));
        assert_eq!(
            crate::services::mfa::verify(&pool, &keys, c, &code).await.unwrap(),
            Some(crate::services::mfa::Verified::Totp)
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_database_with_nothing_old_reports_nothing_to_do(pool: PgPool) {
        let keys = ts::key_service(pool.clone());
        let c = ts::customer(&pool, "free", false).await;
        ts::identity(&pool, &keys, c, "Someone").await;
        assert_eq!(count_legacy(&pool).await.unwrap(), 0);
        let r = migrate_legacy(&pool, &keys, &master()).await.unwrap();
        assert_eq!((r.identity_profiles, r.links, r.mfa_secrets), (0, 0, 0));
    }
}
