//! Two-factor authentication: authenticator-app codes (TOTP) with single-use recovery codes.
//!
//! Lifecycle: `begin_setup` stores a fresh secret that protects nothing yet (`enabled_at` is
//! NULL); `confirm_setup` proves the customer's app produces valid codes, switches protection on
//! and returns the recovery codes once. From then on login needs `verify` to succeed, and
//! `disable` removes everything.
//!
//! The secret is encrypted with the customer's own key (same scheme as their identity data).
//! Recovery codes are stored only as hashes: they are 50 random bits, single use, and every
//! failed attempt goes through the login lockout (see api/auth.rs), so an unsalted SHA-256 is
//! enough. Both checks are atomic in the database, so two simultaneous requests cannot spend
//! the same code twice.

use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::{self, MasterKey};
use crate::services::totp;

/// Shown in the authenticator app next to the account.
pub const ISSUER: &str = "VectorScrub";
pub const RECOVERY_CODE_COUNT: usize = 10;
const RECOVERY_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, thiserror::Error)]
pub enum MfaError {
    #[error("Two-factor authentication is already on")]
    AlreadyEnabled,
    #[error("Start two-factor setup first")]
    NotStarted,
    #[error("That code is not valid")]
    BadCode,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("could not protect the secret: {0}")]
    Crypto(String),
}

/// How a successful `verify` was satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verified {
    Totp,
    RecoveryCode,
}

/// What the customer sees on their security page.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct Status {
    pub enabled: bool,
    pub recovery_codes_remaining: i64,
}

/// What the customer needs to add the account to an authenticator app.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct Setup {
    /// Base32 secret, for typing into an app by hand.
    pub secret: String,
    /// `otpauth://` URI, which the frontend turns into a QR code.
    pub otpauth_uri: String,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub async fn is_enabled(pool: &PgPool, customer_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM customer_mfa WHERE customer_id = $1 AND enabled_at IS NOT NULL)")
        .bind(customer_id)
        .fetch_one(pool)
        .await
}

pub async fn status(pool: &PgPool, customer_id: Uuid) -> Result<Status, sqlx::Error> {
    let enabled = is_enabled(pool, customer_id).await?;
    let remaining: i64 = if enabled {
        sqlx::query_scalar("SELECT count(*) FROM mfa_recovery_codes WHERE customer_id = $1 AND used_at IS NULL")
            .bind(customer_id)
            .fetch_one(pool)
            .await?
    } else {
        0
    };
    Ok(Status { enabled, recovery_codes_remaining: remaining })
}

/// Start (or restart) setup with a new secret. Refused while two-factor is already on: turning
/// it off is a separate, deliberate step.
pub async fn begin_setup(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    account_label: &str,
) -> Result<Setup, MfaError> {
    let secret = totp::generate_secret();
    let key = crypto::derive_customer_key(master, customer_id);
    let blob = crypto::encrypt(&key, &secret).map_err(|e| MfaError::Crypto(e.to_string()))?;
    let r = sqlx::query(
        "INSERT INTO customer_mfa (customer_id, secret_enc) VALUES ($1, $2) \
         ON CONFLICT (customer_id) DO UPDATE \
            SET secret_enc = EXCLUDED.secret_enc, last_used_step = 0, created_at = NOW() \
            WHERE customer_mfa.enabled_at IS NULL",
    )
    .bind(customer_id)
    .bind(blob)
    .execute(pool)
    .await?;
    if r.rows_affected() == 0 {
        return Err(MfaError::AlreadyEnabled);
    }
    Ok(Setup { secret: totp::base32_encode(&secret), otpauth_uri: totp::otpauth_uri(ISSUER, account_label, &secret) })
}

/// Load and decrypt the customer's secret, with the newest step already used.
async fn load_secret(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    enabled: bool,
) -> Result<Option<(Vec<u8>, i64)>, MfaError> {
    let row: Option<(Vec<u8>, i64)> = sqlx::query_as(
        "SELECT secret_enc, last_used_step FROM customer_mfa \
         WHERE customer_id = $1 AND (enabled_at IS NOT NULL) = $2",
    )
    .bind(customer_id)
    .bind(enabled)
    .fetch_optional(pool)
    .await?;
    let Some((blob, last)) = row else { return Ok(None) };
    let key = crypto::derive_customer_key(master, customer_id);
    let secret = crypto::decrypt(&key, &blob).map_err(|e| MfaError::Crypto(e.to_string()))?;
    Ok(Some((secret, last)))
}

