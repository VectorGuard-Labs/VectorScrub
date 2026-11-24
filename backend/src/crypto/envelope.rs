//! Envelope encryption, format 2.
//!
//! Every customer has one random 256-bit data key (DEK). The DEK is never stored in the clear: it
//! sits in the database wrapped by a key-encryption key (KEK) that lives in the key service (see
//! `services::keys`), so the backend process does not hold a master key at all. What this module
//! does is everything on the customer side of that line:
//!
//!   * a separate key for every kind of field, derived from the DEK with HKDF-SHA256, so a
//!     weakness in how one field is used cannot touch another, and a blob cannot be decrypted as
//!     a different field's even by mistake;
//!   * AES-256-GCM with associated data (AAD) naming the customer and the field, so a ciphertext
//!     copied to another row (another customer's) or another column fails authentication instead
//!     of decrypting.
//!
//! Blob layout: `0x56 0x53 0x02` ("VS", format 2) | 12-byte random nonce | ciphertext | 16-byte tag.
//! The old format 1 (key = SHA-256 of master key, label and customer id, no AAD, no prefix) is
//! read only by the migration tool; `is_v2` tells the two apart.

use aes_gcm::{
    aead::{Aead, Payload},
};
use anyhow::{anyhow, Context, Result};
use hkdf::Hkdf;
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroizing;

pub const MAGIC: [u8; 3] = [0x56, 0x53, 0x02];
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const AAD_PREFIX: &[u8] = b"vectorscrub/v2\0";
const KDF_INFO_PREFIX: &str = "vectorscrub/v2/field-key/";

/// Every kind of field that is encrypted, by name. The name is part of both the key derivation and
/// the AAD, so it must never be changed or reused for something else once data exists under it.
pub mod purpose {
    pub const IDENTITY_FULL_NAME: &str = "identity.full_name";
    pub const IDENTITY_ADDRESS: &str = "identity.address";
    pub const IDENTITY_PREVIOUS_ADDRESSES: &str = "identity.previous_addresses";
    pub const IDENTITY_PHONE_NUMBERS: &str = "identity.phone_numbers";
    pub const IDENTITY_EMAIL_ADDRESSES: &str = "identity.email_addresses";
    pub const IDENTITY_DATE_OF_BIRTH: &str = "identity.date_of_birth";
    pub const MFA_SECRET: &str = "mfa.totp_secret";
    pub const LISTING_LINK: &str = "listing.source_url";
    pub const SCAN_TOKEN: &str = "token.scan";
    /// The signed authorization of a deletion-request campaign (names the customer and their email).
    pub const DELETION_SIGNED_AUTHORIZATION: &str = "deletion.signed_authorization";
}

/// Does this blob carry the format-2 prefix?
pub fn is_v2(blob: &[u8]) -> bool {
    blob.len() >= MAGIC.len() + NONCE_LEN + TAG_LEN && blob[..MAGIC.len()] == MAGIC
}

/// One customer's data key, in memory only, zeroed when dropped.
pub struct CustomerKeys {
    customer_id: Uuid,
    dek: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for CustomerKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material, not even by accident in a log line.
        f.debug_struct("CustomerKeys").field("customer_id", &self.customer_id).finish_non_exhaustive()
    }
}

impl CustomerKeys {
    pub fn new(customer_id: Uuid, dek: [u8; 32]) -> Self {
        Self { customer_id, dek: Zeroizing::new(dek) }
    }

    pub fn customer_id(&self) -> Uuid {
        self.customer_id
    }

    /// A fresh random data key.
    pub fn generate_dek() -> Zeroizing<[u8; 32]> {
        use rand::RngCore;
        let mut dek = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(&mut *dek);
        dek
    }

    /// The raw data key. Tests only: nothing outside this module needs to see it.
    #[cfg(test)]
    pub(crate) fn dek_bytes(&self) -> &[u8; 32] {
        &self.dek
    }

    fn field_key(&self, purpose: &str) -> Zeroizing<[u8; 32]> {
        let hk = Hkdf::<Sha256>::new(Some(self.customer_id.as_bytes()), &*self.dek);
        let mut key = Zeroizing::new([0u8; 32]);
        hk.expand(format!("{KDF_INFO_PREFIX}{purpose}").as_bytes(), &mut *key)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        key
    }

