// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plugin lifecycle: install, enable, disable, uninstall, startup loading
//! and the registry persistence table.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use wasmi::Module;

use crate::manifest::{self, PluginManifest};
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
pub async fn install_from_dir(
    db: &SqlitePool,
    staged_dir: &Path,
    enable: bool,
) -> Result<PluginManifest, anyhow::Error> {
    let manifest = manifest::from_dir(staged_dir)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let dest = plugin_dir(&manifest.plugin.id);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(staged_dir, &dest)?;

    create_manifest_tables(db, &manifest).await?;

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
    .bind(manifest.permissions.to_json())
    .bind(now)
    .execute(db)
    .await?;

    load_plugin_dir(&dest, enable)?;
    Ok(manifest)
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

/// Compile + register a plugin from its installed directory.
fn load_plugin_dir(dir: &Path, enabled: bool) -> Result<(), anyhow::Error> {
    let manifest = manifest::from_dir(dir)?;
    let wasm = std::fs::read(dir.join(&manifest.plugin.entry))?;
    let module = Module::new(crate::runtime::engine(), wasm.as_slice())
        .map_err(|e| anyhow::anyhow!("compile {}: {e}", manifest.plugin.entry))?;
    crate::registry::put(
        registry::LoadedPlugin {
            manifest,
            module,
            dir: dir.to_path_buf(),
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

    let rows: Vec<(String, bool)> = sqlx::query_as("SELECT id, enabled FROM plugins ORDER BY id")
        .fetch_all(db)
        .await?;

    let mut loaded = 0;
    for (id, enabled) in rows {
        let dir = plugin_dir(&id);
        if !dir.is_dir() {
            tracing::warn!(plugin = %id, "registered plugin directory missing");
            continue;
        }
        match load_plugin_dir(&dir, enabled) {
            Ok(()) => loaded += 1,
            Err(e) => tracing::error!(plugin = %id, error = %e, "plugin failed to load"),
        }
    }
    tracing::info!(loaded, "plugins loaded");
    Ok(loaded)
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

    let dropped = if keep_data {
        0
    } else {
        sqlx::query("DELETE FROM plugin_kv WHERE plugin_id = ?1")
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
