use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::db::AppState;

mod agent_callbacks;
mod auth;
mod customers;
mod exposures;
mod family;
mod notifications;
mod privacy_score;
mod removals;
mod scans;

pub fn router(state: AppState) -> Router {
    Router::new()
        // Auth
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        // Customer profile & identity
        .route("/me", get(customers::get_profile))
        .route("/me/identity", post(customers::submit_identity))
        // Scans
        .route("/scans", post(scans::initiate))
        .route("/scans", get(scans::list))
        .route("/scans/:job_id", get(scans::get_job))
        // Exposures
        .route("/exposures", get(exposures::list))
        .route("/exposures/summary", get(exposures::summary))
        // Privacy score history
        .route("/privacy-score/history", get(privacy_score::get_history))
        // Family account management
        .route("/family/members", post(family::add_member))
        .route("/family/members", get(family::list_members))
        .route("/family/members/:member_id", delete(family::remove_member))
        .route("/family/stats", get(family::get_stats))
        // Notifications and alerts
        .route("/notifications/preferences", get(notifications::get_preferences))
        .route("/notifications/preferences", put(notifications::update_preferences))
        .route("/notifications/alerts", get(notifications::get_alerts))
        .route("/notifications/alerts/:id/read", post(notifications::mark_read))
        // Removal requests
        .route("/removal-requests", get(removals::list_requests))
        .route("/removal-requests/:exposure_id/authorize", post(removals::authorize))
        .route("/removal-requests/:id", get(removals::get_request))
        // Internal: agent callbacks (require agent API key, not customer JWT)
        .route("/internal/identity", post(agent_callbacks::get_identity))
        .route("/internal/customers/:customer_id/identity", get(agent_callbacks::get_identity_by_customer_id))
        .route("/internal/scan/status", put(agent_callbacks::update_scan_status))
        .route("/internal/exposures", post(agent_callbacks::record_exposure))
        .route("/internal/removal-requests", get(removals::list_pending))
        .route("/internal/removal-requests/:id/status", put(removals::update_status))
        .route("/internal/removal-requests/:id/audit", post(removals::record_audit))
        .with_state(state)
}
