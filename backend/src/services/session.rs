//! Server-side sessions.
//!
//! The signed token (JWT) says who the customer is; the row in `sessions` says whether that
//! login is still allowed. A request is accepted only while its session is live:
//!
//!   * not revoked (logout, "sign out everywhere", revoking one device, enabling or disabling
//!     two-factor, account erasure all end sessions at once);
//!   * inside the absolute limit (`expires_at`, 24 hours after login, the same as the token);
//!   * used within the idle limit (`IDLE_TIMEOUT_MINUTES`).
//!
//! A token with no live session behind it is refused, so a token copied before logout is dead.
//! `last_seen_at` is written at most once a minute per session, so authentication costs one
//! indexed read on almost every request.

use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::middleware::session::SESSION_MAX_AGE_HOURS;

/// A session unused for this long is over, even though its token has not expired.
pub const IDLE_TIMEOUT_MINUTES: i64 = 60;
/// `last_seen_at` is refreshed only when it is older than this.
const TOUCH_AFTER_SECS: i64 = 60;
/// How much of the browser's `User-Agent` is kept, for the "where am I signed in" list.
const MAX_USER_AGENT_CHARS: usize = 200;

/// Start a session for a customer who has just authenticated fully (password, and second
/// factor if they use one). Returns its id, which goes into the token as `jti`.
pub async fn create(pool: &PgPool, customer_id: Uuid, user_agent: Option<&str>) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let ua: Option<String> = user_agent.map(|u| u.chars().take(MAX_USER_AGENT_CHARS).collect());
    sqlx::query(
        "INSERT INTO sessions (id, customer_id, expires_at, user_agent) \
         VALUES ($1, $2, NOW() + make_interval(hours => $3::int), $4)",
    )
    .bind(id)
    .bind(customer_id)
    .bind(SESSION_MAX_AGE_HOURS as i32)
    .bind(ua)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Is this session live for this customer? Also records activity. Any doubt means `false`.
pub async fn validate(pool: &PgPool, session_id: Uuid, customer_id: Uuid) -> Result<bool, sqlx::Error> {
    let row: Option<bool> = sqlx::query_scalar(
        "SELECT last_seen_at < NOW() - make_interval(secs => $3::int) FROM sessions \
         WHERE id = $1 AND customer_id = $2 \
           AND revoked_at IS NULL \
           AND expires_at > NOW() \
           AND last_seen_at > NOW() - make_interval(mins => $4::int)",
    )
    .bind(session_id)
    .bind(customer_id)
    .bind(TOUCH_AFTER_SECS as i32)
    .bind(IDLE_TIMEOUT_MINUTES as i32)
    .fetch_optional(pool)
    .await?;
    let Some(stale) = row else { return Ok(false) };
    if stale {
        // The same liveness conditions, so a session that ended between the two statements is
        // not revived by the touch.
        sqlx::query(
            "UPDATE sessions SET last_seen_at = NOW() \
             WHERE id = $1 AND revoked_at IS NULL AND expires_at > NOW()",
        )
        .bind(session_id)
        .execute(pool)
        .await?;
    }
    Ok(true)
}

/// End one session of this customer. Returns whether a live session was ended.
pub async fn revoke(pool: &PgPool, customer_id: Uuid, session_id: Uuid) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE sessions SET revoked_at = NOW() \
         WHERE id = $1 AND customer_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(customer_id)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// End every live session of the customer, except `keep` if given. Returns how many ended.
pub async fn revoke_all(pool: &PgPool, customer_id: Uuid, keep: Option<Uuid>) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "UPDATE sessions SET revoked_at = NOW() \
         WHERE customer_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR id <> $2)",
    )
    .bind(customer_id)
    .bind(keep)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// A live session, as shown to its owner.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SessionInfo {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_seen_at: OffsetDateTime,
    pub user_agent: Option<String>,
}

/// The customer's live sessions, most recently used first.
pub async fn list(pool: &PgPool, customer_id: Uuid) -> Result<Vec<SessionInfo>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, created_at, last_seen_at, user_agent FROM sessions \
         WHERE customer_id = $1 AND revoked_at IS NULL AND expires_at > NOW() \
           AND last_seen_at > NOW() - make_interval(mins => $2::int) \
         ORDER BY last_seen_at DESC",
    )
    .bind(customer_id)
    .bind(IDLE_TIMEOUT_MINUTES as i32)
    .fetch_all(pool)
    .await
}

