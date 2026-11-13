use crate::models::removal_request::{RemovalAuditEntry, RemovalRequest, RemovalStatus};
use crate::services::notification as notification_svc;
use anyhow::{Context, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

/// How many opt-out submissions a Free account may make in total. Requests that failed (our
/// side, a CAPTCHA, a changed form) do not count: the customer got nothing from them.
pub const FREE_REMOVAL_LIMIT: i64 = 3;

/// Creates a removal request for a specific exposure with status=authorized.
/// Also creates the first audit log entry.
/// Fails with `RemovalError::AlreadyInFlight` if a request for this exposure is already
/// authorized, submitted or acknowledged. The partial unique index
/// `idx_removal_requests_one_in_flight` enforces this under concurrency; the SELECT below
/// is only the early, friendly check.
pub async fn authorize_removal(
    pool: &PgPool,
    customer_id: Uuid,
    exposure_id: Uuid,
) -> Result<RemovalRequest> {
    let mut tx = pool.begin().await.context("begin transaction")?;

    // Lock the customer's row so two simultaneous authorizations cannot both pass the Free
    // limit check below.
    let tier: Option<String> = sqlx::query_scalar(
        "SELECT subscription_tier::text FROM customers WHERE id = $1 FOR UPDATE",
    )
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("lock customer")?;
    let tier = tier.ok_or(RemovalError::NotFound)?;

    // Only an in-flight request blocks a new authorization. Requests that ended in
    // failed / rejected / unverifiable / reappeared / verified_removed are terminal,
    // and a reappeared record must be re-authorizable (the customer approves every
    // submission; the monitoring agent never submits on its own).
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM removal_requests \
         WHERE exposure_id = $1 AND customer_id = $2 \
           AND status IN ('authorized', 'submitted', 'acknowledged')"
    )
    .bind(exposure_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("check existing removal")?;

    if existing.is_some() {
        return Err(RemovalError::AlreadyInFlight.into());
    }

    if tier == "free" {
        let used: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM removal_requests WHERE customer_id = $1 AND status <> 'failed' AND procedure_used <> 'manual'",
        )
        .bind(customer_id)
        .fetch_one(&mut *tx)
        .await
        .context("count removal requests")?;
        if used >= FREE_REMOVAL_LIMIT {
            return Err(RemovalError::FreeLimitReached.into());
        }
    }

    // Get exposure details for denormalization
    // The exposure must belong to this customer. The API handler checks too; the service
    // enforces it as well so no caller can authorize removal of someone else's exposure.
    let (source_name, source_url): (String, String) = sqlx::query_as(
        "SELECT source_name, source_url FROM exposures WHERE id = $1 AND customer_id = $2"
    )
    .bind(exposure_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("fetch exposure details")?
    .ok_or(RemovalError::NotFound)?;

    // Create removal request
    let request_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let request: RemovalRequest = sqlx::query_as(
        r#"
        INSERT INTO removal_requests (
            id, customer_id, exposure_id, source_name, source_url, status, customer_authorized_at, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
        RETURNING *
        "#,
    )
    .bind(request_id)
    .bind(customer_id)
    .bind(exposure_id)
    .bind(source_name)
    .bind(source_url)
    .bind(RemovalStatus::Authorized)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match &e {
        // Lost the race to a concurrent authorization for the same exposure.
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            anyhow::Error::new(RemovalError::AlreadyInFlight)
        }
        _ => anyhow::Error::new(e).context("insert removal request"),
    })?;

    // Create initial audit log entry
    sqlx::query(
        r#"
        INSERT INTO removal_audit_log (
            id, removal_request_id, customer_id, action, actor, details, recorded_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(customer_id)
    .bind("authorized")
    .bind("customer")
    .bind(serde_json::json!({}))
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("insert audit log entry")?;

    tx.commit().await.context("commit transaction")?;

    Ok(request)
}

/// Returns all removal requests for a customer, ordered by created_at DESC.
pub async fn list_removal_requests(
    pool: &PgPool,
    customer_id: Uuid,
) -> Result<Vec<RemovalRequest>> {
    let requests = sqlx::query_as(
        r#"
        SELECT r.*,
               (SELECT left(a.details->>'error', 300) FROM removal_audit_log a
                 WHERE a.removal_request_id = r.id AND a.action = 'failed'
                 ORDER BY a.recorded_at DESC LIMIT 1) AS failure_reason,
               lc.result AS last_check_result,
               lc.recorded_at AS last_check_at
        FROM removal_requests r
        LEFT JOIN LATERAL (
            SELECT CASE a.action
                       WHEN 'verified_removed' THEN 'removed'
                       WHEN 'still_present' THEN 'still_present'
                       WHEN 'still_present_after_timeout' THEN 'still_present'
                       ELSE 'inconclusive' END AS result,
                   a.recorded_at
            FROM removal_audit_log a
            WHERE a.removal_request_id = r.id
              AND a.action IN ('verified_removed', 'still_present', 'still_present_after_timeout',
                               'verification_inconclusive', 'unverifiable_after_timeout')
            ORDER BY a.recorded_at DESC LIMIT 1
        ) lc ON true
        WHERE r.customer_id = $1
        ORDER BY r.created_at DESC
        "#,
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await
    .context("list removal requests")?;

    Ok(requests)
}

/// Days after the opt-out was submitted (by the customer's own account) that the listing is first
/// re-checked. Brokers differ a lot in how long they take, so this is the first of several
/// scheduled checks (see `monitoring::VERIFICATION_SCHEDULE_DAYS`), not a promise of a result.
pub const MANUAL_FIRST_CHECK_DAYS: i64 = 3;

/// How far back a by-hand submission may be dated.
const MANUAL_MAX_BACKDATE_DAYS: i64 = 60;

/// What the site did about email confirmation, as the customer reports it. "I have not clicked
/// the link yet" is deliberately not a value: the opt-out is not submitted until they do, so
/// there is nothing to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailConfirmation {
    /// The site emailed a link and the customer clicked it.
    Confirmed,
    /// The site asked for no email confirmation.
    NotRequired,
}

impl EmailConfirmation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NotRequired => "not_required",
        }
    }
}

