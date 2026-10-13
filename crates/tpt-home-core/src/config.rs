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

    /// Effective session TTL.
    pub fn session_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.session_ttl_hours * 3600)
    }
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
