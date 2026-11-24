//! Deletion requests sent to companies on a customer's behalf (migration 0015).
//!
//! The customer approves a campaign once: which companies, whether the address goes in, and the
//! authorization text, which is stored exactly as shown. The worker then sends one email per
//! company from the channel catalog. The rules that matter:
//!
//! * the recipient address comes from the catalog only (`broker_channels`), and a company is only
//!   offered to a customer in a state its page offers the channel to;
//! * only residents of the pilot states (`DELETION_PILOT_STATES`, default CA) are offered anything;
//! * paid plans only, a 90-day cooldown per company, and no company twice in a campaign;
//! * the customer chooses the kinds of request (delete, opt out of sale or sharing) separately, and a
//!   request asks only for what was chosen;
//! * a request not yet claimed for sending can be withdrawn; one already sent cannot be recalled;
//! * each campaign is signed: the customer types their full name, which must match their saved
//!   details, after agreeing to sign electronically. The signed document is stored encrypted;
//! * an interrupted send is never retried automatically. An operator who has checked the mail
//!   provider's logs for its Message-ID may mark it sent, or put it back in the queue;
//! * a request is claimed before sending and is never retried if interrupted: it fails closed;
//! * what we know is what we sent. A company's answer goes to the customer, who reports it.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use time::format_description::well_known::Rfc3339;
use zeroize::Zeroizing;

use crate::crypto::envelope::purpose;
use crate::services::broker_channels;
use crate::services::keys::KeyService;

/// Bump when the wording changes: a customer must accept the current version.
pub const AUTHORIZATION_VERSION: &str = "2026-11-draft-3";

/// DRAFT wording. A lawyer's recommendations are applied (third draft), but it still needs the
/// approval of California privacy counsel before this is switched on for real customers
/// (`DELETION_REQUESTS_ENABLED`). Stored verbatim with each campaign.
pub const AUTHORIZATION_TEXT: &str = "\
I authorize VectorScrub, a product of VectorGuard Labs, to act as my authorized agent in sending \
the privacy requests I select on this page to the companies I select on this page. This covers only \
the companies and the kinds of request I select now: a request to delete the personal information a \
company holds about me, and, if I select it and the company's page offers it, a request to stop \
selling or sharing it. I choose each kind separately.\n\
\n\
I agree that VectorScrub may send each selected company my name and email address. It will send my \
street address only if I tick the separate box to include it, or if the company's page says it needs \
the address to find my records (this page tells me before I submit). It will not share my phone \
numbers or date of birth. A company may contact me directly at my email address to verify my \
identity or to confirm that I authorized this, and may ask for more information or for proof of my \
signed permission. I may have to complete those steps with the company myself, and a company may \
decline a request.\n\
\n\
I understand that VectorScrub sends these emails from its own email service, does not read the \
company's replies (they come to me), and records only what it sent and what I report back. A request \
is not a guarantee of removal, and the law that gives me these rights depends on where I live.\n\
\n\
I can withdraw this authorization for any request that has not yet been sent, with the Withdraw \
button on the Deletion requests page or by contacting VectorScrub. Withdrawing cannot recall an \
email that has already been sent. VectorScrub keeps a record of this authorization and of what was \
sent, as its Privacy Policy describes.\n\
\n\
I sign this authorization electronically by typing my full name. I agree that my typed name is my \
signature on it, and that VectorScrub keeps the signed copy, which I can download and give to a \
company that asks for proof of my signed permission.";

/// A company is not sent a second request from the same customer inside this many days.
pub const COOLDOWN_DAYS: i32 = 90;
/// California's CCPA gives a business 45 days to respond to a deletion request, and one extension of
/// up to 45 more if it tells the consumer. We cannot see replies (they go to the customer), so after
/// 45 days with nothing reported the dashboard says "no answer reported", not "overdue": the company
/// may have answered, or may have used its extension. After 90 days even an extension has run out.
pub const FOLLOW_UP_DAYS: i64 = 45;
/// Days after which even an extended response period has passed.
pub const EXTENDED_FOLLOW_UP_DAYS: i64 = 90;
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
    #[error("the pilot is not open in this customer's state")]
    OutsidePilot,
    #[error("no kind of request selected")]
    NoRequestKind,
    #[error("a selected company does not offer the kind of request chosen")]
    KindNotOffered,
    #[error("that request has already been sent, or is being sent")]
    TooLateToWithdraw,
    #[error("the authorization was not signed")]
    SignatureRequired,
    #[error("the signature does not match the saved name")]
    SignatureMismatch,
    #[error("the name or email address needed for the requests is not saved")]
    DetailsMissing,
    #[error("the campaign was withdrawn")]
    CampaignWithdrawn,
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
    #[error("key service error: {0}")]
    Keys(anyhow::Error),
}

