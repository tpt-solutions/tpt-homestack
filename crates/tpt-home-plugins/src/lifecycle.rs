// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plugin lifecycle: install, enable, disable, uninstall, startup loading
//! and the registry persistence table.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use wasmi::Module;

use crate::manifest::{self, Permissions, PluginManifest, Runtime};
use crate::{package, registry};

inventory::submit! {
    tpt_home_core::MigrationEntry {
        key: "tpt-home-plugins:001_registry",
        sql: r#"
            CREATE TABLE IF NOT EXISTS plugins (
                id                   TEXT    PRIMARY KEY,
                name                 TEXT    NOT NULL,
                version              TEXT    NOT NULL,
                enabled              INTEGER NOT NULL DEFAULT 0,
                approved_permissions TEXT    NOT NULL DEFAULT '[]',
                installed_at         INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_kv (
                plugin_id TEXT NOT NULL,
                key       TEXT NOT NULL,
                value     TEXT NOT NULL,
                PRIMARY KEY (plugin_id, key)
            );
        "#,
    }
}

inventory::submit! {
    tpt_home_core::MigrationEntry {
        key: "tpt-home-plugins:004_notification_channels",
        sql: r#"
            CREATE TABLE IF NOT EXISTS notification_channels (
                name         TEXT    PRIMARY KEY,
                url          TEXT    NOT NULL,
                created_at   INTEGER NOT NULL,
                last_sent_at INTEGER,
                last_error   TEXT
            );
        "#,
    }
}

inventory::submit! {
    tpt_home_core::MigrationEntry {
        key: "tpt-home-plugins:003_plugin_hooks",
        sql: r#"
            CREATE TABLE IF NOT EXISTS plugin_hooks (
                plugin_id        TEXT    NOT NULL,
                name             TEXT    NOT NULL,
                secret           TEXT    NOT NULL,
                created_at       INTEGER NOT NULL,
                last_received_at INTEGER,
                PRIMARY KEY (plugin_id, name)
            );
        "#,
    }
}

inventory::submit! {
    tpt_home_core::MigrationEntry {
        key: "tpt-home-plugins:002_plugin_migrations",
        sql: r#"
            CREATE TABLE IF NOT EXISTS plugin_migrations (
                plugin_id  TEXT    NOT NULL,
                version    INTEGER NOT NULL,
                checksum   TEXT    NOT NULL,
                applied_at INTEGER NOT NULL,
                PRIMARY KEY (plugin_id, version)
            );
        "#,
    }
}

/// Checksum of a migration's statements. Recorded when it runs, so an edited
/// migration is detected instead of silently skipped.
fn migration_checksum(statements: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for statement in statements {
        hasher.update(statement.as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

/// Run the manifest's pending migrations inside `tx`, in version order.
/// Already-applied versions are skipped after their checksum is verified.
/// Returns how many ran.
async fn apply_migrations(
    conn: &mut sqlx::SqliteConnection,
    manifest: &PluginManifest,
    now: i64,
) -> Result<usize, anyhow::Error> {
    let id = &manifest.plugin.id;
    let applied: Vec<(i64, String)> =
        sqlx::query_as("SELECT version, checksum FROM plugin_migrations WHERE plugin_id = ?1")
            .bind(id)
            .fetch_all(&mut *conn)
            .await?;

    let mut ran = 0;
    for migration in &manifest.migrations {
        let checksum = migration_checksum(&migration.statements);
        if let Some((_, recorded)) = applied
            .iter()
            .find(|(version, _)| *version == i64::from(migration.version))
        {
            if *recorded != checksum {
                anyhow::bail!(
                    "migration {} changed after it was applied; add a new migration instead",
                    migration.version
                );
            }
            continue;
        }
        for statement in &migration.statements {
            sqlx::query(statement.as_str())
                .execute(&mut *conn)
                .await
                .map_err(|e| anyhow::anyhow!("migration {}: {e}", migration.version))?;
        }
        sqlx::query(
            "INSERT INTO plugin_migrations (plugin_id, version, checksum, applied_at)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(id)
        .bind(i64::from(migration.version))
        .bind(&checksum)
        .bind(now)
        .execute(&mut *conn)
        .await?;
        ran += 1;
    }
    Ok(ran)
}

/// Register the runtime nav provider (one sidebar entry per enabled
/// plugin's declarative pages). Called once at startup.
fn install_nav_provider() {
    tpt_home_core::registry::set_nav_provider(|| {
        crate::registry::all()
            .iter()
            .filter(|(_, enabled)| *enabled)
            .flat_map(|(plugin, _)| {
                plugin.manifest.pages.iter().map(move |page| {
                    (
                        page.title.clone(),
                        plugin_page_path(&plugin.manifest.plugin.id, &page.path),
                    )
                })
            })
            .collect()
    });
}

pub fn plugin_page_path(plugin_id: &str, page_path: &str) -> String {
    format!("/plugins/{plugin_id}{}", page_path.trim_end_matches('/'))
}

/// Directory containing all installed plugins.
pub fn plugins_root() -> PathBuf {
    tpt_home_core::config::global().data_dir.join("plugins")
}

fn plugin_dir(id: &str) -> PathBuf {
    plugins_root().join(id)
}

/// Record + load a freshly unpacked package. Creates the manifest-declared
/// tables and (optionally) enables the plugin immediately.
///
/// The module is compiled before anything on disk or in the database is
/// touched, so a broken package leaves the previous install intact. Any
/// later failure rolls the package directory and database row back.
pub async fn install_from_dir(
    db: &SqlitePool,
    staged_dir: &Path,
    enable: bool,
    granted: &Permissions,
) -> Result<PluginManifest, anyhow::Error> {
    let manifest = manifest::from_dir(staged_dir)?;
    let backend = load_backend(staged_dir, &manifest)?;
    let id = manifest.plugin.id.clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM plugins WHERE id = ?1")
        .bind(&id)
        .fetch_optional(db)
        .await?;
    let is_upgrade = existing.is_some();

    // Keep the previous package aside so a failed upgrade can restore it.
    let dest = plugin_dir(&id);
    let backup = plugins_root().join(format!(".{id}.previous"));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    if dest.exists() {
        std::fs::rename(&dest, &backup)?;
    }
    std::fs::rename(staged_dir, &dest)?;

    // Install-time approval: the admin names what to grant. Only what the
    // manifest requests can be approved, and runtime enforces the intersection.
    let approved = granted.intersect(&manifest.permissions);
    // Migrations and the registry row commit together: a failed migration
    // leaves no half-applied schema and no row.
    let applied = async {
        create_manifest_tables(db, &manifest).await?;
        let mut tx = db.begin().await?;
        apply_migrations(&mut tx, &manifest, now).await?;
        sqlx::query(
            "INSERT INTO plugins (id, name, version, enabled, approved_permissions, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                enabled = excluded.enabled,
                approved_permissions = excluded.approved_permissions,
                installed_at = excluded.installed_at",
        )
        .bind(&manifest.plugin.id)
        .bind(&manifest.plugin.name)
        .bind(&manifest.plugin.version)
        .bind(enable)
        .bind(approved.to_json())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Err(e) = applied {
        rollback_install(db, &id, &dest, &backup, is_upgrade).await;
        return Err(e);
    }

    registry::put(
        registry::LoadedPlugin {
            manifest: manifest.clone(),
            backend,
            dir: dest.clone(),
            approved,
        },
        enable,
    );
    if backup.exists() {
        std::fs::remove_dir_all(&backup)?;
    }
    Ok(manifest)
}

/// Undo a failed install: restore the previous package for an upgrade, or
/// remove the new package and its tables and row for a fresh install.
async fn rollback_install(db: &SqlitePool, id: &str, dest: &Path, backup: &Path, is_upgrade: bool) {
    if dest.exists()
        && let Err(e) = std::fs::remove_dir_all(dest)
    {
        tracing::error!(plugin = %id, error = %e, "rollback: could not remove new package");
    }
    if is_upgrade {
        if backup.exists()
            && let Err(e) = std::fs::rename(backup, dest)
        {
            tracing::error!(plugin = %id, error = %e, "rollback: could not restore previous package");
        }
        return;
    }
    // Fresh install: nothing existed before, so every table and row is ours.
    let _ = sqlx::query("DELETE FROM plugin_migrations WHERE plugin_id = ?1")
        .bind(id)
        .execute(db)
        .await;
    if let Err(e) = drop_plugin_tables(db, id).await {
        tracing::error!(plugin = %id, error = %e, "rollback: could not drop tables");
    }
    if let Err(e) = sqlx::query("DELETE FROM plugins WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await
    {
        tracing::error!(plugin = %id, error = %e, "rollback: could not delete row");
    }
}

/// Create the manifest-declared tables (`<id>_<name>`), idempotently.
pub async fn create_manifest_tables(
    db: &SqlitePool,
    manifest: &PluginManifest,
) -> Result<(), anyhow::Error> {
    for table in &manifest.tables {
        let mut cols = String::from("id INTEGER PRIMARY KEY");
        for col in &table.columns {
            let sql_type = match col.kind.as_str() {
                "text" => "TEXT",
                "integer" => "INTEGER",
                "real" => "REAL",
                "bool" => "INTEGER",
                _ => continue, // manifest validation already rejected this
            };
            // Column names come from the validated manifest (lowercase,
            // charset-checked) and are still quote-doubled here.
            let quoted = format!("\"{}\"", col.name.replace('"', "\"\""));
            cols.push_str(&format!(", {quoted} {sql_type}"));
        }
        let table_name = format!("{}_{}", manifest.plugin.id, table.name);
        let quoted_table = format!("\"{table_name}\"");
        sqlx::raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {quoted_table} ({cols})"
        ))
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Remove plugin data tables (`<id>_*`). Only touches tables whose names
/// match the plugin prefix exactly.
pub async fn drop_plugin_tables(db: &SqlitePool, plugin_id: &str) -> Result<u32, anyhow::Error> {
    let prefix = format!("{plugin_id}_");
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE ?1 || '%'",
    )
    .bind(&prefix)
    .fetch_all(db)
    .await?;

    let mut dropped = 0;
    for (name,) in rows {
        // Exact prefix + charset check before any DDL.
        if !name.starts_with(&prefix)
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            continue;
        }
        let quoted = format!("\"{name}\"");
        sqlx::raw_sql(&format!("DROP TABLE IF EXISTS {quoted}"))
            .execute(db)
            .await?;
        dropped += 1;
    }
    Ok(dropped)
}

/// Build the execution backend for an installed or staged package, compiling
/// or validating its entry file. Failing here, before anything is recorded,
/// is what keeps a broken package from being installed.
fn load_backend(dir: &Path, manifest: &PluginManifest) -> Result<registry::Backend, anyhow::Error> {
    match manifest.plugin.runtime {
        Runtime::Wasm => {
            let wasm = std::fs::read(dir.join(&manifest.plugin.entry))?;
            let module = Module::new(crate::runtime::engine(), wasm.as_slice())
                .map_err(|e| anyhow::anyhow!("compile {}: {e}", manifest.plugin.entry))?;
            Ok(registry::Backend::Wasm(module))
        }
        Runtime::Js => script_backend(dir, manifest),
        Runtime::External => Ok(registry::Backend::External(
            manifest.plugin.entry.trim_end_matches('/').to_string(),
        )),
    }
}

#[cfg(feature = "js")]
fn script_backend(
    dir: &Path,
    manifest: &PluginManifest,
) -> Result<registry::Backend, anyhow::Error> {
    let script = crate::script::ScriptSource::load(dir, manifest)?;
    Ok(registry::Backend::Script(std::sync::Arc::new(script)))
}

#[cfg(not(feature = "js"))]
fn script_backend(
    _dir: &Path,
    _manifest: &PluginManifest,
) -> Result<registry::Backend, anyhow::Error> {
    anyhow::bail!("runtime 'js' needs a build with the 'js' feature")
}

/// Load + register a plugin from its installed directory.
fn load_plugin_dir(dir: &Path, enabled: bool, approved: &Permissions) -> Result<(), anyhow::Error> {
    let manifest = manifest::from_dir(dir)?;
    let backend = load_backend(dir, &manifest)?;
    crate::registry::put(
        registry::LoadedPlugin {
            manifest,
            backend,
            dir: dir.to_path_buf(),
            approved: approved.clone(),
        },
        enabled,
    );
    Ok(())
}

/// Load all installed plugins at server startup; enabled ones become
/// callable immediately.
pub async fn load_installed(db: &SqlitePool) -> Result<usize, anyhow::Error> {
    install_nav_provider();
    std::fs::create_dir_all(plugins_root())?;

    let rows: Vec<(String, bool, String)> =
        sqlx::query_as("SELECT id, enabled, approved_permissions FROM plugins ORDER BY id")
            .fetch_all(db)
            .await?;

    let mut loaded = 0;
    for (id, enabled, approved_json) in rows {
        let dir = plugin_dir(&id);
        if !dir.is_dir() {
            tracing::warn!(plugin = %id, "registered plugin directory missing");
            continue;
        }
        let approved = Permissions::from_json(&approved_json);
        match load_plugin_dir(&dir, enabled, &approved) {
            Ok(()) => loaded += 1,
            Err(e) => tracing::error!(plugin = %id, error = %e, "plugin failed to load"),
        }
    }
    tracing::info!(loaded, "plugins loaded");
    Ok(loaded)
}

/// Whether an installed plugin is currently enabled. Unknown plugins are not.
pub async fn is_enabled(db: &SqlitePool, id: &str) -> Result<bool, anyhow::Error> {
    let row: Option<(bool,)> = sqlx::query_as("SELECT enabled FROM plugins WHERE id = ?1")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.is_some_and(|(enabled,)| enabled))
}

/// Enable a plugin (hot). Recreates manifest tables if missing.
pub async fn enable(db: &SqlitePool, id: &str) -> Result<(), anyhow::Error> {
    let Some(plugin) = registry::get(id) else {
        anyhow::bail!("plugin '{id}' is not loaded");
    };
    create_manifest_tables(db, &plugin.manifest).await?;
    sqlx::query("UPDATE plugins SET enabled = 1 WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    registry::set_enabled(id, true);
    Ok(())
}

/// Disable a plugin (hot). Data is kept.
pub async fn disable(db: &SqlitePool, id: &str) -> Result<(), anyhow::Error> {
    sqlx::query("UPDATE plugins SET enabled = 0 WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    registry::set_enabled(id, false);
    Ok(())
}

/// Uninstall: remove from registry, delete the directory and (optionally)
/// the data tables + kv entries.
pub async fn uninstall(db: &SqlitePool, id: &str, keep_data: bool) -> Result<u32, anyhow::Error> {
    registry::remove(id);
    let dir = plugin_dir(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    sqlx::query("DELETE FROM plugins WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    // Signing secrets go even when data is kept: a removed plugin must not
    // keep accepting signed requests.
    sqlx::query("DELETE FROM plugin_hooks WHERE plugin_id = ?1")
        .bind(id)
        .execute(db)
        .await?;

    let dropped = if keep_data {
        0
    } else {
        sqlx::query("DELETE FROM plugin_kv WHERE plugin_id = ?1")
            .bind(id)
            .execute(db)
            .await?;
        // Migration history goes with the data. Kept data keeps its history,
        // so a reinstall does not run migrations against tables it already has.
        sqlx::query("DELETE FROM plugin_migrations WHERE plugin_id = ?1")
            .bind(id)
            .execute(db)
            .await?;
        drop_plugin_tables(db, id).await?
    };
    Ok(dropped)
}

/// Unpack a `.tptp` into a staging directory (caller installs from there).
pub fn stage_package(archive: &[u8]) -> Result<(PathBuf, PluginManifest), anyhow::Error> {
    let staging = plugins_root().join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&staging)?;
    let manifest = match package::unpack_verify(archive, &staging) {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    Ok((staging, manifest))
}

inventory::submit! {
    tpt_home_core::MigrationEntry {
        key: "tpt-home-plugins:005_email_settings",
        sql: r#"
            CREATE TABLE IF NOT EXISTS email_settings (
                id          INTEGER PRIMARY KEY CHECK (id = 1),
                host        TEXT    NOT NULL,
                port        INTEGER NOT NULL,
                security    TEXT    NOT NULL,
                username    TEXT    NOT NULL DEFAULT '',
                password    TEXT    NOT NULL DEFAULT '',
                from_addr   TEXT    NOT NULL,
                to_addr     TEXT    NOT NULL,
                updated_at  INTEGER NOT NULL
            );
        "#,
    }
}
