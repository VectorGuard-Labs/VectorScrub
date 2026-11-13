//! Deletion requests sent to companies on a customer's behalf (migration 0015).
//!
//! The customer approves a campaign once: which companies, whether the address goes in, and the
//! authorization text, which is stored exactly as shown. The worker then sends one email per
//! company from the channel catalog. The rules that matter:
//!
//! * the recipient address comes from the catalog only (`broker_channels`), and a company is only
//!   offered to a customer in a state its page offers the channel to;
//! * paid plans only, a 90-day cooldown per company, and no company twice in a campaign;
//! * a request is claimed before sending and is never retried if interrupted: it fails closed;
//! * what we know is what we sent. A company's answer goes to the customer, who reports it.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::services::broker_channels;

/// Bump when the wording changes: a customer must accept the current version.
pub const AUTHORIZATION_VERSION: &str = "2026-11-draft-1";

/// DRAFT wording. It has not been reviewed by a lawyer and must be before this is switched on
/// for real customers (`DELETION_REQUESTS_ENABLED`). Stored verbatim with each campaign.
pub const AUTHORIZATION_TEXT: &str = "\
I authorize VectorScrub, a product of VectorGuard Labs, to act as my authorized agent in sending \
the privacy requests I select below to the companies I select. Each request asks the company to \
delete the personal information it holds about me and, where the company's page offers it, to stop \
selling or sharing it.\n\
\n\
I agree that VectorScrub may share with each selected company my name and email address, and my \
address only if I tick the box to include it. It will not share my phone numbers or date of birth. \
Each company may contact me directly at my email address to verify my identity, and I understand \
a company may decline a request, or may ask me to confirm that I authorized this.\n\
\n\
I understand that a request is not a guarantee of removal, that the law that gives me these rights \
depends on where I live, and that VectorScrub will tell me only what it sent and what I report back.\n\
\n\
This authorization covers only the companies I select now. I can withdraw it for a company that has \
not yet been sent a request by contacting VectorScrub.";

/// A company is not sent a second request from the same customer inside this many days.
pub const COOLDOWN_DAYS: i32 = 90;
/// Companies are expected to answer a deletion request within 45 days (CCPA); after that with no
/// reply, the dashboard says so.
pub const FOLLOW_UP_DAYS: i64 = 45;
/// A request still 'sending' after this long was interrupted.
pub const STALE_SENDING_SECS: i64 = 60 * 60;
/// Marks a failure whose outcome is unknown, which keeps the cooldown in force.
pub const ABANDONED_MARKER: &str = "abandoned:";

pub fn authorization_sha256() -> String {
    hex::encode(Sha256::digest(AUTHORIZATION_TEXT.as_bytes()))
}

#[derive(Debug, thiserror::Error)]
pub enum DeletionError {
    #[error("a paid plan is required")]
    PaidPlanRequired,
    #[error("authorization not given")]
    ConsentRequired,
    #[error("the authorization text has changed")]
    AuthorizationOutdated,
    #[error("no company selected")]
    NothingSelected,
    #[error("unknown company")]
    UnknownChannel,
    #[error("not available to this customer")]
    NotEligible,
    #[error("a request to this company was sent recently")]
    TooSoon,
    #[error("a selected company needs the address")]
    AddressRequired,
    #[error("request not found")]
    NotFound,
    #[error("that status change is not allowed")]
    IllegalTransition,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

pub struct NewCampaign<'a> {
    pub channel_ids: &'a [String],
    pub include_address: bool,
    pub consent: bool,
    pub authorization_version: &'a str,
}

