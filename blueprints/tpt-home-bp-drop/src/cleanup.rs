// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Expiry cleanup. The sweep runs from a `#[bp_job]` in `lib.rs` once a
//! minute; a failed sweep is logged by the job runner and retried next tick.

use tpt_home_core::db::DbPool;
use tpt_home_core::files;

/// Delete expired rows and their files. Returns the number removed.
pub async fn sweep(db: &DbPool) -> Result<u32, tpt_home_core::ApiError> {
    let now = unix_now();
    let expired: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, stored_path FROM drop_files
         WHERE expires_at IS NOT NULL AND expires_at <= ?1",
    )
    .bind(now)
    .fetch_all(db)
    .await
    .map_err(tpt_home_core::ApiError::from)?;

    for (_, stored) in &expired {
        if let Ok(path) = files::safe_join(&upload_dir(), &[stored]) {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
    if !expired.is_empty() {
        sqlx::query("DELETE FROM drop_files WHERE expires_at IS NOT NULL AND expires_at <= ?1")
            .bind(now)
            .execute(db)
            .await
            .map_err(tpt_home_core::ApiError::from)?;
        tracing::debug!(count = expired.len(), "drop: expired files removed");
    }
    Ok(expired.len() as u32)
}

fn upload_dir() -> std::path::PathBuf {
    tpt_home_core::config::global().uploads_dir().join("drop")
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
