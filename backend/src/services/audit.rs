use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::net::IpAddr;
use uuid::Uuid;

/// Audit log entry for tracking sensitive operations
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub customer_id: Option<Uuid>,
    pub user_email: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<Uuid>,
    pub ip_address: Option<IpAddr>,
    pub user_agent: Option<String>,
    pub details: JsonValue,
    pub status: AuditStatus,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum AuditStatus {
    Success,
    Failure,
    Error,
}

impl AuditStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditStatus::Success => "success",
            AuditStatus::Failure => "failure",
            AuditStatus::Error => "error",
        }
    }
}

impl AuditEntry {
    /// Create a new audit entry builder
    pub fn builder(action: impl Into<String>, resource_type: impl Into<String>) -> AuditEntryBuilder {
        AuditEntryBuilder {
            customer_id: None,
            user_email: None,
            action: action.into(),
            resource_type: resource_type.into(),
            resource_id: None,
            ip_address: None,
            user_agent: None,
            details: serde_json::json!({}),
            status: AuditStatus::Success,
            error_message: None,
        }
    }
}

/// Builder for audit entries
pub struct AuditEntryBuilder {
    customer_id: Option<Uuid>,
    user_email: Option<String>,
    action: String,
    resource_type: String,
    resource_id: Option<Uuid>,
    ip_address: Option<IpAddr>,
    user_agent: Option<String>,
    details: JsonValue,
    status: AuditStatus,
    error_message: Option<String>,
}

impl AuditEntryBuilder {
    pub fn customer_id(mut self, id: Uuid) -> Self {
        self.customer_id = Some(id);
        self
    }

    pub fn user_email(mut self, email: impl Into<String>) -> Self {
        self.user_email = Some(email.into());
        self
    }

    pub fn resource_id(mut self, id: Uuid) -> Self {
        self.resource_id = Some(id);
        self
    }

    pub fn ip_address(mut self, ip: IpAddr) -> Self {
        self.ip_address = Some(ip);
        self
    }

    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub fn details(mut self, details: JsonValue) -> Self {
        self.details = details;
        self
    }

    pub fn status(mut self, status: AuditStatus) -> Self {
        self.status = status;
        self
    }

    pub fn error_message(mut self, msg: impl Into<String>) -> Self {
        self.error_message = Some(msg.into());
        self
    }

    pub fn build(self) -> AuditEntry {
        AuditEntry {
            customer_id: self.customer_id,
            user_email: self.user_email,
            action: self.action,
            resource_type: self.resource_type,
            resource_id: self.resource_id,
            ip_address: self.ip_address,
            user_agent: self.user_agent,
            details: self.details,
            status: self.status,
            error_message: self.error_message,
        }
    }

    /// Build and log immediately
    pub async fn log(self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let entry = self.build();
        log_audit_entry(pool, &entry).await
    }
}

/// Log an audit entry to the database
pub async fn log_audit_entry(pool: &PgPool, entry: &AuditEntry) -> Result<(), sqlx::Error> {
    // Convert IP address to string for storage
    let ip_str = entry.ip_address.map(|ip| ip.to_string());

    sqlx::query!(
        r#"
        INSERT INTO audit_log (
            customer_id, user_email, action, resource_type, resource_id,
            ip_address, user_agent, details, status, error_message
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
        entry.customer_id,
        entry.user_email.as_deref(),
        entry.action,
        entry.resource_type,
        entry.resource_id,
        ip_str.as_deref(),
        entry.user_agent.as_deref(),
        entry.details,
        entry.status.as_str(),
        entry.error_message.as_deref(),
    )
    .execute(pool)
    .await?;

    // Also log to structured logs for real-time monitoring
    tracing::info!(
        customer_id = ?entry.customer_id,
        action = %entry.action,
        resource_type = %entry.resource_type,
        resource_id = ?entry.resource_id,
        status = %entry.status.as_str(),
        "Audit log entry created"
    );

    Ok(())
}

/// Common audit actions
pub mod actions {
    // Authentication
    pub const USER_REGISTER: &str = "user.register";
    pub const USER_LOGIN: &str = "user.login";
    pub const USER_LOGOUT: &str = "user.logout";

    // Identity management
    pub const IDENTITY_SUBMIT: &str = "identity.submit";
    pub const IDENTITY_UPDATE: &str = "identity.update";

    // Scans
    pub const SCAN_INITIATE: &str = "scan.initiate";
    pub const SCAN_COMPLETE: &str = "scan.complete";
    pub const SCAN_FAILED: &str = "scan.failed";

    // Exposures
    pub const EXPOSURE_DISCOVER: &str = "exposure.discover";
    pub const EXPOSURE_AUTHORIZE_REMOVAL: &str = "exposure.authorize_removal";

    // Removals
    pub const REMOVAL_REQUEST_CREATE: &str = "removal.request_create";
    pub const REMOVAL_REQUEST_UPDATE: &str = "removal.request_update";
    pub const REMOVAL_COMPLETE: &str = "removal.complete";

    // Family
    pub const FAMILY_MEMBER_ADD: &str = "family.member_add";
    pub const FAMILY_MEMBER_REMOVE: &str = "family.member_remove";

    // Notifications
    pub const NOTIFICATION_PREFERENCES_UPDATE: &str = "notification.preferences_update";
    pub const ALERT_READ: &str = "alert.read";
}

/// Common resource types
pub mod resources {
    pub const CUSTOMER: &str = "customer";
    pub const IDENTITY: &str = "identity";
    pub const SCAN: &str = "scan";
    pub const EXPOSURE: &str = "exposure";
    pub const REMOVAL_REQUEST: &str = "removal_request";
    pub const FAMILY_MEMBER: &str = "family_member";
    pub const NOTIFICATION: &str = "notification";
    pub const ALERT: &str = "alert";
}