/// The customer's account of a by-hand opt-out.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ManualReport {
    pub email_confirmation: EmailConfirmation,
    /// The day they submitted it (UTC date); today when absent.
    #[serde(default, with = "manual_date::option")]
    pub submitted_on: Option<time::Date>,
    /// Optional: the exact profile link they opted out. Brokers often reject a search-results link
    /// and want one profile; that same profile page is also the better page to re-check. Must be on
    /// the same site. The API handler validates and seals it before it reaches the service.
    #[serde(default)]
    pub listing_url: Option<String>,
}

/// A replacement listing link, already validated (`listing::site_of`) and sealed for the customer.
pub struct NewListingLink {
    pub site: String,
    pub sealed: String,
}

/// True when `site` (from `listing::site_of`) is the stored source or one of its subdomains.
pub fn same_site(site: &str, source_name: &str) -> bool {
    let source = source_name.trim_start_matches("www.").to_ascii_lowercase();
    site == source || site.ends_with(&format!(".{source}"))
}

time::serde::format_description!(manual_date, Date, "[year]-[month]-[day]");

/// The customer says they completed, by hand, an opt-out that our automation could not (the site
/// blocks automated browsers). The request moves from `failed` to `submitted` with its first
/// re-check scheduled, and from there it is the same as any other submitted request: the
/// monitoring worker re-fetches the listing, and ONLY evidence that it is gone moves it to
/// `verified_removed`. This records a claim; it proves nothing. A customer who did not do it
/// ends up `rejected`/`unverifiable` after the verification timeout like anyone else.
///
/// Refused unless the request failed, belongs to this customer, its listing is still active,
/// and no other request for that listing is in progress.
pub async fn mark_done_by_hand(
    pool: &PgPool,
    customer_id: Uuid,
    request_id: Uuid,
    report: ManualReport,
    new_link: Option<NewListingLink>,
) -> Result<RemovalRequest> {
    let today = OffsetDateTime::now_utc().date();
    let submitted_on = report.submitted_on.unwrap_or(today);
    if submitted_on > today || (today - submitted_on).whole_days() > MANUAL_MAX_BACKDATE_DAYS {
        return Err(RemovalError::BadReport.into());
    }

    let mut tx = pool.begin().await.context("begin transaction")?;

    let row: Option<(RemovalStatus, Uuid, String)> = sqlx::query_as(
        "SELECT status, exposure_id, source_name FROM removal_requests WHERE id = $1 AND customer_id = $2 FOR UPDATE",
    )
    .bind(request_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("read removal request")?;
    let (status, exposure_id, source_name) = row.ok_or(RemovalError::NotFound)?;
    if let Some(link) = &new_link {
        if !same_site(&link.site, &source_name) {
            return Err(RemovalError::OtherSiteLink.into());
        }
    }

    if status != RemovalStatus::Failed {
        return Err(RemovalError::IllegalTransition { from: status, to: RemovalStatus::Submitted }.into());
    }

    let active: Option<bool> = sqlx::query_scalar("SELECT is_active FROM exposures WHERE id = $1 AND customer_id = $2")
        .bind(exposure_id)
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await
        .context("read exposure")?;
    if active != Some(true) {
        return Err(RemovalError::NotActionable.into());
    }

    let in_flight: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM removal_requests \
         WHERE exposure_id = $1 AND id <> $2 AND status IN ('authorized', 'submitted', 'acknowledged'))",
    )
    .bind(exposure_id)
    .bind(request_id)
    .fetch_one(&mut *tx)
    .await
    .context("check for a request in progress")?;
    if in_flight {
        return Err(RemovalError::AlreadyInFlight.into());
    }

    if let Some(link) = &new_link {
        // Re-check (and later monitor) the exact page they opted out, on both rows.
        sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1 AND customer_id = $3")
            .bind(exposure_id)
            .bind(&link.sealed)
            .bind(customer_id)
            .execute(&mut *tx)
            .await
            .context("update listing link")?;
        sqlx::query("UPDATE removal_requests SET source_url = $2 WHERE id = $1")
            .bind(request_id)
            .bind(&link.sealed)
            .execute(&mut *tx)
            .await
            .context("update listing link")?;
    }

    let request: RemovalRequest = sqlx::query_as(
        r#"
        UPDATE removal_requests
        SET status = 'submitted',
            procedure_used = 'manual',
            submitted_at = $3::date::timestamptz,
            next_check_at = GREATEST($3::date + $2::int, CURRENT_DATE)::timestamptz + interval '1 hour',
            manual_reported_at = NOW(),
            manual_email_confirmation = $4,
            manual_submitted_on = $3,
            submission_claimed_at = NULL,
            updated_at = NOW()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(request_id)
    .bind(MANUAL_FIRST_CHECK_DAYS as i32)
    .bind(submitted_on)
    .bind(report.email_confirmation.as_str())
    .fetch_one(&mut *tx)
    .await
    .context("mark removal as done by hand")?;

    tx.commit().await.context("commit")?;

    record_audit_entry(
        pool,
        request_id,
        customer_id,
        "submitted_manually".to_string(),
        "customer".to_string(),
        serde_json::json!({
            "reported_by": "customer",
            "email_confirmation": report.email_confirmation.as_str(),
            "submitted_on": submitted_on.to_string(),
            "first_check_in_days": MANUAL_FIRST_CHECK_DAYS,
            "listing_link_updated": new_link.is_some(),
        }),
    )
    .await
    .context("record the customer's report")?;
    Ok(request)
}