    fn aad(&self, purpose: &str) -> Vec<u8> {
        let mut aad = Vec::with_capacity(AAD_PREFIX.len() + 16 + purpose.len());
        aad.extend_from_slice(AAD_PREFIX);
        aad.extend_from_slice(self.customer_id.as_bytes());
        aad.extend_from_slice(purpose.as_bytes());
        aad
    }

    /// Encrypt `plaintext` as the field `purpose` of this customer.
    pub fn seal(&self, purpose: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let key = self.field_key(purpose);
        let cipher = super::cipher_for(&*key);
        let nonce = super::random_nonce();
        let aad = self.aad(purpose);
        let ct = cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: &aad })
            .map_err(|e| anyhow!("Encryption failed: {e}"))?;
        let mut blob = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
        blob.extend_from_slice(&MAGIC);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        Ok(blob)
    }

    /// Decrypt a blob made by `seal` for this customer and this `purpose`. A blob for any other
    /// customer or field, a damaged one, and an old-format one all fail the same way.
    pub fn open(&self, purpose: &str, blob: &[u8]) -> Result<Vec<u8>> {
        if !is_v2(blob) {
            return Err(anyhow!("Ciphertext is not in the current format"));
        }
        let (nonce, ct) = blob[MAGIC.len()..].split_at(NONCE_LEN);
        let key = self.field_key(purpose);
        let cipher = super::cipher_for(&*key);
        let aad = self.aad(purpose);
        cipher
            .decrypt(&super::nonce_from(nonce), Payload { msg: ct, aad: &aad })
            .map_err(|_| anyhow!("Decryption failed"))
    }

    pub fn seal_str(&self, purpose: &str, plaintext: &str) -> Result<Vec<u8>> {
        self.seal(purpose, plaintext.as_bytes())
    }

    pub fn open_str(&self, purpose: &str, blob: &[u8]) -> Result<String> {
        String::from_utf8(self.open(purpose, blob)?).context("Decrypted bytes are not valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(customer: Uuid, byte: u8) -> CustomerKeys {
        CustomerKeys::new(customer, [byte; 32])
    }

    #[test]
    fn a_sealed_field_opens_with_the_same_customer_and_purpose() {
        let k = keys(Uuid::new_v4(), 1);
        let blob = k.seal_str(purpose::IDENTITY_FULL_NAME, "Jane Q. Smith").unwrap();
        assert!(is_v2(&blob));
        assert_eq!(k.open_str(purpose::IDENTITY_FULL_NAME, &blob).unwrap(), "Jane Q. Smith");
        assert_eq!(k.open(purpose::IDENTITY_FULL_NAME, &k.seal(purpose::IDENTITY_FULL_NAME, b"").unwrap()).unwrap(), b"");
    }

    #[test]
    fn the_blob_has_the_documented_layout_and_hides_the_plaintext() {
        let k = keys(Uuid::new_v4(), 2);
        let blob = k.seal(purpose::MFA_SECRET, b"top secret value").unwrap();
        assert_eq!(&blob[..3], &[0x56, 0x53, 0x02]);
        assert_eq!(blob.len(), 3 + 12 + b"top secret value".len() + 16);
        assert!(!blob.windows(6).any(|w| w == b"secret"));
    }

    #[test]
    fn a_field_moved_to_another_column_does_not_open() {
        let k = keys(Uuid::new_v4(), 3);
        let blob = k.seal_str(purpose::IDENTITY_FULL_NAME, "Jane").unwrap();
        for other in [purpose::IDENTITY_ADDRESS, purpose::MFA_SECRET, purpose::LISTING_LINK, purpose::SCAN_TOKEN] {
            assert!(k.open_str(other, &blob).is_err(), "{other}");
        }
    }

    #[test]
    fn another_customers_key_does_not_open_it_even_with_the_same_data_key() {
        // Same DEK bytes, different customer id: the HKDF salt and the AAD both differ.
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let blob = keys(a, 4).seal_str(purpose::IDENTITY_ADDRESS, "1 Main St").unwrap();
        assert!(keys(b, 4).open_str(purpose::IDENTITY_ADDRESS, &blob).is_err());
        assert!(keys(a, 5).open_str(purpose::IDENTITY_ADDRESS, &blob).is_err(), "different data key");
        assert!(keys(a, 4).open_str(purpose::IDENTITY_ADDRESS, &blob).is_ok());
    }

    #[test]
    fn any_change_to_the_blob_is_detected() {
        let k = keys(Uuid::new_v4(), 6);
        let blob = k.seal_str(purpose::LISTING_LINK, "https://example.com/p").unwrap();
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            assert!(k.open_str(purpose::LISTING_LINK, &bad).is_err(), "byte {i}");
        }
        assert!(k.open_str(purpose::LISTING_LINK, &blob[..blob.len() - 1]).is_err(), "truncated");
        assert!(k.open_str(purpose::LISTING_LINK, &[]).is_err());
        assert!(k.open_str(purpose::LISTING_LINK, &blob[..3]).is_err());
    }

    #[test]
    fn the_same_plaintext_never_produces_the_same_blob() {
        let k = keys(Uuid::new_v4(), 7);
        let blobs: std::collections::HashSet<_> =
            (0..200).map(|_| k.seal(purpose::SCAN_TOKEN, b"same").unwrap()).collect();
        assert_eq!(blobs.len(), 200);
    }

    #[test]
    fn old_format_blobs_are_refused_here_and_recognised_as_old() {
        let master = crate::crypto::MasterKey::from_base64(&crate::test_support::master_key_b64()).unwrap();
        let customer = Uuid::new_v4();
        let legacy_key = crate::crypto::derive_customer_key(&master, customer);
        let old = crate::crypto::encrypt_str(&legacy_key, "old data").unwrap();
        assert!(!is_v2(&old));
        assert!(keys(customer, 8).open_str(purpose::IDENTITY_FULL_NAME, &old).is_err());
    }

    /// Frozen against an independent implementation (Python `cryptography` HKDF), so the
    /// derivation cannot drift silently: a change would make every stored field unreadable.
    #[test]
    fn the_field_key_derivation_matches_an_independent_hkdf() {
        let customer: Uuid = "11111111-2222-3333-4444-555555555555".parse().unwrap();
        let mut dek = [0u8; 32];
        for (i, b) in dek.iter_mut().enumerate() {
            *b = i as u8;
        }
        let k = CustomerKeys::new(customer, dek);
        assert_eq!(
            hex::encode(*k.field_key(purpose::IDENTITY_FULL_NAME)),
            "f72b2253a284fc7012f435dd2856db0ea1bd684884dde91b22a115a66f1018ff"
        );
        assert_ne!(k.field_key(purpose::IDENTITY_FULL_NAME), k.field_key(purpose::IDENTITY_ADDRESS));
    }

    /// A blob made outside this code base (Python `cryptography`: HKDF-SHA256 field key, AES-256-GCM,
    /// the documented layout and associated data). Data written by any earlier build must keep
    /// opening, so this has to pass unchanged across a change of AES-GCM library version.
    #[test]
    fn a_blob_sealed_by_an_independent_implementation_opens() {
        let customer: Uuid = "11111111-2222-3333-4444-555555555555".parse().unwrap();
        let mut dek = [0u8; 32];
        for (i, b) in dek.iter_mut().enumerate() {
            *b = i as u8;
        }
        let k = CustomerKeys::new(customer, dek);
        let blob = hex::decode("5653026465666768696a6b6c6d6e6f5014ab1f9c5b58473dd5d2a4737e2726069fb0fa2daf86f40891a8be8b").unwrap();
        assert_eq!(k.open_str(purpose::IDENTITY_FULL_NAME, &blob).unwrap(), "Jane Q. Smith");
        assert!(k.open_str(purpose::IDENTITY_ADDRESS, &blob).is_err(), "bound to its field");
        assert!(CustomerKeys::new(Uuid::nil(), dek).open_str(purpose::IDENTITY_FULL_NAME, &blob).is_err(), "bound to its customer");
        // And what this build seals is something the independent implementation would also open:
        // same layout, nonce and tag lengths.
        let mine = k.seal_str(purpose::IDENTITY_FULL_NAME, "Jane Q. Smith").unwrap();
        assert_eq!(mine.len(), blob.len());
        assert_eq!(&mine[..3], &blob[..3]);
    }

    #[test]
    fn generated_data_keys_are_random_and_debug_never_prints_them() {
        assert_ne!(*CustomerKeys::generate_dek(), *CustomerKeys::generate_dek());
        let k = keys(Uuid::nil(), 0xAB);
        let shown = format!("{k:?}");
        assert!(!shown.contains("171") && !shown.to_lowercase().contains("ab"), "{shown}");
    }
}
