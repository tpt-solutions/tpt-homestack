// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Cron-style expiry cleanup. Registered via `StartupEntry`; the server
//! spawns it once at boot. A crashed task must never take the server down —
//! the loop only logs errors.

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

/// Spawn the 60-second cleanup loop onto the current runtime.
pub fn spawn_cleanup_loop(db: DbPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = sweep(&db).await {
                tracing::warn!(error = ?e, "drop cleanup sweep failed");
            }
        }
    });
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
