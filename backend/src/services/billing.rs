//! Stripe billing: turns Stripe webhooks into a customer's plan.
//!
//! Design, in one paragraph. Stripe delivers webhooks at least once, possibly out of order,
//! and retries for days. So an event is never read as an instruction ("upgrade this person");
//! it is read as a snapshot of one subscription, stored in `stripe_subscriptions`, and the
//! customer's plan is then RECOMPUTED from all of their entitled subscriptions
//! (`sync_customer`). Replaying an event, or applying events in the wrong order, converges on
//! the same plan. Only a webhook with a valid Stripe signature can change a plan; the browser
//! returning from checkout changes nothing.
//!
//! Cancelling: the customer cancels in Stripe's portal, the subscription stays `active` with
//! `cancel_at_period_end` until the paid period is over, and then Stripe sends
//! `customer.subscription.deleted`. That is the moment the plan drops to Free, so a customer
//! keeps what they paid for.
//!
//! No personal data is kept here: the checkout payload (name, email, address) is read for
//! three fields and dropped.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::crypto::secrets_equal;
use crate::services::audit;
use crate::services::config::{PaidTier, StripeConfig};

type HmacSha256 = Hmac<Sha256>;

/// A webhook whose timestamp is further than this from now is refused (replay protection).
/// Five minutes is Stripe's own default.
pub const SIGNATURE_TOLERANCE_SECS: i64 = 300;

/// Subscription statuses that carry a paid plan. `past_due` keeps access while Stripe retries
/// the card; if it never succeeds Stripe cancels the subscription and the plan drops then.
/// `unpaid`, `incomplete`, `incomplete_expired`, `paused` and `canceled` carry none.
pub(crate) const ENTITLED_STATUSES: [&str; 3] = ["active", "trialing", "past_due"];

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    /// The payload is not an event we can read. The sender gets a 400.
    #[error("malformed Stripe event")]
    Malformed,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// What happened to an event. Only used for logging.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Ignored(&'static str),
    Applied { plan_changed: bool },
}

// ---------------------------------------------------------------------------
// Webhook signature
// ---------------------------------------------------------------------------

/// Check a `Stripe-Signature` header (`t=<unix>,v1=<hex hmac>[,v1=...]`) against the raw body.
///
/// The signed text is `"<t>.<body>"` under the endpoint's signing secret. The comparison is
/// constant-time, and a timestamp outside the tolerance fails. Returns only yes or no: the
/// caller must not tell a sender why a signature was refused.
pub fn verify_signature(secret: &str, header: &str, payload: &[u8], now_unix: i64) -> bool {
    let mut timestamp_raw: Option<&str> = None;
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    for part in header.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else { continue };
        match key {
            "t" => timestamp_raw = Some(value),
            "v1" => {
                if let Ok(bytes) = hex::decode(value) {
                    candidates.push(bytes);
                }
            }
            _ => {}
        }
    }
    let Some(raw) = timestamp_raw else { return false };
    let Ok(timestamp) = raw.parse::<i64>() else { return false };
    if (now_unix - timestamp).abs() > SIGNATURE_TOLERANCE_SECS {
        return false;
    }

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(raw.as_bytes());
    mac.update(b".");
    mac.update(payload);
    candidates.iter().any(|sig| mac.clone().verify_slice(sig).is_ok())
}

// ---------------------------------------------------------------------------
// Customer reference carried through checkout
// ---------------------------------------------------------------------------

fn reference_tag(secret: &str, customer: Uuid) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(b"vectorscrub-client-reference:");
    mac.update(customer.as_bytes());
    hex::encode(&mac.finalize().into_bytes()[..16])
}

/// The value passed to a Payment Link as `client_reference_id` (letters, digits, `-`, `_`).
///
/// It is signed because the link is built in the customer's browser, where it can be edited:
/// without the signature a buyer could attach their subscription to someone else's account.
pub fn client_reference_id(secret: &str, customer: Uuid) -> String {
    format!("{}_{}", customer.simple(), reference_tag(secret, customer))
}

/// The customer a `client_reference_id` names, if its signature is ours.
pub fn parse_client_reference_id(secret: &str, reference: &str) -> Option<Uuid> {
    let (id, tag) = reference.split_once('_')?;
    let customer = Uuid::parse_str(id).ok()?;
    secrets_equal(tag.as_bytes(), reference_tag(secret, customer).as_bytes()).then_some(customer)
}

