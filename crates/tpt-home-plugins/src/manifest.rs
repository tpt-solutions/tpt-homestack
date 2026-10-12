// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plugin manifests (`plugin.toml`): parsing and validation.
//!
//! The manifest is the plugin's contract with the host: identity, the wasm
//! entry binary, declared capabilities (granted by the admin at install
//! time), resource limits, host-created tables and declarative pages.

use serde::Deserialize;
use std::path::Path;

/// Current plugin ABI version understood by this host.
pub const SUPPORTED_API_VERSION: u32 = 1;

/// `[plugin]` section.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PluginMeta {
    /// Short id, lowercase letters/digits/underscore. Doubles as the DB and
    /// route namespace: `/api/<id>/…`, tables `<id>_<name>`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Plugin semver (informational).
    pub version: String,
    /// ABI version; must equal [`SUPPORTED_API_VERSION`].
    #[serde(default = "default_api_version")]
    pub api_version: u32,
    /// Wasm file name inside the package (e.g. `plugin.wasm`).
    pub entry: String,
}

fn default_api_version() -> u32 {
    SUPPORTED_API_VERSION
}

/// `[permissions]` section. Everything defaults to *denied*; the admin sees
/// these at install time and the runtime enforces the approved set.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Permissions {
    /// SQL access, scoped to `<id>_*` tables only.
    pub db: bool,
    /// Key/value store scoped to the plugin.
    pub kv: bool,
    /// File storage scoped to the plugin's data directory.
    pub storage: bool,
}

impl Permissions {
    /// Serialize as a sorted JSON array for the `plugins.approved_permissions`
    /// column.
    pub fn to_json(&self) -> String {
        let mut granted = Vec::new();
        if self.db {
            granted.push("db");
        }
        if self.kv {
            granted.push("kv");
        }
        if self.storage {
            granted.push("storage");
        }
        serde_json::to_string(&granted).expect("permission list serialization")
    }

    /// Parse the `plugins.approved_permissions` JSON array. Unknown entries
    /// and malformed JSON grant nothing, so a corrupt row fails closed.
    pub fn from_json(text: &str) -> Self {
        let names: Vec<String> = serde_json::from_str(text).unwrap_or_default();
        let has = |name: &str| names.iter().any(|n| n == name);
        Self {
            db: has("db"),
            kv: has("kv"),
            storage: has("storage"),
        }
    }

    /// Capabilities that are both requested by the manifest and approved by
    /// the admin. The runtime uses this, never the manifest alone.
    pub fn intersect(&self, approved: &Self) -> Self {
        Self {
            db: self.db && approved.db,
            kv: self.kv && approved.kv,
            storage: self.storage && approved.storage,
        }
    }
}

/// `[limits]` section. Defaults are deliberately modest.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct Limits {
    /// Wasm fuel per invocation (roughly instructions/10k).
    pub max_fuel: u64,
    /// Linear memory cap in MiB per instance.
    pub max_memory_mb: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_fuel: 50_000_000,
            max_memory_mb: 16,
        }
    }
}

/// A host-created table (`[[tables]]`); created namespaced as `<id>_<name>`
/// when the plugin is installed/enabled.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TableDef {
    /// Table name *without* the plugin prefix.
    pub name: String,
    pub columns: Vec<ColumnDef>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    /// SQLite-ish column kind: `text`, `integer`, `real` or `bool`.
    pub kind: String,
}

/// A declarative page (`[[pages]]`), rendered by the host shell under
/// `/plugins/<id><path>`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PageDef {
    /// Page path below the plugin mount, e.g. `/events`.
    pub path: String,
    /// Page + sidebar title.
    pub title: String,
    /// `table` (read-only rows from a scoped table) or `form` (posts JSON to
    /// the plugin API).
    pub kind: String,
    /// `table` pages: scoped table name (without plugin prefix).
    #[serde(default)]
    pub table: Option<String>,
    /// `table` pages: columns to show.
    #[serde(default)]
    pub columns: Vec<String>,
    /// `form` pages: field names.
    #[serde(default)]
    pub fields: Vec<String>,
    /// `form` pages: API path that receives the POST (below `/api/<id>`).
    #[serde(default)]
    pub action: Option<String>,
}

/// Parsed `plugin.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub tables: Vec<TableDef>,
    #[serde(default)]
    pub pages: Vec<PageDef>,
}

/// Parse and validate a manifest from TOML text.
pub fn parse(text: &str) -> Result<PluginManifest, anyhow::Error> {
    let manifest: PluginManifest =
        toml::from_str(text).map_err(|e| anyhow::anyhow!("invalid plugin.toml: {e}"))?;
    validate(&manifest)?;
    Ok(manifest)
}

/// Parse a manifest from a package directory.
pub fn from_dir(dir: &Path) -> Result<PluginManifest, anyhow::Error> {
    let text = std::fs::read_to_string(dir.join("plugin.toml"))
        .map_err(|e| anyhow::anyhow!("{}: {e}", dir.join("plugin.toml").display()))?;
    parse(&text)
}

