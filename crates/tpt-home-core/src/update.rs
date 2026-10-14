// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Update checks: the release manifest the server reads, and the version
//! comparison. Only checking is built. Downloading and applying a release is
//! not: applying needs the release pipeline and a rollback design first.
//!
//! The manifest is a JSON file published at `update_url`:
//!
//! ```json
//! {"version": "0.2.0", "url": "https://example.org/tpt-home-0.2.0.zip",
//!  "sha256": "<64 hex characters>", "notes": "optional text"}
//! ```

use serde::{Deserialize, Serialize};

/// Largest manifest the server will read. Real manifests are a few hundred bytes.
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

/// Parse and check a manifest. Refuses anything that could not be trusted to
/// point at a release: a malformed version or checksum, or a download address
/// that is not HTTPS (plain HTTP is allowed only for loopback, for testing).
pub fn parse_manifest(text: &str) -> Result<Release, String> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err("the update manifest is too large".into());
    }
    let release: Release =
        serde_json::from_str(text).map_err(|e| format!("the update manifest is not valid: {e}"))?;
    parse_version(&release.version).ok_or("the manifest version is not major.minor.patch")?;
    if release.sha256.len() != 64 || !release.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the manifest checksum is not 64 hex characters".into());
    }
    let loopback =
        release.url.starts_with("http://127.0.0.1") || release.url.starts_with("http://localhost");
    if !(release.url.starts_with("https://") || loopback) {
        return Err("the download address must use https".into());
    }
    if release.notes.len() > 2000 {
        return Err("the release notes are too long".into());
    }
    Ok(release)
}

/// `major.minor.patch` as numbers. Anything else (pre-release tags, missing
/// parts) is `None`, so it never counts as an update.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// True when `latest` is a higher version than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"version":"0.2.0","url":"https://example.org/t-0.2.0.zip","sha256":"0000000000000000000000000000000000000000000000000000000000000000","notes":"fixes"}"#;

    #[test]
    fn a_good_manifest_parses() {
        let r = parse_manifest(GOOD).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.notes, "fixes");
    }

    #[test]
    fn bad_manifests_are_refused_with_a_reason() {
        let no_version = GOOD.replace("0.2.0", "two");
        assert!(
            parse_manifest(&no_version)
                .unwrap_err()
                .contains("major.minor.patch")
        );
        let short_sum = GOOD.replace(&"0".repeat(64), "abc");
        assert!(parse_manifest(&short_sum).unwrap_err().contains("checksum"));
        let plain = GOOD.replace("https://", "http://example.org/");
        assert!(parse_manifest(&plain).unwrap_err().contains("https"));
        assert!(parse_manifest("not json").is_err());
        let huge = " ".repeat(MAX_MANIFEST_BYTES + 1);
        assert!(parse_manifest(&huge).unwrap_err().contains("too large"));
    }

    #[test]
    fn loopback_is_allowed_for_local_testing() {
        let local = GOOD.replace("https://example.org/", "http://127.0.0.1:9/");
        assert!(parse_manifest(&local).is_ok());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(
            !is_newer("1.0.0-rc1", "0.9.0"),
            "pre-releases are not offered"
        );
    }
}
