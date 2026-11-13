//! Pilot tooling for `vectorscrub pilot ...` (see main.rs): a scorecard of the funnel, a case
//! view of one customer, and time logging. Nothing here reads or prints identity details, names,
//! listing links or emails (the operator types an email to pick a customer; output uses a short
//! id). Looking at one customer's case is recorded in the audit log.
//!
//! The scorecard keeps the funnel stages apart on purpose: a request being submitted says nothing
//! about the listing being gone, and a page we could not read says nothing either.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::removal_request::RemovalRequest;
use crate::services::{audit, exposure, removal};

pub const TIME_KINDS: [&str; 7] = ["intake", "review", "submission", "followup", "verification", "report", "other"];

#[derive(Debug, sqlx::FromRow)]
struct ParticipantRow {
    cohort: String,
    assessed: bool,
    exposures: i64,
    submitted: i64,
    submitted_manual: i64,
    verified: i64,
    conclusively_checked: i64,
    awaiting: i64,
    still_listed: i64,
    inconclusive: i64,
    help_asked: i64,
    minutes: i64,
    interview_ok: bool,
    story_ok: bool,
    manual_needed: i64,
    manual_reported: i64,
    manual_verified: i64,
    self_added: i64,
}

/// One group's funnel, in counts. `participants` is customers; the rest are customers or requests
/// as named.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Funnel {
    pub participants: i64,
    /// Customers with at least one completed scan.
    pub assessed: i64,
    /// Customers for whom a scan detected at least one listing.
    pub with_detected_exposures: i64,
    pub exposures_detected: i64,
    /// Requests that were actually submitted (by us or, by the customer's report, by hand).
    pub submitted: i64,
    pub submitted_by_hand: i64,
    /// Submitted requests that have had at least one check with a clear result (gone, or still there).
    pub conclusively_checked: i64,
    pub verified_removed: i64,
    pub customers_with_verified_removal: i64,
    pub awaiting_result: i64,
    pub still_listed_at_close: i64,
    pub inconclusive_at_close: i64,
    pub help_requests: i64,
    pub minutes: i64,
    /// Participants who separately agreed to a follow-up interview / an anonymized success story.
    pub interview_ok: i64,
    pub story_ok: i64,
    /// The manual path: requests that need (or got) a by-hand opt-out, how many the customer
    /// reported doing, and how many of those our own check then verified.
    pub manual_needed: i64,
    pub manual_reported: i64,
    pub manual_verified: i64,
    /// Listings customers added themselves (counted in the funnel above too).
    pub self_added: i64,
}

fn pct(n: i64, d: i64) -> String {
    if d == 0 { "n/a".into() } else { format!("{:.0}% ({n}/{d})", 100.0 * n as f64 / d as f64) }
}

/// Fold per-customer rows into one funnel per cohort, plus the total under key "all".
fn fold(rows: Vec<ParticipantRow>) -> BTreeMap<String, Funnel> {
    let mut out: BTreeMap<String, Funnel> = BTreeMap::new();
    for r in rows {
        for key in [r.cohort.clone(), "all".to_string()] {
            let f = out.entry(key).or_default();
            f.participants += 1;
            f.assessed += r.assessed as i64;
            f.with_detected_exposures += (r.exposures > 0) as i64;
            f.exposures_detected += r.exposures;
            f.submitted += r.submitted;
            f.submitted_by_hand += r.submitted_manual;
            f.conclusively_checked += r.conclusively_checked;
            f.verified_removed += r.verified;
            f.customers_with_verified_removal += (r.verified > 0) as i64;
            f.awaiting_result += r.awaiting;
            f.still_listed_at_close += r.still_listed;
            f.inconclusive_at_close += r.inconclusive;
            f.help_requests += r.help_asked;
            f.minutes += r.minutes;
            f.interview_ok += r.interview_ok as i64;
            f.story_ok += r.story_ok as i64;
            f.manual_needed += r.manual_needed;
            f.manual_reported += r.manual_reported;
            f.manual_verified += r.manual_verified;
            f.self_added += r.self_added;
        }
    }
    out
}

