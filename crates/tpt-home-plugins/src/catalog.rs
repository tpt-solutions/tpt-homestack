// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The plugin catalog: browse the packages listed in an index and install one
//! with the admin's approvals. The index format is in `docs/PLUGIN_INDEX.md`.
//!
//! Nothing from the index runs. An install downloads the package over HTTPS,
//! checks its SHA-256 against the index, verifies its signature when the index
//! names one, and then takes the normal install path with the approvals the
//! admin chose. An entry whose manifest id differs from the index id is
//! refused before anything is installed.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use tpt_home_core::config::Config;
use tpt_home_core::db::DbPool;
use tpt_home_core::error::ApiError;
use tpt_home_core::fetch::{check_url, https_get};
use tpt_home_core::update::{is_newer, parse_version};
use tpt_home_ui::{CAPABILITY_KEYS, capability_label};

use crate::lifecycle;
use crate::manifest::Permissions;
use crate::registry;
use crate::signing;

pub const MAX_INDEX_BYTES: u64 = 1024 * 1024;
pub const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: u64 = 1024;
pub const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub package: String,
    pub sha256: String,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub signed_by: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Deserialize)]
struct RawIndex {
    format: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    plugins: Vec<Entry>,
}

/// A checked index: its name and entries.
#[derive(Debug, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub entries: Vec<Entry>,
}

/// Parse and check an index. One bad entry refuses the whole index and names
/// the entry, so the admin never sees a partly trusted list.
pub fn parse_index(text: &str) -> Result<Index, String> {
    let raw: RawIndex =
        serde_json::from_str(text).map_err(|e| format!("the catalog is not valid: {e}"))?;
    if raw.format != 1 {
        return Err(format!(
            "the catalog format {} is not supported",
            raw.format
        ));
    }
    if raw.plugins.len() > MAX_ENTRIES {
        return Err(format!("the catalog lists more than {MAX_ENTRIES} plugins"));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, entry) in raw.plugins.iter().enumerate() {
        check_entry(entry).map_err(|e| format!("catalog entry {} ({}): {e}", i + 1, entry.id))?;
        if !seen.insert(entry.id.clone()) {
            return Err(format!(
                "catalog entry {} repeats the id {}",
                i + 1,
                entry.id
            ));
        }
    }
    Ok(Index {
        name: raw.name,
        entries: raw.plugins,
    })
}

