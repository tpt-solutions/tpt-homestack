// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Admin update check: reads the release manifest at `update_url` and says
//! whether a newer version exists. It never downloads or installs a release;
//! applying one waits for the release pipeline and a rollback design.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use serde_json::{Value, json};

use tpt_home_core::config::Config;
use tpt_home_core::error::ApiError;
use tpt_home_core::update::{MAX_MANIFEST_BYTES, Release, is_newer, parse_manifest};

/// Fetch and check the manifest. Blocking: call from `spawn_blocking`.
pub fn fetch(url: &str) -> Result<Release, String> {
    let bytes = tpt_home_core::fetch::https_get(url, MAX_MANIFEST_BYTES as u64)?;
    let text =
        String::from_utf8(bytes).map_err(|_| "the update manifest is not UTF-8".to_string())?;
    parse_manifest(&text)
}

/// GET /api/admin/update: whether a newer release is published.
pub async fn status(Extension(cfg): Extension<Arc<Config>>) -> Result<Json<Value>, ApiError> {
    let current = tpt_home_core::VERSION;
    if cfg.update_url.is_empty() {
        return Ok(Json(json!({"configured": false, "current": current})));
    }
    let url = cfg.update_url.clone();
    let checked = tokio::task::spawn_blocking(move || fetch(&url))
        .await
        .map_err(|e| ApiError::internal(format!("update check: {e}")))?;
    Ok(Json(match checked {
        Ok(release) => json!({
            "configured": true,
            "current": current,
            "latest": release.version,
            "update_available": is_newer(&release.version, current),
            "download_url": release.url,
            "sha256": release.sha256,
            "notes": release.notes,
        }),
        Err(error) => json!({
            "configured": true,
            "current": current,
            "error": error,
        }),
    }))
}
