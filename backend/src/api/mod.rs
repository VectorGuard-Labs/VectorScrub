use axum::{
    routing::{get, post, put},
    Router,
};

use crate::db::AppState;
use crate::middleware::rate_limiter;

mod agent_callbacks;
pub mod auth;
pub mod billing;
pub mod client_ip;
mod csrf;
mod customers;
pub mod deletion;
pub mod error;
mod exposures;
mod health;
mod metrics;
mod notifications;
mod privacy_score;
pub mod public;
mod removals;
mod scans;
#[cfg(test)]
mod tests;

pub fn router(state: AppState) -> Router {
    let cfg = state.config.clone();

    // Login and register: strict per-IP limit (credential stuffing, Argon2 CPU burn).
    let auth_routes = rate_limiter::limit_auth(
        Router::new()
            .route("/auth/register", post(auth::register))
            .route("/auth/login", post(auth::login)),
        &cfg,
    );

    // Customer-facing API: general per-IP limit.
    let customer_routes = rate_limiter::limit_general(
        Router::new()
            // Metrics (Prometheus)
            .route("/metrics", get(metrics::get_metrics))
            // CSRF token
            .route("/csrf-token", get(csrf::get_csrf_token))
            // Ends the browser session (clears the HttpOnly cookie). Under /auth/, which the
            // CSRF check exempts: signing someone out is harmless.
            .route("/auth/logout", post(auth::logout))
            // Customer profile & identity
            .route("/me", get(customers::get_profile).delete(auth::delete_account))
            .route("/me/identity", post(customers::submit_identity))
            // The state of the saved address (two letters or null); never the address itself
            .route("/me/residence", get(customers::get_residence))
            .route("/me/listings", post(customers::add_listing))
            // Which brokers recent scans could read (public: shown on the pricing page)
            .route("/public/coverage", get(public::coverage))
            .route("/public/optout-health", get(public::optout_health))
            // Deletion requests sent to companies on the customer's behalf (off unless enabled)
            .route("/deletion-requests/info", get(deletion::info))
            .route("/deletion-requests", get(deletion::list))
            .route("/deletion-campaigns", post(deletion::create_campaign))
            .route("/deletion-requests/:id/outcome", post(deletion::outcome))
            // Billing: plans on offer and current subscription
            .route("/me/billing", get(billing::get_billing))
            // Stripe calls this; authenticated by its signature, not a session (see handler)
            .route("/webhooks/stripe", post(billing::stripe_webhook))
            // Scans
            .route("/scans", post(scans::initiate))
            .route("/scans", get(scans::list))
            .route("/scans/:job_id", get(scans::get_job))
            // Exposures
            .route("/exposures", get(exposures::list))
            .route("/exposures/summary", get(exposures::summary))
            // Privacy score history
            .route("/privacy-score/history", get(privacy_score::get_history))
            // Notifications and alerts
            .route("/notifications/preferences", get(notifications::get_preferences))
            .route("/notifications/preferences", put(notifications::update_preferences))
            .route("/notifications/alerts", get(notifications::get_alerts))
            .route("/notifications/alerts/:id/read", post(notifications::mark_read))
            // Removal requests
            .route("/removal-requests", get(removals::list_requests))
            .route("/removal-requests/:exposure_id/authorize", post(removals::authorize))
            .route("/removal-requests/:id/done-by-hand", post(removals::done_by_hand))
            .route("/removal-requests/:id/request-assistance", post(removals::request_assistance))
            .route("/removal-requests/:id/listing-link", post(removals::update_listing_link))
            .route("/removal-requests/:id", get(removals::get_request)),
        &cfg,
    );

    // Probes: unauthenticated and not rate limited.
    let probes = Router::new()
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready));

    // Internal: agent callbacks. Authenticated by the agent key (not a customer JWT) and
    // not subject to the per-IP limit, since every agent shares one address.
    //
    // Identity is released only against a concrete, currently-actionable work item (see
    // services/identity_access.rs); there is deliberately no "identity by customer id" route.
    let internal = Router::new()
        // scans: claim -> progress/heartbeat -> complete|fail
        .route("/internal/scan/:job_id/claim", post(agent_callbacks::claim_scan))
        .route("/internal/scan/status", put(agent_callbacks::update_scan_status))
        .route("/internal/identity", post(agent_callbacks::scan_identity))
        .route("/internal/exposures", post(agent_callbacks::record_exposure))
        // deletion requests: queue -> claim -> scoped identity -> result
        .route("/internal/deletion-requests", get(deletion::queued))
        .route("/internal/deletion-requests/:id/claim", post(deletion::claim))
        .route("/internal/deletion-requests/:id/identity", post(deletion::identity))
        .route("/internal/deletion-requests/:id/result", put(deletion::result))
        // nightly check that brokers' public opt-out pages still load (no customer data)
        .route("/internal/optout-health", put(agent_callbacks::report_optout_health))
        // removal submission and verification
        .route("/internal/removal-requests", get(removals::list_pending))
        .route("/internal/removal-requests/:id/identity", post(agent_callbacks::removal_identity))
        .route("/internal/removal-requests/:id/claim", post(removals::claim_submission))
        .route("/internal/removal-requests/:id/status", put(removals::update_status))
        .route("/internal/removal-requests/:id/audit", post(removals::record_audit))
        .route(
            "/internal/removal-requests/due-for-verification",
            get(agent_callbacks::removals_due_for_verification),
        )
        .route(
            "/internal/removal-requests/:id/verification-result",
            post(agent_callbacks::record_verification_result),
        )
        // reappearance monitoring
        .route(
            "/internal/exposures/due-for-monitoring",
            get(agent_callbacks::exposures_due_for_monitoring),
        )
        .route("/internal/exposures/:id/identity", post(agent_callbacks::monitoring_identity))
        .route(
            "/internal/exposures/:id/monitoring-result",
            post(agent_callbacks::record_monitoring_result),
        );

    Router::new()
        .merge(auth_routes)
        .merge(customer_routes)
        .merge(probes)
        .merge(internal)
        .with_state(state)
}
