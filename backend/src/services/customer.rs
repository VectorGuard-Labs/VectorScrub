use anyhow::{anyhow, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::crypto::{decrypt_str, derive_customer_key, encrypt_str, MasterKey};
use crate::models::customer::{
    CleartextIdentity, CreateAccountRequest, Customer, IdentityProfile, SubmitIdentityRequest,
};
use crate::services::auth::hash_password_async;

/// Returned by `create_account` when the email is already registered. The API maps this
/// (and only this) to 409; any other failure is an internal error.
pub const EMAIL_TAKEN: &str = "An account with this email address already exists";

/// Email addresses are compared case-insensitively everywhere: `Jane@x.com` and `jane@x.com`
/// are one account, as they are for every mail system and for the login throttle (which keys
/// on the lowercased address). The comparison is `lower(email) = lower($1)` on both sides so
/// Postgres applies the same folding to the stored and the supplied value, and a unique index
/// on `lower(email)` (migration 0010) makes that the rule the database enforces. The address
/// is stored as the customer typed it (trimmed), so what we show and mail back is theirs.
fn clean_email(email: &str) -> &str {
    email.trim()
}

/// Create a new customer account. Returns the created customer.
pub async fn create_account(
    pool: &PgPool,
    req: &CreateAccountRequest,
) -> Result<Customer> {
    let email = clean_email(&req.email);
    let existing: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM customers WHERE lower(email) = lower($1)")
            .bind(email)
            .fetch_optional(pool)
            .await?;

    if existing.is_some() {
        return Err(anyhow!(EMAIL_TAKEN));
    }

    // Off the async runtime: Argon2 is deliberately slow and would otherwise pin a worker.
    let password_hash = hash_password_async(req.password.clone()).await?;
    let id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let customer = sqlx::query_as::<_, Customer>(
        r#"
        INSERT INTO customers (id, email, password_hash, subscription_tier,
                               is_verified, is_active, created_at, updated_at)
        VALUES ($1, $2, $3, 'free', false, true, $4, $4)
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(email)
    .bind(&password_hash)
    .bind(now)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        // Two simultaneous registrations can both pass the check above; the unique
        // constraint is the real guard, and losing that race is a conflict, not a crash.
        sqlx::Error::Database(db) if db.is_unique_violation() => anyhow!(EMAIL_TAKEN),
        _ => anyhow::Error::from(e),
    })?;

    Ok(customer)
}

/// Fetch a customer by email address, ignoring case (see `clean_email`).
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<Customer>> {
    Ok(
        sqlx::query_as::<_, Customer>(
            "SELECT * FROM customers WHERE lower(email) = lower($1) AND is_active = true",
        )
            .bind(clean_email(email))
            .fetch_optional(pool)
            .await?,
    )
}

/// Fetch a customer by ID.
pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Customer>> {
    Ok(
        sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE id = $1 AND is_active = true")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Persist an encrypted identity profile for a customer.
/// Raw PII is zeroized after encryption.
pub async fn save_identity(
    pool: &PgPool,
    master: &MasterKey,
    req: &SubmitIdentityRequest,
    customer_id: Uuid,
) -> Result<IdentityProfile> {
    let key = derive_customer_key(master, customer_id);

    let full_name_enc = encrypt_str(&key, &req.full_name)?;
    let address_enc = encrypt_str(&key, &req.current_address)?;
    let prev_json = serde_json::to_string(&req.previous_addresses)?;
    let previous_addresses_enc = encrypt_str(&key, &prev_json)?;
    let phones_json = serde_json::to_string(&req.phone_numbers)?;
    let phone_numbers_enc = encrypt_str(&key, &phones_json)?;
    let emails_json = serde_json::to_string(&req.email_addresses)?;
    let email_addresses_enc = encrypt_str(&key, &emails_json)?;
    let dob_enc = req
        .date_of_birth
        .as_deref()
        .map(|d| encrypt_str(&key, d))
        .transpose()?;

    let id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();

    let profile = sqlx::query_as::<_, IdentityProfile>(
        r#"
        INSERT INTO identity_profiles
            (id, customer_id, full_name_enc, address_enc, previous_addresses_enc,
             phone_numbers_enc, email_addresses_enc, date_of_birth_enc, created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)
        ON CONFLICT (customer_id) DO UPDATE SET
            full_name_enc = EXCLUDED.full_name_enc,
            address_enc = EXCLUDED.address_enc,
            previous_addresses_enc = EXCLUDED.previous_addresses_enc,
            phone_numbers_enc = EXCLUDED.phone_numbers_enc,
            email_addresses_enc = EXCLUDED.email_addresses_enc,
            date_of_birth_enc = EXCLUDED.date_of_birth_enc,
            updated_at = EXCLUDED.updated_at
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(customer_id)
    .bind(&full_name_enc)
    .bind(&address_enc)
    .bind(&previous_addresses_enc)
    .bind(&phone_numbers_enc)
    .bind(&email_addresses_enc)
    .bind(&dob_enc)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(profile)
}

/// Whether the customer has submitted an identity profile.
pub async fn has_identity(pool: &PgPool, customer_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM identity_profiles WHERE customer_id = $1)")
        .bind(customer_id)
        .fetch_one(pool)
        .await?)
}

/// Load and decrypt an identity profile. Only services/identity_access.rs should call
/// this on behalf of the agent layer: it enforces scoping and writes the audit record.
/// The returned CleartextIdentity is zeroized on drop.
pub async fn load_cleartext_identity(
    pool: &PgPool,
    master: &MasterKey,
    customer_id: Uuid,
) -> Result<CleartextIdentity> {
    let profile: Option<IdentityProfile> = sqlx::query_as(
        "SELECT * FROM identity_profiles WHERE customer_id = $1",
    )
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;

    let profile = profile.ok_or_else(|| anyhow!("No identity profile found for customer"))?;
    let key = derive_customer_key(master, customer_id);

    Ok(CleartextIdentity {
        customer_id: customer_id.to_string(),
        full_name: decrypt_str(&key, &profile.full_name_enc)?,
        current_address: decrypt_str(&key, &profile.address_enc)?,
        previous_addresses: serde_json::from_str(&decrypt_str(
            &key,
            &profile.previous_addresses_enc,
        )?)?,
        phone_numbers: serde_json::from_str(&decrypt_str(&key, &profile.phone_numbers_enc)?)?,
        email_addresses: serde_json::from_str(&decrypt_str(&key, &profile.email_addresses_enc)?)?,
        date_of_birth: profile
            .date_of_birth_enc
            .as_deref()
            .map(|b| decrypt_str(&key, b))
            .transpose()?,
    })
}
