use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// Customer notification preferences for alerts.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct NotificationPreferences {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub email_enabled: bool,
    pub sms_enabled: bool,
    pub phone_number: Option<String>,
    pub alert_new_exposures: bool,
    pub alert_removal_failures: bool,
    pub alert_reappearances: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Request to update notification preferences.
#[derive(Debug, Deserialize)]
pub struct UpdateNotificationPreferencesRequest {
    pub email_enabled: Option<bool>,
    pub sms_enabled: Option<bool>,
    pub phone_number: Option<String>,
    pub alert_new_exposures: Option<bool>,
    pub alert_removal_failures: Option<bool>,
    pub alert_reappearances: Option<bool>,
}