pub struct NewCampaign<'a> {
    pub channel_ids: &'a [String],
    /// What the customer asks for, chosen separately: `delete` and/or `opt_out_sale`.
    pub request_kinds: &'a [String],
    pub include_address: bool,
    pub consent: bool,
    pub authorization_version: &'a str,
    /// The full name the customer typed as their signature.
    pub signature: &'a str,
    /// The customer agreed to sign electronically.
    pub esign_consent: bool,
}

/// Who signs: the name and the email address the requests will carry, from the customer's saved,
/// encrypted details. Held only in memory, cleared when dropped.
pub struct Signer {
    pub full_name: Zeroizing<String>,
    pub email: Zeroizing<String>,
}

/// The customer's saved name and first email address, or `None` when either is missing.
pub async fn signer_for(pool: &PgPool, keys: &KeyService, customer_id: Uuid) -> anyhow::Result<Option<Signer>> {
    let saved: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM identity_profiles WHERE customer_id = $1)")
        .bind(customer_id)
        .fetch_one(pool)
        .await?;
    if !saved {
        return Ok(None);
    }
    let identity = crate::services::customer::load_cleartext_identity(pool, keys, customer_id).await?;
    let full_name = Zeroizing::new(normalize_name(&identity.full_name));
    let email = Zeroizing::new(identity.email_addresses.first().map(|e| e.trim().to_string()).unwrap_or_default());
    if full_name.is_empty() || email.is_empty() {
        return Ok(None);
    }
    Ok(Some(Signer { full_name, email }))
}

/// Runs of whitespace become one space; ends trimmed. Used to compare a typed signature with a name.
fn normalize_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A typed signature counts when it is the saved name, ignoring case and spacing.
pub fn signature_matches(typed: &str, saved_name: &str) -> bool {
    let typed = normalize_name(typed);
    !typed.is_empty() && typed.chars().count() <= 200 && !typed.chars().any(char::is_control)
        && typed.to_lowercase() == normalize_name(saved_name).to_lowercase()
}

const KIND_SENTENCE: [(&str, &str); 2] = [
    ("delete", "a request that the company delete the personal information it holds about me"),
    ("opt_out_sale", "a request that the company stop selling or sharing my personal information"),
];

/// One company in a signed authorization: its name, the address the request goes to, and what it is asked.
pub struct SignedCompany<'a> {
    pub company: &'a str,
    pub address: &'a str,
    pub asked: Vec<String>,
}