pub async fn scorecard(pool: &PgPool) -> Result<BTreeMap<String, Funnel>> {
    let rows: Vec<ParticipantRow> = sqlx::query_as(
        r#"
        SELECT c.pilot_cohort AS cohort,
          EXISTS (SELECT 1 FROM scan_jobs s WHERE s.customer_id = c.id AND s.status = 'completed') AS assessed,
          (SELECT count(*) FROM exposures e WHERE e.customer_id = c.id)::int8 AS exposures,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.submitted_at IS NOT NULL)::int8 AS submitted,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.submitted_at IS NOT NULL AND r.procedure_used = 'manual')::int8 AS submitted_manual,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.status = 'verified_removed')::int8 AS verified,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.submitted_at IS NOT NULL
             AND EXISTS (SELECT 1 FROM removal_audit_log a WHERE a.removal_request_id = r.id
                         AND a.action IN ('verified_removed', 'still_present', 'still_present_after_timeout')))::int8 AS conclusively_checked,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.status IN ('submitted', 'acknowledged'))::int8 AS awaiting,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.status = 'rejected')::int8 AS still_listed,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.status = 'unverifiable')::int8 AS inconclusive,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.assistance_status IS NOT NULL)::int8 AS help_asked,
          (COALESCE((SELECT sum(t.minutes) FROM operator_time t WHERE t.customer_id = c.id), 0)
           + COALESCE((SELECT sum(r.assistance_minutes) FROM removal_requests r WHERE r.customer_id = c.id), 0))::int8 AS minutes,
          c.pilot_interview_ok AS interview_ok,
          c.pilot_story_ok AS story_ok,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND (r.status = 'failed' OR r.procedure_used = 'manual'))::int8 AS manual_needed,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.manual_reported_at IS NOT NULL)::int8 AS manual_reported,
          (SELECT count(*) FROM removal_requests r WHERE r.customer_id = c.id AND r.procedure_used = 'manual' AND r.status = 'verified_removed')::int8 AS manual_verified,
          (SELECT count(*) FROM exposures e WHERE e.customer_id = c.id AND e.origin = 'customer')::int8 AS self_added
        FROM customers c
        WHERE c.pilot_cohort IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await
    .context("read pilot participants")?;
    Ok(fold(rows))
}

pub fn render_scorecard(groups: &BTreeMap<String, Funnel>) -> String {
    if groups.is_empty() {
        return "No participants yet. Tag customers with: vectorscrub pilot cohort <email> <a|b|c>\n".into();
    }
    let mut out = String::new();
    // Total first, then each cohort.
    let order = std::iter::once("all").chain(groups.keys().map(String::as_str).filter(|k| *k != "all"));
    for key in order {
        let f = &groups[key];
        let label = if key == "all" { "ALL PARTICIPANTS".to_string() } else { format!("COHORT {}", key.to_uppercase()) };
        out += &format!("== {label} ({} participants) ==\n", f.participants);
        out += &format!("Assessment completed        {}\n", pct(f.assessed, f.participants));
        out += &format!("Customers with detections   {}   ({} listings detected)\n", pct(f.with_detected_exposures, f.participants), f.exposures_detected);
        out += &format!("Requests submitted          {}   ({} of them by hand, as reported by the customer)\n", f.submitted, f.submitted_by_hand);
        out += &format!("Verification coverage       {}  (submitted requests with at least one clear check)\n", pct(f.conclusively_checked, f.submitted));
        out += &format!("Verified removed            {}   ({} of {} customers have one)\n", f.verified_removed, f.customers_with_verified_removal, f.participants);
        out += &format!("  verified rate             {}  (of submitted)\n", pct(f.verified_removed, f.submitted));
        out += &format!("  awaiting a result         {}\n", f.awaiting_result);
        out += &format!("  still listed when closed  {}\n", f.still_listed_at_close);
        out += &format!("  inconclusive when closed  {}\n", f.inconclusive_at_close);
        out += &format!(
            "Manual path                 {} needed -> {} reported -> {} verified   ({} listings added by customers)\n",
            f.manual_needed, f.manual_reported, f.manual_verified, f.self_added
        );
        out += &format!("Help requests               {}\n", f.help_requests);
        let per = if f.participants > 0 { f.minutes as f64 / f.participants as f64 } else { 0.0 };
        out += &format!("Labor logged                {} min total, {per:.0} min per participant\n", f.minutes);
        out += &format!("Agreed to interview / story {} / {} of {}\n\n", f.interview_ok, f.story_ok, f.participants);
    }
    out += "Not measured here (collect in interviews): match accuracy, satisfaction, willingness to pay.\n";
    out += "Submitted is not removed: read the verified rate, not the submission count.\n";
    out
}

pub async fn customer_by_email(pool: &PgPool, email: &str) -> Result<Uuid> {
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM customers WHERE lower(email) = lower($1)")
        .bind(email.trim())
        .fetch_optional(pool)
        .await?;
    id.context("no account with that email")
}

