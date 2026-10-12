// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Install, upgrade, rollback and uninstall against a real (in-memory) database
//! and a temporary data directory. The rollback tests force a failure after
//! the package has been moved into place, with a trigger on the `plugins` table.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::connect_in_memory;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::registry;

/// A wasm module that is valid but answers nothing useful; enough to load.
const VALID_WASM: &str = r#"(module
    (memory (export "memory") 1)
    (func (export "tpt_alloc") (param i32) (result i32) i32.const 0)
    (func (export "tpt_entry") (param i32 i32) (result i64) i64.const -1))"#;

fn data_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
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

/// Write a staged package directory for `id` at `version` with the given wasm bytes.
fn stage(id: &str, version: &str, wasm: &[u8]) -> PathBuf {
    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{version}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            r#"
[plugin]
id = "{id}"
name = "{id}"
version = "{version}"
entry = "plugin.wasm"

[permissions]
db = true

[[tables]]
name = "items"
[[tables.columns]]
name = "label"
kind = "text"
"#
        ),
    )
    .unwrap();
    std::fs::write(dir.join("plugin.wasm"), wasm).unwrap();
    dir
}

/// wasmi's `Module::new` accepts WAT text, so the entry file can be the source.
fn valid_wasm() -> Vec<u8> {
    VALID_WASM.as_bytes().to_vec()
}

/// The admin approves everything the fixtures request.
fn grant_all() -> tpt_home_plugins::manifest::Permissions {
    tpt_home_plugins::manifest::Permissions::parse_list("db,kv,storage").unwrap()
}

/// A migrated database. Server startup creates the plugins directory; tests do
/// the same, since install renames packages into it.
async fn fresh_db() -> tpt_home_core::db::DbPool {
    data_root(); // installs the global config the plugins directory hangs off
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    db
}

async fn table_exists(db: &tpt_home_core::db::DbPool, name: &str) -> bool {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1")
            .bind(name)
            .fetch_optional(db)
            .await
            .unwrap();
    row.is_some()
}

/// Make every insert into `plugins` fail, after the package is already on disk.
async fn break_plugin_rows(db: &tpt_home_core::db::DbPool) {
    sqlx::raw_sql(
        "CREATE TRIGGER fail_plugin_insert BEFORE INSERT ON plugins
         BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
    )
    .execute(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn install_then_upgrade_replaces_the_package() {
    let db = fresh_db().await;
    let id = "upgradeok";
    lifecycle::install_from_dir(&db, &stage(id, "0.1.0", &valid_wasm()), true, &grant_all())
        .await
        .unwrap();
    assert_eq!(registry::get(id).unwrap().manifest.plugin.version, "0.1.0");

    lifecycle::install_from_dir(&db, &stage(id, "0.2.0", &valid_wasm()), true, &grant_all())
        .await
        .unwrap();
    assert_eq!(registry::get(id).unwrap().manifest.plugin.version, "0.2.0");
    assert!(
        registry::get_enabled(id).is_some(),
        "upgrade keeps the enabled state"
    );
    assert!(table_exists(&db, &format!("{id}_items")).await);
}

#[tokio::test]
async fn failed_upgrade_restores_the_previous_package() {
    let db = fresh_db().await;
    let id = "upgradefail";
    lifecycle::install_from_dir(&db, &stage(id, "0.1.0", &valid_wasm()), true, &grant_all())
        .await
        .unwrap();

    break_plugin_rows(&db).await;
    let err =
        lifecycle::install_from_dir(&db, &stage(id, "0.2.0", &valid_wasm()), true, &grant_all())
            .await;
    assert!(err.is_err(), "the forced failure must surface");

    // The loaded plugin and the package on disk are still version 0.1.0.
    assert_eq!(registry::get(id).unwrap().manifest.plugin.version, "0.1.0");
    let on_disk =
        std::fs::read_to_string(data_root().join("plugins").join(id).join("plugin.toml")).unwrap();
    assert!(on_disk.contains("version = \"0.1.0\""), "{on_disk}");
    assert!(
        !data_root()
            .join("plugins")
            .join(format!(".{id}.previous"))
            .exists()
    );
}

#[tokio::test]
async fn failed_fresh_install_leaves_nothing_behind() {
    let db = fresh_db().await;
    let id = "freshfail";
    break_plugin_rows(&db).await;
    assert!(
        lifecycle::install_from_dir(&db, &stage(id, "0.1.0", &valid_wasm()), true, &grant_all())
            .await
            .is_err()
    );

    assert!(registry::get(id).is_none());
    assert!(
        !data_root().join("plugins").join(id).exists(),
        "package dir removed"
    );
    assert!(
        !table_exists(&db, &format!("{id}_items")).await,
        "tables removed"
    );
}

#[tokio::test]
async fn uncompilable_package_is_rejected_before_anything_changes() {
    let db = fresh_db().await;
    let id = "badwasm";
    let err =
        lifecycle::install_from_dir(&db, &stage(id, "0.1.0", b"not wasm"), true, &grant_all())
            .await;
    assert!(err.is_err());
    assert!(registry::get(id).is_none());
    assert!(!table_exists(&db, &format!("{id}_items")).await);
}

#[tokio::test]
async fn uninstall_keeps_or_drops_tables_as_asked() {
    let db = fresh_db().await;
    let keep = "keepdata";
    lifecycle::install_from_dir(
        &db,
        &stage(keep, "0.1.0", &valid_wasm()),
        true,
        &grant_all(),
    )
    .await
    .unwrap();
    lifecycle::uninstall(&db, keep, true).await.unwrap();
    assert!(registry::get(keep).is_none());
    assert!(
        table_exists(&db, &format!("{keep}_items")).await,
        "data retained"
    );

    let drop = "dropdata";
    lifecycle::install_from_dir(
        &db,
        &stage(drop, "0.1.0", &valid_wasm()),
        true,
        &grant_all(),
    )
    .await
    .unwrap();
    let dropped = lifecycle::uninstall(&db, drop, false).await.unwrap();
    assert!(dropped >= 1);
    assert!(
        !table_exists(&db, &format!("{drop}_items")).await,
        "data removed"
    );
}

#[tokio::test]
async fn a_plugin_installed_without_a_grant_has_no_capabilities() {
    let db = fresh_db().await;
    let id = "nogrant";
    // The manifest asks for db; the admin approves nothing.
    lifecycle::install_from_dir(
        &db,
        &stage(id, "0.1.0", &valid_wasm()),
        true,
        &tpt_home_plugins::manifest::Permissions::default(),
    )
    .await
    .unwrap();
    let plugin = registry::get(id).unwrap();
    assert!(plugin.manifest.permissions.db, "requested");
    assert!(
        !plugin.effective_permissions().db,
        "not approved, so denied"
    );

    let stored: (String,) =
        sqlx::query_as("SELECT approved_permissions FROM plugins WHERE id = ?1")
            .bind(id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(stored.0, "[]");
}