/// The signed authorization, as plain text. Its SHA-256 is stored next to the encrypted copy, so a
/// printed or downloaded copy can be checked against it. Changing this format changes new documents
/// only: each stored document is kept exactly as it was signed.
pub fn signed_document(
    signer: &Signer,
    kinds: &[String],
    companies: &[SignedCompany<'_>],
    include_address: bool,
    authorized_at: OffsetDateTime,
) -> String {
    let when = authorized_at.format(&Rfc3339).unwrap_or_default();
    let mut doc = String::new();
    doc.push_str("AUTHORIZATION OF AN AUTHORIZED AGENT\n");
    doc.push_str(&format!("Authorization version: {AUTHORIZATION_VERSION}\n\n"));
    doc.push_str(&format!("Consumer: {}\n", one_line(&signer.full_name)));
    doc.push_str(&format!("Consumer's email address: {}\n", one_line(&signer.email)));
    doc.push_str("Authorized agent: VectorScrub, a product of VectorGuard Labs\n\n");
    doc.push_str("What I authorize the agent to send, each chosen separately:\n");
    for (kind, sentence) in KIND_SENTENCE {
        let mark = if kinds.iter().any(|k| k == kind) { "AUTHORIZED" } else { "NOT AUTHORIZED" };
        doc.push_str(&format!("  [{mark}] {sentence}\n"));
    }
    doc.push_str("\nThe companies, the address each request goes to, and what each is asked:\n");
    for c in companies {
        let asked: Vec<&str> = KIND_SENTENCE.iter().filter(|(k, _)| c.asked.iter().any(|a| a == k)).map(|(k, _)| match *k {
            "delete" => "delete my personal information",
            _ => "stop selling or sharing it",
        }).collect();
        doc.push_str(&format!("  - {} ({}): {}\n", one_line(c.company), one_line(c.address), asked.join("; ")));
    }
    doc.push_str(&format!(
        "\nMy street address is included in the requests: {}\n",
        if include_address { "yes" } else { "no" }
    ));
    doc.push_str("My phone numbers and date of birth are never included.\n\n");
    doc.push_str("The authorization I accepted:\n\n");
    doc.push_str(AUTHORIZATION_TEXT);
    doc.push_str("\n\nSigned electronically. I agreed to sign electronically and typed my full name as my signature.\n");
    doc.push_str(&format!("Signature: {}\n", one_line(&signer.full_name)));
    doc.push_str(&format!("Date and time (UTC): {when}\n"));
    doc
}

/// Control characters and line breaks become spaces, so a value cannot add a line to the document.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Create a campaign and queue one request per selected company. `state` is the US state of the
/// customer's saved address, if it can be told. One transaction: all of it, or nothing.
#[allow(clippy::too_many_arguments)]
pub async fn create_campaign(
    pool: &PgPool,
    keys: &KeyService,
    customer_id: Uuid,
    state: Option<&str>,
    pilot_states: &[String],
    signer: Option<&Signer>,
    new: NewCampaign<'_>,
) -> Result<(Uuid, Vec<Uuid>), DeletionError> {
    if !new.consent {
        return Err(DeletionError::ConsentRequired);
    }
    if !new.esign_consent || new.signature.trim().is_empty() {
        return Err(DeletionError::SignatureRequired);
    }
    if new.authorization_version != AUTHORIZATION_VERSION {
        return Err(DeletionError::AuthorizationOutdated);
    }
    if !broker_channels::in_pilot(state, pilot_states) {
        return Err(DeletionError::OutsidePilot);
    }
    // Only the two known kinds, each once, in the catalog's order.
    let kinds: Vec<String> = broker_channels::REQUEST_KINDS
        .iter()
        .filter(|k| new.request_kinds.iter().any(|r| r == *k))
        .map(|k| k.to_string())
        .collect();
    if kinds.is_empty() || new.request_kinds.iter().any(|r| !broker_channels::REQUEST_KINDS.contains(&r.as_str())) {
        return Err(DeletionError::NoRequestKind);
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
    // Each company must offer at least one of the kinds chosen, or there is nothing to send it.
    if channels.iter().any(|c| !c.requests.iter().any(|r| kinds.contains(r))) {
        return Err(DeletionError::KindNotOffered);
    }
    // A company whose page asks for the address cannot find the person without it. Better to say
    // so now than to send a request it cannot act on.
    if !new.include_address && channels.iter().any(|c| c.requires_address) {
        return Err(DeletionError::AddressRequired);
    }

    // The signature must be the saved name: it is the name the companies receive.
    let signer = signer.ok_or(DeletionError::DetailsMissing)?;
    if !signature_matches(new.signature, &signer.full_name) {
        return Err(DeletionError::SignatureMismatch);
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
               AND (status NOT IN ('failed', 'withdrawn') OR (status = 'failed' AND error LIKE $4)) \
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

    // The signed document: what is asked of each company, in the customer's name, with the time.
    let authorized_at = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap_or_else(|_| OffsetDateTime::now_utc());
    let companies: Vec<SignedCompany<'_>> = channels
        .iter()
        .map(|c| SignedCompany { company: &c.company, address: &c.address, asked: asked_of(&c.id, &kinds) })
        .collect();
    let document = Zeroizing::new(signed_document(signer, &kinds, &companies, new.include_address, authorized_at));
    let document_sha256 = hex::encode(Sha256::digest(document.as_bytes()));
    let ck = keys.customer(customer_id).await.map_err(|e| DeletionError::Keys(e.into()))?;
    let sealed = ck.seal_str(purpose::DELETION_SIGNED_AUTHORIZATION, &document).map_err(DeletionError::Keys)?;

    let campaign_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deletion_campaigns \
            (id, customer_id, authorization_version, authorization_text, authorization_sha256, include_address, request_kinds, \
             authorized_at, esign_consent_at, signed_document_enc, signed_document_sha256) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $9, $10)",
    )
    .bind(campaign_id)
    .bind(customer_id)
    .bind(AUTHORIZATION_VERSION)
    .bind(AUTHORIZATION_TEXT)
    .bind(authorization_sha256())
    .bind(new.include_address)
    .bind(&kinds)
    .bind(authorized_at)
    .bind(&sealed)
    .bind(&document_sha256)
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
    pub campaign_id: Uuid,
    pub channel_id: String,
    pub company: String,
    pub status: String,
    /// What was asked of this company: the campaign's kinds that the company offers.
    pub requests: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub queued_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub outcome_at: Option<OffsetDateTime>,
    /// Sent 45 or more days ago and no answer reported. Not a finding that the company is late: it may
    /// have replied to the customer, or taken the extension it is allowed.
    pub follow_up_due: bool,
    /// Sent 90 or more days ago and no answer reported: even an extended response period has passed.
    pub extended_period_over: bool,
    /// What went wrong, in fixed words, for a failed request.
    pub error: Option<String>,
}

type Row = (
    Uuid,
    Uuid,
    String,
    String,
    Vec<String>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<String>,
);

/// The kinds of request one company is sent: the campaign's choices that the company offers.
pub fn asked_of(channel_id: &str, kinds: &[String]) -> Vec<String> {
    broker_channels::find(channel_id)
        .map(|c| c.requests.iter().filter(|r| kinds.contains(r)).cloned().collect())
        .unwrap_or_default()
}

pub async fn list_for_customer(pool: &PgPool, customer_id: Uuid) -> Result<Vec<DeletionRequestView>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.id, r.campaign_id, r.channel_id, r.status, c.request_kinds, r.queued_at, r.sent_at, r.outcome_at, r.error \
         FROM deletion_requests r JOIN deletion_campaigns c ON c.id = r.campaign_id \
         WHERE r.customer_id = $1 ORDER BY r.queued_at DESC, r.id",
    )
    .bind(customer_id)
    .fetch_all(pool)
    .await?;
    let now = OffsetDateTime::now_utc();
    Ok(rows
        .into_iter()
        .map(|(id, campaign_id, channel_id, status, kinds, queued_at, sent_at, outcome_at, error)| {
            let company = broker_channels::find(&channel_id).map(|c| c.company.clone()).unwrap_or_else(|| channel_id.clone());
            let days_unanswered = |limit: i64| status == "sent" && sent_at.is_some_and(|t| (now - t).whole_days() >= limit);
            let requests = asked_of(&channel_id, &kinds);
            DeletionRequestView {
                id,
                campaign_id,
                channel_id,
                company,
                status: status.clone(),
                requests,
                queued_at,
                sent_at,
                outcome_at,
                follow_up_due: days_unanswered(FOLLOW_UP_DAYS),
                extended_period_over: days_unanswered(EXTENDED_FOLLOW_UP_DAYS),
                error,
            }
        })
        .collect())
}

