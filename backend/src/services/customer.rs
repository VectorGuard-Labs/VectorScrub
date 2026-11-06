use anyhow::{anyhow, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::crypto::{decrypt_str, derive_customer_key, encrypt_str, MasterKey};
use crate::models::customer::{
    CleartextIdentity, CreateAccountRequest, Customer, IdentityProfile, SubmitIdentityRequest,
};
use crate::services::auth::hash_password;

/// Create a new customer account. Returns the created customer.
pub async fn create_account(
    pool: &PgPool,
    req: &CreateAccountRequest,
) -> Result<Customer> {
    let existing: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM customers WHERE email = $1")
            .bind(&req.email)
            .fetch_optional(pool)
            .await?;

    if existing.is_some() {
        return Err(anyhow!("An account with this email address already exists"));
    }

    let password_hash = hash_password(&req.password)?;
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
    .bind(&req.email)
    .bind(&password_hash)
    .bind(now)
    .fetch_one(pool)
    .await?;

    Ok(customer)
}

/// Fetch a customer by email address.
pub async fn find_by_email(pool: &PgPool, email: &str) -> Result<Option<Customer>> {
    Ok(
        sqlx::query_as::<_, Customer>("SELECT * FROM customers WHERE email = $1 AND is_active = true")
            .bind(email)
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

/// Load and decrypt an identity profile for internal use (e.g., before dispatching a scan job).
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

/// Update a customer's privacy score based on current exposure data.
pub async fn update_privacy_score(pool: &PgPool, customer_id: Uuid, score: f32) -> Result<()> {
    sqlx::query(
        "UPDATE customers SET privacy_score = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(score)
    .bind(OffsetDateTime::now_utc())
    .bind(customer_id)
    .execute(pool)
    .await?;
    Ok(())
}
