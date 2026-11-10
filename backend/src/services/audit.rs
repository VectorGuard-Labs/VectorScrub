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
}

impl AuditStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditStatus::Success => "success",
            AuditStatus::Failure => "failure",
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

/// Audit actions that are actually emitted. Add a constant only together with the code
/// that logs it; an action listed here that nothing writes is a false promise to whoever
/// reads the audit trail.
pub mod actions {
    // Authentication
    pub const USER_REGISTER: &str = "user.register";
    pub const USER_LOGIN: &str = "user.login";
    /// An email reached the failed-login threshold and was locked out.
    pub const USER_LOGIN_LOCKED: &str = "user.login_locked";

    // Identity
    pub const IDENTITY_SUBMIT: &str = "identity.submit";
    /// Decrypted identity was released to the agent layer (success), or a release was
    /// refused (failure). See services/identity_access.rs.
    pub const IDENTITY_RELEASE: &str = "identity.release";

    // Scans
    pub const SCAN_INITIATE: &str = "scan.initiate";
    /// The recovery loop requeued a scan whose worker went silent.
    pub const SCAN_RECOVERED: &str = "scan.recovered";
    /// A scan was failed because it kept getting lost.
    pub const SCAN_FAILED: &str = "scan.failed";

    // Removals
    pub const EXPOSURE_AUTHORIZE_REMOVAL: &str = "exposure.authorize_removal";
}

/// Resource types
pub mod resources {
    pub const CUSTOMER: &str = "customer";
    pub const IDENTITY: &str = "identity";
    pub const SCAN: &str = "scan";
    pub const EXPOSURE: &str = "exposure";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    async fn log_one(pool: &PgPool, customer: Uuid) -> Uuid {
        AuditEntry::builder(actions::USER_LOGIN, resources::CUSTOMER)
            .customer_id(customer)
            .user_email("jane@example.com")
            .resource_id(customer)
            .details(serde_json::json!({ "k": "v" }))
            .log(pool)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT id FROM audit_log WHERE customer_id = $1 ORDER BY timestamp DESC LIMIT 1")
            .bind(customer)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn rejected(pool: &PgPool, sql: &str, id: Uuid) -> bool {
        match sqlx::query(sql).bind(id).execute(pool).await {
            Err(e) => e.to_string().contains("append-only"),
            Ok(_) => false,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn entries_can_be_appended_and_read(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let id = log_one(&pool, c).await;
        let (action, status): (String, String) =
            sqlx::query_as("SELECT action, status FROM audit_log WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!((action.as_str(), status.as_str()), ("user.login", "success"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn records_cannot_be_modified_deleted_or_truncated(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let id = log_one(&pool, c).await;

        for edit in [
            "UPDATE audit_log SET action = 'user.nothing' WHERE id = $1",
            "UPDATE audit_log SET status = 'success', details = '{}'::jsonb WHERE id = $1",
            "UPDATE audit_log SET resource_id = NULL WHERE id = $1",
            "UPDATE audit_log SET timestamp = NOW() - interval '1 year' WHERE id = $1",
            "UPDATE audit_log SET error_message = 'x' WHERE id = $1",
            "UPDATE audit_log SET ip_address = '1.2.3.4' WHERE id = $1",
        ] {
            assert!(rejected(&pool, edit, id).await, "must be rejected: {edit}");
        }
        assert!(rejected(&pool, "DELETE FROM audit_log WHERE id = $1", id).await, "DELETE rejected");
        assert!(
            sqlx::query("TRUNCATE audit_log").execute(&pool).await.unwrap_err().to_string().contains("append-only"),
            "TRUNCATE rejected"
        );

        // the record is exactly as written
        let (action, details): (String, serde_json::Value) =
            sqlx::query_as("SELECT action, details FROM audit_log WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(action, "user.login");
        assert_eq!(details, serde_json::json!({ "k": "v" }));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_only_permitted_edit_is_clearing_identifiers(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let id = log_one(&pool, c).await;

        // pseudonymization is allowed...
        sqlx::query("UPDATE audit_log SET customer_id = NULL, user_email = NULL WHERE id = $1")
            .bind(id).execute(&pool).await.expect("clearing identifiers is allowed");
        let (cust, email): (Option<Uuid>, Option<String>) =
            sqlx::query_as("SELECT customer_id, user_email FROM audit_log WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!((cust, email), (None, None));

        // ...but only that: it cannot be combined with changing anything else, nor used to re-attribute
        assert!(rejected(&pool, "UPDATE audit_log SET customer_id = NULL, action = 'forged' WHERE id = $1", id).await);
        let other = ts::customer(&pool, "free", false).await;
        let id2 = log_one(&pool, c).await;
        let reassign = sqlx::query("UPDATE audit_log SET customer_id = $2 WHERE id = $1").bind(id2).bind(other).execute(&pool).await;
        assert!(reassign.is_err(), "cannot re-attribute an entry to a different customer");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasing_a_customer_pseudonymizes_their_audit_entries_instead_of_blocking(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let id = log_one(&pool, c).await;
        sqlx::query("DELETE FROM customers WHERE id = $1").bind(c).execute(&pool).await
            .expect("ON DELETE SET NULL must still be allowed to pseudonymize");
        let (cust, action): (Option<Uuid>, String) =
            sqlx::query_as("SELECT customer_id, action FROM audit_log WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(cust, None);
        assert_eq!(action, "user.login", "the record itself survives");
    }
}