/// Withdraw one request that has not been claimed for sending. A request that is being sent or was
/// sent cannot be recalled, and says so.
pub async fn withdraw_request(pool: &PgPool, customer_id: Uuid, request_id: Uuid) -> Result<(), DeletionError> {
    let done = sqlx::query(
        "UPDATE deletion_requests SET status = 'withdrawn', withdrawn_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND customer_id = $2 AND status = 'queued'",
    )
    .bind(request_id)
    .bind(customer_id)
    .execute(pool)
    .await?
    .rows_affected();
    if done == 1 {
        return Ok(());
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM deletion_requests WHERE id = $1 AND customer_id = $2)")
        .bind(request_id)
        .bind(customer_id)
        .fetch_one(pool)
        .await?;
    Err(if exists { DeletionError::TooLateToWithdraw } else { DeletionError::NotFound })
}

/// Withdraw every request of a campaign that has not been claimed for sending. Returns how many were
/// withdrawn (0 when all had already gone out). The campaign must be the customer's.
pub async fn withdraw_campaign(pool: &PgPool, customer_id: Uuid, campaign_id: Uuid) -> Result<u64, DeletionError> {
    let mut tx = pool.begin().await?;
    let owned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM deletion_campaigns WHERE id = $1 AND customer_id = $2)")
        .bind(campaign_id)
        .bind(customer_id)
        .fetch_one(&mut *tx)
        .await?;
    if !owned {
        return Err(DeletionError::NotFound);
    }
    // From now on nothing is sent under this authorization, whatever happens to its requests.
    sqlx::query("UPDATE deletion_campaigns SET withdrawn_at = COALESCE(withdrawn_at, NOW()) WHERE id = $1")
        .bind(campaign_id)
        .execute(&mut *tx)
        .await?;
    let n = sqlx::query(
        "UPDATE deletion_requests SET status = 'withdrawn', withdrawn_at = NOW(), updated_at = NOW() \
         WHERE campaign_id = $1 AND customer_id = $2 AND status = 'queued'",
    )
    .bind(campaign_id)
    .bind(customer_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(n)
}