/// Create a campaign and queue one request per selected company. `state` is the US state of the
/// customer's saved address, if it can be told. One transaction: all of it, or nothing.
pub async fn create_campaign(
    pool: &PgPool,
    customer_id: Uuid,
    state: Option<&str>,
    new: NewCampaign<'_>,
) -> Result<(Uuid, Vec<Uuid>), DeletionError> {
    if !new.consent {
        return Err(DeletionError::ConsentRequired);
    }
    if new.authorization_version != AUTHORIZATION_VERSION {
        return Err(DeletionError::AuthorizationOutdated);
    }
    let mut ids: Vec<&String> = Vec::new();
    for id in new.channel_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(DeletionError::NothingSelected);
    }
    let channels: Vec<_> = ids
        .iter()
        .map(|id| broker_channels::find(id).ok_or(DeletionError::UnknownChannel))
        .collect::<Result<_, _>>()?;
    if channels.iter().any(|c| !broker_channels::eligible(c, state)) {
        return Err(DeletionError::NotEligible);
    }
    // A company whose page asks for the address cannot find the person without it. Better to say
    // so now than to send a request it cannot act on.
    if !new.include_address && channels.iter().any(|c| c.requires_address) {
        return Err(DeletionError::AddressRequired);
    }

    let mut tx = pool.begin().await?;
    // Serialise this customer's campaigns, so two at once cannot both pass the cooldown check.
    let tier: Option<String> = sqlx::query_scalar("SELECT subscription_tier::text FROM customers WHERE id = $1 FOR UPDATE")
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await?;
    match tier.as_deref() {
        None => return Err(DeletionError::NotFound),
        Some("free") => return Err(DeletionError::PaidPlanRequired),
        Some(_) => {}
    }

    for c in &channels {
        // A recent request counts unless it plainly failed; an "abandoned" one may have gone out.
        let recent: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM deletion_requests \
             WHERE customer_id = $1 AND channel_id = $2 \
               AND (status <> 'failed' OR error LIKE $4) \
               AND queued_at > NOW() - make_interval(days => $3))",
        )
        .bind(customer_id)
        .bind(&c.id)
        .bind(COOLDOWN_DAYS)
        .bind(format!("{ABANDONED_MARKER}%"))
        .fetch_one(&mut *tx)
        .await?;
        if recent {
            return Err(DeletionError::TooSoon);
        }
    }

    let campaign_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deletion_campaigns \
            (id, customer_id, authorization_version, authorization_text, authorization_sha256, include_address) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(campaign_id)
    .bind(customer_id)
    .bind(AUTHORIZATION_VERSION)
    .bind(AUTHORIZATION_TEXT)
    .bind(authorization_sha256())
    .bind(new.include_address)
    .execute(&mut *tx)
    .await?;

    let mut request_ids = Vec::new();
    for c in &channels {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO deletion_requests (id, campaign_id, customer_id, channel_id) VALUES ($1, $2, $3, $4)")
            .bind(id)
            .bind(campaign_id)
            .bind(customer_id)
            .bind(&c.id)
            .execute(&mut *tx)
            .await?;
        request_ids.push(id);
    }
    tx.commit().await?;
    Ok((campaign_id, request_ids))
}

// ---------------------------------------------------------------------------
// What the customer sees
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DeletionRequestView {
    pub id: Uuid,
    pub channel_id: String,
    pub company: String,
    pub status: String,
    #[serde(with = "time::serde::rfc3339")]
    pub queued_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub outcome_at: Option<OffsetDateTime>,
    /// Sent more than 45 days ago and nothing reported since: the company is overdue.
    pub follow_up_due: bool,
    /// What went wrong, in fixed words, for a failed request.
    pub error: Option<String>,
}

type Row = (Uuid, String, String, OffsetDateTime, Option<OffsetDateTime>, Option<OffsetDateTime>, Option<String>);

pub async fn list_for_customer(pool: &PgPool, customer_id: Uuid) -> Result<Vec<DeletionRequestView>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, channel_id, status, queued_at, sent_at, outcome_at, error \
         FROM deletion_requests WHERE customer_id = $1 ORDER BY queued_at DESC, id",
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await?;
    let now = OffsetDateTime::now_utc();
    Ok(rows
        .into_iter()
        .map(|(id, channel_id, status, queued_at, sent_at, outcome_at, error)| {
            let company = broker_channels::find(&channel_id).map(|c| c.company.clone()).unwrap_or_else(|| channel_id.clone());
            let follow_up_due = status == "sent" && sent_at.is_some_and(|t| (now - t).whole_days() >= FOLLOW_UP_DAYS);
            DeletionRequestView { id, channel_id, company, status, queued_at, sent_at, outcome_at, follow_up_due, error }
        })
        .collect())
}