/// Returns removal requests with a specific status, at most 100.
/// Used by agents to poll for pending work. Premium customers' requests come first, then
/// Personal, then Free, oldest first within each (this is the "priority" in Premium).
pub async fn list_removal_requests_by_status(
    pool: &PgPool,
    status: RemovalStatus,
) -> Result<Vec<RemovalRequest>> {
    let requests = sqlx::query_as(
        r#"
        SELECT r.* FROM removal_requests r
        JOIN customers c ON c.id = r.customer_id
        WHERE r.status = $1
        ORDER BY CASE c.subscription_tier WHEN 'premium' THEN 0 WHEN 'personal' THEN 1 ELSE 2 END,
                 r.created_at ASC
        LIMIT 100
        "#,
    )
    .bind(status)
    .fetch_all(pool)
    .await
    .context("list removal requests by status")?;

    Ok(requests)
}

/// Returns a single removal request if it belongs to the specified customer.
pub async fn get_removal_request(
    pool: &PgPool,
    request_id: Uuid,
    customer_id: Uuid,
) -> Result<Option<RemovalRequest>> {
    let request = sqlx::query_as(
        "SELECT * FROM removal_requests WHERE id = $1 AND customer_id = $2",
    )
    .bind(request_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await
    .context("get removal request")?;

    Ok(request)
}

/// Why a status update was refused.
#[derive(Debug, thiserror::Error)]
pub enum RemovalError {
    #[error("removal request not found")]
    NotFound,
    #[error("a removal request for this exposure is already in flight")]
    AlreadyInFlight,
    #[error("the Free plan's opt-out limit has been reached")]
    FreeLimitReached,
    #[error("this listing is no longer active, so there is nothing to remove")]
    NotActionable,
    #[error("illegal removal status transition: {from:?} -> {to:?}")]
    IllegalTransition { from: RemovalStatus, to: RemovalStatus },
    #[error("the submission date is in the future or too long ago")]
    BadReport,
    #[error("the listing link is on a different site")]
    OtherSiteLink,
    #[error("assistance can only be requested for a request that needs manual action")]
    AssistanceNotApplicable,
    #[error("too many open assistance requests")]
    TooManyAssistanceRequests,
}

/// Replace the link we re-check for a request that is still open (manual action required, or
/// submitted and awaiting verification), for example when the customer opted out a specific
/// profile rather than the search page our scan found. Same site only; the new link is already
/// validated and sealed. Updates the exposure too, so later reappearance checks use it.
pub async fn update_listing_link(pool: &PgPool, customer_id: Uuid, request_id: Uuid, link: NewListingLink) -> Result<RemovalRequest> {
    let mut tx = pool.begin().await.context("begin transaction")?;
    let row: Option<(RemovalStatus, Uuid, String)> = sqlx::query_as(
        "SELECT status, exposure_id, source_name FROM removal_requests WHERE id = $1 AND customer_id = $2 FOR UPDATE",
    )
    .bind(request_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("read removal request")?;
    let (status, exposure_id, source_name) = row.ok_or(RemovalError::NotFound)?;
    if !matches!(status, RemovalStatus::Failed | RemovalStatus::Submitted | RemovalStatus::Acknowledged) {
        return Err(RemovalError::NotActionable.into());
    }
    if !same_site(&link.site, &source_name) {
        return Err(RemovalError::OtherSiteLink.into());
    }
    sqlx::query("UPDATE exposures SET source_url = $2 WHERE id = $1 AND customer_id = $3")
        .bind(exposure_id).bind(&link.sealed).bind(customer_id)
        .execute(&mut *tx).await.context("update exposure link")?;
    let request: RemovalRequest = sqlx::query_as("UPDATE removal_requests SET source_url = $2, updated_at = NOW() WHERE id = $1 RETURNING *")
        .bind(request_id).bind(&link.sealed)
        .fetch_one(&mut *tx).await.context("update request link")?;
    sqlx::query(
        "INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) \
         VALUES ($1, $2, 'listing_link_updated', 'customer', '{}'::jsonb)",
    )
    .bind(request_id).bind(customer_id)
    .execute(&mut *tx).await.context("audit")?;
    tx.commit().await.context("commit")?;
    Ok(request)
}

/// Most help requests one customer may have open at a time. Help is a hand-worked pilot queue.
pub const MAX_OPEN_ASSISTANCE_PER_CUSTOMER: i64 = 3;

/// Ask for a person to help with a request that automation could not finish. Allowed only for a
/// `failed` request (the "manual action required" state) whose listing is still active. Safe to
/// repeat: asking again changes nothing. Records the ask; it does not promise a result or a time.
pub async fn request_assistance(pool: &PgPool, customer_id: Uuid, request_id: Uuid) -> Result<RemovalRequest> {
    let mut tx = pool.begin().await.context("begin transaction")?;
    let row: Option<(RemovalStatus, bool, Option<String>)> = sqlx::query_as(
        "SELECT r.status, COALESCE(e.is_active, false), r.assistance_status          FROM removal_requests r LEFT JOIN exposures e ON e.id = r.exposure_id          WHERE r.id = $1 AND r.customer_id = $2 FOR UPDATE OF r",
    )
    .bind(request_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx)
    .await
    .context("read removal request")?;
    let (status, active, existing) = row.ok_or(RemovalError::NotFound)?;
    if status != RemovalStatus::Failed || !active {
        return Err(RemovalError::AssistanceNotApplicable.into());
    }
    if existing.as_deref().is_none_or(|s| s == "declined" || s == "completed") {
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM removal_requests WHERE customer_id = $1 AND assistance_status IN ('requested', 'in_progress')",
        )
        .bind(customer_id)
        .fetch_one(&mut *tx)
        .await
        .context("count open assistance requests")?;
        if open >= MAX_OPEN_ASSISTANCE_PER_CUSTOMER {
            return Err(RemovalError::TooManyAssistanceRequests.into());
        }
        sqlx::query(
            "UPDATE removal_requests SET assistance_status = 'requested', assistance_requested_at = NOW(),              assistance_minutes = NULL, updated_at = NOW() WHERE id = $1",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .context("record assistance request")?;
    }
    let request: RemovalRequest = sqlx::query_as("SELECT * FROM removal_requests WHERE id = $1")
        .bind(request_id)
        .fetch_one(&mut *tx)
        .await
        .context("reload")?;
    tx.commit().await.context("commit")?;
    Ok(request)
}

/// A submission claim that has not been resolved for this long is presumed abandoned (the
/// worker crashed or lost the backend mid-submission). See `fail_abandoned_claims`.
pub const SUBMISSION_CLAIM_STALE_AFTER_SECS: i64 = 60 * 60;

/// Atomically claim an authorized request for submission. Returns true for exactly one
/// caller; every other caller (a second worker, or the same worker after a restart) gets
/// false and must not submit. The claim lives in the database, so it survives restarts and
/// holds across worker replicas, unlike the in-memory set it replaces.
pub async fn claim_submission(pool: &PgPool, request_id: Uuid) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE removal_requests \
         SET submission_claimed_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'authorized' AND submission_claimed_at IS NULL",
    )
    .bind(request_id)
    .execute(pool)
    .await
    .context("claim removal submission")?;
    Ok(result.rows_affected() == 1)
}