/// What the customer accepted, as one record they can keep or hand to a company that asks for proof.
/// It holds no identity details: the name, email and address are not stored with a campaign.
#[derive(Debug, Serialize)]
pub struct AuthorizationRecord {
    pub campaign_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub authorized_at: OffsetDateTime,
    pub authorization_version: String,
    pub authorization_text: String,
    pub authorization_sha256: String,
    pub include_address: bool,
    pub request_kinds: Vec<String>,
    /// The signed document, opened for its owner, and the SHA-256 of its text. `None` for a campaign
    /// made before signing existed.
    pub signed_document: Option<String>,
    pub signed_document_sha256: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub esign_consent_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub withdrawn_at: Option<OffsetDateTime>,
    pub companies: Vec<AuthorizedCompany>,
}

#[derive(Debug, Serialize)]
pub struct AuthorizedCompany {
    pub request_id: Uuid,
    pub company: String,
    /// The address the request is (or would have been) sent to, from the evidence-checked catalog.
    pub to_address: Option<String>,
    pub status: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub withdrawn_at: Option<OffsetDateTime>,
}

type AuthorizedRow = (Uuid, String, String, Option<OffsetDateTime>, Option<OffsetDateTime>);

#[derive(sqlx::FromRow)]
struct CampaignHead {
    authorized_at: OffsetDateTime,
    authorization_version: String,
    authorization_text: String,
    authorization_sha256: String,
    include_address: bool,
    request_kinds: Vec<String>,
    signed_document_enc: Option<Vec<u8>>,
    signed_document_sha256: Option<String>,
    esign_consent_at: Option<OffsetDateTime>,
    withdrawn_at: Option<OffsetDateTime>,
}

pub async fn authorization_record(
    pool: &PgPool,
    keys: &KeyService,
    customer_id: Uuid,
    campaign_id: Uuid,
) -> Result<AuthorizationRecord, DeletionError> {
    let head: Option<CampaignHead> = sqlx::query_as(
        "SELECT authorized_at, authorization_version, authorization_text, authorization_sha256, include_address, request_kinds, \
                signed_document_enc, signed_document_sha256, esign_consent_at, withdrawn_at \
         FROM deletion_campaigns WHERE id = $1 AND customer_id = $2",
    )
    .bind(campaign_id)
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;
    let head = head.ok_or(DeletionError::NotFound)?;
    let signed_document = match &head.signed_document_enc {
        Some(blob) => {
            let ck = keys.customer(customer_id).await.map_err(|e| DeletionError::Keys(e.into()))?;
            Some(ck.open_str(purpose::DELETION_SIGNED_AUTHORIZATION, blob).map_err(DeletionError::Keys)?)
        }
        None => None,
    };
    let rows: Vec<AuthorizedRow> = sqlx::query_as(
        "SELECT id, channel_id, status, sent_at, withdrawn_at FROM deletion_requests \
         WHERE campaign_id = $1 AND customer_id = $2 ORDER BY queued_at, id",
    )
    .bind(campaign_id)
    .bind(customer_id)
    .fetch_all(pool)
    .await?;
    let companies = rows
        .into_iter()
        .map(|(request_id, channel_id, status, sent_at, withdrawn_at)| {
            let channel = broker_channels::find(&channel_id);
            AuthorizedCompany {
                request_id,
                company: channel.map(|c| c.company.clone()).unwrap_or(channel_id),
                to_address: channel.map(|c| c.address.clone()),
                status,
                sent_at,
                withdrawn_at,
            }
        })
        .collect();
    Ok(AuthorizationRecord {
        campaign_id,
        authorized_at: head.authorized_at,
        authorization_version: head.authorization_version,
        authorization_text: head.authorization_text,
        authorization_sha256: head.authorization_sha256,
        include_address: head.include_address,
        request_kinds: head.request_kinds,
        signed_document,
        signed_document_sha256: head.signed_document_sha256,
        esign_consent_at: head.esign_consent_at,
        withdrawn_at: head.withdrawn_at,
        companies,
    })
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
    /// The kinds of request the customer chose for the campaign.
    pub request_kinds: Vec<String>,
    /// Fingerprint of the signed authorization (no personal data), quoted in the email.
    pub signed_document_sha256: Option<String>,
}