// ---------------------------------------------------------------------------
// Event payloads (only the fields we use; everything else is ignored)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    created: i64,
    data: EventData,
}

#[derive(Deserialize)]
struct EventData {
    object: serde_json::Value,
}

#[derive(Deserialize)]
struct CheckoutSession {
    mode: Option<String>,
    client_reference_id: Option<String>,
    customer: Option<String>,
    subscription: Option<String>,
}

#[derive(Deserialize)]
struct SubscriptionObject {
    id: String,
    customer: String,
    status: String,
    #[serde(default)]
    cancel_at_period_end: bool,
    current_period_end: Option<i64>,
    #[serde(default)]
    items: Items,
}

#[derive(Deserialize, Default)]
struct Items {
    #[serde(default)]
    data: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    price: Option<Price>,
    /// Newer Stripe API versions report the period per item instead of on the subscription.
    current_period_end: Option<i64>,
}

#[derive(Deserialize)]
struct Price {
    id: String,
}

// ---------------------------------------------------------------------------
// Processing
// ---------------------------------------------------------------------------

/// Apply one verified webhook payload. Errors that are worth a Stripe retry (the database)
/// are returned as such; anything we merely do not care about is `Ok(Outcome::Ignored)`.
pub async fn process_event(
    pool: &PgPool,
    cfg: &StripeConfig,
    payload: &[u8],
) -> Result<Outcome, BillingError> {
    let event: Event = serde_json::from_slice(payload).map_err(|_| BillingError::Malformed)?;

    match event.kind.as_str() {
        "checkout.session.completed" => {
            let session: CheckoutSession =
                serde_json::from_value(event.data.object).map_err(|_| BillingError::Malformed)?;
            link_checkout(pool, cfg, session).await
        }
        "customer.subscription.created"
        | "customer.subscription.updated"
        | "customer.subscription.deleted" => {
            let mut sub: SubscriptionObject =
                serde_json::from_value(event.data.object).map_err(|_| BillingError::Malformed)?;
            if event.kind == "customer.subscription.deleted" {
                sub.status = "canceled".to_string();
            }
            apply_subscription(pool, cfg, event.created, sub).await
        }
        _ => Ok(Outcome::Ignored("event type not handled")),
    }
}

/// `checkout.session.completed`: tie the new subscription to the account that bought it.
async fn link_checkout(
    pool: &PgPool,
    cfg: &StripeConfig,
    session: CheckoutSession,
) -> Result<Outcome, BillingError> {
    if session.mode.as_deref() != Some("subscription") {
        return Ok(Outcome::Ignored("not a subscription checkout"));
    }
    let (Some(subscription_id), Some(stripe_customer)) = (session.subscription, session.customer) else {
        return Ok(Outcome::Ignored("checkout has no subscription"));
    };
    let Some(customer_id) = session
        .client_reference_id
        .as_deref()
        .and_then(|r| parse_client_reference_id(&cfg.webhook_secret, r))
    else {
        // Bought through a link that did not carry a valid reference (opened outside the
        // dashboard, or edited). There is no account to give the plan to.
        tracing::warn!(subscription_id = %subscription_id, "Checkout has no valid customer reference; not linked");
        return Ok(Outcome::Ignored("checkout not tied to an account"));
    };

    // The first account a subscription is linked to keeps it.
    sqlx::query(
        "INSERT INTO stripe_subscriptions (subscription_id, stripe_customer_id, customer_id, status, event_created) \
         SELECT $1, $2, c.id, 'pending', 0 FROM customers c WHERE c.id = $3 \
         ON CONFLICT (subscription_id) DO UPDATE \
            SET customer_id = COALESCE(stripe_subscriptions.customer_id, EXCLUDED.customer_id), \
                updated_at = NOW()",
    )
    .bind(&subscription_id)
    .bind(&stripe_customer)
    .bind(customer_id)
    .execute(pool)
    .await?;

    sync_linked(pool, cfg, &subscription_id).await
}

