use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::db::AppState;
use crate::middleware::auth_extractor::AuthenticatedCustomer;
use crate::models::alert::Alert;
use crate::models::notification::{NotificationPreferences, UpdateNotificationPreferencesRequest};
use crate::services::notification as notification_svc;

/// GET /api/v1/notifications/preferences
/// Get notification preferences for the authenticated customer.
pub async fn get_preferences(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<NotificationPreferences>, (StatusCode, String)> {
    let prefs = notification_svc::get_preferences(&state.pool, auth.id)
        .await
        .map_err(super::error::internal_error)?;

    Ok(Json(prefs))
}

/// PUT /api/v1/notifications/preferences
/// Update notification preferences for the authenticated customer.
pub async fn update_preferences(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Json(req): Json<UpdateNotificationPreferencesRequest>,
) -> Result<Json<NotificationPreferences>, (StatusCode, String)> {
    let prefs = notification_svc::update_preferences(&state.pool, auth.id, req)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(prefs))
}

/// GET /api/v1/notifications/alerts
/// Get unread alerts for the authenticated customer.
pub async fn get_alerts(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
) -> Result<Json<Vec<Alert>>, (StatusCode, String)> {
    let alerts = notification_svc::get_unread_alerts(&state.pool, auth.id, 50)
        .await
        .map_err(super::error::internal_error)?;

    Ok(Json(alerts))
}

/// POST /api/v1/notifications/alerts/:id/read
/// Mark an alert as read.
pub async fn mark_read(
    auth: AuthenticatedCustomer,
    State(state): State<AppState>,
    Path(alert_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    notification_svc::mark_alert_read(&state.pool, alert_id, auth.id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