fn check_entry(e: &Entry) -> Result<(), String> {
    if !valid_id(&e.id) {
        return Err("the id may use lowercase letters, digits and underscores".into());
    }
    if e.name.is_empty() || e.name.len() > 100 {
        return Err("the name must be 1 to 100 characters".into());
    }
    if parse_version(&e.version).is_none() {
        return Err("the version is not major.minor.patch".into());
    }
    if e.description.len() > 1000 {
        return Err("the description is longer than 1000 characters".into());
    }
    check_url(&e.package)?;
    if e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the checksum is not 64 hex characters".into());
    }
    if let Some(sig) = &e.signature {
        check_url(sig)?;
    }
    if let Some(key) = &e.signed_by
        && (key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("signed_by is not a 64-character public key".into());
    }
    Permissions::parse_list(&e.permissions.join(",")).map_err(|e| format!("permissions: {e}"))?;
    Ok(())
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The plain-language sentences for a list of permission keys.
pub fn plain_permissions(keys: &[String]) -> Vec<String> {
    keys.iter()
        .map(|k| {
            if CAPABILITY_KEYS.contains(&k.as_str()) {
                capability_label(k).to_string()
            } else {
                format!("Use an unknown capability ({k})")
            }
        })
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fetch_index(url: &str) -> Result<Index, String> {
    let bytes = https_get(url, MAX_INDEX_BYTES)?;
    let text = String::from_utf8(bytes).map_err(|_| "the catalog is not UTF-8".to_string())?;
    parse_index(&text)
}

fn config() -> Config {
    tpt_home_core::config::try_global()
        .cloned()
        .unwrap_or_default()
}

/// GET /api/plugins/catalog: the entries, with what is installed here.
#[tpt_home_macros::bp_route(get, "/api/plugins/catalog", role = "admin")]
pub async fn list_catalog() -> Result<Json<Value>, ApiError> {
    let cfg = config();
    if cfg.catalog_url.is_empty() {
        return Ok(Json(json!({"configured": false, "plugins": []})));
    }
    let url = cfg.catalog_url.clone();
    let index = tokio::task::spawn_blocking(move || fetch_index(&url))
        .await
        .map_err(|e| ApiError::internal(format!("catalog: {e}")))?
        .map_err(ApiError::bad_request)?;

    let plugins: Vec<Value> = index
        .entries
        .iter()
        .map(|e| {
            let installed = registry::get(&e.id).map(|p| p.manifest.plugin.version.clone());
            let status = match &installed {
                None => "not installed".to_string(),
                Some(v) if is_newer(&e.version, v) => format!("update available (installed {v})"),
                Some(v) => format!("installed ({v})"),
            };
            json!({
                "id": e.id,
                "name": e.name,
                "version": e.version,
                "description": e.description,
                "permissions": e.permissions,
                "permissions_plain": plain_permissions(&e.permissions),
                "signed": e.signature.is_some(),
                "signed_by": e.signed_by,
                "installed_version": installed,
                "status": status,
            })
        })
        .collect();
    Ok(Json(json!({
        "configured": true,
        "name": index.name,
        "plugins": plugins,
    })))
}

#[derive(Deserialize)]
pub struct CatalogInstall {
    pub id: String,
    /// The permissions the admin approves, as the install form names them.
    #[serde(default)]
    pub approve: Vec<String>,
    /// Enable after install; absent keeps the plugin's current state.
    #[serde(default)]
    pub enable: Option<bool>,
}

/// POST /api/plugins/catalog/install: fetch, check and install one entry.
#[tpt_home_macros::bp_route(post, "/api/plugins/catalog/install", role = "admin")]
pub async fn install_from_catalog(
    State(db): State<DbPool>,
    Json(req): Json<CatalogInstall>,
) -> Result<impl IntoResponse, ApiError> {
    let cfg = config();
    if cfg.catalog_url.is_empty() {
        return Err(ApiError::bad_request(
            "no plugin catalog is set (catalog_url)",
        ));
    }
    let granted = Permissions::parse_list(&req.approve.join(",")).map_err(ApiError::bad_request)?;

    let url = cfg.catalog_url.clone();
    let index = tokio::task::spawn_blocking(move || fetch_index(&url))
        .await
        .map_err(|e| ApiError::internal(format!("catalog: {e}")))?
        .map_err(ApiError::bad_request)?;
    let entry = index
        .entries
        .into_iter()
        .find(|e| e.id == req.id)
        .ok_or_else(|| ApiError::not_found(format!("no catalog entry {}", req.id)))?;

    let package_url = entry.package.clone();
    let archive = tokio::task::spawn_blocking(move || https_get(&package_url, MAX_PACKAGE_BYTES))
        .await
        .map_err(|e| ApiError::internal(format!("package: {e}")))?
        .map_err(ApiError::bad_request)?;
    if sha256_hex(&archive) != entry.sha256.to_ascii_lowercase() {
        return Err(ApiError::bad_request(
            "the package does not match the checksum in the catalog; it was not installed",
        ));
    }

    let signature = match entry.signature.clone() {
        None => None,
        Some(sig_url) => {
            let text =
                tokio::task::spawn_blocking(move || https_get(&sig_url, MAX_SIGNATURE_BYTES))
                    .await
                    .map_err(|e| ApiError::internal(format!("signature: {e}")))?
                    .map_err(ApiError::bad_request)?;
            Some(
                String::from_utf8(text)
                    .map_err(|_| ApiError::bad_request("the signature is not text"))?,
            )
        }
    };
    if let Some(expected) = &entry.signed_by {
        let sig = signature.as_deref().ok_or_else(|| {
            ApiError::bad_request(
                "the catalog says this plugin is signed, but no signature was given",
            )
        })?;
        let key = signing::verify(&archive, sig, &cfg.trusted_plugin_keys)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        if &key != expected {
            return Err(ApiError::bad_request(
                "the package is signed by a different key than the catalog names",
            ));
        }
    }

    // The manifest, not the index, decides the id. Refuse a mismatch before the
    // package is installed.
    let (staged, manifest) = lifecycle::stage_package(&archive)
        .map_err(|e| ApiError::bad_request(format!("invalid package: {e}")))?;
    let _ = std::fs::remove_dir_all(&staged);
    if manifest.plugin.id != entry.id {
        return Err(ApiError::bad_request(format!(
            "the package is {} but the catalog lists it as {}",
            manifest.plugin.id, entry.id
        )));
    }

    let (status, body) =
        crate::dispatch::install_archive(&db, &archive, signature.as_deref(), &granted, req.enable)
            .await?;
    Ok((status, Json(body)))
}

/// Keeps the plugin-independent helpers in one place for the catalog tests.
#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"format":1,"name":"Test","plugins":[{"id":"logbook","name":"Logbook","version":"0.1.0","description":"Dated log.","package":"https://example.org/logbook-0.1.0.tptp","sha256":"0000000000000000000000000000000000000000000000000000000000000000","permissions":["db","kv"]}]}"#;

    #[test]
    fn a_good_index_parses() {
        let index = parse_index(GOOD).unwrap();
        assert_eq!(index.name, "Test");
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].id, "logbook");
    }

    #[test]
    fn a_bad_entry_refuses_the_whole_index_and_names_it() {
        let bad = GOOD.replace("logbook", "Log Book");
        let err = parse_index(&bad).unwrap_err();
        assert!(
            err.contains("entry 1") && err.contains("lowercase"),
            "{err}"
        );

        let plain = GOOD.replace("https://example.org", "http://example.org");
        assert!(parse_index(&plain).unwrap_err().contains("https"));

        let unknown = GOOD.replace("\"db\",\"kv\"", "\"teleport\"");
        assert!(parse_index(&unknown).unwrap_err().contains("permissions"));
    }

    #[test]
    fn the_format_and_duplicates_are_checked() {
        assert!(parse_index(&GOOD.replace("\"format\":1", "\"format\":2")).is_err());
        let entry = r#"{"id":"logbook","name":"Logbook","version":"0.1.0","package":"https://example.org/a.tptp","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#;
        let twice = format!(r#"{{"format":1,"plugins":[{entry},{entry}]}}"#);
        let err = parse_index(&twice).unwrap_err();
        assert!(err.contains("repeats the id logbook"), "{err}");
    }

    #[test]
    fn plain_permissions_describe_each_key() {
        let plain = plain_permissions(&["kv".into(), "teleport".into()]);
        assert_eq!(plain[0], capability_label("kv"));
        assert!(plain[1].contains("unknown capability"));
    }

    #[test]
    fn checksums_are_lowercase_sha256_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