/// `customer.subscription.created|updated|deleted`: store the snapshot, then re-derive the plan.
async fn apply_subscription(
    pool: &PgPool,
    cfg: &StripeConfig,
    event_created: i64,
    sub: SubscriptionObject,
) -> Result<Outcome, BillingError> {
    // The subscription's price: the first item with a price we sell, else the first price.
    let price_id = sub
        .items
        .data
        .iter()
        .filter_map(|i| i.price.as_ref())
        .find(|p| cfg.plans.iter().any(|plan| plan.sells(&p.id)))
        .or_else(|| sub.items.data.iter().find_map(|i| i.price.as_ref()))
        .map(|p| p.id.clone());

    let period_end = sub
        .current_period_end
        .or_else(|| sub.items.data.iter().find_map(|i| i.current_period_end))
        .and_then(|t| OffsetDateTime::from_unix_timestamp(t).ok());

    // `WHERE` makes the update conditional: an older event never replaces a newer snapshot,
    // and a canceled subscription is final (a new purchase is a new subscription id).
    // If another subscription of the same Stripe customer is already linked to an account,
    // this one is linked to the same account.
    sqlx::query(
        "INSERT INTO stripe_subscriptions \
            (subscription_id, stripe_customer_id, customer_id, status, price_id, \
             cancel_at_period_end, current_period_end, event_created) \
         VALUES ($1, $2, \
            (SELECT customer_id FROM stripe_subscriptions \
              WHERE stripe_customer_id = $2 AND customer_id IS NOT NULL LIMIT 1), \
            $3, $4, $5, $6, $7) \
         ON CONFLICT (subscription_id) DO UPDATE SET \
            stripe_customer_id = EXCLUDED.stripe_customer_id, \
            customer_id = COALESCE(stripe_subscriptions.customer_id, EXCLUDED.customer_id), \
            status = EXCLUDED.status, \
            price_id = EXCLUDED.price_id, \
            cancel_at_period_end = EXCLUDED.cancel_at_period_end, \
            current_period_end = EXCLUDED.current_period_end, \
            event_created = EXCLUDED.event_created, \
            updated_at = NOW() \
         WHERE stripe_subscriptions.event_created <= EXCLUDED.event_created \
           AND stripe_subscriptions.status <> 'canceled'",
    )
    .bind(&sub.id)
    .bind(&sub.customer)
    .bind(&sub.status)
    .bind(price_id)
    .bind(sub.cancel_at_period_end)
    .bind(period_end)
    .bind(event_created)
    .execute(pool)
    .await?;

    sync_linked(pool, cfg, &sub.id).await
}

/// Re-derive the plan of whichever account the subscription is linked to (if any yet).
async fn sync_linked(
    pool: &PgPool,
    cfg: &StripeConfig,
    subscription_id: &str,
) -> Result<Outcome, BillingError> {
    let linked: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT customer_id FROM stripe_subscriptions WHERE subscription_id = $1")
            .bind(subscription_id)
            .fetch_optional(pool)
            .await?;
    match linked.flatten() {
        // The checkout that names the account has not arrived yet; it will trigger the sync.
        None => Ok(Outcome::Ignored("subscription not linked to an account yet")),
        Some(customer_id) => {
            let changed = sync_customer(pool, cfg, customer_id).await?;
            Ok(Outcome::Applied { plan_changed: changed })
        }
    }
}

