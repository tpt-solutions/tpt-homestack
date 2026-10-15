// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Runtime configuration.
//!
//! Resolution order (lowest to highest): built-in defaults, values from
//! `<data_dir>/config.toml`, `TPT_HOME_*` environment variables, CLI
//! overrides. The config file lives inside the data directory so a deployment
//! is fully described by one folder plus the binary.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// Resolved runtime configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root directory for all runtime state (`tpt.db`, `uploads/`, config).
    pub data_dir: PathBuf,
    /// Bind address for the HTTP server.
    pub host: String,
    /// Bind port for the HTTP server.
    pub port: u16,
    /// Emit `Secure` on session cookies (enable behind HTTPS).
    pub secure_cookies: bool,
    /// Session lifetime in hours.
    pub session_ttl_hours: u64,
    /// Per-request upload cap in megabytes (drop blueprint).
    pub max_upload_mb: u64,
    /// Override for the database file (default `<data_dir>/tpt.db`).
    pub db_path: Option<PathBuf>,
    /// Tracing filter (default `info`), same syntax as `RUST_LOG`.
    pub log_level: Option<String>,
    /// Browser origins allowed to call the API with credentials (CORS).
    /// Empty (default) = same-origin only, which the shell needs.
    pub cors_origins: Vec<String>,
    /// Trust `X-Forwarded-For` for rate limiting. Enable only when a reverse
    /// proxy you control (Caddy, cloudflared) is the sole way to reach the
    /// server; otherwise clients can spoof the header.
    pub trust_proxy: bool,
    /// Hours between scheduled backups. 0 (default) turns them off.
    pub backup_every_hours: u64,
    /// Scheduled backups kept; the oldest are deleted. At least 1.
    pub backup_keep: usize,
    /// Announce the server on the local network over mDNS (default true). Only
    /// happens when the server listens beyond loopback.
    pub advertise: bool,
    /// Name announced on the LAN as `<lan_name>.local` (default `homestack`).
    /// One DNS label: letters, digits and hyphens, not starting or ending with
    /// a hyphen, at most 63 characters.
    pub lan_name: String,
    /// Ed25519 public keys (64 hex characters each) whose signatures on plugin
    /// packages are trusted. Empty (default) means no signed package installs.
    pub trusted_plugin_keys: Vec<String>,
    /// Refuse plugin packages that carry no signature (default false).
    pub require_signed_plugins: bool,
    /// Where the release manifest is published (see `update.rs`). Empty
    /// (default) turns update checks off. Checks only read; nothing is installed.
    pub update_url: String,
    /// Refuse plain HTTP: GET and HEAD are redirected to HTTPS, other methods
    /// get a 403. A request counts as HTTPS when it came through a trusted
    /// proxy that sets `X-Forwarded-Proto: https` (needs `trust_proxy`).
    /// Default false, so a home network without certificates keeps working.
    pub require_https: bool,
    /// Address of the plugin catalog index (format in `docs/PLUGIN_INDEX.md`).
    /// Empty (default) turns the catalog off.
    pub catalog_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            host: "127.0.0.1".to_string(),
            port: 8080,
            secure_cookies: false,
            session_ttl_hours: 72,
            max_upload_mb: 512,
            db_path: None,
            log_level: None,
            cors_origins: Vec::new(),
            trust_proxy: false,
            backup_every_hours: 0,
            backup_keep: 7,
            advertise: true,
            lan_name: "homestack".to_string(),
            trusted_plugin_keys: Vec::new(),
            require_signed_plugins: false,
            update_url: String::new(),
            require_https: false,
            catalog_url: String::new(),
        }
    }
}

impl Config {
    /// Load configuration: defaults <- `<data_dir>/config.toml` <- env vars.
    ///
    /// `data_dir_override` (from the CLI) wins over every other source for
    /// the data directory itself.
    pub fn load(data_dir_override: Option<PathBuf>) -> anyhow::Result<Self> {
        let mut cfg = Config::default();

        if let Some(dir) = data_dir_override
            .clone()
            .or_else(|| read_env("DATA_DIR").map(PathBuf::from))
        {
            cfg.data_dir = dir;
        }

        let file = cfg.data_dir.join("config.toml");
        if file.is_file() {
            let raw = std::fs::read_to_string(&file)?;
            let parsed: Config =
                toml::from_str(&raw).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            cfg = parsed;
            // An explicit data_dir override still wins after file parsing.
            if let Some(dir) = data_dir_override {
                cfg.data_dir = dir;
            }
        }

        apply_env(&mut cfg);
        if !valid_lan_name(&cfg.lan_name) {
            anyhow::bail!(
                "lan_name {:?} is not a valid name: use letters, digits and hyphens (1-63 characters, no leading or trailing hyphen)",
                cfg.lan_name
            );
        }
        Ok(cfg)
    }

    /// Path to the SQLite database file.
    pub fn db_path(&self) -> PathBuf {
        self.db_path
            .clone()
            .unwrap_or_else(|| self.data_dir.join("tpt.db"))
    }

