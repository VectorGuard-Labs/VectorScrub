//! Cryptographic primitives for VectorScrub.
//!
//! All PII is encrypted with AES-256-GCM using per-customer derived keys.
//! The master key is loaded from the environment (or PyHSM integration).
//! Nonces are randomly generated and prepended to ciphertext.
//!
//! Layout of encrypted blob: [ 12-byte nonce | ciphertext | 16-byte tag ]

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroize;

/// Master encryption key loaded at startup — 32 bytes, base64-encoded in env.
pub struct MasterKey([u8; 32]);

impl MasterKey {
    pub fn from_base64(encoded: &str) -> Result<Self> {
        let bytes = BASE64
            .decode(encoded)
            .context("Invalid base64 for master key")?;
        if bytes.len() != 32 {
            return Err(anyhow!("Master key must be exactly 32 bytes"));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Derive a per-customer AES-256-GCM key by HMAC-SHA256(master_key, customer_id).
/// This ensures a compromise of one customer's data does not expose others.
pub fn derive_customer_key(master: &MasterKey, customer_id: Uuid) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(&master.0);
    hasher.update(b"vectorscrub-v1-customer-key");
    hasher.update(customer_id.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypt plaintext bytes with AES-256-GCM under the given customer key.
/// Returns: base64( nonce || ciphertext+tag )
pub fn encrypt(customer_key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let key = Key::<Aes256Gcm>::from_slice(customer_key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| anyhow!("Encryption failed: {e}"))?;
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a blob produced by `encrypt`.
pub fn decrypt(customer_key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 28 {
        // 12 nonce + 16 tag minimum
        return Err(anyhow!("Ciphertext blob too short"));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let key = Key::<Aes256Gcm>::from_slice(customer_key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow!("Decryption failed: {e}"))
}

/// Convenience: encrypt a UTF-8 string.
pub fn encrypt_str(customer_key: &[u8; 32], plaintext: &str) -> Result<Vec<u8>> {
    encrypt(customer_key, plaintext.as_bytes())
}

/// Convenience: decrypt and return a UTF-8 string.
pub fn decrypt_str(customer_key: &[u8; 32], blob: &[u8]) -> Result<String> {
    let bytes = decrypt(customer_key, blob)?;
    String::from_utf8(bytes).context("Decrypted bytes are not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn round_trip_encryption() {
        let master_key_bytes: [u8; 32] = [0xAB; 32];
        let master = MasterKey(master_key_bytes);
        let customer_id = Uuid::new_v4();
        let key = derive_customer_key(&master, customer_id);

        let plaintext = "123 Main Street, Springfield, CA 90210";
        let blob = encrypt_str(&key, plaintext).unwrap();
        let recovered = decrypt_str(&key, &blob).unwrap();
        assert_eq!(plaintext, recovered);
    }

    #[test]
    fn different_customers_get_different_keys() {
        let master = MasterKey([0xCD; 32]);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        assert_ne!(derive_customer_key(&master, id1), derive_customer_key(&master, id2));
    }
}
