//! Failed-login counting and temporary lockout.
//!
//! Keyed by sha256(lower(email)), not by customer id, so an email address with no
//! account is throttled exactly like one that has an account. Otherwise the lockout
//! itself would tell an attacker which emails are registered. No raw email is stored.
//!
//! Policy: after MAX_FAILURES consecutive failures the key is locked, and every
//! further failure re-locks it, for BASE_LOCK_SECS doubling every MAX_FAILURES more
//! failures up to MAX_LOCK_SECS. Failures older than FAILURE_WINDOW_SECS are forgotten
//! (unless currently locked). A correct password clears the record.
//!
//! Known trade-off: lockout keyed on the email lets someone who knows a victim's email
//! lock that victim out of logging in by failing on purpose. It is capped (an hour),
//! and the per-IP rate limit (middleware/rate_limiter.rs) bounds how fast one source
//! can do it. The alternative, keying on IP only, does nothing against a distributed
//! guessing attack.

use sha2::{Digest, Sha256};
use sqlx::PgPool;

pub const MAX_FAILURES: i32 = 5;
pub const BASE_LOCK_SECS: i64 = 15 * 60;
pub const MAX_LOCK_SECS: i64 = 60 * 60;
pub const FAILURE_WINDOW_SECS: i64 = 60 * 60;

/// Stable key for an email: case- and whitespace-insensitive.
pub fn key_hash(email: &str) -> String {
    hex::encode(Sha256::digest(email.trim().to_lowercase().as_bytes()))
}

/// Lock duration after the `failures`-th consecutive failure (0 if not locked yet).
pub fn lock_secs_for(failures: i32) -> i64 {
    if failures < MAX_FAILURES {
        return 0;
    }
    let doublings = ((failures - MAX_FAILURES) / MAX_FAILURES).min(10) as u32;
    (BASE_LOCK_SECS << doublings).min(MAX_LOCK_SECS)
}

/// Seconds until this email may try again, or None if it is not locked.
pub async fn locked_for(pool: &PgPool, email: &str) -> Result<Option<i64>, sqlx::Error> {
    let secs: Option<i64> = sqlx::query_scalar(
        "SELECT CEIL(EXTRACT(EPOCH FROM (locked_until - NOW())))::bigint \
         FROM login_throttle WHERE key_hash = $1 AND locked_until > NOW()",
    )
    .bind(key_hash(email))
    .fetch_optional(pool)
    .await?;
    Ok(secs)
}

/// Record a failed attempt. Returns Some(lock seconds) if this failure locked the email.
pub async fn record_failure(pool: &PgPool, email: &str) -> Result<Option<i64>, sqlx::Error> {
    let key = key_hash(email);
    let failures: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO login_throttle (key_hash, failures, last_failure_at)
        VALUES ($1, 1, NOW())
        ON CONFLICT (key_hash) DO UPDATE SET
            failures = CASE
                WHEN login_throttle.last_failure_at < NOW() - make_interval(secs => $2::int)
                     AND (login_throttle.locked_until IS NULL OR login_throttle.locked_until < NOW())
                THEN 1
                ELSE login_throttle.failures + 1
            END,
            last_failure_at = NOW()
        RETURNING failures
        "#,
    )
    .bind(&key)
    .bind(FAILURE_WINDOW_SECS)
    .fetch_one(pool)
    .await?;

    let lock = lock_secs_for(failures);
    if lock == 0 {
        return Ok(None);
    }
    sqlx::query("UPDATE login_throttle SET locked_until = NOW() + make_interval(secs => $2::int) WHERE key_hash = $1")
        .bind(&key)
        .bind(lock)
        .execute(pool)
        .await?;
    Ok(Some(lock))
}

