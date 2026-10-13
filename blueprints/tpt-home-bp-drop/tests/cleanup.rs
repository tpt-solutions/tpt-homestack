// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Expiry sweep: expired rows and their files are removed, live and
//! never-expiring drops are kept, and a second sweep does nothing.

use std::path::PathBuf;
use std::sync::OnceLock;

use tpt_home_bp_drop::cleanup::sweep;
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::{DbPool, connect_in_memory};

fn data_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::mem::forget(dir);
        set_global_for_tests(Config {
            data_dir: root.clone(),
            ..Config::default()
        });
        root
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Writes a file under `uploads/drop` and inserts its row.
async fn add_drop(db: &DbPool, stored: &str, expires_at: Option<i64>) {
    let dir = data_dir().join("uploads").join("drop");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(stored), b"payload").unwrap();
    sqlx::query(
        "INSERT INTO drop_files (filename, stored_path, size, created_at, expires_at)
         VALUES (?1, ?1, 7, ?2, ?3)",
    )
    .bind(stored)
    .bind(now())
    .bind(expires_at)
    .execute(db)
    .await
    .unwrap();
}

async fn remaining(db: &DbPool) -> Vec<String> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT stored_path FROM drop_files ORDER BY stored_path")
            .fetch_all(db)
            .await
            .unwrap();
    rows.into_iter().map(|(s,)| s).collect()
}

#[tokio::test]
async fn sweep_removes_only_expired_drops_and_their_files() {
    data_dir();
    let db = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    add_drop(&db, "expired.bin", Some(now() - 60)).await;
    add_drop(&db, "live.bin", Some(now() + 3600)).await;
    add_drop(&db, "forever.bin", None).await;

    let removed = sweep(&db).await.unwrap();
    assert_eq!(removed, 1);
    assert_eq!(remaining(&db).await, vec!["forever.bin", "live.bin"]);

    let dir = data_dir().join("uploads").join("drop");
    assert!(!dir.join("expired.bin").exists(), "expired file deleted");
    assert!(dir.join("live.bin").exists(), "live file kept");
    assert!(dir.join("forever.bin").exists(), "never-expiring file kept");

    assert_eq!(sweep(&db).await.unwrap(), 0, "a second sweep finds nothing");
}

#[tokio::test]
async fn sweep_with_a_missing_file_still_removes_the_row() {
    data_dir();
    let db = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    sqlx::query(
        "INSERT INTO drop_files (filename, stored_path, size, created_at, expires_at)
         VALUES ('gone.bin', 'gone.bin', 1, ?1, ?2)",
    )
    .bind(now())
    .bind(now() - 1)
    .execute(&db)
    .await
    .unwrap();

    assert_eq!(sweep(&db).await.unwrap(), 1);
    assert!(remaining(&db).await.is_empty());
}
