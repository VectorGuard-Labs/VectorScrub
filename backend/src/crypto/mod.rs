//! Cryptographic primitives for VectorScrub.
//!
//! All PII is encrypted with AES-256-GCM using per-customer derived keys.
//! The master key is loaded from the environment.
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

/// Derive a per-customer AES-256-GCM key: SHA-256(master_key || label || customer_id).
///
/// This is a keyed hash over fixed-length inputs, not an HMAC or HKDF (earlier
/// comments here and in the docs said HMAC-SHA256; that was inaccurate). For a
/// uniformly random 256-bit master key and fixed-size inputs it yields independent
/// per-customer keys, so a compromise of one customer's data does not expose
/// others. It is NOT changed to HKDF because that would change every derived key
/// and make all stored ciphertext undecryptable; do that only with a data
/// migration.
///
/// Known limit: ciphertext is not bound to its owner or field (no AEAD associated
/// data), so a blob copied between two fields of the same customer would decrypt.
/// Adding AAD changes the stored format and needs the same kind of migration.
pub fn derive_customer_key(master: &MasterKey, customer_id: Uuid) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(master.0);
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

/// Compare two secrets (API keys, tokens) without leaking where they differ or how
/// long the correct one is: both sides are hashed to a fixed length first, then the
/// digests are compared in constant time.
pub fn secrets_equal(a: &[u8], b: &[u8]) -> bool {
    let da = Sha256::digest(a);
    let db = Sha256::digest(b);
    da.iter().zip(db.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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

    fn master(byte: u8) -> MasterKey {
        MasterKey([byte; 32])
    }

    #[test]
    fn round_trip_encryption() {
        let key = derive_customer_key(&master(0xAB), Uuid::new_v4());
        let plaintext = "123 Main Street, Springfield, CA 90210";
        let blob = encrypt_str(&key, plaintext).unwrap();
        assert_eq!(plaintext, decrypt_str(&key, &blob).unwrap());
    }

    #[test]
    fn round_trip_handles_empty_and_unicode() {
        let key = derive_customer_key(&master(1), Uuid::new_v4());
        for text in ["", "María García-Núñez", "田中太郎", "a\nb\tc"] {
            let blob = encrypt_str(&key, text).unwrap();
            assert_eq!(text, decrypt_str(&key, &blob).unwrap());
        }
    }

    #[test]
    fn blob_layout_is_nonce_then_ciphertext_and_tag() {
        let key = derive_customer_key(&master(2), Uuid::new_v4());
        let blob = encrypt(&key, b"hello").unwrap();
        // 12-byte nonce + 5 bytes of ciphertext + 16-byte tag
        assert_eq!(blob.len(), 12 + 5 + 16);
    }

    #[test]
    fn same_plaintext_encrypts_differently_each_time() {
        let key = derive_customer_key(&master(3), Uuid::new_v4());
        let a = encrypt(&key, b"same").unwrap();
        let b = encrypt(&key, b"same").unwrap();
        assert_ne!(a, b, "nonce must be random per encryption");
        assert_ne!(a[..12], b[..12]);
    }

    #[test]
    fn different_customers_get_different_keys() {
        let m = master(0xCD);
        assert_ne!(
            derive_customer_key(&m, Uuid::new_v4()),
            derive_customer_key(&m, Uuid::new_v4())
        );
    }

    #[test]
    fn key_derivation_is_deterministic_and_depends_on_master() {
        let id = Uuid::new_v4();
        assert_eq!(derive_customer_key(&master(7), id), derive_customer_key(&master(7), id));
        assert_ne!(derive_customer_key(&master(7), id), derive_customer_key(&master(8), id));
    }

    #[test]
    fn one_customers_key_cannot_decrypt_anothers_data() {
        let m = master(9);
        let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());
        let blob = encrypt_str(&derive_customer_key(&m, alice), "alice's address").unwrap();
        assert!(decrypt_str(&derive_customer_key(&m, bob), &blob).is_err());
    }

    #[test]
    fn tampering_with_any_part_of_the_blob_is_detected() {
        let key = derive_customer_key(&master(4), Uuid::new_v4());
        let blob = encrypt(&key, b"sensitive").unwrap();
        for i in [0, 11, 12, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            assert!(decrypt(&key, &bad).is_err(), "flip at byte {i} must fail authentication");
        }
    }

    #[test]
    fn truncated_or_empty_blobs_are_rejected() {
        let key = derive_customer_key(&master(5), Uuid::new_v4());
        assert!(decrypt(&key, &[]).is_err());
        assert!(decrypt(&key, &[0u8; 27]).is_err());
        let blob = encrypt(&key, b"data").unwrap();
        assert!(decrypt(&key, &blob[..blob.len() - 1]).is_err());
    }

    #[test]
    fn master_key_must_be_valid_base64_of_exactly_32_bytes() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        assert!(MasterKey::from_base64(&STANDARD.encode([1u8; 32])).is_ok());
        assert!(MasterKey::from_base64(&STANDARD.encode([1u8; 31])).is_err());
        assert!(MasterKey::from_base64(&STANDARD.encode([1u8; 33])).is_err());
        assert!(MasterKey::from_base64("not base64 !!").is_err());
        assert!(MasterKey::from_base64("").is_err());
    }

    #[test]
    fn secrets_equal_compares_correctly() {
        assert!(secrets_equal(b"agent-key-123", b"agent-key-123"));
        assert!(!secrets_equal(b"agent-key-123", b"agent-key-124"));
        assert!(!secrets_equal(b"agent-key-123", b"agent-key-1234"), "different length");
        assert!(!secrets_equal(b"agent-key-123", b""), "empty never matches a real key");
        assert!(secrets_equal(b"", b""));
    }
}
