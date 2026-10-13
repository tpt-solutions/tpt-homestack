// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The upgrade path on a real database file: a fresh install needs no copy,
//! an existing database is copied (and the copy checked) before it changes,
//! dry run changes nothing, and a database from a newer version is refused.

use tpt_home_api::{backup, migrate_database};
use tpt_home_core::config::Config;
use tpt_home_core::db;
use tpt_home_core::migrate;

fn config(dir: &tempfile::TempDir) -> Config {
    Config {
        data_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

#[tokio::test]
async fn upgrade_path_keeps_a_checked_copy_and_refuses_newer_databases() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);

    // A fresh install: everything is pending, nothing needs saving first.
    let fresh = migrate_database(&cfg, false).await.unwrap();
    assert!(!fresh.pending.is_empty());
    assert!(
        fresh.snapshot.is_none(),
        "a new database has nothing to save"
    );
    assert!(!fresh.applied.is_empty());

    // Pretend an older version never applied one core migration.
    let pool = db::connect(&cfg.db_path()).await.unwrap();
    sqlx::query("DELETE FROM _tpt_schema_migrations WHERE key = ?")
        .bind("tpt-home-core:004_api_tokens")
        .execute(&pool)
        .await
        .unwrap();
    drop(pool);

    // Dry run lists it and changes nothing.
    let dry = migrate_database(&cfg, true).await.unwrap();
    assert_eq!(dry.pending, ["tpt-home-core:004_api_tokens"]);
    assert!(dry.applied.is_empty() && dry.snapshot.is_none());
    assert!(!dir.path().join("backups").join("upgrade").exists());

    // The real run keeps a copy first, then applies the migration.
    let up = migrate_database(&cfg, false).await.unwrap();
    assert_eq!(up.applied, ["tpt-home-core:004_api_tokens"]);
    let copy = up.snapshot.expect("an existing database is copied first");
    assert!(copy.starts_with(dir.path().join("backups").join("upgrade")));
    backup::check_db_file(&copy)
        .await
        .expect("the copy is a valid tpt-home database");

    // Up to date now.
    assert!(
        migrate_database(&cfg, false)
            .await
            .unwrap()
            .pending
            .is_empty()
    );
}

#[tokio::test]
async fn a_database_from_a_newer_version_is_refused_without_changes() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(&dir);
    migrate_database(&cfg, false).await.unwrap();

    let pool = db::connect(&cfg.db_path()).await.unwrap();
    sqlx::query("INSERT INTO _tpt_schema_migrations (key, applied_at) VALUES ('tpt-home-core:900_later', 0)")
        .execute(&pool)
        .await
        .unwrap();
    drop(pool);

    let err = migrate_database(&cfg, false).await.unwrap_err().to_string();
    assert!(err.contains("newer tpt-homestack"), "{err}");
    let pool = db::connect(&cfg.db_path()).await.unwrap();
    assert!(migrate::pending_keys(&pool).await.unwrap().is_empty());
    assert!(
        !dir.path().join("backups").join("upgrade").exists(),
        "no copy for a refused upgrade"
    );
}