    /// Path to the uploads directory.
    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    /// Create the data directory tree if missing.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(self.uploads_dir())?;
        Ok(())
    }

    /// True when the server can be reached from beyond this machine: it
    /// listens on a non-loopback address, or it trusts a proxy (a tunnel).
    /// The UI shows a warning while this holds.
    pub fn exposed(&self) -> bool {
        self.trust_proxy || !matches!(self.host.as_str(), "127.0.0.1" | "localhost" | "::1")
    }

    /// Effective session TTL.
    pub fn session_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.session_ttl_hours * 3600)
    }
}

/// True when `name` is a single DNS label that can be announced on the LAN.
pub fn valid_lan_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn read_env(key: &str) -> Option<String> {
    std::env::var(format!("TPT_HOME_{key}"))
        .ok()
        .filter(|v| !v.is_empty())
}

fn apply_env(cfg: &mut Config) {
    if let Some(v) = read_env("HOST") {
        cfg.host = v;
    }
    if let Some(v) = read_env("PORT").and_then(|v| v.parse().ok()) {
        cfg.port = v;
    }
    if let Some(v) = read_env("SECURE_COOKIES").and_then(|v| v.parse().ok()) {
        cfg.secure_cookies = v;
    }
    if let Some(v) = read_env("TRUST_PROXY").and_then(|v| v.parse().ok()) {
        cfg.trust_proxy = v;
    }
    if let Some(v) = read_env("SESSION_TTL_HOURS").and_then(|v| v.parse().ok()) {
        cfg.session_ttl_hours = v;
    }
    if let Some(v) = read_env("MAX_UPLOAD_MB").and_then(|v| v.parse().ok()) {
        cfg.max_upload_mb = v;
    }
    if let Some(v) = read_env("BACKUP_EVERY_HOURS").and_then(|v| v.parse().ok()) {
        cfg.backup_every_hours = v;
    }
    if let Some(v) = read_env("ADVERTISE").and_then(|v| v.parse().ok()) {
        cfg.advertise = v;
    }
    if let Some(v) = read_env("LAN_NAME") {
        cfg.lan_name = v;
    }
    if let Some(v) = read_env("UPDATE_URL") {
        cfg.update_url = v;
    }
    if let Some(v) = read_env("CATALOG_URL") {
        cfg.catalog_url = v;
    }
    if let Some(v) = read_env("REQUIRE_HTTPS").and_then(|v| v.parse().ok()) {
        cfg.require_https = v;
    }
    if let Some(v) = read_env("BACKUP_KEEP").and_then(|v| v.parse().ok()) {
        cfg.backup_keep = v;
    }
    if let Some(v) = read_env("DB_PATH") {
        cfg.db_path = Some(PathBuf::from(v));
    }
    if let Some(v) = read_env("LOG_LEVEL") {
        cfg.log_level = Some(v);
    }
    if let Some(v) = read_env("CORS_ORIGINS") {
        cfg.cors_origins = v
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
}

static GLOBAL: OnceLock<Config> = OnceLock::new();

/// Install the process-wide config. Returns `false` if a config was already
/// installed (double startup).
pub fn init_global(cfg: Config) -> bool {
    GLOBAL.set(cfg).is_ok()
}

/// Process-wide config, installed by the server before any handler runs.
///
/// Blueprint handlers can use this to reach settings (e.g. the uploads dir)
/// without widening the router state type beyond [`crate::db::DbPool`].
pub fn global() -> &'static Config {
    GLOBAL
        .get()
        .expect("config not initialised; call config::init_global first")
}

/// The global config if it has been set, for callers that can run without it.
pub fn try_global() -> Option<&'static Config> {
    GLOBAL.get()
}

/// Helper for tests that need a global config without a server.
pub fn set_global_for_tests(cfg: Config) {
    let _ = GLOBAL.set(cfg);
}

/// Validate that `path` stays inside `root` and exists under it.
pub fn path_within(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_names_are_single_dns_labels() {
        assert!(valid_lan_name("homestack"));
        assert!(valid_lan_name("home-stack-2"));
        assert!(valid_lan_name(&"a".repeat(63)));
        assert!(!valid_lan_name(""));
        assert!(!valid_lan_name("-home"));
        assert!(!valid_lan_name("home-"));
        assert!(!valid_lan_name("home.stack"));
        assert!(!valid_lan_name("home stack"));
        assert!(!valid_lan_name(&"a".repeat(64)));
        assert_eq!(Config::default().lan_name, "homestack");
        assert!(Config::default().advertise);
    }

    #[test]
    fn defaults_are_lean() {
        let c = Config::default();
        assert_eq!(c.host, "127.0.0.1");
        assert_eq!(c.port, 8080);
        assert!(!c.secure_cookies);
        assert!(!c.trust_proxy);
        assert_eq!(c.db_path(), PathBuf::from("data").join("tpt.db"));
        assert_eq!(c.uploads_dir(), PathBuf::from("data").join("uploads"));
    }

    #[test]
    fn data_dir_override_moves_derived_paths() {
        let c = Config {
            data_dir: PathBuf::from("/srv/tpt"),
            ..Config::default()
        };
        assert_eq!(c.db_path(), PathBuf::from("/srv/tpt/tpt.db"));
        assert_eq!(c.uploads_dir(), PathBuf::from("/srv/tpt/uploads"));
    }
}
