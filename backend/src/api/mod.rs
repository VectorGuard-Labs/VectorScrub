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
pub mod error;
mod exposures;
mod health;
mod metrics;
mod notifications;
mod privacy_score;
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
            .route("/me", get(customers::get_profile))
            .route("/me/identity", post(customers::submit_identity))
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