/// A recovery code as shown to the customer: `ABCDE-FGHJK`.
fn new_recovery_code() -> String {
    let mut raw = [0u8; 10];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let chars: String = raw.iter().map(|b| RECOVERY_ALPHABET[(*b & 31) as usize] as char).collect();
    format!("{}-{}", &chars[..5], &chars[5..])
}

/// What is compared and stored: case, dashes and spaces do not matter.
fn normalize_recovery(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect()
}

fn hash_recovery(code: &str) -> String {
    hex::encode(Sha256::digest(normalize_recovery(code).as_bytes()))
}

/// Confirm setup with a code from the app, switch two-factor on, and return fresh recovery
/// codes. This is the only time they are shown.
pub async fn confirm_setup(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    code: &str,
) -> Result<Vec<String>, MfaError> {
    let Some((secret, last)) = load_secret(pool, master, customer_id, false).await? else {
        return Err(if is_enabled(pool, customer_id).await? { MfaError::AlreadyEnabled } else { MfaError::NotStarted });
    };
    let step = totp::verify(&secret, code, now_unix(), last).ok_or(MfaError::BadCode)?;

    let codes: Vec<String> = (0..RECOVERY_CODE_COUNT).map(|_| new_recovery_code()).collect();
    let mut tx = pool.begin().await?;
    let r = sqlx::query(
        "UPDATE customer_mfa SET enabled_at = NOW(), last_used_step = $2 \
         WHERE customer_id = $1 AND enabled_at IS NULL",
    )
    .bind(customer_id)
    .bind(step as i64)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() == 0 {
        return Err(MfaError::AlreadyEnabled);
    }
    sqlx::query("DELETE FROM mfa_recovery_codes WHERE customer_id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;
    for c in &codes {
        sqlx::query("INSERT INTO mfa_recovery_codes (customer_id, code_hash) VALUES ($1, $2)")
            .bind(customer_id)
            .bind(hash_recovery(c))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(codes)
}

/// Check a second-factor code for a customer who has two-factor on: an authenticator code (six
/// digits) or a recovery code (which is spent). `Ok(None)` means the code was wrong.
pub async fn verify(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
    code: &str,
) -> Result<Option<Verified>, MfaError> {
    let normalized = normalize_recovery(code);
    if normalized.len() == totp::DIGITS && normalized.bytes().all(|b| b.is_ascii_digit()) {
        let Some((secret, last)) = load_secret(pool, master, customer_id, true).await? else {
            return Ok(None);
        };
        let Some(step) = totp::verify(&secret, &normalized, now_unix(), last) else {
            return Ok(None);
        };
        // Atomic: of two simultaneous requests with the same code, only one moves the step forward.
        let r = sqlx::query(
            "UPDATE customer_mfa SET last_used_step = $2 \
             WHERE customer_id = $1 AND enabled_at IS NOT NULL AND last_used_step < $2",
        )
        .bind(customer_id)
        .bind(step as i64)
        .execute(pool)
        .await?;
        return Ok((r.rows_affected() == 1).then_some(Verified::Totp));
    }

    if normalized.len() != 10 {
        return Ok(None);
    }
    let r = sqlx::query(
        "UPDATE mfa_recovery_codes SET used_at = NOW() \
         WHERE customer_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(customer_id)
    .bind(hash_recovery(&normalized))
    .execute(pool)
    .await?;
    Ok((r.rows_affected() == 1).then_some(Verified::RecoveryCode))
}

/// Turn two-factor off and forget the secret and every recovery code.
pub async fn disable(pool: &PgPool, customer_id: Uuid) -> Result<(), sqlx::Error> {
    // The recovery codes go with the row only through the customer, so remove both explicitly.
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM mfa_recovery_codes WHERE customer_id = $1").bind(customer_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM customer_mfa WHERE customer_id = $1").bind(customer_id).execute(&mut *tx).await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    fn master() -> MasterKey {
        MasterKey::from_base64(&ts::master_key_b64()).unwrap()
    }

    /// The code the customer's app would show now, from the secret `begin_setup` returned.
    fn app_code(setup: &Setup) -> String {
        totp::code_for_step(&totp::base32_decode(&setup.secret), totp::step_at(now_unix()))
    }

    async fn enabled_customer(pool: &PgPool) -> (Uuid, Setup, Vec<String>) {
        let c = ts::customer(pool, "free", false).await;
        let setup = begin_setup(pool, &master(), c, "a@example.com").await.unwrap();
        let codes = confirm_setup(pool, &master(), c, &app_code(&setup)).await.unwrap();
        (c, setup, codes)
    }

    #[test]
    fn recovery_codes_have_the_expected_shape_and_are_distinct() {
        let codes: Vec<String> = (0..50).map(|_| new_recovery_code()).collect();
        for c in &codes {
            assert_eq!(c.len(), 11);
            assert_eq!(c.as_bytes()[5], b'-');
            assert!(c.chars().all(|ch| ch == '-' || RECOVERY_ALPHABET.contains(&(ch as u8))), "{c}");
        }
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
    }

    #[test]
    fn recovery_codes_compare_without_regard_to_case_dashes_or_spaces() {
        assert_eq!(hash_recovery("ABCDE-FGHJK"), hash_recovery("abcde fghjk"));
        assert_eq!(hash_recovery("ABCDE-FGHJK"), hash_recovery(" abcdefghjk "));
        assert_ne!(hash_recovery("ABCDE-FGHJK"), hash_recovery("ABCDE-FGHJL"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn setup_protects_nothing_until_a_code_is_confirmed(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let setup = begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        assert!(!is_enabled(&pool, c).await.unwrap());
        assert_eq!(verify(&pool, &master(), c, &app_code(&setup)).await.unwrap(), None, "not on yet");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn confirming_with_a_wrong_code_leaves_it_off(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let setup = begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        let right = app_code(&setup);
        let wrong = if right == "123456" { "654321" } else { "123456" };
        assert!(matches!(confirm_setup(&pool, &master(), c, wrong).await, Err(MfaError::BadCode)));
        assert!(!is_enabled(&pool, c).await.unwrap());
        assert!(matches!(confirm_setup(&pool, &master(), ts::customer(&pool, "free", false).await, &right).await, Err(MfaError::NotStarted)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn confirming_turns_it_on_and_returns_ten_recovery_codes(pool: PgPool) {
        let (c, _, codes) = enabled_customer(&pool).await;
        assert!(is_enabled(&pool, c).await.unwrap());
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let st = status(&pool, c).await.unwrap();
        assert!(st.enabled);
        assert_eq!(st.recovery_codes_remaining, RECOVERY_CODE_COUNT as i64);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_secret_is_stored_encrypted_and_recovery_codes_only_hashed(pool: PgPool) {
        let (c, setup, codes) = enabled_customer(&pool).await;
        let blob: Vec<u8> = sqlx::query_scalar("SELECT secret_enc FROM customer_mfa WHERE customer_id = $1")
            .bind(c).fetch_one(&pool).await.unwrap();
        let raw = totp::base32_decode(&setup.secret);
        assert!(!blob.windows(raw.len()).any(|w| w == raw.as_slice()), "the raw secret is not in the column");
        let stored: Vec<String> = sqlx::query_scalar("SELECT code_hash FROM mfa_recovery_codes WHERE customer_id = $1")
            .bind(c).fetch_all(&pool).await.unwrap();
        for code in &codes {
            assert!(!stored.iter().any(|h| h.contains(&normalize_recovery(code))), "no plaintext code stored");
            assert!(stored.contains(&hash_recovery(code)));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_valid_code_is_accepted_once_and_cannot_be_replayed(pool: PgPool) {
        let (c, setup, _) = enabled_customer(&pool).await;
        // Setup used up the current step; rewind it to stand in for a login some time later.
        sqlx::query("UPDATE customer_mfa SET last_used_step = 0 WHERE customer_id = $1")
            .bind(c).execute(&pool).await.unwrap();
        let code = app_code(&setup);
        assert_eq!(verify(&pool, &master(), c, &code).await.unwrap(), Some(Verified::Totp));
        assert_eq!(verify(&pool, &master(), c, &code).await.unwrap(), None, "the same code twice is a replay");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_code_used_to_confirm_setup_cannot_be_used_to_log_in(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let setup = begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        let code = app_code(&setup);
        confirm_setup(&pool, &master(), c, &code).await.unwrap();
        assert_eq!(verify(&pool, &master(), c, &code).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_wrong_or_malformed_code_is_refused(pool: PgPool) {
        let (c, _, _) = enabled_customer(&pool).await;
        for bad in ["", "000000x", "12345", "ABCDE-FGHJK", "not a code", "'; DROP TABLE customers;--"] {
            assert_eq!(verify(&pool, &master(), c, bad).await.unwrap(), None, "{bad:?}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_recovery_code_works_exactly_once(pool: PgPool) {
        let (c, _, codes) = enabled_customer(&pool).await;
        assert_eq!(verify(&pool, &master(), c, &codes[0]).await.unwrap(), Some(Verified::RecoveryCode));
        assert_eq!(verify(&pool, &master(), c, &codes[0]).await.unwrap(), None, "spent");
        assert_eq!(verify(&pool, &master(), c, &codes[1].to_lowercase().replace('-', " ")).await.unwrap(), Some(Verified::RecoveryCode));
        assert_eq!(status(&pool, c).await.unwrap().recovery_codes_remaining, RECOVERY_CODE_COUNT as i64 - 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn simultaneous_uses_of_one_recovery_code_succeed_once(pool: PgPool) {
        let (c, _, codes) = enabled_customer(&pool).await;
        let m = master();
        let (a, b, d, e) = tokio::join!(
            verify(&pool, &m, c, &codes[0]),
            verify(&pool, &m, c, &codes[0]),
            verify(&pool, &m, c, &codes[0]),
            verify(&pool, &m, c, &codes[0]),
        );
        let wins = [a, b, d, e].iter().filter(|r| r.as_ref().unwrap().is_some()).count();
        assert_eq!(wins, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn simultaneous_uses_of_one_authenticator_code_succeed_once(pool: PgPool) {
        let (c, setup, _) = enabled_customer(&pool).await;
        sqlx::query("UPDATE customer_mfa SET last_used_step = 0 WHERE customer_id = $1")
            .bind(c).execute(&pool).await.unwrap();
        let (m, code) = (master(), app_code(&setup));
        let (a, b, d, e) = tokio::join!(
            verify(&pool, &m, c, &code),
            verify(&pool, &m, c, &code),
            verify(&pool, &m, c, &code),
            verify(&pool, &m, c, &code),
        );
        let wins = [a, b, d, e].iter().filter(|r| r.as_ref().unwrap().is_some()).count();
        assert_eq!(wins, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn one_customers_recovery_code_does_not_work_for_another(pool: PgPool) {
        let (_, _, codes) = enabled_customer(&pool).await;
        let (other, _, _) = enabled_customer(&pool).await;
        assert_eq!(verify(&pool, &master(), other, &codes[0]).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn setup_cannot_be_restarted_while_on(pool: PgPool) {
        let (c, setup, _) = enabled_customer(&pool).await;
        assert!(matches!(begin_setup(&pool, &master(), c, "a@example.com").await, Err(MfaError::AlreadyEnabled)));
        assert!(matches!(confirm_setup(&pool, &master(), c, &app_code(&setup)).await, Err(MfaError::AlreadyEnabled)));
        assert!(is_enabled(&pool, c).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn restarting_an_unconfirmed_setup_replaces_the_secret(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let first = begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        let second = begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        assert_ne!(first.secret, second.secret);
        assert!(confirm_setup(&pool, &master(), c, &app_code(&first)).await.is_err(), "the old secret no longer works");
        assert!(confirm_setup(&pool, &master(), c, &app_code(&second)).await.is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn disabling_removes_the_secret_and_the_codes(pool: PgPool) {
        let (c, _, codes) = enabled_customer(&pool).await;
        disable(&pool, c).await.unwrap();
        assert!(!is_enabled(&pool, c).await.unwrap());
        assert_eq!(verify(&pool, &master(), c, &codes[0]).await.unwrap(), None);
        let rows: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM customer_mfa) + (SELECT count(*) FROM mfa_recovery_codes)")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
        // and it can be set up again from scratch
        assert!(begin_setup(&pool, &master(), c, "a@example.com").await.is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_secret_cannot_be_read_with_the_wrong_master_key(pool: PgPool) {
        let (c, setup, _) = enabled_customer(&pool).await;
        let other = MasterKey::from_base64(&ts::other_master_key_b64()).unwrap();
        assert!(matches!(verify(&pool, &other, c, &app_code(&setup)).await, Err(MfaError::Crypto(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasing_the_account_removes_the_secret_and_codes(pool: PgPool) {
        let (c, _, _) = enabled_customer(&pool).await;
        crate::services::account_deletion::delete_account(&pool, c).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM customer_mfa) + (SELECT count(*) FROM mfa_recovery_codes)")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }
}
