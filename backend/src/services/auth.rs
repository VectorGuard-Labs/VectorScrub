use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// Claims embedded in a VectorScrub JWT.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // customer UUID
    pub tier: String,
    pub iat: i64,
    pub exp: i64,
}

/// Hash a password using Argon2id with a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow!("Failed to hash password: {e}"))
}

/// Verify a plaintext password against an Argon2id hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| anyhow!("Invalid hash format: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Issue a signed JWT valid for 24 hours.
pub fn issue_token(
    customer_id: Uuid,
    tier: &str,
    secret: &str,
) -> Result<String> {
    let now = OffsetDateTime::now_utc();
    let claims = Claims {
        sub: customer_id.to_string(),
        tier: tier.to_string(),
        iat: now.unix_timestamp(),
        exp: (now + Duration::hours(24)).unix_timestamp(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| anyhow!("Failed to issue JWT: {e}"))
}

/// Validate and decode a JWT; returns the embedded claims.
pub fn validate_token(token: &str, secret: &str) -> Result<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| anyhow!("Invalid or expired token: {e}"))
}