/// The customer reports what a company answered. Only a request that was sent can have an
/// outcome, and a final outcome (removed, denied) cannot be changed.
pub async fn report_outcome(pool: &PgPool, customer_id: Uuid, request_id: Uuid, outcome: &str) -> Result<(), DeletionError> {
    if !matches!(outcome, "replied" | "removed" | "denied") {
        return Err(DeletionError::IllegalTransition);
    }
    let mut tx = pool.begin().await?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT status FROM deletion_requests WHERE id = $1 AND customer_id = $2 FOR UPDATE")
            .bind(request_id)
            .bind(customer_id)
            .fetch_optional(&mut *tx)
            .await?;
    let current = current.ok_or(DeletionError::NotFound)?;
    let allowed = matches!(
        (current.as_str(), outcome),
        ("sent", "replied" | "removed" | "denied") | ("replied", "removed" | "denied")
    );
    if !allowed {
        return Err(DeletionError::IllegalTransition);
    }
    sqlx::query("UPDATE deletion_requests SET status = $2, outcome_at = NOW(), updated_at = NOW() WHERE id = $1")
        .bind(request_id)
        .bind(outcome)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The sending worker
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
pub struct QueuedRow {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub channel_id: String,
    pub include_address: bool,
    pub authorized_at: OffsetDateTime,
    pub authorization_version: String,
    pub authorization_text: String,
}

/// Requests waiting to be sent, oldest first (at most 50). Anything interrupted while sending is
/// failed first, so it can never be picked up again.
pub async fn list_queued(pool: &PgPool) -> Result<Vec<QueuedRow>, sqlx::Error> {
    fail_stale_sending(pool).await?;
    sqlx::query_as(
        "SELECT r.id, r.customer_id, r.channel_id, c.include_address, c.authorized_at, \
                c.authorization_version, c.authorization_text \
         FROM deletion_requests r \
         JOIN deletion_campaigns c ON c.id = r.campaign_id \
         JOIN customers u ON u.id = r.customer_id \
         WHERE r.status = 'queued' AND u.is_active = true AND u.subscription_tier <> 'free' \
         ORDER BY r.queued_at, r.id LIMIT 50",
    )
    .fetch_all(pool)
    .await
}

/// Take a queued request. True for exactly one caller.
pub async fn claim(pool: &PgPool, request_id: Uuid) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE deletion_requests SET status = 'sending', claimed_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(request_id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// Record how sending ended. Only a request being sent can be finished, once.
pub async fn finish(
    pool: &PgPool,
    request_id: Uuid,
    sent: bool,
    message_id: Option<&str>,
    error: Option<&str>,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE deletion_requests SET status = $2, message_id = $3, error = $4, \
             sent_at = CASE WHEN $2 = 'sent' THEN NOW() END, updated_at = NOW() \
         WHERE id = $1 AND status = 'sending'",
    )
    .bind(request_id)
    .bind(if sent { "sent" } else { "failed" })
    .bind(message_id.map(|m| m.chars().take(300).collect::<String>()))
    .bind(error.map(|e| e.chars().take(300).collect::<String>()))
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

/// A request left 'sending' (the worker died) failed rather than retried: it is not known whether
/// the email went out, and a company must not get the same request twice.
pub async fn fail_stale_sending(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE deletion_requests SET status = 'failed', updated_at = NOW(), \
             error = $2 \
         WHERE status = 'sending' AND claimed_at < NOW() - make_interval(secs => $1::double precision)",
    )
    .bind(STALE_SENDING_SECS as f64)
    .bind(format!("{ABANDONED_MARKER} sending was interrupted, so it is not known whether the email was sent"))
    .execute(pool)
    .await?
    .rows_affected())
}

pub async fn customer_of(pool: &PgPool, request_id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT customer_id FROM deletion_requests WHERE id = $1")
        .bind(request_id)
        .fetch_optional(pool)
        .await
}

/// Fail a request that cannot be sent (for example its company left the catalog) before it is claimed.
pub async fn fail_queued(pool: &PgPool, request_id: Uuid, reason: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE deletion_requests SET status = 'failed', error = $2, updated_at = NOW() WHERE id = $1 AND status = 'queued'")
        .bind(request_id)
        .bind(reason.chars().take(300).collect::<String>())
        .execute(pool)
        .await?;
    Ok(())
}