/// Set the customer's plan to the best of their entitled subscriptions, or Free if there is
/// none. Returns whether the plan changed. Safe to call any number of times.
///
/// Paid plans turn monitoring on and Free turns it off: monitoring and automatic rescans are
/// gated by `monitoring_enabled` as well as by the plan, and nothing else sets it.
pub async fn sync_customer(
    pool: &PgPool,
    cfg: &StripeConfig,
    customer_id: Uuid,
) -> Result<bool, BillingError> {
    let mut tx = pool.begin().await?;

    let current: Option<String> =
        sqlx::query_scalar("SELECT subscription_tier::text FROM customers WHERE id = $1 FOR UPDATE")
            .bind(customer_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(current) = current else { return Ok(false) };

    let price_ids: Vec<String> = sqlx::query_scalar(
        "SELECT price_id FROM stripe_subscriptions \
         WHERE customer_id = $1 AND price_id IS NOT NULL AND status = ANY($2)",
    )
    .bind(customer_id)
    .bind(ENTITLED_STATUSES.map(String::from).to_vec())
    .fetch_all(&mut *tx)
    .await?;

    let mut best: Option<PaidTier> = None;
    for price_id in &price_ids {
        match cfg.plans.iter().find(|p| p.sells(price_id)) {
            Some(plan) => best = best.max(Some(plan.tier)),
            // An active subscription for a price we do not sell grants nothing. Loud, because
            // it usually means STRIPE_PRICE_* is missing a plan that was just created.
            None => tracing::error!(price_id = %price_id, "Active subscription for a price that is not configured"),
        }
    }
    let target = best.map(PaidTier::as_str).unwrap_or("free");

    if target == current {
        tx.commit().await?;
        return Ok(false);
    }

    sqlx::query(
        "UPDATE customers \
         SET subscription_tier = $2::text::subscription_tier, \
             monitoring_enabled = ($2::text <> 'free'), \
             updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(customer_id)
    .bind(target)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    audit::AuditEntry::builder(audit::actions::BILLING_PLAN_CHANGED, audit::resources::CUSTOMER)
        .customer_id(customer_id)
        .resource_id(customer_id)
        .details(serde_json::json!({ "from": current, "to": target }))
        .log_or_warn(pool)
        .await;
    tracing::info!(customer_id = %customer_id, from = %current, to = %target, "Plan changed by Stripe");

    Ok(true)
}

// ---------------------------------------------------------------------------
// What the dashboard shows
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanView {
    pub tier: &'static str,
    pub interval: &'static str,
    /// The Payment Link with this customer's signed reference and email filled in.
    pub checkout_url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubscriptionView {
    pub tier: &'static str,
    pub interval: &'static str,
    pub status: String,
    /// True once the customer has cancelled: the plan stays until `current_period_end`.
    pub cancel_at_period_end: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub current_period_end: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BillingView {
    /// False when Stripe is not configured; the page then offers nothing to buy.
    pub enabled: bool,
    pub plans: Vec<PlanView>,
    pub subscription: Option<SubscriptionView>,
    pub portal_url: Option<String>,
}

/// A Payment Link with the customer's reference attached. `None` if the link does not parse.
fn checkout_url(cfg: &StripeConfig, link: &str, customer_id: Uuid, email: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(link).ok()?;
    url.query_pairs_mut()
        .append_pair("client_reference_id", &client_reference_id(&cfg.webhook_secret, customer_id))
        .append_pair("prefilled_email", email);
    Some(url.into())
}

pub async fn billing_view(
    pool: &PgPool,
    cfg: Option<&StripeConfig>,
    customer_id: Uuid,
    email: &str,
) -> Result<BillingView, sqlx::Error> {
    let Some(cfg) = cfg else {
        return Ok(BillingView { enabled: false, plans: vec![], subscription: None, portal_url: None });
    };

    let plans = cfg
        .plans
        .iter()
        .filter_map(|p| {
            let url = checkout_url(cfg, p.link.as_deref()?, customer_id, email)?;
            Some(PlanView { tier: p.tier.as_str(), interval: p.interval.as_str(), checkout_url: url })
        })
        .collect();

    let row: Option<(Option<String>, String, bool, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT price_id, status, cancel_at_period_end, current_period_end \
         FROM stripe_subscriptions \
         WHERE customer_id = $1 AND price_id IS NOT NULL AND status = ANY($2) \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(customer_id)
    .bind(ENTITLED_STATUSES.map(String::from).to_vec())
    .fetch_optional(pool)
    .await?;

    let subscription = row.and_then(|(price_id, status, cancel, end)| {
        let plan = cfg.plans.iter().find(|p| price_id.as_deref().is_some_and(|id| p.sells(id)))?;
        Some(SubscriptionView {
            tier: plan.tier.as_str(),
            interval: plan.interval.as_str(),
            status,
            cancel_at_period_end: cancel,
            current_period_end: end,
        })
    });

    let portal_url = cfg.portal_url.as_deref().and_then(|u| {
        let mut url = reqwest::Url::parse(u).ok()?;
        url.query_pairs_mut().append_pair("prefilled_email", email);
        Some(String::from(url))
    });

    Ok(BillingView { enabled: true, plans, subscription, portal_url })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::config::{BillingInterval, StripePlan};
    use crate::test_support as ts;
    use serde_json::{json, Value};

    const SECRET: &str = "whsec_test_signing_secret_value";

    fn cfg() -> StripeConfig {
        let plan = |tier, interval, price: &str, link: Option<&str>| StripePlan {
            tier,
            interval,
            price_id: price.to_string(),
            legacy_price_ids: vec![],
            link: link.map(String::from),
        };
        StripeConfig {
            webhook_secret: SECRET.to_string(),
            plans: vec![
                plan(PaidTier::Personal, BillingInterval::Month, "price_personal_m", Some("https://buy.stripe.com/test_pm")),
                plan(PaidTier::Personal, BillingInterval::Year, "price_personal_y", Some("https://buy.stripe.com/test_py")),
                plan(PaidTier::Premium, BillingInterval::Month, "price_premium_m", Some("https://buy.stripe.com/test_rm")),
                plan(PaidTier::Premium, BillingInterval::Year, "price_premium_y", None),
            ],
            portal_url: Some("https://billing.stripe.com/p/login/test".to_string()),
        }
    }

    // ---- signature ---------------------------------------------------------------

    fn sign(secret: &str, t: i64, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{t}.").as_bytes());
        mac.update(body);
        format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn a_correct_signature_is_accepted() {
        let body = br#"{"id":"evt_1"}"#;
        assert!(verify_signature(SECRET, &sign(SECRET, 1_700_000_000, body), body, 1_700_000_010));
    }

    #[test]
    fn a_changed_body_a_wrong_secret_or_a_forged_tag_is_refused() {
        let body = br#"{"id":"evt_1"}"#;
        let header = sign(SECRET, 1_700_000_000, body);
        let now = 1_700_000_000;
        assert!(!verify_signature(SECRET, &header, br#"{"id":"evt_2"}"#, now), "body changed");
        assert!(!verify_signature("whsec_other_secret_value_x", &header, body, now), "wrong secret");
        assert!(!verify_signature(SECRET, "t=1700000000,v1=00ff", body, now), "forged tag");
        assert!(!verify_signature(SECRET, "t=1700000000,v1=not-hex", body, now), "not hex");
    }

    #[test]
    fn missing_or_malformed_headers_are_refused() {
        let body = b"{}";
        for header in ["", "garbage", "v1=abcd", "t=1700000000", "t=abc,v1=abcd", "t=,v1="] {
            assert!(!verify_signature(SECRET, header, body, 1_700_000_000), "{header:?}");
        }
    }

    #[test]
    fn the_timestamp_must_be_recent_in_both_directions() {
        let body = b"{}";
        let t = 1_700_000_000;
        let header = sign(SECRET, t, body);
        assert!(verify_signature(SECRET, &header, body, t + SIGNATURE_TOLERANCE_SECS));
        assert!(!verify_signature(SECRET, &header, body, t + SIGNATURE_TOLERANCE_SECS + 1), "too old: replay");
        assert!(verify_signature(SECRET, &header, body, t - SIGNATURE_TOLERANCE_SECS));
        assert!(!verify_signature(SECRET, &header, body, t - SIGNATURE_TOLERANCE_SECS - 1), "from the future");
    }

    #[test]
    fn any_one_matching_v1_is_enough_and_extra_fields_are_ignored() {
        // Stripe sends several v1 values while a signing secret is being rotated.
        let body = b"{}";
        let good = sign(SECRET, 1_700_000_000, body);
        let tag = good.split("v1=").nth(1).unwrap();
        let header = format!("t=1700000000,v1=deadbeef,v0=ignored,v1={tag}");
        assert!(verify_signature(SECRET, &header, body, 1_700_000_000));
    }

    // ---- customer reference --------------------------------------------------------

    #[test]
    fn a_customer_reference_round_trips_and_is_stripe_safe() {
        let id = Uuid::new_v4();
        let r = client_reference_id(SECRET, id);
        assert!(r.len() <= 200 && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{r}");
        assert_eq!(parse_client_reference_id(SECRET, &r), Some(id));
    }

    #[test]
    fn an_edited_or_foreign_customer_reference_is_refused() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let ra = client_reference_id(SECRET, a);
        // someone swaps in another account's id but keeps their own tag
        let tag = ra.split_once('_').unwrap().1;
        assert_eq!(parse_client_reference_id(SECRET, &format!("{}_{tag}", b.simple())), None);
        assert_eq!(parse_client_reference_id("whsec_a_different_secret_x", &ra), None, "other secret");
        for bad in ["", "_", "abc", &b.simple().to_string(), &format!("{}_", a.simple()), "not-a-uuid_00"] {
            assert_eq!(parse_client_reference_id(SECRET, bad), None, "{bad:?}");
        }
    }

    // ---- events ----------------------------------------------------------------------

    fn checkout(reference: Option<&str>, sub: &str, cus: &str, created: i64) -> Vec<u8> {
        json!({
            "id": "evt_checkout", "type": "checkout.session.completed", "created": created,
            "data": { "object": {
                "mode": "subscription", "client_reference_id": reference, "customer": cus,
                "subscription": sub,
                "customer_details": { "email": "buyer@example.com", "name": "Not Stored" }
            }}
        }).to_string().into_bytes()
    }

    fn subscription(kind: &str, sub: &str, cus: &str, status: &str, price: &str, cancel: bool, created: i64) -> Vec<u8> {
        json!({
            "id": format!("evt_{kind}_{created}"), "type": kind, "created": created,
            "data": { "object": {
                "id": sub, "customer": cus, "status": status, "cancel_at_period_end": cancel,
                "items": { "data": [ { "price": { "id": price }, "current_period_end": 1_800_000_000 } ] }
            }}
        }).to_string().into_bytes()
    }

    async fn tier(pool: &PgPool, c: Uuid) -> (String, bool) {
        sqlx::query_as("SELECT subscription_tier::text, monitoring_enabled FROM customers WHERE id = $1")
            .bind(c).fetch_one(pool).await.unwrap()
    }

    async fn feed(pool: &PgPool, payload: Vec<u8>) -> Outcome {
        process_event(pool, &cfg(), &payload).await.unwrap()
    }

    fn reference(c: Uuid) -> String { client_reference_id(SECRET, c) }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_purchase_upgrades_the_account_and_turns_monitoring_on(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        assert_eq!(tier(&pool, c).await, ("free".into(), false), "a checkout alone grants nothing");
        let out = feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_premium_m", false, 101)).await;
        assert_eq!(out, Outcome::Applied { plan_changed: true });
        assert_eq!(tier(&pool, c).await, ("premium".into(), true));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_two_events_may_arrive_in_either_order(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        // subscription first: no account known yet, nothing happens, nothing is lost
        let out = feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_y", false, 100)).await;
        assert_eq!(out, Outcome::Ignored("subscription not linked to an account yet"));
        assert_eq!(tier(&pool, c).await.0, "free");
        // the checkout then arrives and completes the picture
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 101)).await;
        assert_eq!(tier(&pool, c).await, ("personal".into(), true));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn replaying_an_event_changes_nothing(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        let ev = subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 101);
        assert_eq!(feed(&pool, ev.clone()).await, Outcome::Applied { plan_changed: true });
        assert_eq!(feed(&pool, ev).await, Outcome::Applied { plan_changed: false });
        assert_eq!(tier(&pool, c).await.0, "personal");
        let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'billing.plan_changed'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(audits, 1, "one real change, one audit entry");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cancelling_keeps_the_plan_until_the_paid_period_ends(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_premium_y", false, 101)).await;

        // the customer clicks cancel: Stripe keeps it active and flags the end of the period
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "active", "price_premium_y", true, 200)).await;
        assert_eq!(tier(&pool, c).await, ("premium".into(), true), "still Premium: they paid for the period");
        let view = billing_view(&pool, Some(&cfg()), c, "a@b.c").await.unwrap();
        let sub = view.subscription.expect("still subscribed");
        assert!(sub.cancel_at_period_end);

        // the period ends: Stripe deletes the subscription
        let out = feed(&pool, subscription("customer.subscription.deleted", "sub_1", "cus_1", "active", "price_premium_y", true, 300)).await;
        assert_eq!(out, Outcome::Applied { plan_changed: true });
        assert_eq!(tier(&pool, c).await, ("free".into(), false), "back to Free, monitoring off");
        assert!(billing_view(&pool, Some(&cfg()), c, "a@b.c").await.unwrap().subscription.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_late_old_event_cannot_undo_a_cancellation(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 101)).await;
        feed(&pool, subscription("customer.subscription.deleted", "sub_1", "cus_1", "canceled", "price_personal_m", false, 300)).await;
        assert_eq!(tier(&pool, c).await.0, "free");
        // Stripe retries an old "active" update after the deletion
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "active", "price_personal_m", false, 150)).await;
        assert_eq!(tier(&pool, c).await.0, "free", "older event ignored");
        // even one stamped later: a canceled subscription is final
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "active", "price_personal_m", false, 400)).await;
        assert_eq!(tier(&pool, c).await.0, "free", "canceled stays canceled");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn switching_plan_in_the_portal_changes_the_tier(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 101)).await;
        assert_eq!(tier(&pool, c).await.0, "personal");
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "active", "price_premium_m", false, 200)).await;
        assert_eq!(tier(&pool, c).await.0, "premium");
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "active", "price_personal_m", false, 300)).await;
        assert_eq!(tier(&pool, c).await.0, "personal", "a downgrade works too");
    }

    /// The price was raised: "price_personal_m" is now sold, "price_personal_m_old" is what
    /// early members pay.
    fn cfg_after_a_price_rise() -> StripeConfig {
        let mut c = cfg();
        let plan = c.plans.iter_mut().find(|p| p.price_id == "price_personal_m").unwrap();
        plan.legacy_price_ids = vec!["price_personal_m_old".to_string()];
        c
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn early_members_keep_their_plan_on_the_old_price_after_the_price_is_raised(pool: PgPool) {
        let cfg = cfg_after_a_price_rise();
        let (early, new) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        for (c, sub, price) in [(early, "sub_early", "price_personal_m_old"), (new, "sub_new", "price_personal_m")] {
            process_event(&pool, &cfg, &checkout(Some(&reference(c)), sub, &format!("cus_{sub}"), 100)).await.unwrap();
            process_event(&pool, &cfg, &subscription("customer.subscription.created", sub, &format!("cus_{sub}"), "active", price, false, 101)).await.unwrap();
        }
        assert_eq!(tier(&pool, early).await, ("personal".into(), true), "the locked price still gives the plan");
        assert_eq!(tier(&pool, new).await, ("personal".into(), true));

        // the dashboard shows both as the same plan, and offers only the current price's link
        let view = billing_view(&pool, Some(&cfg), early, "a@b.c").await.unwrap();
        let sub = view.subscription.expect("early member's subscription is recognised");
        assert_eq!((sub.tier, sub.interval), ("personal", "month"));
        assert_eq!(view.plans.iter().filter(|p| p.tier == "personal" && p.interval == "month").count(), 1);

        // and cancelling the old-price subscription still ends in Free
        process_event(&pool, &cfg, &subscription("customer.subscription.deleted", "sub_early", "cus_sub_early", "canceled", "price_personal_m_old", false, 300)).await.unwrap();
        assert_eq!(tier(&pool, early).await.0, "free");
        assert_eq!(tier(&pool, new).await.0, "personal", "the other member is untouched");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_price_that_was_never_configured_still_grants_nothing_even_with_legacy_prices(pool: PgPool) {
        let cfg = cfg_after_a_price_rise();
        let c = ts::customer(&pool, "free", false).await;
        process_event(&pool, &cfg, &checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await.unwrap();
        process_event(&pool, &cfg, &subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_something_else", false, 101)).await.unwrap();
        assert_eq!(tier(&pool, c).await.0, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn failed_payments_keep_access_while_stripe_retries_then_drop_it(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 101)).await;
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "past_due", "price_personal_m", false, 200)).await;
        assert_eq!(tier(&pool, c).await.0, "personal", "past_due: card being retried");
        feed(&pool, subscription("customer.subscription.updated", "sub_1", "cus_1", "unpaid", "price_personal_m", false, 300)).await;
        assert_eq!(tier(&pool, c).await.0, "free", "unpaid: no access");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unpaid_first_attempt_grants_nothing(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        for status in ["incomplete", "incomplete_expired", "paused"] {
            feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", status, "price_premium_m", false, 101)).await;
            assert_eq!(tier(&pool, c).await.0, "free", "{status}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_best_of_several_subscriptions_wins_and_one_ending_does_not_drop_the_rest(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_a", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_a", "cus_1", "active", "price_personal_m", false, 101)).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_b", "cus_1", 110)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_b", "cus_1", "active", "price_premium_m", false, 111)).await;
        assert_eq!(tier(&pool, c).await.0, "premium");
        feed(&pool, subscription("customer.subscription.deleted", "sub_b", "cus_1", "canceled", "price_premium_m", false, 200)).await;
        assert_eq!(tier(&pool, c).await.0, "personal", "the other subscription still pays for Personal");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_price_that_is_not_configured_grants_nothing(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_unknown", false, 101)).await;
        assert_eq!(tier(&pool, c).await.0, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_checkout_without_a_valid_reference_is_not_linked(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let forged = format!("{}_{}", c.simple(), "0".repeat(32));
        for r in [None, Some(forged.as_str()), Some("garbage")] {
            let out = feed(&pool, checkout(r, "sub_1", "cus_1", 100)).await;
            assert_eq!(out, Outcome::Ignored("checkout not tied to an account"), "{r:?}");
        }
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_premium_m", false, 101)).await;
        assert_eq!(tier(&pool, c).await.0, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_subscription_stays_with_the_first_account_it_was_linked_to(pool: PgPool) {
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        feed(&pool, checkout(Some(&reference(a)), "sub_1", "cus_1", 100)).await;
        feed(&pool, checkout(Some(&reference(b)), "sub_1", "cus_1", 101)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 102)).await;
        assert_eq!(tier(&pool, a).await.0, "personal");
        assert_eq!(tier(&pool, b).await.0, "free");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn other_events_and_other_checkouts_are_ignored_and_junk_is_malformed(pool: PgPool) {
        let other = json!({"id":"evt_x","type":"invoice.paid","created":1,"data":{"object":{}}}).to_string().into_bytes();
        assert!(matches!(feed(&pool, other).await, Outcome::Ignored(_)));
        let one_off = json!({"id":"e","type":"checkout.session.completed","created":1,
            "data":{"object":{"mode":"payment","customer":"cus_1"}}}).to_string().into_bytes();
        assert!(matches!(feed(&pool, one_off).await, Outcome::Ignored(_)));
        for junk in [&b"not json"[..], b"{}", b"[]"] {
            assert!(matches!(process_event(&pool, &cfg(), junk).await, Err(BillingError::Malformed)));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn accounts_never_touched_by_stripe_keep_a_hand_set_plan(pool: PgPool) {
        let manual = ts::customer(&pool, "premium", true).await;
        let buyer = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(buyer)), "sub_1", "cus_1", 100)).await;
        feed(&pool, subscription("customer.subscription.created", "sub_1", "cus_1", "active", "price_personal_m", false, 101)).await;
        assert_eq!(tier(&pool, manual).await, ("premium".into(), true));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_stored_row_holds_no_personal_data(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        feed(&pool, checkout(Some(&reference(c)), "sub_1", "cus_1", 100)).await;
        let row: Value = sqlx::query_scalar("SELECT to_jsonb(s) FROM stripe_subscriptions s").fetch_one(&pool).await.unwrap();
        let text = row.to_string();
        assert!(!text.contains("buyer@example.com") && !text.contains("Not Stored"), "{text}");
    }

    // ---- what the dashboard shows ---------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn the_billing_view_offers_the_configured_links_with_a_signed_reference(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let v = billing_view(&pool, Some(&cfg()), c, "jane+test@example.com").await.unwrap();
        assert!(v.enabled);
        assert_eq!(v.plans.len(), 3, "the annual Premium plan has no link configured, so it is not offered");
        let first = reqwest::Url::parse(&v.plans[0].checkout_url).unwrap();
        let params: std::collections::HashMap<_, _> = first.query_pairs().into_owned().collect();
        assert_eq!(parse_client_reference_id(SECRET, &params["client_reference_id"]), Some(c));
        assert_eq!(params["prefilled_email"], "jane+test@example.com", "encoded and decoded losslessly");
        assert!(v.portal_url.unwrap().starts_with("https://billing.stripe.com/"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn with_stripe_unconfigured_nothing_is_offered(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let v = billing_view(&pool, None, c, "a@b.c").await.unwrap();
        assert!(!v.enabled && v.plans.is_empty() && v.subscription.is_none() && v.portal_url.is_none());
    }
}
