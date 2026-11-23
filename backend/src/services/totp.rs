//! Time-based one-time passwords (RFC 6238, HMAC-SHA1, 6 digits, 30 second steps).
//!
//! These are the parameters every authenticator app supports (Google Authenticator, Authy,
//! 1Password, Aegis ...). SHA-1 is safe here: HMAC does not depend on SHA-1's collision
//! resistance, and RFC 6238 apps ignore other algorithm hints anyway.
//!
//! Pure functions: no clock, no storage. `services::mfa` supplies both.

use hmac::{Hmac, KeyInit, Mac};
use rand::RngCore;
use sha1::Sha1;

use crate::crypto::secrets_equal;

pub const STEP_SECS: u64 = 30;
pub const DIGITS: usize = 6;
/// Secret length in bytes (160 bits, the RFC 4226 recommendation).
pub const SECRET_LEN: usize = 20;
/// How many steps either side of "now" are accepted, to tolerate a slow clock or a slow typist.
const SKEW_STEPS: i64 = 1;

/// A fresh random secret.
pub fn generate_secret() -> [u8; SECRET_LEN] {
    let mut s = [0u8; SECRET_LEN];
    rand::rngs::OsRng.fill_bytes(&mut s);
    s
}

/// RFC 4648 base32 without padding: the form authenticator apps take for manual entry.
pub fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((data.len() * 8).div_ceil(5));
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in data {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Inverse of `base32_encode`, for tests that play the part of an authenticator app.
#[cfg(test)]
pub fn base32_decode(s: &str) -> Vec<u8> {
    let (mut buffer, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("not base32"),
        } as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    out
}

/// RFC 4226 HOTP for one counter value, as a zero-padded decimal string.
fn hotp(secret: &[u8], counter: u64) -> String {
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([digest[offset] & 0x7f, digest[offset + 1], digest[offset + 2], digest[offset + 3]]);
    format!("{:0width$}", bin % 10u32.pow(DIGITS as u32), width = DIGITS)
}

/// The step number a Unix time falls in.
pub fn step_at(unix_secs: u64) -> u64 {
    unix_secs / STEP_SECS
}

/// The code an authenticator shows during `step`.
pub fn code_for_step(secret: &[u8], step: u64) -> String {
    hotp(secret, step)
}

/// Check `code` against the steps around `now_unix`. Returns the step it matched, or `None`.
///
/// Only a step newer than `last_used_step` is accepted, so a code cannot be used twice (or
/// replayed after it was seen); the caller stores the returned step. All candidate steps are
/// always compared, in constant time, so timing does not say which one was close.
pub fn verify(secret: &[u8], code: &str, now_unix: u64, last_used_step: i64) -> Option<u64> {
    let code = code.trim();
    if code.len() != DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let now = step_at(now_unix) as i64;
    let mut matched = None;
    for delta in -SKEW_STEPS..=SKEW_STEPS {
        let step = now + delta;
        if step < 0 {
            continue;
        }
        let hit = secrets_equal(code.as_bytes(), code_for_step(secret, step as u64).as_bytes());
        if hit && step > last_used_step && matched.is_none_or(|m| step as u64 > m) {
            matched = Some(step as u64);
        }
    }
    matched
}

/// Percent-encode for the label and query of an `otpauth://` URI.
fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The URI an authenticator app reads from a QR code.
pub fn otpauth_uri(issuer: &str, account: &str, secret: &[u8]) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        pct(issuer),
        pct(account),
        base32_encode(secret),
        pct(issuer),
        DIGITS,
        STEP_SECS
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B, SHA-1 secret, truncated to 6 digits (the RFC lists 8).
    const RFC_SECRET: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_the_rfc_6238_test_vectors() {
        for (time, eight_digit) in [
            (59u64, "94287082"),
            (1111111109, "07081804"),
            (1111111111, "14050471"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
            (20000000000, "65353130"),
        ] {
            assert_eq!(code_for_step(RFC_SECRET, step_at(time)), eight_digit[2..], "t={time}");
        }
    }

    #[test]
    fn base32_matches_the_rfc_4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_encode(input.as_bytes()), want);
        }
    }

    #[test]
    fn base32_round_trips() {
        for len in 0..40 {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(base32_decode(&base32_encode(&data)), data);
        }
    }

    #[test]
    fn a_current_code_is_accepted_and_returns_its_step() {
        let now = 1_700_000_000;
        let code = code_for_step(RFC_SECRET, step_at(now));
        assert_eq!(verify(RFC_SECRET, &code, now, 0), Some(step_at(now)));
        assert_eq!(verify(RFC_SECRET, &format!(" {code} "), now, 0), Some(step_at(now)), "surrounding spaces are fine");
    }

    #[test]
    fn one_step_of_clock_skew_is_tolerated_but_not_two() {
        let now = 1_700_000_000;
        let prev = code_for_step(RFC_SECRET, step_at(now) - 1);
        let next = code_for_step(RFC_SECRET, step_at(now) + 1);
        let far = code_for_step(RFC_SECRET, step_at(now) - 2);
        assert!(verify(RFC_SECRET, &prev, now, 0).is_some());
        assert!(verify(RFC_SECRET, &next, now, 0).is_some());
        assert!(verify(RFC_SECRET, &far, now, 0).is_none());
    }

    #[test]
    fn a_code_cannot_be_used_twice() {
        let now = 1_700_000_000;
        let code = code_for_step(RFC_SECRET, step_at(now));
        let step = verify(RFC_SECRET, &code, now, 0).unwrap();
        assert_eq!(verify(RFC_SECRET, &code, now, step as i64), None, "same step again is a replay");
        // An older code from within the window is a replay too once a newer one was used.
        let older = code_for_step(RFC_SECRET, step - 1);
        assert_eq!(verify(RFC_SECRET, &older, now, step as i64), None);
    }

    #[test]
    fn malformed_codes_are_rejected() {
        let now = 1_700_000_000;
        for bad in ["", "12345", "1234567", "12345a", "１２３４５６", "      ", "-12345", "12 345"] {
            assert_eq!(verify(RFC_SECRET, bad, now, 0), None, "{bad:?}");
        }
    }

    #[test]
    fn the_wrong_secret_does_not_verify() {
        let now = 1_700_000_000;
        let code = code_for_step(RFC_SECRET, step_at(now));
        assert_eq!(verify(b"another secret value!", &code, now, 0), None);
    }

    #[test]
    fn generated_secrets_are_distinct_and_the_right_size() {
        let (a, b) = (generate_secret(), generate_secret());
        assert_eq!(a.len(), SECRET_LEN);
        assert_ne!(a, b);
    }

    #[test]
    fn the_otpauth_uri_is_escaped_and_carries_the_secret() {
        let uri = otpauth_uri("Vector Scrub", "jane+test@example.com", RFC_SECRET);
        assert!(uri.starts_with("otpauth://totp/Vector%20Scrub:jane%2Btest%40example.com?"), "{uri}");
        assert!(uri.contains(&format!("secret={}", base32_encode(RFC_SECRET))), "{uri}");
        assert!(uri.contains("issuer=Vector%20Scrub") && uri.contains("digits=6") && uri.contains("period=30"), "{uri}");
    }
}
