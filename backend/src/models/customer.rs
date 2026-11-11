use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;
use zeroize::Zeroize;

/// Subscription tier for a customer account.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::Type, PartialEq)]
#[sqlx(type_name = "subscription_tier", rename_all = "lowercase")]
pub enum SubscriptionTier {
    Free,
    Personal,
    Premium,
}

/// A VectorScrub customer account.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Customer {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub subscription_tier: SubscriptionTier,
    pub privacy_score: Option<f32>,
    pub is_verified: bool,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_scan_at: Option<OffsetDateTime>,
}

/// Identity profile stored in the database.
/// All PII fields are AES-256-GCM ciphertext blobs — no cleartext.
/// Zeroize not applied here: ciphertext without the key is not sensitive.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IdentityProfile {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub full_name_enc: Vec<u8>,
    pub address_enc: Vec<u8>,
    pub previous_addresses_enc: Vec<u8>,
    pub phone_numbers_enc: Vec<u8>,
    pub email_addresses_enc: Vec<u8>,
    pub date_of_birth_enc: Option<Vec<u8>>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Cleartext identity — NEVER persisted, held in memory only during a scan.
/// All fields are types that implement Zeroize. Dropped and zeroed automatically.
/// customer_id stored as String because Uuid does not implement Zeroize.
#[allow(dead_code)]
#[derive(Debug, Clone, Zeroize)]
#[zeroize(drop)]
pub struct CleartextIdentity {
    pub customer_id: String, // UUID string — Uuid itself doesn't impl Zeroize
    pub full_name: String,
    pub current_address: String,
    pub previous_addresses: Vec<String>,
    pub phone_numbers: Vec<String>,
    pub email_addresses: Vec<String>,
    pub date_of_birth: Option<String>,
}

/// Request body for account creation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateAccountRequest {
    #[validate(email(message = "Must be a valid email address"), length(max = 254))]
    #[schema(example = "user@example.com")]
    pub email: String,
    #[validate(length(min = 12, max = 128, message = "Password must be between 12 and 128 characters"))]
    #[schema(example = "SecurePassword123!")]
    pub password: String,
}

/// Request body for submitting an identity profile.
#[derive(Debug, Deserialize, Validate)]
pub struct SubmitIdentityRequest {
    #[validate(length(min = 2, max = 200, message = "Name must be between 2 and 200 characters"))]
    pub full_name: String,
    #[validate(length(min = 5, message = "Address is required"))]
    pub current_address: String,
    pub previous_addresses: Vec<String>,
    pub phone_numbers: Vec<String>,
    #[validate(length(min = 1, message = "At least one email address is required"))]
    pub email_addresses: Vec<String>,
    pub date_of_birth: Option<String>,
}

/// Public-facing customer profile returned by the API (no PII, no password hash).
#[derive(Debug, Serialize, ToSchema)]
pub struct CustomerProfile {
    pub id: Uuid,
    pub email: String,
    pub subscription_tier: SubscriptionTier,
    pub privacy_score: Option<f32>,
    pub is_verified: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_scan_at: Option<OffsetDateTime>,
}

impl From<Customer> for CustomerProfile {
    fn from(c: Customer) -> Self {
        Self {
            id: c.id,
            email: c.email,
            subscription_tier: c.subscription_tier,
            privacy_score: c.privacy_score,
            is_verified: c.is_verified,
            created_at: c.created_at,
            last_scan_at: c.last_scan_at,
        }
    }
}
