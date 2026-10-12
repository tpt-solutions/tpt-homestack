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

/// Stage a package whose manifest carries the given `[[migrations]]` TOML.
fn stage_with_migrations(id: &str, version: &str, migrations: &str) -> PathBuf {
    let dir = stage(id, version, &valid_wasm());
    let mut text = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    text.push_str(migrations);
    std::fs::write(dir.join("plugin.toml"), text).unwrap();
    dir
}

async fn column_names(db: &tpt_home_core::db::DbPool, table: &str) -> Vec<String> {
    let rows: Vec<(String,)> =
        sqlx::query_as(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .fetch_all(db)
            .await
            .unwrap();
    rows.into_iter().map(|r| r.0).collect()
}

async fn applied_versions(db: &tpt_home_core::db::DbPool, id: &str) -> Vec<i64> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT version FROM plugin_migrations WHERE plugin_id = ?1 ORDER BY version",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .unwrap();
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn fresh_install_runs_every_migration_in_order() {
    let db = fresh_db().await;
    let id = "migr";
    let migrations = r#"
[[migrations]]
version = 1
statements = ["ALTER TABLE migr_items ADD COLUMN note TEXT"]

[[migrations]]
version = 2
statements = ["ALTER TABLE migr_items ADD COLUMN done INTEGER"]
"#;
    lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.1.0", migrations),
        true,
        &grant_all(),
    )
    .await
    .unwrap();
    let cols = column_names(&db, "migr_items").await;
    assert!(
        cols.contains(&"note".to_string()) && cols.contains(&"done".to_string()),
        "{cols:?}"
    );
    assert_eq!(applied_versions(&db, id).await, vec![1, 2]);
}

#[tokio::test]
async fn upgrade_runs_only_the_new_migration() {
    let db = fresh_db().await;
    let id = "migrup";
    let v1 = r#"
[[migrations]]
version = 1
statements = ["ALTER TABLE migrup_items ADD COLUMN note TEXT"]
"#;
    lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.1.0", v1),
        true,
        &grant_all(),
    )
    .await
    .unwrap();

    let v2 = format!(
        "{v1}\n[[migrations]]\nversion = 2\nstatements = [\"ALTER TABLE migrup_items ADD COLUMN done INTEGER\"]\n"
    );
    lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.2.0", &v2),
        true,
        &grant_all(),
    )
    .await
    .unwrap();
    assert_eq!(applied_versions(&db, id).await, vec![1, 2]);
    assert!(
        column_names(&db, "migrup_items")
            .await
            .contains(&"done".to_string())
    );
}

#[tokio::test]
async fn an_edited_migration_refuses_the_upgrade() {
    let db = fresh_db().await;
    let id = "migredit";
    let v1 = r#"
[[migrations]]
version = 1
statements = ["ALTER TABLE migredit_items ADD COLUMN note TEXT"]
"#;
    lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.1.0", v1),
        true,
        &grant_all(),
    )
    .await
    .unwrap();

    // Same version, different SQL: the recorded checksum no longer matches.
    let edited = v1.replace("note TEXT", "note TEXT NOT NULL DEFAULT ''");
    let err = lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.1.1", &edited),
        true,
        &grant_all(),
    )
    .await
    .expect_err("editing an applied migration must be refused");
    assert!(
        format!("{err:?}").contains("changed after it was applied"),
        "{err:?}"
    );
    // The previous version still runs.
    assert_eq!(registry::get(id).unwrap().manifest.plugin.version, "0.1.0");
}

#[tokio::test]
async fn a_failing_migration_leaves_no_partial_schema() {
    let db = fresh_db().await;
    let id = "migrbad";
    // The first statement succeeds; the second names a missing column. The
    // whole migration must roll back, including the first statement.
    let bad = r#"
[[migrations]]
version = 1
statements = [
  "ALTER TABLE migrbad_items ADD COLUMN extra TEXT",
  "ALTER TABLE migrbad_items DROP COLUMN no_such_column"
]
"#;
    let err = lifecycle::install_from_dir(
        &db,
        &stage_with_migrations(id, "0.1.0", bad),
        true,
        &grant_all(),
    )
    .await
    .expect_err("a failing statement must fail the install");
    assert!(format!("{err:?}").contains("migration 1"), "{err:?}");
    assert!(
        !column_names(&db, "migrbad_items")
            .await
            .contains(&"extra".to_string()),
        "partial DDL must roll back"
    );
    assert!(applied_versions(&db, id).await.is_empty());
    assert!(registry::get(id).is_none());
}
