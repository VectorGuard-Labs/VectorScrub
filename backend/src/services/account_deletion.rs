//! Erasing a customer's account.
//!
//! What is removed: the account row and, through ON DELETE CASCADE, everything owned by it
//! (identity profile, scans, exposures, removal requests, alerts, notification preferences,
//! deletion requests, privacy-score history).
//!
//! What is kept, and how it is made non-identifying first:
//!   * `audit_log` and `removal_audit_log` are append-only. Their rows for this customer are
//!     pseudonymized (see migration 0017): customer, email, IP address, user agent and the free
//!     text `details` of removal entries are cleared; what happened, to which kind of thing, and
//!     when, stays.
//!   * `stripe_subscriptions` keeps Stripe's identifiers with the customer link cut. Payment
//!     records are held by Stripe and may have to be kept for accounting.
//!
//! The whole thing is one transaction: either the customer is gone and the trail pseudonymized,
//! or nothing changed.

use sqlx::PgPool;
use uuid::Uuid;

use super::billing::ENTITLED_STATUSES;

#[derive(Debug, thiserror::Error)]
pub enum DeleteError {
    #[error("The account does not exist")]
    NotFound,
    /// A subscription that will keep billing. Deleting the account would not stop the charges.
    #[error("Cancel your subscription first (Billing page), then delete your account.")]
    SubscriptionActive,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Erase the account. `Ok(())` means the customer row is gone.
pub async fn delete_account(pool: &PgPool, customer_id: Uuid) -> Result<(), DeleteError> {
    let mut tx = pool.begin().await?;

    // Locked so a concurrent webhook or request cannot slip a new subscription in mid-erasure.
    let email: Option<String> = sqlx::query_scalar("SELECT email FROM customers WHERE id = $1 FOR UPDATE")
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(email) = email else { return Err(DeleteError::NotFound) };

    // A subscription set to end at the period end will not bill again, so it does not block.
    let still_billing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM stripe_subscriptions \
         WHERE customer_id = $1 AND status = ANY($2) AND NOT cancel_at_period_end)",
    )
    .bind(customer_id)
    .bind(ENTITLED_STATUSES.map(String::from).to_vec())
    .fetch_one(&mut *tx)
    .await?;
    if still_billing {
        return Err(DeleteError::SubscriptionActive);
    }

    sqlx::query(
        "UPDATE removal_audit_log SET customer_id = NULL, removal_request_id = NULL, details = '{}'::jsonb \
         WHERE customer_id = $1",
    )
    .bind(customer_id)
    .execute(&mut *tx)
    .await?;

    // Also rows that only carry the email (failed logins have no customer_id).
    sqlx::query(
        "UPDATE audit_log SET customer_id = NULL, user_email = NULL, ip_address = NULL, user_agent = NULL \
         WHERE customer_id = $1 OR lower(user_email) = lower($2)",
    )
    .bind(customer_id)
    .bind(&email)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM customers WHERE id = $1")
        .bind(customer_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}
