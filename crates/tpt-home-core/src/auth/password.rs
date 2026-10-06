// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Argon2id password hashing (OWASP-friendly defaults from the `argon2`
//! crate). Hashes embed their salt and parameters, so verification needs
//! nothing else.

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

use super::AuthError;

/// Hash a plaintext password with Argon2id.
pub fn hash_password(plain: &str) -> Result<String, AuthError> {
    if plain.chars().count() < 8 {
        return Err(AuthError::WeakPassword(
            "password must be at least 8 characters".into(),
        ));
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("argon2 hash failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored PHC-format hash.
pub fn verify_password(plain: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = hash_password("correct horse battery").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn rejects_short_passwords() {
        assert!(matches!(
            hash_password("short"),
            Err(AuthError::WeakPassword(_))
        ));
    }

    #[test]
    fn garbage_hash_fails_closed() {
        assert!(!verify_password("x", "not-a-hash"));
    }
}
