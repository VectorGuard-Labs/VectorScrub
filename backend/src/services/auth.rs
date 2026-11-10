use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Claims embedded in a VectorScrub JWT.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // customer UUID
    pub tier: String,
    pub iat: i64,
    pub exp: i64,
}

/// How many Argon2 operations may run at once. Each one is deliberately slow (tens of
/// milliseconds of CPU) and holds ~19 MiB, so unbounded parallelism lets a burst of logins
/// from many addresses exhaust memory and starve the machine. Excess requests wait their turn
/// (asynchronously, they do not block a worker); the 30 s request timeout bounds the wait.
const MAX_CONCURRENT_HASHES: usize = 4;

fn hash_slots() -> &'static tokio::sync::Semaphore {
    static SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_HASHES))
}

/// Run CPU-heavy work on the blocking pool, at most MAX_CONCURRENT_HASHES at a time. Calling
/// Argon2 directly inside an async handler would pin a Tokio worker thread for its whole
/// duration, so a modest number of simultaneous logins could stall every other request.
async fn run_hashing<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    let _slot = hash_slots().acquire().await.map_err(|_| anyhow!("password hashing is unavailable"))?;
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| anyhow!("password hashing task failed: {e}"))?
}

/// `hash_password`, off the async runtime.
pub async fn hash_password_async(password: String) -> Result<String> {
    run_hashing(move || hash_password(&password)).await
}

/// `verify_password`, off the async runtime.
pub async fn verify_password_async(password: String, hash: String) -> Result<bool> {
    run_hashing(move || verify_password(&password, &hash)).await
}

/// Spend the same time as a real verification, for an email that has no account. Without it,
/// "no such account" answers measurably faster than "wrong password" and reveals which emails
/// are registered. The dummy hash is created on first use, so call `warm_up` at startup to
/// move that one-time cost out of the first unknown-email login (where it would itself be a
/// timing difference).
pub async fn verify_against_dummy_async(password: String) {
    let _ = run_hashing(move || verify_password(&password, dummy_hash())).await;
}

fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password("timing-equalization-only").expect("hash a constant password"))
}

/// Create the dummy hash now. Call once at startup.
pub async fn warm_up() {
    let _ = run_hashing(|| Ok(dummy_hash().len())).await;
}

/// Hash a password using Argon2id with a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("Failed to hash password: {e}"))
}

/// Verify a plaintext password against an Argon2id hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| anyhow!("Invalid hash format: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Issue a signed JWT valid for 24 hours.
pub fn issue_token(
    customer_id: Uuid,
    tier: &str,
    secret: &str,
) -> Result<String> {
    let now = OffsetDateTime::now_utc();
    let claims = Claims {
        sub: customer_id.to_string(),
        tier: tier.to_string(),
        iat: now.unix_timestamp(),
        exp: (now + Duration::hours(crate::middleware::session::SESSION_MAX_AGE_HOURS)).unix_timestamp(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| anyhow!("Failed to issue JWT: {e}"))
}

/// Validate and decode a JWT; returns the embedded claims.
pub fn validate_token(token: &str, secret: &str) -> Result<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| anyhow!("Invalid or expired token: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn async_hashing_round_trips() {
        let h = hash_password_async("correct horse battery staple".into()).await.unwrap();
        assert!(verify_password_async("correct horse battery staple".into(), h.clone()).await.unwrap());
        assert!(!verify_password_async("wrong".into(), h).await.unwrap());
    }

    /// The point of moving Argon2 off the async runtime. On a single-threaded runtime a
    /// synchronous hash would freeze the heartbeat task for its whole duration; with the work
    /// on the blocking pool the heartbeat keeps ticking.
    #[tokio::test(flavor = "current_thread")]
    async fn hashing_does_not_stall_other_tasks_on_the_runtime() {
        let ticks = Arc::new(AtomicU32::new(0));
        let t = ticks.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                t.fetch_add(1, Ordering::Relaxed);
            }
        });
        let started = std::time::Instant::now();
        hash_password_async("correct horse battery staple".into()).await.unwrap();
        let elapsed = started.elapsed();
        heartbeat.abort();

        let ticked = ticks.load(Ordering::Relaxed);
        // Hashing takes long enough here for many ticks to be due; a blocked runtime gets ~0.
        let due = (elapsed.as_millis() / 5) as u32;
        assert!(ticked * 2 >= due.saturating_sub(2), "the runtime was blocked: {ticked} ticks of {due} due in {elapsed:?}");
    }

    #[tokio::test]
    async fn more_hashes_than_slots_all_complete() {
        let jobs: Vec<_> = (0..MAX_CONCURRENT_HASHES * 2 + 1)
            .map(|i| tokio::spawn(hash_password_async(format!("password number {i} here"))))
            .collect();
        // More jobs than slots: the extra ones wait for a slot instead of failing or hanging.
        // (The semaphore is process-wide and other tests hash concurrently, so how many permits
        // are free at any instant is not something to assert here.)
        for j in jobs {
            j.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn warm_up_and_the_dummy_verification_work() {
        warm_up().await;
        verify_against_dummy_async("anything at all".into()).await;
    }
}