/// Housekeeping: drop sessions that ended more than a week ago. Returns how many.
pub async fn prune(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let r = sqlx::query(
        "DELETE FROM sessions \
         WHERE expires_at < NOW() - interval '7 days' OR revoked_at < NOW() - interval '7 days'",
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    async fn set(pool: &PgPool, id: Uuid, sql_set: &str) {
        sqlx::query(&format!("UPDATE sessions SET {sql_set} WHERE id = $1"))
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_new_session_is_live(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let s = create(&pool, c, Some("Mozilla/5.0")).await.unwrap();
        assert!(validate(&pool, s, c).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_session_belongs_to_one_customer(pool: PgPool) {
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        let s = create(&pool, a, None).await.unwrap();
        assert!(!validate(&pool, s, b).await.unwrap(), "another customer's id does not unlock it");
        assert!(!revoke(&pool, b, s).await.unwrap(), "nor can another customer revoke it");
        assert!(validate(&pool, s, a).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_session_is_refused(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        assert!(!validate(&pool, Uuid::new_v4(), c).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_revoked_session_is_refused(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let s = create(&pool, c, None).await.unwrap();
        assert!(revoke(&pool, c, s).await.unwrap());
        assert!(!validate(&pool, s, c).await.unwrap());
        assert!(!revoke(&pool, c, s).await.unwrap(), "revoking twice reports nothing to do");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_idle_session_ends_and_activity_keeps_one_alive(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let s = create(&pool, c, None).await.unwrap();

        set(&pool, s, &format!("last_seen_at = NOW() - interval '{} minutes'", IDLE_TIMEOUT_MINUTES - 5)).await;
        assert!(validate(&pool, s, c).await.unwrap(), "almost idle, but not yet");
        let seen: OffsetDateTime = sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE id = $1")
            .bind(s)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(seen > OffsetDateTime::now_utc() - time::Duration::minutes(1), "the use was recorded");

        set(&pool, s, &format!("last_seen_at = NOW() - interval '{} minutes'", IDLE_TIMEOUT_MINUTES + 1)).await;
        assert!(!validate(&pool, s, c).await.unwrap(), "idle too long");
        assert!(!validate(&pool, s, c).await.unwrap(), "and a refused request does not revive it");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn activity_is_written_at_most_once_a_minute(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let s = create(&pool, c, None).await.unwrap();
        set(&pool, s, "last_seen_at = NOW() - interval '10 seconds'").await;
        let before: OffsetDateTime = sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE id = $1")
            .bind(s).fetch_one(&pool).await.unwrap();
        assert!(validate(&pool, s, c).await.unwrap());
        let after: OffsetDateTime = sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE id = $1")
            .bind(s).fetch_one(&pool).await.unwrap();
        assert_eq!(before, after, "a recent value is left alone");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_absolute_limit_ends_a_session_however_active(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let s = create(&pool, c, None).await.unwrap();
        set(&pool, s, "expires_at = NOW() - interval '1 second'").await;
        assert!(!validate(&pool, s, c).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn revoke_all_can_spare_the_current_session(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let other_customer = ts::customer(&pool, "free", false).await;
        let (a, b, d) = (
            create(&pool, c, None).await.unwrap(),
            create(&pool, c, None).await.unwrap(),
            create(&pool, other_customer, None).await.unwrap(),
        );
        assert_eq!(revoke_all(&pool, c, Some(a)).await.unwrap(), 1);
        assert!(validate(&pool, a, c).await.unwrap());
        assert!(!validate(&pool, b, c).await.unwrap());
        assert!(validate(&pool, d, other_customer).await.unwrap(), "other customers are untouched");
        assert_eq!(revoke_all(&pool, c, None).await.unwrap(), 1);
        assert!(!validate(&pool, a, c).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_list_shows_only_live_sessions_and_caps_the_user_agent(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let live = create(&pool, c, Some(&"x".repeat(1000))).await.unwrap();
        let revoked = create(&pool, c, None).await.unwrap();
        let expired = create(&pool, c, None).await.unwrap();
        revoke(&pool, c, revoked).await.unwrap();
        set(&pool, expired, "expires_at = NOW() - interval '1 hour'").await;
        let listed = list(&pool, c).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, live);
        assert_eq!(listed[0].user_agent.as_deref().map(str::len), Some(MAX_USER_AGENT_CHARS));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn prune_removes_only_long_dead_sessions(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let live = create(&pool, c, None).await.unwrap();
        let old_expired = create(&pool, c, None).await.unwrap();
        let recent_revoked = create(&pool, c, None).await.unwrap();
        let old_revoked = create(&pool, c, None).await.unwrap();
        set(&pool, old_expired, "expires_at = NOW() - interval '8 days'").await;
        revoke(&pool, c, recent_revoked).await.unwrap();
        set(&pool, old_revoked, "revoked_at = NOW() - interval '8 days'").await;
        assert_eq!(prune(&pool).await.unwrap(), 2);
        let left: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sessions").fetch_all(&pool).await.unwrap();
        assert!(left.contains(&live) && left.contains(&recent_revoked) && left.len() == 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasing_the_account_removes_its_sessions(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        create(&pool, c, None).await.unwrap();
        crate::services::account_deletion::delete_account(&pool, c).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions").fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0);
    }
}
