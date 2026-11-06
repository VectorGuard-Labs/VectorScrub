use axum::{
    routing::{get, post, put},
    Router,
};

use crate::db::AppState;

mod auth;
mod customers;
mod exposures;
mod removals;
mod scans;
mod agent_callbacks;

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
        // Removal requests
        .route("/removal-requests", get(removals::list_requests))
        .route("/removal-requests/:exposure_id/authorize", post(removals::authorize))
        .route("/removal-requests/:id", get(removals::get_request))
        // Internal: agent callbacks (require agent API key, not customer JWT)
        .route("/internal/scan/status", put(agent_callbacks::update_scan_status))
        .route("/internal/exposures", post(agent_callbacks::record_exposure))
        .route("/internal/removal-requests", get(removals::list_pending))
        .route("/internal/removal-requests/:id/status", put(removals::update_status))
        .route("/internal/removal-requests/:id/audit", post(removals::record_audit))
        .with_state(state)
}
