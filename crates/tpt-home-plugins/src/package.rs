// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `.tptp` plugin packages: an uncompressed tar containing
//! `plugin.toml`, the wasm entry, optional `assets/…`, and a
//! `checksums.sha256` manifest (`"<sha256-hex>  <name>"` lines).
//!
//! Unpacking rejects path traversal, unexpected files, oversized entries,
//! checksum mismatches and invalid manifests before anything is written.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::manifest;

fn allowed_path(name: &str) -> bool {
    name == "plugin.toml"
        || name == "checksums.sha256"
        || name == "plugin.wasm"
        || (name.starts_with("assets/") && !name.ends_with('/'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut s = String::with_capacity(64);
    for b in hasher.finalize() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Packed plugin archive ready to write to disk.
pub struct Package {
    pub bytes: Vec<u8>,
}

/// Build a `.tptp` from a package directory (`plugin.toml` + entry +
/// optional `assets/`).
pub fn pack(dir: &std::path::Path) -> Result<Package, anyhow::Error> {
    let manifest_text = std::fs::read_to_string(dir.join("plugin.toml"))
        .map_err(|e| anyhow::anyhow!("read plugin.toml: {e}"))?;
    let manifest = manifest::parse(&manifest_text)?;

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    files.push(("plugin.toml".into(), manifest_text.into_bytes()));

    let entry_bytes = std::fs::read(dir.join(&manifest.plugin.entry))
        .map_err(|e| anyhow::anyhow!("read {}: {e}", manifest.plugin.entry))?;
    files.push((manifest.plugin.entry.clone(), entry_bytes));

    let assets = dir.join("assets");
    if assets.is_dir() {
        collect_files(&assets, "assets", &mut files)?;
    }

    // Checksum manifest over everything except itself.
    let mut sums = String::new();
    for (name, bytes) in &files {
        sums.push_str(&format!("{}  {}\n", sha256_hex(bytes), name));
    }
    files.push(("checksums.sha256".into(), sums.into_bytes()));

    let mut builder = tar::Builder::new(Vec::new());
    for (name, bytes) in &files {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|e| anyhow::anyhow!("tar append {name}: {e}"))?;
    }
    Ok(Package {
        bytes: builder.into_inner()?,
    })
}

fn collect_files(
    dir: &std::path::Path,
    prefix: &str,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), anyhow::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), &name, files)?;
        } else {
            files.push((name, std::fs::read(entry.path())?));
        }
    }
    Ok(())
}

/// Verify a `.tptp` archive and unpack it into `dest_dir` (created if
/// needed). Returns the parsed manifest.
pub fn unpack_verify(
    archive: &[u8],
    dest_dir: &std::path::Path,
) -> Result<manifest::PluginManifest, anyhow::Error> {
    if archive.len() > 64 * 1024 * 1024 {
        anyhow::bail!("package too large (>64 MiB)");
    }

    let mut ar = tar::Archive::new(std::io::Cursor::new(archive));
    ar.set_overwrite(false);

    let mut manifest_text: Option<String> = None;
    let mut checksums: HashMap<String, String> = HashMap::new();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    for entry in ar.entries()? {
        let mut entry = entry.map_err(|e| anyhow::anyhow!("bad archive: {e}"))?;
        let path = entry
            .path()?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        if path.is_empty()
            || path.contains("..")
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains(':')
        {
            anyhow::bail!("illegal path in package: {path}");
        }
        if !allowed_path(&path) {
            anyhow::bail!("unexpected file in package: {path}");
        }
        if entry.size() > 32 * 1024 * 1024 {
            anyhow::bail!("entry {path} too large");
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        match path.as_str() {
            "plugin.toml" => {
                manifest_text =
                    Some(String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("plugin.toml not UTF-8"))?);
            }
            "checksums.sha256" => {
                let text =
                    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("checksums.sha256 not UTF-8"))?;
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let mut parts = line.split_whitespace();
                    let (Some(sum), Some(name)) = (parts.next(), parts.next()) else {
                        anyhow::bail!("malformed checksum line: {line}");
                    };
                    checksums.insert(name.to_string(), sum.to_string());
                }
            }
            _ => entries.push((path, bytes)),
        }
    }

    let manifest_text =
        manifest_text.ok_or_else(|| anyhow::anyhow!("package has no plugin.toml"))?;
    let manifest = manifest::parse(&manifest_text)?;

    // Every payload entry must have a matching checksum entry, and every
    // checksum must belong to a present file.
    for (path, bytes) in &entries {
        match checksums.remove(path) {
            Some(sum) if sum.eq_ignore_ascii_case(&sha256_hex(bytes)) => {}
            _ => anyhow::bail!("checksum mismatch or missing for {path}"),
        }
    }
    if !checksums.is_empty() {
        anyhow::bail!(
            "checksum manifest lists absent files: {:?}",
            checksums.keys().collect::<Vec<_>>()
        );
    }
    if !entries.iter().any(|(p, _)| *p == manifest.plugin.entry) {
        anyhow::bail!("package is missing its entry {}", manifest.plugin.entry);
    }

    // All checks passed — write the package.
    std::fs::create_dir_all(dest_dir)?;
    std::fs::write(dest_dir.join("plugin.toml"), &manifest_text)?;
    for (path, bytes) in &entries {
        let target = tpt_home_core::files::safe_join(dest_dir, &[path])?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_WASM: &[u8] = b"\0asm\x01\0\0\0"; // header-only module

    fn sample_dir(root: &std::path::Path) -> std::path::PathBuf {
        let dir = root.join("calendar");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            r#"
[plugin]
id = "calendar"
name = "Calendar"
version = "0.1.0"
entry = "plugin.wasm"

[permissions]
db = true
"#,
        )
        .unwrap();
        std::fs::write(dir.join("plugin.wasm"), MINIMAL_WASM).unwrap();
        std::fs::write(dir.join("assets").join("icon.txt"), "icon").unwrap();
        dir
    }

    #[test]
    fn pack_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sample_dir(tmp.path());
        let pkg = pack(&dir).unwrap();

        let dest = tmp.path().join("installed");
        let manifest = unpack_verify(&pkg.bytes, &dest).unwrap();
        assert_eq!(manifest.plugin.id, "calendar");
        assert!(dest.join("plugin.wasm").is_file());
        assert!(dest.join("assets/icon.txt").is_file());
    }

    #[test]
    fn tampered_payload_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = sample_dir(tmp.path());
        let mut pkg = pack(&dir).unwrap();
        // Flip one byte of the payload (inside the wasm content area).
        let last = pkg.bytes.len() - 8;
        pkg.bytes[last] ^= 0xFF;
        let dest = tmp.path().join("installed");
        assert!(unpack_verify(&pkg.bytes, &dest).is_err());
        assert!(!dest.exists(), "nothing is written on failure");
    }

    #[test]
    fn traversal_and_foreign_files_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        // Hand-craft an archive with a bad path.
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_cksum();
        builder
            .append_data(&mut header, "../evil.txt", b"evil".as_slice())
            .unwrap();
        let bytes = builder.into_inner().unwrap();
        assert!(unpack_verify(&bytes, &tmp.path().join("x")).is_err());

        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_cksum();
        builder
            .append_data(&mut header, "scripts/run.sh", b"evil".as_slice())
            .unwrap();
        let bytes = builder.into_inner().unwrap();
        assert!(unpack_verify(&bytes, &tmp.path().join("y")).is_err());
    }
}