fn validate(m: &PluginManifest) -> Result<(), anyhow::Error> {
    let id = &m.plugin.id;
    let ok = (1..=48usize).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !id.starts_with(|c: char| c.is_ascii_digit());
    if !ok {
        anyhow::bail!("plugin id '{id}' must be 1-48 chars of lowercase letters/digits/'_'");
    }
    if m.plugin.name.trim().is_empty() || m.plugin.name.len() > 96 {
        anyhow::bail!("plugin name must be 1-96 characters");
    }
    if m.plugin.api_version != SUPPORTED_API_VERSION {
        anyhow::bail!(
            "plugin api_version {} not supported (host supports {})",
            m.plugin.api_version,
            SUPPORTED_API_VERSION
        );
    }
    if m.plugin.entry.is_empty()
        || !m
            .plugin
            .entry
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        anyhow::bail!("entry must be a plain file name like 'plugin.wasm'");
    }

    for table in &m.tables {
        valid_component(&table.name, "table name")?;
        if table.columns.is_empty() || table.columns.len() > 64 {
            anyhow::bail!("table '{}' needs 1-64 columns", table.name);
        }
        let mut seen = std::collections::HashSet::new();
        for col in &table.columns {
            valid_component(&col.name, "column name")?;
            if !matches!(col.kind.as_str(), "text" | "integer" | "real" | "bool") {
                anyhow::bail!(
                    "column '{}.{}' kind must be text|integer|real|bool",
                    table.name,
                    col.name
                );
            }
            if !seen.insert(&col.name) {
                anyhow::bail!("duplicate column '{}' in table '{}'", col.name, table.name);
            }
        }
    }

    for page in &m.pages {
        if !page.path.starts_with('/') || page.path.contains("..") || page.path.contains('?') {
            anyhow::bail!("page path '{}' must start with '/' and be plain", page.path);
        }
        match page.kind.as_str() {
            "table" => {
                let Some(table) = &page.table else {
                    anyhow::bail!("table page '{}' needs `table`", page.path);
                };
                valid_component(table, "table name")?;
            }
            "form" => {
                if page.action.is_none() {
                    anyhow::bail!("form page '{}' needs `action`", page.path);
                }
            }
            other => anyhow::bail!("page '{}' kind '{other}' must be table|form", page.path),
        }
    }

    if m.limits.max_memory_mb == 0 || m.limits.max_memory_mb > 256 {
        anyhow::bail!("limits.max_memory_mb must be 1-256");
    }
    if m.limits.max_fuel < 1000 {
        anyhow::bail!("limits.max_fuel must be at least 1000");
    }
    Ok(())
}

fn valid_component(name: &str, what: &str) -> Result<(), anyhow::Error> {
    let ok = (1..=64usize).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "{what} '{name}' must be 1-64 chars: lowercase letters, digits, '_', leading letter"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [plugin]
        id = "calendar"
        name = "Calendar"
        version = "0.1.0"
        entry = "plugin.wasm"

        [permissions]
        db = true

        [[tables]]
        name = "events"
        [[tables.columns]]
        name = "title"
        kind = "text"
        [[tables.columns]]
        name = "date"
        kind = "text"

        [[pages]]
        path = "/events"
        title = "Events"
        kind = "table"
        table = "events"
        columns = ["title", "date"]
    "#;

    #[test]
    fn parses_sample() {
        let m = parse(SAMPLE).unwrap();
        assert_eq!(m.plugin.id, "calendar");
        assert!(m.permissions.db);
        assert!(!m.permissions.kv);
        assert_eq!(m.tables.len(), 1);
        assert_eq!(m.pages.len(), 1);
        assert_eq!(m.permissions.to_json(), "[\"db\"]");
    }

    #[test]
    fn approved_permissions_round_trip_and_fail_closed() {
        let all = Permissions {
            db: true,
            kv: true,
            storage: true,
        };
        assert_eq!(Permissions::from_json(&all.to_json()), all);
        assert_eq!(Permissions::from_json("not json"), Permissions::default());
        assert_eq!(
            Permissions::from_json("[\"bogus\"]"),
            Permissions::default()
        );
    }

    #[test]
    fn effective_permissions_need_request_and_approval() {
        let requested = Permissions {
            db: true,
            kv: true,
            storage: false,
        };
        let approved = Permissions {
            db: true,
            kv: false,
            storage: true,
        };
        // db is requested and approved; kv is requested but not approved;
        // storage is approved but never requested.
        let effective = requested.intersect(&approved);
        assert_eq!(
            effective,
            Permissions {
                db: true,
                kv: false,
                storage: false
            }
        );
    }

    #[test]
    fn rejects_bad_ids_and_versions() {
        let bad_id = SAMPLE.replace("id = \"calendar\"", "id = \"Bad Id!\"");
        assert!(parse(&bad_id).is_err());

        let bad_api = SAMPLE.replace(
            "entry = \"plugin.wasm\"",
            "api_version = 99\n        entry = \"plugin.wasm\"",
        );
        assert!(parse(&bad_api).is_err());
    }

    #[test]
    fn rejects_bad_kinds_and_paths() {
        let bad_kind = SAMPLE.replace("kind = \"text\"", "kind = \"blob\"");
        assert!(parse(&bad_kind).is_err());

        let bad_page = SAMPLE.replace("path = \"/events\"", "path = \"events/../..\"");
        assert!(parse(&bad_page).is_err());
    }
}
