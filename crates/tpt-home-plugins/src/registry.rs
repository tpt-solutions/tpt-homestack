// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The plugin registry: which plugins are installed and enabled right now.
//!
//! The registry is process-global (like the config) because the router is
//! built once with `DbPool` state — plugin dispatch resolves the plugin at
//! request time, which is what makes hot enable/disable possible without a
//! restart.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use wasmi::Module;

use crate::manifest::{Permissions, PluginManifest};

/// One loaded plugin: manifest, compiled module and package directory.
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub module: Module,
    /// Directory containing `plugin.toml` + the wasm entry.
    pub dir: PathBuf,
    /// Admin-approved capabilities from `plugins.approved_permissions`.
    pub approved: Permissions,
}

impl LoadedPlugin {
    /// The capabilities the runtime grants: requested and approved.
    pub fn effective_permissions(&self) -> Permissions {
        self.manifest.permissions.intersect(&self.approved)
    }

    /// Convenience: namespaced table name `<id>_<name>`.
    pub fn scoped_table(&self, table: &str) -> String {
        format!("{}_{}", self.manifest.plugin.id, table)
    }
}

#[derive(Default)]
struct RegistryInner {
    plugins: HashMap<String, Arc<LoadedPlugin>>,
    enabled: HashSet<String>,
}

static REGISTRY: OnceLock<RwLock<RegistryInner>> = OnceLock::new();

fn registry() -> &'static RwLock<RegistryInner> {
    REGISTRY.get_or_init(|| RwLock::new(RegistryInner::default()))
}

/// Install/replace a loaded plugin in the registry (hot).
pub fn put(plugin: LoadedPlugin, enabled: bool) {
    let id = plugin.manifest.plugin.id.clone();
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    if enabled {
        guard.enabled.insert(id.clone());
    } else {
        guard.enabled.remove(&id);
    }
    guard.plugins.insert(id, Arc::new(plugin));
}

/// Remove a plugin from the registry. Returns whether it was loaded.
pub fn remove(id: &str) -> bool {
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    guard.enabled.remove(id);
    guard.plugins.remove(id).is_some()
}

/// Look up an enabled plugin.
pub fn get_enabled(id: &str) -> Option<Arc<LoadedPlugin>> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    if !guard.enabled.contains(id) {
        return None;
    }
    guard.plugins.get(id).cloned()
}

/// Look up any plugin (enabled or not).
pub fn get(id: &str) -> Option<Arc<LoadedPlugin>> {
    registry()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .plugins
        .get(id)
        .cloned()
}

/// All loaded plugins (for the admin list), sorted by id.
pub fn all() -> Vec<(Arc<LoadedPlugin>, bool)> {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    let mut list: Vec<(Arc<LoadedPlugin>, bool)> = guard
        .plugins
        .values()
        .map(|p| (p.clone(), guard.enabled.contains(&p.manifest.plugin.id)))
        .collect();
    list.sort_by(|a, b| a.0.manifest.plugin.id.cmp(&b.0.manifest.plugin.id));
    list
}

/// Set the enabled flag on a loaded plugin. Returns whether it was loaded.
pub fn set_enabled(id: &str, enabled: bool) -> bool {
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    if !guard.plugins.contains_key(id) {
        return false;
    }
    if enabled {
        guard.enabled.insert(id.to_string());
    } else {
        guard.enabled.remove(id);
    }
    true
}