/// Ends submission claims that were never resolved as `failed`, so the request stops
/// being polled, the customer is told, and they can re-authorize. A request is never
/// handed to a second worker: if the first one did submit and only failed to report it,
/// the worst outcome is that the customer is asked to approve again, not a double send.
/// Safe to run on several instances: the status change locks and re-checks the row, so
/// whichever instance gets there second sees an illegal transition and skips it.
/// Returns the (request, customer) pairs that were ended.
pub async fn fail_abandoned_claims(pool: &PgPool, stale_after_secs: i64) -> Result<Vec<(Uuid, Uuid)>> {
    let stale: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, customer_id FROM removal_requests \
         WHERE status = 'authorized' AND submission_claimed_at IS NOT NULL \
           AND submission_claimed_at < NOW() - make_interval(secs => $1::int) \
         ORDER BY submission_claimed_at \
         LIMIT 100",
    )
    .bind(stale_after_secs)
    .fetch_all(pool)
    .await
    .context("find abandoned submission claims")?;

    let mut ended = Vec::new();
    for (request_id, customer_id) in stale {
        match update_removal_status(pool, request_id, RemovalStatus::Failed, None, None, None).await {
            Ok(_) => {
                record_audit_entry(
                    pool,
                    request_id,
                    customer_id,
                    "claim_expired".to_string(),
                    "system".to_string(),
                    serde_json::json!({
                        "reason": "the worker that claimed this submission never reported an outcome; \
                                   it is not retried automatically in case it was already sent"
                    }),
                )
                .await?;
                ended.push((request_id, customer_id));
            }
            // The worker reported in between: nothing to do.
            Err(e) if matches!(e.downcast_ref::<RemovalError>(), Some(RemovalError::IllegalTransition { .. })) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(ended)
}

/// Updates removal request status and related metadata, enforcing the lifecycle in
/// `RemovalStatus::can_transition_to`. The check and the write happen in one
/// transaction on a locked row, so two concurrent updates cannot both pass the check.
/// Sets submitted_at when status=Submitted, verified_at when status=VerifiedRemoved.
pub async fn update_removal_status(
    pool: &PgPool,
    request_id: Uuid,
    new_status: RemovalStatus,
    confirmation_ref: Option<String>,
    evidence_hash: Option<String>,
    next_check_at: Option<OffsetDateTime>,
) -> Result<RemovalRequest> {
    let now = OffsetDateTime::now_utc();
    let mut tx = pool.begin().await.context("begin transaction")?;

    let current: RemovalStatus = sqlx::query_scalar(
        "SELECT status FROM removal_requests WHERE id = $1 FOR UPDATE",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await
    .context("read removal status")?
    .ok_or(RemovalError::NotFound)?;

    if !current.can_transition_to(&new_status) {
        return Err(RemovalError::IllegalTransition { from: current, to: new_status }.into());
    }

    let request: RemovalRequest = sqlx::query_as(
        r#"
        UPDATE removal_requests
        SET
            status = $2,
            confirmation_ref = COALESCE($3, confirmation_ref),
            evidence_hash = COALESCE($4, evidence_hash),
            next_check_at = COALESCE($5, next_check_at),
            submitted_at = CASE WHEN $2 = 'submitted' AND submitted_at IS NULL THEN $6 ELSE submitted_at END,
            verified_at = CASE WHEN $2 = 'verified_removed' AND verified_at IS NULL THEN $6 ELSE verified_at END,
            updated_at = $6
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(request_id)
    .bind(&new_status)
    .bind(confirmation_ref)
    .bind(evidence_hash)
    .bind(next_check_at)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .context("update removal status")?;

    // If verified removed, mark the exposure as inactive in the same transaction.
    // (exposures has no updated_at column; removed_at is the timestamp that matters.)
    if new_status == RemovalStatus::VerifiedRemoved {
        sqlx::query(
            r#"
            UPDATE exposures
            SET is_active = false, removed_at = COALESCE(removed_at, $2)
            WHERE id = $1
            "#,
        )
        .bind(request.exposure_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .context("mark exposure inactive")?;
    }

    tx.commit().await.context("commit status update")?;

    // The privacy score changes when an exposure is removed. Best-effort, after commit.
    if new_status == RemovalStatus::VerifiedRemoved {
        if let Err(e) =
            crate::services::exposure::update_privacy_score(pool, request.customer_id).await
        {
            tracing::warn!(
                customer_id = %request.customer_id,
                error = %e,
                "Failed to refresh privacy score after verified removal"
            );
        }
    }

    // Send alerts for specific status changes
    match new_status {
        RemovalStatus::Rejected | RemovalStatus::Unverifiable | RemovalStatus::Failed => {
            let error = match new_status {
                RemovalStatus::Rejected => "the data source did not honor the request",
                RemovalStatus::Unverifiable => "we could not confirm the record was removed",
                _ => "our automated submission could not be completed",
            };
            let _ = notification_svc::alert_removal_failed(
                pool,
                request.customer_id,
                &request.source_name,
                error,
            )
            .await;
        }
        RemovalStatus::Reappeared => {
            let _ = notification_svc::alert_reappearance(
                pool,
                request.customer_id,
                &request.source_name,
            )
            .await;
        }
        _ => {}
    }

    Ok(request)
}

/// Appends an entry to the removal audit log.
/// The audit log is immutable (enforced by DB rules). The entry is written only if
/// `customer_id` is the actual owner of the request, so an entry can never be attributed
/// to the wrong customer.
pub async fn record_audit_entry(
    pool: &PgPool,
    removal_request_id: Uuid,
    customer_id: Uuid,
    action: String,
    actor: String,
    details: serde_json::Value,
) -> Result<RemovalAuditEntry> {
    let entry: Option<RemovalAuditEntry> = sqlx::query_as(
        r#"
        INSERT INTO removal_audit_log (
            id, removal_request_id, customer_id, action, actor, details, recorded_at
        )
        SELECT $1, r.id, r.customer_id, $4, $5, $6, $7
        FROM removal_requests r
        WHERE r.id = $2 AND r.customer_id = $3
        RETURNING *
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(removal_request_id)
    .bind(customer_id)
    .bind(action)
    .bind(actor)
    .bind(details)
    .bind(OffsetDateTime::now_utc())
    .fetch_optional(pool)
    .await
    .context("insert audit entry")?;

    Ok(entry.ok_or(RemovalError::NotFound)?)
}

/// Returns the full audit trail for a removal request.
pub async fn get_removal_audit_log(
    pool: &PgPool,
    removal_request_id: Uuid,
) -> Result<Vec<RemovalAuditEntry>> {
    let entries = sqlx::query_as(
        r#"
        SELECT * FROM removal_audit_log
        WHERE removal_request_id = $1
        ORDER BY recorded_at ASC
        "#,
    )
    .bind(removal_request_id)
    .fetch_all(pool)
    .await
    .context("get audit log")?;

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    async fn status_of(pool: &PgPool, id: Uuid) -> String {
        sqlx::query_scalar("SELECT status::text FROM removal_requests WHERE id = $1")
            .bind(id).fetch_one(pool).await.unwrap()
    }
    async fn count(pool: &PgPool, sql: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql).bind(id).fetch_one(pool).await.unwrap()
    }

    // ---- authorization -----------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn authorizing_creates_an_authorized_request_and_an_audit_entry(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;

        let r = authorize_removal(&pool, c, e).await.unwrap();

        assert_eq!(r.status, RemovalStatus::Authorized);
        assert_eq!(r.customer_id, c);
        assert_eq!(r.exposure_id, e);
        let audit: (String, String) = sqlx::query_as(
            "SELECT action, actor FROM removal_audit_log WHERE removal_request_id = $1",
        ).bind(r.id).fetch_one(&pool).await.unwrap();
        assert_eq!(audit, ("authorized".into(), "customer".into()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_authorization_is_refused_while_one_is_in_flight(pool: PgPool) {
        for in_flight in ["authorized", "submitted", "acknowledged"] {
            let c = ts::customer(&pool, "personal", true).await;
            let e = ts::exposure(&pool, c, true, None).await;
            ts::removal_request(&pool, c, e, in_flight, Some(1)).await;
            let err = authorize_removal(&pool, c, e).await.unwrap_err();
            assert!(
                matches!(err.downcast_ref::<RemovalError>(), Some(RemovalError::AlreadyInFlight)),
                "{in_flight}: {err}"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_authorizations_create_exactly_one_request(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;

        let attempts = (0..8).map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { authorize_removal(&pool, c, e).await })
        });
        let results = join_all(attempts).await;

        let created = results.iter().filter(|r| r.is_ok()).count();
        let refused = results
            .iter()
            .filter(|r| matches!(r.as_ref().err().and_then(|e| e.downcast_ref::<RemovalError>()), Some(RemovalError::AlreadyInFlight)))
            .count();
        assert_eq!((created, refused), (1, 7), "one winner; the rest get a clean conflict, not a 500");
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM removal_requests WHERE exposure_id = $1", e).await, 1);
    }

    async fn join_all(
        handles: impl Iterator<Item = tokio::task::JoinHandle<Result<RemovalRequest>>>,
    ) -> Vec<Result<RemovalRequest>> {
        let mut out = Vec::new();
        for h in handles.collect::<Vec<_>>() {
            out.push(h.await.unwrap());
        }
        out
    }

    // ---- submission claim ---------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn a_submission_can_be_claimed_by_exactly_one_worker(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "authorized", None).await;

        assert!(claim_submission(&pool, r).await.unwrap(), "first worker wins");
        assert!(!claim_submission(&pool, r).await.unwrap(), "a second worker (or a restart) must not submit");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_claims_have_one_winner(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "authorized", None).await;

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let pool = pool.clone();
                tokio::spawn(async move { claim_submission(&pool, r).await.unwrap() })
            })
            .collect();
        let mut winners = 0;
        for h in handles {
            if h.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn only_authorized_requests_can_be_claimed(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        for status in ["submitted", "acknowledged", "failed", "verified_removed"] {
            let e = ts::exposure(&pool, c, true, None).await;
            let r = ts::removal_request(&pool, c, e, status, None).await;
            assert!(!claim_submission(&pool, r).await.unwrap(), "{status}");
        }
        assert!(!claim_submission(&pool, Uuid::new_v4()).await.unwrap(), "unknown id");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_abandoned_claim_is_failed_with_an_audit_entry_and_never_reissued(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "authorized", None).await;
        let fresh = ts::removal_request(&pool, c, ts::exposure(&pool, c, true, None).await, "authorized", None).await;
        let unclaimed = ts::removal_request(&pool, c, ts::exposure(&pool, c, true, None).await, "authorized", None).await;
        claim_submission(&pool, r).await.unwrap();
        claim_submission(&pool, fresh).await.unwrap();
        sqlx::query("UPDATE removal_requests SET submission_claimed_at = NOW() - interval '2 hours' WHERE id = $1")
            .bind(r).execute(&pool).await.unwrap();

        let ended = fail_abandoned_claims(&pool, SUBMISSION_CLAIM_STALE_AFTER_SECS).await.unwrap();

        assert_eq!(ended, vec![(r, c)]);
        assert_eq!(status_of(&pool, r).await, "failed");
        assert_eq!(status_of(&pool, fresh).await, "authorized", "a recent claim is still in progress");
        assert_eq!(status_of(&pool, unclaimed).await, "authorized", "unclaimed work is left for a worker");
        let action: String = sqlx::query_scalar(
            "SELECT action FROM removal_audit_log WHERE removal_request_id = $1 AND actor = 'system'",
        ).bind(r).fetch_one(&pool).await.unwrap();
        assert_eq!(action, "claim_expired");
        assert!(!claim_submission(&pool, r).await.unwrap(), "a failed request is never claimable again");
        // second pass finds nothing
        assert!(fail_abandoned_claims(&pool, SUBMISSION_CLAIM_STALE_AFTER_SECS).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_claim_that_resolved_in_time_is_not_failed(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "authorized", None).await;
        claim_submission(&pool, r).await.unwrap();
        update_removal_status(&pool, r, RemovalStatus::Submitted, None, None, None).await.unwrap();
        sqlx::query("UPDATE removal_requests SET submission_claimed_at = NOW() - interval '2 hours' WHERE id = $1")
            .bind(r).execute(&pool).await.unwrap();

        assert!(fail_abandoned_claims(&pool, SUBMISSION_CLAIM_STALE_AFTER_SECS).await.unwrap().is_empty());
        assert_eq!(status_of(&pool, r).await, "submitted");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_finished_request_can_be_authorized_again(pool: PgPool) {
        // reappeared / failed / rejected / unverifiable: the customer may try again
        for finished in ["reappeared", "failed", "rejected", "unverifiable"] {
            let c = ts::customer(&pool, "personal", true).await;
            let e = ts::exposure(&pool, c, true, None).await;
            ts::removal_request(&pool, c, e, finished, None).await;
            let again = authorize_removal(&pool, c, e).await.unwrap_or_else(|err| panic!("{finished}: {err}"));
            assert_eq!(again.status, RemovalStatus::Authorized);
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn nobody_can_authorize_removal_of_someone_elses_exposure(pool: PgPool) {
        let (owner, mallory) = (ts::customer(&pool, "personal", true).await, ts::customer(&pool, "personal", true).await);
        let e = ts::exposure(&pool, owner, true, None).await;
        let err = authorize_removal(&pool, mallory, e).await.unwrap_err();
        assert!(matches!(err.downcast_ref::<RemovalError>(), Some(RemovalError::NotFound)));
        assert_eq!(count(&pool, "SELECT count(*) FROM removal_requests WHERE exposure_id = $1", e).await, 0);
    }

    // ---- the lifecycle -------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn the_happy_path_walks_the_whole_lifecycle(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;

        let sub = update_removal_status(&pool, r, RemovalStatus::Submitted, Some("REF-1".into()), Some("abc".into()), Some(OffsetDateTime::now_utc() + time::Duration::days(7))).await.unwrap();
        assert_eq!(sub.status, RemovalStatus::Submitted);
        assert!(sub.submitted_at.is_some() && sub.next_check_at.is_some());
        assert_eq!(sub.confirmation_ref.as_deref(), Some("REF-1"));
        assert_eq!(sub.evidence_hash.as_deref(), Some("abc"));

        update_removal_status(&pool, r, RemovalStatus::Acknowledged, None, None, None).await.unwrap();
        let done = update_removal_status(&pool, r, RemovalStatus::VerifiedRemoved, None, None, None).await.unwrap();
        assert!(done.verified_at.is_some());
        assert_eq!(done.confirmation_ref.as_deref(), Some("REF-1"), "earlier metadata is kept");

        // exposure deactivated with a removal date, and the score recorded
        let (active, removed): (bool, bool) = sqlx::query_as("SELECT is_active, removed_at IS NOT NULL FROM exposures WHERE id = $1")
            .bind(e).fetch_one(&pool).await.unwrap();
        assert!(!active && removed);
        assert_eq!(count(&pool, "SELECT count(*) FROM privacy_score_history WHERE customer_id = $1", c).await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn submitted_at_is_set_once_even_if_submitted_is_reported_again(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;
        let first = update_removal_status(&pool, r, RemovalStatus::Submitted, None, None, None).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let again = update_removal_status(&pool, r, RemovalStatus::Submitted, None, None, None).await.unwrap();
        assert_eq!(first.submitted_at, again.submitted_at);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn illegal_transitions_are_refused_and_change_nothing(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;

        // cannot be "verified removed" without ever being submitted
        let err = update_removal_status(&pool, r, RemovalStatus::VerifiedRemoved, None, None, None).await.unwrap_err();
        assert!(matches!(
            err.downcast_ref::<RemovalError>(),
            Some(RemovalError::IllegalTransition { from: RemovalStatus::Authorized, to: RemovalStatus::VerifiedRemoved })
        ));
        assert_eq!(status_of(&pool, r).await, "authorized");
        let still_active: bool = sqlx::query_scalar("SELECT is_active FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        assert!(still_active, "a refused transition must not deactivate the exposure");

        // terminal states stay terminal
        update_removal_status(&pool, r, RemovalStatus::Failed, None, None, None).await.unwrap();
        for next in [RemovalStatus::Submitted, RemovalStatus::Authorized, RemovalStatus::VerifiedRemoved] {
            assert!(update_removal_status(&pool, r, next, None, None, None).await.is_err());
        }
        assert_eq!(status_of(&pool, r).await, "failed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_requests_are_not_found(pool: PgPool) {
        let err = update_removal_status(&pool, Uuid::new_v4(), RemovalStatus::Submitted, None, None, None).await.unwrap_err();
        assert!(matches!(err.downcast_ref::<RemovalError>(), Some(RemovalError::NotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn racing_updates_cannot_both_win(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = ts::removal_request(&pool, c, e, "submitted", Some(-1)).await;

        // "removed" and "rejected" race; after one commits the other is an illegal transition
        let (a, b) = tokio::join!(
            update_removal_status(&pool, r, RemovalStatus::VerifiedRemoved, None, None, None),
            update_removal_status(&pool, r, RemovalStatus::Rejected, None, None, None),
        );
        assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(), 1, "exactly one succeeds");
        let status = status_of(&pool, r).await;
        assert!(status == "verified_removed" || status == "rejected");
        // and the exposure agrees with whichever won
        let active: bool = sqlx::query_scalar("SELECT is_active FROM exposures WHERE id = $1").bind(e).fetch_one(&pool).await.unwrap();
        assert_eq!(active, status == "rejected");
    }

    // ---- alerts -----------------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn each_failure_outcome_alerts_the_customer_once_with_a_clear_reason(pool: PgPool) {
        let cases = [
            ("authorized", RemovalStatus::Failed, "could not be completed"),
            ("submitted", RemovalStatus::Rejected, "did not honor"),
            ("submitted", RemovalStatus::Unverifiable, "could not confirm"),
        ];
        for (from, to, phrase) in cases {
            let c = ts::customer(&pool, "personal", true).await;
            let e = ts::exposure(&pool, c, true, None).await;
            let r = ts::removal_request(&pool, c, e, from, Some(-1)).await;
            let label = format!("{to:?}");
            update_removal_status(&pool, r, to, None, None, None).await.unwrap();
            let messages: Vec<String> = sqlx::query_scalar("SELECT message FROM alerts WHERE customer_id = $1 AND alert_type = 'removal_failed'")
                .bind(c).fetch_all(&pool).await.unwrap();
            assert_eq!(messages.len(), 1, "{label}");
            assert!(messages[0].contains(phrase), "{label}: {}", messages[0]);
            assert!(messages[0].contains("re-authorize"), "tells the customer what they can do");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn successful_steps_do_not_alert(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;
        update_removal_status(&pool, r, RemovalStatus::Submitted, None, None, None).await.unwrap();
        update_removal_status(&pool, r, RemovalStatus::VerifiedRemoved, None, None, None).await.unwrap();
        assert_eq!(count(&pool, "SELECT count(*) FROM alerts WHERE customer_id = $1", c).await, 0);
    }

    // ---- audit entries -------------------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn audit_entries_are_attributed_to_the_requests_real_owner(pool: PgPool) {
        let (owner, other) = (ts::customer(&pool, "personal", true).await, ts::customer(&pool, "personal", true).await);
        let e = ts::exposure(&pool, owner, true, None).await;
        let r = authorize_removal(&pool, owner, e).await.unwrap().id;

        let ok = record_audit_entry(&pool, r, owner, "submitted".into(), "removal_executor".into(), serde_json::json!({})).await.unwrap();
        assert_eq!(ok.customer_id, owner);

        let wrong = record_audit_entry(&pool, r, other, "submitted".into(), "removal_executor".into(), serde_json::json!({})).await.unwrap_err();
        assert!(matches!(wrong.downcast_ref::<RemovalError>(), Some(RemovalError::NotFound)));
        assert_eq!(count(&pool, "SELECT count(*) FROM removal_audit_log WHERE removal_request_id = $1", r).await, 2, "authorize + one valid entry, nothing for the wrong customer");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_removal_audit_log_cannot_be_rewritten(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;

        // Each of these used to "succeed" and silently do nothing (RULE ... DO INSTEAD NOTHING).
        // They now fail loudly, so tampering is noticed instead of looking like it worked.
        let update = sqlx::query("UPDATE removal_audit_log SET action = 'tampered' WHERE removal_request_id = $1")
            .bind(r).execute(&pool).await.unwrap_err();
        let delete = sqlx::query("DELETE FROM removal_audit_log WHERE removal_request_id = $1")
            .bind(r).execute(&pool).await.unwrap_err();
        let truncate = sqlx::query("TRUNCATE removal_audit_log").execute(&pool).await.unwrap_err();
        for (what, err) in [("UPDATE", update), ("DELETE", delete), ("TRUNCATE", truncate)] {
            assert!(err.to_string().contains("append-only"), "{what}: {err}");
        }

        let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM removal_audit_log WHERE removal_request_id = $1").bind(r).fetch_all(&pool).await.unwrap();
        assert_eq!(actions, vec!["authorized".to_string()], "the history is intact");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn appending_to_the_removal_audit_log_still_works(pool: PgPool) {
        let c = ts::customer(&pool, "personal", true).await;
        let e = ts::exposure(&pool, c, true, None).await;
        let r = authorize_removal(&pool, c, e).await.unwrap().id;
        record_audit_entry(&pool, r, c, "submitted".into(), "removal_executor".into(), serde_json::json!({})).await.unwrap();
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM removal_audit_log WHERE removal_request_id = $1", r).await, 2);
    }
}
