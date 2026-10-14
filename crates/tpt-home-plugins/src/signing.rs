// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Publisher signatures for `.tptp` packages.
//!
//! A publisher signs the exact bytes of a package with an Ed25519 key and
//! ships the signature as hex text next to it (`<package>.sig`). The server
//! installs a signed package only when the signature checks out against a
//! public key the admin trusts (`trusted_plugin_keys` in `config.toml`). With
//! `require_signed_plugins = true`, unsigned packages are refused as well.
//!
//! Signing covers the whole archive, so any change to the package, including
//! its manifest, invalidates the signature.

use anyhow::{anyhow, bail};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use tpt_home_core::config::Config;

/// Create a new signing key. Returns `(secret_hex, public_hex)`. Keep the
/// secret private; publish only the public key.
pub fn generate_keypair() -> (String, String) {
    use rand_core::RngCore;
    let mut seed = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let key = SigningKey::from_bytes(&seed);
    (to_hex(&seed), to_hex(key.verifying_key().as_bytes()))
}

/// The public key (hex) that belongs to a secret key (hex).
pub fn public_key_hex(secret_hex: &str) -> anyhow::Result<String> {
    Ok(to_hex(signing_key(secret_hex)?.verifying_key().as_bytes()))
}

/// Sign the exact package bytes. Returns the signature as 128 hex characters.
pub fn sign(package: &[u8], secret_hex: &str) -> anyhow::Result<String> {
    let key = signing_key(secret_hex)?;
    Ok(to_hex(&key.sign(package).to_bytes()))
}

/// Check `signature_hex` over `package` against the trusted public keys.
/// Returns the public key that matched.
pub fn verify(package: &[u8], signature_hex: &str, trusted: &[String]) -> anyhow::Result<String> {
    if trusted.is_empty() {
        bail!("no trusted plugin keys are set (trusted_plugin_keys in config.toml)");
    }
    let sig_bytes: [u8; 64] = from_hex(signature_hex.trim())?
        .try_into()
        .map_err(|_| anyhow!("a signature is 64 bytes (128 hex characters)"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    for key_hex in trusted {
        let key_bytes: [u8; 32] = from_hex(key_hex)?
            .try_into()
            .map_err(|_| anyhow!("a trusted key is 32 bytes (64 hex characters)"))?;
        let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
            continue;
        };
        if key.verify_strict(package, &signature).is_ok() {
            return Ok(key_hex.clone());
        }
    }
    bail!("the signature does not match any trusted plugin key")
}

/// The install-time policy. `Ok(Some(key))` when a signature matched a trusted
/// key, `Ok(None)` when the package is unsigned and allowed, and an error when
/// the package must be refused.
pub fn check_install(
    package: &[u8],
    signature: Option<&str>,
    cfg: &Config,
) -> anyhow::Result<Option<String>> {
    match signature.map(str::trim).filter(|s| !s.is_empty()) {
        Some(sig) => verify(package, sig, &cfg.trusted_plugin_keys).map(Some),
        None if cfg.require_signed_plugins => {
            bail!(
                "this server installs only signed plugins; sign the package and send its signature"
            )
        }
        None => Ok(None),
    }
}

fn signing_key(secret_hex: &str) -> anyhow::Result<SigningKey> {
    let seed: [u8; 32] = from_hex(secret_hex.trim())?
        .try_into()
        .map_err(|_| anyhow!("a secret key is 32 bytes (64 hex characters)"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> anyhow::Result<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        bail!("expected hex text with an even number of characters");
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).unwrap_or("");
            u8::from_str_radix(s, 16).map_err(|_| anyhow!("expected hex text"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &[u8] = b"pretend this is a .tptp archive";

    fn trusted_for(public_hex: &str) -> Vec<String> {
        vec![public_hex.to_string()]
    }

    #[test]
    fn a_signature_verifies_against_its_trusted_key() {
        let (secret, public) = generate_keypair();
        assert_eq!(public_key_hex(&secret).unwrap(), public);
        let sig = sign(PACKAGE, &secret).unwrap();
        assert_eq!(sig.len(), 128);
        assert_eq!(
            verify(PACKAGE, &sig, &trusted_for(&public)).unwrap(),
            public
        );
    }

    #[test]
    fn a_changed_package_fails() {
        let (secret, public) = generate_keypair();
        let sig = sign(PACKAGE, &secret).unwrap();
        let tampered = b"pretend this is a .tptp archive!";
        assert!(verify(tampered, &sig, &trusted_for(&public)).is_err());
    }

    #[test]
    fn a_key_the_admin_does_not_trust_fails() {
        let (secret, _) = generate_keypair();
        let (_, other_public) = generate_keypair();
        let sig = sign(PACKAGE, &secret).unwrap();
        let err = verify(PACKAGE, &sig, &trusted_for(&other_public)).unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[test]
    fn no_trusted_keys_means_no_signed_install() {
        let (secret, _) = generate_keypair();
        let sig = sign(PACKAGE, &secret).unwrap();
        let err = verify(PACKAGE, &sig, &[]).unwrap_err();
        assert!(err.to_string().contains("no trusted plugin keys"), "{err}");
    }

    #[test]
    fn malformed_signatures_and_keys_are_refused_with_a_reason() {
        let (_, public) = generate_keypair();
        let trusted = trusted_for(&public);
        assert!(verify(PACKAGE, "abcd", &trusted).is_err());
        assert!(verify(PACKAGE, "zz", &trusted).is_err());
        assert!(verify(PACKAGE, &"ab".repeat(64), &["not-hex".to_string()]).is_err());
    }

    #[test]
    fn install_policy_allows_unsigned_only_when_not_required() {
        let mut cfg = Config::default();
        assert_eq!(check_install(PACKAGE, None, &cfg).unwrap(), None);
        assert_eq!(check_install(PACKAGE, Some("  "), &cfg).unwrap(), None);
        cfg.require_signed_plugins = true;
        let err = check_install(PACKAGE, None, &cfg).unwrap_err();
        assert!(err.to_string().contains("only signed plugins"), "{err}");
    }

    #[test]
    fn install_policy_checks_a_given_signature_even_when_not_required() {
        let (secret, public) = generate_keypair();
        let sig = sign(PACKAGE, &secret).unwrap();
        let mut cfg = Config::default();
        assert!(
            check_install(PACKAGE, Some(&sig), &cfg).is_err(),
            "no trusted keys yet"
        );
        cfg.trusted_plugin_keys = vec![public.clone()];
        assert_eq!(
            check_install(PACKAGE, Some(&sig), &cfg).unwrap(),
            Some(public)
        );
        assert!(check_install(b"other", Some(&sig), &cfg).is_err());
    }
}