pub async fn set_cohort(pool: &PgPool, customer_id: Uuid, cohort: &str) -> Result<()> {
    if !matches!(cohort, "a" | "b" | "c") {
        bail!("cohort must be a, b or c");
    }
    sqlx::query("UPDATE customers SET pilot_cohort = $2 WHERE id = $1")
        .bind(customer_id)
        .bind(cohort)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn log_time(pool: &PgPool, customer_id: Uuid, kind: &str, minutes: i32) -> Result<()> {
    if !TIME_KINDS.contains(&kind) {
        bail!("kind must be one of: {}", TIME_KINDS.join(", "));
    }
    if !(1..=600).contains(&minutes) {
        bail!("minutes must be between 1 and 600");
    }
    sqlx::query("INSERT INTO operator_time (customer_id, kind, minutes) VALUES ($1, $2, $3)")
        .bind(customer_id)
        .bind(kind)
        .bind(minutes)
        .execute(pool)
        .await?;
    Ok(())
}

/// What a person should do next about one listing, from its state alone.
pub fn next_action(active: bool, req: Option<&RemovalRequest>) -> (&'static str, String) {
    use crate::models::removal_request::RemovalStatus as S;
    let Some(r) = req else {
        return if active {
            ("Detected", "Customer decision: review the listing and authorize removal".into())
        } else {
            ("Removed", "None".into())
        };
    };
    let waiting_on_help = matches!(r.assistance_status.as_deref(), Some("requested" | "in_progress"));
    match r.status {
        S::Authorized => ("Authorized", "Waiting for the agent to submit".into()),
        S::Failed => {
            let blocked = r.failure_reason.as_deref().is_some_and(|f| f.contains("blocked our automated browser") || f.contains("no longer exists"));
            let stage = if blocked { "Manual action required" } else { "Failed" };
            let action = if waiting_on_help {
                "Customer asked for help: contact them and walk them through the opt-out".to_string()
            } else if blocked {
                "Customer must complete the opt-out by hand (offer help), then report it".to_string()
            } else {
                "Read the failure reason; retry or take it manual".to_string()
            };
            (stage, action)
        }
        S::Submitted | S::Acknowledged => {
            let who = if r.procedure_used == "manual" { "Reported submitted by customer" } else { "Submitted" };
            let action = match (&r.last_check_result, r.next_check_at) {
                (Some(res), Some(at)) => format!("Last check: {res}. Next check {}", at.date()),
                (None, Some(at)) => format!("First check {}", at.date()),
                _ => "Check is overdue or unscheduled: investigate".into(),
            };
            (who, action)
        }
        S::VerifiedRemoved => ("Verified removed", "None. Monitor for reappearance".into()),
        S::Rejected => ("Still listed after 45 days", "Decide: try again, go to the broker directly, or tell the customer plainly".into()),
        S::Unverifiable => ("Inconclusive", "Page unreadable by us: consider looking by hand with the customer".into()),
        S::Reappeared => ("Reappeared", "Customer must re-authorize a removal".into()),
    }
}

/// One customer's listings and what to do about each, for a person working the case. No names, no
/// links, no identity details. Reading it is audited.
pub async fn case_view(pool: &PgPool, customer_id: Uuid) -> Result<String> {
    let exposures = exposure::list_exposures(pool, customer_id).await?;
    let requests = removal::list_removal_requests(pool, customer_id).await?; // newest first
    audit::AuditEntry::builder(audit::actions::OPERATOR_CASE_VIEW, audit::resources::CUSTOMER)
        .customer_id(customer_id)
        .resource_id(customer_id)
        .log_or_warn(pool)
        .await;

    let mut out = format!("Case {}  ({} listings)\n", &customer_id.to_string()[..8], exposures.len());
    for e in &exposures {
        let latest = requests.iter().find(|r| r.exposure_id == e.id);
        let (stage, action) = next_action(e.is_active, latest);
        let help = match latest.and_then(|r| r.assistance_status.as_deref()) {
            Some(s) => format!("  [help: {s}]"),
            None => String::new(),
        };
        out += &format!("- {:<24} {:<30}{help}\n    next: {action}\n", e.source_name, stage);
    }
    if exposures.is_empty() {
        out += "No listings detected yet.\n";
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    async fn participant(pool: &PgPool, cohort: &str) -> Uuid {
        let c = ts::customer(pool, "personal", true).await;
        set_cohort(pool, c, cohort).await.unwrap();
        c
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_scorecard_keeps_submitted_checked_and_removed_apart(pool: PgPool) {
        let a = participant(&pool, "a").await;
        let b = participant(&pool, "b").await;
        let _untagged = ts::customer(&pool, "free", false).await;
        ts::scan_job(&pool, a, "completed").await;
        let e1 = ts::exposure(&pool, a, true, None).await;
        let e2 = ts::exposure(&pool, a, true, None).await;
        let e3 = ts::exposure(&pool, a, true, None).await;
        let _undecided = ts::exposure(&pool, a, true, None).await;
        let removed = ts::removal_request(&pool, a, e1, "verified_removed", None).await;
        let pending = ts::removal_request(&pool, a, e2, "submitted", Some(2)).await;
        let _failed = ts::removal_request(&pool, a, e3, "failed", None).await;
        sqlx::query("UPDATE removal_requests SET submitted_at = NOW() WHERE id = ANY($1)").bind(vec![removed, pending]).execute(&pool).await.unwrap();
        // the verified one had a clear check; the pending one only an unreadable page
        for (r, action) in [(removed, "verified_removed"), (pending, "verification_inconclusive")] {
            sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, $3, 'verification_agent', '{}')")
                .bind(r).bind(a).bind(action).execute(&pool).await.unwrap();
        }
        sqlx::query("UPDATE removal_requests SET procedure_used = 'manual' WHERE id = $1").bind(pending).execute(&pool).await.unwrap();
        log_time(&pool, a, "intake", 20).await.unwrap();
        log_time(&pool, a, "followup", 10).await.unwrap();
        sqlx::query("UPDATE removal_requests SET assistance_status = 'completed', assistance_minutes = 15 WHERE id = $1").bind(pending).execute(&pool).await.unwrap();
        let _ = b;

        let sc = scorecard(&pool).await.unwrap();
        assert_eq!(sc.len(), 3, "a, b, all; the untagged customer is not a participant");
        let all = &sc["all"];
        assert_eq!(all.participants, 2);
        assert_eq!((all.assessed, all.with_detected_exposures, all.exposures_detected), (1, 1, 4));
        assert_eq!((all.submitted, all.submitted_by_hand), (2, 1));
        assert_eq!(all.conclusively_checked, 1, "an unreadable page is not a clear check");
        assert_eq!((all.verified_removed, all.customers_with_verified_removal), (1, 1));
        assert_eq!(all.awaiting_result, 1);
        assert_eq!(all.help_requests, 1);
        assert_eq!((all.manual_needed, all.manual_reported, all.manual_verified), (2, 0, 0), "the failed one and the manual-procedure one need it; none reported through the form");
        assert_eq!(all.minutes, 45, "20 + 10 logged, plus 15 spent on the help request");
        assert_eq!(sc["b"].participants, 1);
        assert_eq!(sc["b"].submitted, 0);

        let text = render_scorecard(&sc);
        assert!(text.contains("ALL PARTICIPANTS (2 participants)") && text.contains("COHORT A") && text.contains("COHORT B"));
        assert!(text.contains("verified rate             50% (1/2)"), "{text}");
        assert!(text.contains("Submitted is not removed"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cohort_and_time_input_are_validated(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        assert!(set_cohort(&pool, c, "d").await.is_err());
        assert!(set_cohort(&pool, c, "executive").await.is_err(), "no descriptive labels in the database");
        assert!(log_time(&pool, c, "chatting", 5).await.is_err());
        assert!(log_time(&pool, c, "intake", 0).await.is_err());
        assert!(log_time(&pool, c, "intake", 601).await.is_err());
        log_time(&pool, c, "review", 30).await.unwrap();
        // erasing the customer keeps the cost record but not the link to the person
        crate::services::account_deletion::delete_account(&pool, c).await.unwrap();
        let (n, linked): (i64, i64) = sqlx::query_as("SELECT count(*), count(customer_id) FROM operator_time").fetch_one(&pool).await.unwrap();
        assert_eq!((n, linked), (1, 0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_case_view_says_what_to_do_next_shows_no_personal_data_and_is_audited(pool: PgPool) {
        let c = participant(&pool, "a").await;
        let blocked_e = ts::exposure(&pool, c, true, None).await;
        let blocked = ts::removal_request(&pool, c, blocked_e, "failed", None).await;
        sqlx::query("INSERT INTO removal_audit_log (removal_request_id, customer_id, action, actor, details) VALUES ($1, $2, 'failed', 'removal_executor', '{\"error\":\"The site blocked our automated browser (HTTP 403).\"}')")
            .bind(blocked).bind(c).execute(&pool).await.unwrap();
        sqlx::query("UPDATE removal_requests SET assistance_status = 'requested', assistance_requested_at = NOW() WHERE id = $1").bind(blocked).execute(&pool).await.unwrap();
        let _fresh = ts::exposure(&pool, c, true, None).await;
        let done_e = ts::exposure(&pool, c, false, Some(3)).await;
        ts::removal_request(&pool, c, done_e, "verified_removed", None).await;

        let text = case_view(&pool, c).await.unwrap();
        assert!(text.starts_with(&format!("Case {}", &c.to_string()[..8])), "{text}");
        assert!(text.contains("Manual action required") && text.contains("[help: requested]"), "{text}");
        assert!(text.contains("Customer asked for help: contact them"), "{text}");
        assert!(text.contains("Customer decision: review the listing and authorize removal"), "{text}");
        assert!(text.contains("Verified removed"), "{text}");
        assert!(!text.contains("http") && !text.contains('@'), "no links or emails: {text}");
        let reads: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'operator.case_view' AND customer_id = $1").bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!(reads, 1);
    }
}