/// Requests waiting to be sent, oldest first (at most 50). Call `fail_stale_sending` first, so
/// anything interrupted while sending is failed and never picked up again. Nothing under a
/// withdrawn authorization is listed.
pub async fn list_queued(pool: &PgPool) -> Result<Vec<QueuedRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT r.id, r.customer_id, r.channel_id, c.include_address, c.authorized_at, \
                c.authorization_version, c.authorization_text, c.request_kinds, c.signed_document_sha256 \
         FROM deletion_requests r \
         JOIN deletion_campaigns c ON c.id = r.campaign_id \
         JOIN customers u ON u.id = r.customer_id \
         WHERE r.status = 'queued' AND c.withdrawn_at IS NULL AND u.is_active = true AND u.subscription_tier <> 'free' \
         ORDER BY r.queued_at, r.id LIMIT 50",
    )
    .fetch_all(pool)
    .await
}

/// Take a queued request. True for exactly one caller. `message_id` is the Message-ID the worker will
/// send the email with, recorded now so that an interrupted send can be traced in the mail logs.
pub async fn claim(pool: &PgPool, request_id: Uuid, message_id: Option<&str>) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE deletion_requests r SET status = 'sending', claimed_at = NOW(), updated_at = NOW(), message_id = $2 \
         FROM deletion_campaigns c \
         WHERE r.id = $1 AND r.status = 'queued' AND c.id = r.campaign_id AND c.withdrawn_at IS NULL",
    )
    .bind(request_id)
    .bind(message_id.map(|m| m.chars().take(300).collect::<String>()))
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
        "UPDATE deletion_requests SET status = $2, message_id = COALESCE($3, message_id), error = $4, \
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

// ---------------------------------------------------------------------------
// Operator: interrupted sends (`vectorscrub deletion interrupted | resolve`)
// ---------------------------------------------------------------------------

/// A request whose send was interrupted, so it is not known whether the email left. No personal data.
#[derive(Debug, sqlx::FromRow)]
pub struct Interrupted {
    pub id: Uuid,
    pub channel_id: String,
    pub claimed_at: Option<OffsetDateTime>,
    /// The Message-ID it was (or would have been) sent with. Search the mail provider's logs for it.
    pub message_id: Option<String>,
}

pub async fn interrupted(pool: &PgPool) -> Result<Vec<Interrupted>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, channel_id, claimed_at, message_id FROM deletion_requests \
         WHERE status = 'failed' AND error LIKE $1 ORDER BY claimed_at, id",
    )
    .bind(format!("{ABANDONED_MARKER}%"))
    .fetch_all(pool)
    .await
}

/// What an operator found in the mail provider's logs for an interrupted send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailLogFinding {
    /// The message left: record it as sent (at the time it was claimed).
    Sent,
    /// The message never left: put the request back in the queue, to be sent once.
    NotSent,
}

/// Apply an operator's finding to one interrupted request. Only an interrupted request can be resolved,
/// once. A request under a withdrawn authorization is never put back in the queue.
pub async fn resolve_interrupted(pool: &PgPool, request_id: Uuid, finding: MailLogFinding) -> Result<Uuid, DeletionError> {
    let mut tx = pool.begin().await?;
    let row: Option<(Uuid, String, Option<String>, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT r.customer_id, r.status, r.error, c.withdrawn_at FROM deletion_requests r \
         JOIN deletion_campaigns c ON c.id = r.campaign_id WHERE r.id = $1 FOR UPDATE OF r",
    )
    .bind(request_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (customer_id, status, error, withdrawn_at) = row.ok_or(DeletionError::NotFound)?;
    if status != "failed" || !error.as_deref().is_some_and(|e| e.starts_with(ABANDONED_MARKER)) {
        return Err(DeletionError::IllegalTransition);
    }
    match finding {
        MailLogFinding::Sent => {
            sqlx::query(
                "UPDATE deletion_requests SET status = 'sent', sent_at = claimed_at, error = NULL, updated_at = NOW() WHERE id = $1",
            )
            .bind(request_id)
            .execute(&mut *tx)
            .await?;
        }
        MailLogFinding::NotSent => {
            if withdrawn_at.is_some() {
                return Err(DeletionError::CampaignWithdrawn);
            }
            sqlx::query(
                "UPDATE deletion_requests SET status = 'queued', claimed_at = NULL, message_id = NULL, error = NULL, \
                     updated_at = NOW() WHERE id = $1",
            )
            .bind(request_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(customer_id)
}