/// Forget an email's failures (called after a successful login).
pub async fn clear(pool: &PgPool, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM login_throttle WHERE key_hash = $1")
        .bind(key_hash(email))
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete records that can no longer matter (old failures, expired locks).
pub async fn purge_stale(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "DELETE FROM login_throttle \
         WHERE last_failure_at < NOW() - make_interval(secs => $1::int) \
           AND (locked_until IS NULL OR locked_until < NOW())",
    )
    .bind(FAILURE_WINDOW_SECS * 24)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_ignores_case_and_surrounding_whitespace() {
        assert_eq!(key_hash("Jane@Example.com"), key_hash("  jane@example.com "));
        assert_ne!(key_hash("jane@example.com"), key_hash("john@example.com"));
        assert!(!key_hash("jane@example.com").contains("jane"), "raw email is not stored");
    }

    #[test]
    fn lock_duration_schedule() {
        for f in 0..MAX_FAILURES {
            assert_eq!(lock_secs_for(f), 0, "no lock below the threshold ({f})");
        }
        assert_eq!(lock_secs_for(5), 900);
        assert_eq!(lock_secs_for(9), 900);
        assert_eq!(lock_secs_for(10), 1800);
        assert_eq!(lock_secs_for(15), 3600);
        assert_eq!(lock_secs_for(500), MAX_LOCK_SECS, "capped");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn locks_after_the_threshold_and_not_before(pool: PgPool) {
        let email = "victim@example.com";
        for i in 1..MAX_FAILURES {
            assert_eq!(record_failure(&pool, email).await.unwrap(), None, "failure {i}");
            assert_eq!(locked_for(&pool, email).await.unwrap(), None);
        }
        assert_eq!(record_failure(&pool, email).await.unwrap(), Some(900));
        let remaining = locked_for(&pool, email).await.unwrap().expect("locked");
        assert!((895..=900).contains(&remaining), "{remaining}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_emails_are_throttled_identically(pool: PgPool) {
        // No customer exists for this address; the behaviour must be the same.
        for _ in 0..MAX_FAILURES {
            record_failure(&pool, "nobody-registered@example.com").await.unwrap();
        }
        assert!(locked_for(&pool, "nobody-registered@example.com").await.unwrap().is_some());
        // and the casing/whitespace trick does not dodge it
        assert!(locked_for(&pool, " NOBODY-registered@Example.com").await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn success_clears_the_record(pool: PgPool) {
        for _ in 0..3 {
            record_failure(&pool, "u@example.com").await.unwrap();
        }
        clear(&pool, "u@example.com").await.unwrap();
        // counting starts over: it takes a full MAX_FAILURES again to lock
        for _ in 0..MAX_FAILURES - 1 {
            assert_eq!(record_failure(&pool, "u@example.com").await.unwrap(), None);
        }
        assert_eq!(locked_for(&pool, "u@example.com").await.unwrap(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn old_failures_are_forgotten_but_an_active_lock_is_not(pool: PgPool) {
        for _ in 0..3 {
            record_failure(&pool, "slow@example.com").await.unwrap();
        }
        sqlx::query("UPDATE login_throttle SET last_failure_at = NOW() - interval '2 hours'")
            .execute(&pool).await.unwrap();
        // window expired: this counts as failure #1 again
        assert_eq!(record_failure(&pool, "slow@example.com").await.unwrap(), None);
        let failures: i32 = sqlx::query_scalar("SELECT failures FROM login_throttle WHERE key_hash = $1")
            .bind(key_hash("slow@example.com")).fetch_one(&pool).await.unwrap();
        assert_eq!(failures, 1);

        // while locked, an old last_failure_at does not reset the count
        for _ in 0..MAX_FAILURES {
            record_failure(&pool, "locked@example.com").await.unwrap();
        }
        sqlx::query("UPDATE login_throttle SET last_failure_at = NOW() - interval '2 hours' WHERE key_hash = $1")
            .bind(key_hash("locked@example.com")).execute(&pool).await.unwrap();
        record_failure(&pool, "locked@example.com").await.unwrap();
        assert!(locked_for(&pool, "locked@example.com").await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn further_failures_extend_the_lock_and_expiry_unlocks(pool: PgPool) {
        for _ in 0..MAX_FAILURES {
            record_failure(&pool, "e@example.com").await.unwrap();
        }
        // lock expires
        sqlx::query("UPDATE login_throttle SET locked_until = NOW() - interval '1 second'").execute(&pool).await.unwrap();
        assert_eq!(locked_for(&pool, "e@example.com").await.unwrap(), None);
        // but one more failure inside the window locks again straight away (6th failure)
        assert_eq!(record_failure(&pool, "e@example.com").await.unwrap(), Some(900));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn purge_removes_only_dead_records(pool: PgPool) {
        record_failure(&pool, "old@example.com").await.unwrap();
        sqlx::query("UPDATE login_throttle SET last_failure_at = NOW() - interval '30 days'").execute(&pool).await.unwrap();
        for _ in 0..MAX_FAILURES {
            record_failure(&pool, "active-lock@example.com").await.unwrap();
        }
        assert_eq!(purge_stale(&pool).await.unwrap(), 1);
        assert!(locked_for(&pool, "active-lock@example.com").await.unwrap().is_some());
    }
}
