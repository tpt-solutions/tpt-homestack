// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Signature checks for inbound webhooks.
//!
//! A sender signs `"<timestamp>.<body>"` with HMAC-SHA256 and a shared secret,
//! and sends the timestamp and the hex digest in headers. [`verify`] accepts
//! the request only if the digest matches and the timestamp falls inside a
//! window around the server's clock, which stops a captured request from
//! being replayed later. Replays *inside* the window are not caught; keep the
//! window short (minutes, not hours).

use sha2::{Digest, Sha256};

const BLOCK_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookError {
    /// The timestamp header is not a whole number of seconds.
    BadTimestamp,
    /// The timestamp is outside the accepted window.
    Stale,
    /// The signature does not match the body and timestamp.
    BadSignature,
}

impl std::fmt::Display for WebhookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebhookError::BadTimestamp => write!(f, "timestamp header is not a number"),
            WebhookError::Stale => write!(f, "timestamp is outside the accepted window"),
            WebhookError::BadSignature => write!(f, "signature does not match"),
        }
    }
}

/// Hex HMAC-SHA256 of `"<timestamp>.<body>"`.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut message = format!("{timestamp}.").into_bytes();
    message.extend_from_slice(body);
    hex(&hmac_sha256(secret, &message))
}

/// Check a request. `now` is the server's Unix time in seconds, `window_secs`
/// how far the timestamp may differ from it in either direction.
pub fn verify(
    secret: &[u8],
    timestamp_header: &str,
    signature_header: &str,
    body: &[u8],
    now: i64,
    window_secs: i64,
) -> Result<(), WebhookError> {
    let timestamp: i64 = timestamp_header
        .trim()
        .parse()
        .map_err(|_| WebhookError::BadTimestamp)?;
    if (now - timestamp).abs() > window_secs {
        return Err(WebhookError::Stale);
    }
    let expected = sign(secret, timestamp, body);
    if constant_time_eq(expected.as_bytes(), signature_header.trim().as_bytes()) {
        Ok(())
    } else {
        Err(WebhookError::BadSignature)
    }
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // Keys longer than a block are hashed first; shorter keys are zero-padded.
    let mut block_key = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        block_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }

    let mut inner = Sha256::new();
    inner.update(block_key.map(|b| b ^ 0x36));
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(block_key.map(|b| b ^ 0x5c));
    outer.update(inner_digest);
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare without returning early, so timing does not reveal how many leading
/// bytes matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_test_case_1() {
        // RFC 4231, test case 1.
        let key = [0x0bu8; 20];
        let digest = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex(&digest),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn hmac_matches_rfc4231_test_case_6_long_key() {
        // Key longer than one block: must be hashed first.
        let key = [0xaau8; 131];
        let digest = hmac_sha256(
            &key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        assert_eq!(
            hex(&digest),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn valid_signature_inside_window_is_accepted() {
        let secret = b"shared-secret";
        let body = br#"{"event":"door.open"}"#;
        let sig = sign(secret, 1_000, body);
        assert_eq!(verify(secret, "1000", &sig, body, 1_030, 60), Ok(()));
    }

    #[test]
    fn tampered_body_or_wrong_secret_is_rejected() {
        let body = b"{\"amount\":1}";
        let sig = sign(b"secret", 1_000, body);
        assert_eq!(
            verify(b"secret", "1000", &sig, b"{\"amount\":9}", 1_000, 60),
            Err(WebhookError::BadSignature)
        );
        assert_eq!(
            verify(b"other", "1000", &sig, body, 1_000, 60),
            Err(WebhookError::BadSignature)
        );
    }

    #[test]
    fn replay_outside_window_is_rejected() {
        let body = b"{}";
        let sig = sign(b"secret", 1_000, body);
        assert_eq!(
            verify(b"secret", "1000", &sig, body, 1_061, 60),
            Err(WebhookError::Stale)
        );
        // A timestamp from the future is held to the same window.
        let future = sign(b"secret", 2_000, body);
        assert_eq!(
            verify(b"secret", "2000", &future, body, 1_000, 60),
            Err(WebhookError::Stale)
        );
    }

    #[test]
    fn malformed_timestamp_is_rejected() {
        assert_eq!(
            verify(b"secret", "yesterday", "00", b"{}", 1_000, 60),
            Err(WebhookError::BadTimestamp)
        );
    }
}
