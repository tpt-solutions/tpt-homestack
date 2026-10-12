// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Capability-checked host services shared by every plugin runtime.
//!
//! The wasm import closures in [`crate::runtime`] are thin wrappers around
//! [`HostServices`], so the JS and external tiers get the same permission
//! checks, scope rules and size caps.
//!
//! Failure contract (ABI v1):
//! - A denied capability, an out-of-scope statement, a missing file or any
//!   other ordinary failure is [`HostError::Failed`]. The guest sees `-1`.
//! - A call made after the per-call deadline is [`HostError::Deadline`]. The
//!   guest is trapped and the whole invocation fails.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::manifest::Permissions;
use crate::scope;

/// Maximum log lines captured per invocation.
pub const MAX_LOG_LINES: usize = 100;
/// Maximum size of any string or buffer crossing the boundary.
pub const MAX_STRING_RESULT: u32 = 4 * 1024 * 1024;
/// Maximum rows returned by one query.
pub const MAX_QUERY_ROWS: usize = 1_000;
/// Upper bound on how long SQLite waits for a lock inside one call.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a host service call did not succeed.
#[derive(Debug)]
pub enum HostError {
    /// Denied, out of scope, or failed. The guest receives `-1`.
    Failed(String),
    /// The invocation's wall-clock deadline has passed. The guest is trapped.
    Deadline,
}

pub type HostResult<T> = Result<T, HostError>;

fn failed(e: impl std::fmt::Display) -> HostError {
    HostError::Failed(e.to_string())
}

/// The services one plugin invocation may use. Created per call, so the
/// SQLite handle and the deadline never outlive the invocation.
pub struct HostServices {
    plugin_id: String,
    perms: Permissions,
    db_path: PathBuf,
    storage_dir: PathBuf,
    deadline: Instant,
    conn: Option<rusqlite::Connection>,
    logs: Vec<String>,
}

impl HostServices {
    /// `perms` must already be the effective set (requested and approved).
    pub fn new(
        plugin_id: &str,
        perms: &Permissions,
        db_path: &Path,
        storage_dir: &Path,
        deadline: Instant,
    ) -> Self {
        Self {
            plugin_id: plugin_id.to_string(),
            perms: perms.clone(),
            db_path: db_path.to_path_buf(),
            storage_dir: storage_dir.to_path_buf(),
            deadline,
            conn: None,
            logs: Vec::new(),
        }
    }

    /// Log lines captured so far (bounded by [`MAX_LOG_LINES`]).
    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    fn check_deadline(&self) -> HostResult<()> {
        if Instant::now() >= self.deadline {
            Err(HostError::Deadline)
        } else {
            Ok(())
        }
    }

    fn require(&self, granted: bool, capability: &str) -> HostResult<()> {
        if granted {
            Ok(())
        } else {
            Err(HostError::Failed(format!(
                "capability '{capability}' not granted"
            )))
        }
    }

    fn check_size(len: usize) -> HostResult<()> {
        if len > MAX_STRING_RESULT as usize {
            Err(HostError::Failed("payload exceeds size cap".into()))
        } else {
            Ok(())
        }
    }

    fn conn(&mut self) -> HostResult<&mut rusqlite::Connection> {
        if self.conn.is_none() {
            let conn = rusqlite::Connection::open(&self.db_path).map_err(failed)?;
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            conn.busy_timeout(remaining.min(BUSY_TIMEOUT))
                .map_err(failed)?;
            self.conn = Some(conn);
        }
        Ok(self.conn.as_mut().expect("just set"))
    }

    /// `tpt_log`: always allowed, bounded in count and length.
    pub fn log(&mut self, message: String) -> HostResult<()> {
        self.check_deadline()?;
        tracing::debug!(target: "plugin", plugin = %self.plugin_id, "{message}");
        if self.logs.len() < MAX_LOG_LINES {
            self.logs.push(message);
        }
        Ok(())
    }

    /// Run one scoped SELECT and return rows as JSON objects.
    pub fn db_query(&mut self, sql: &str) -> HostResult<Vec<Map<String, Value>>> {
        self.check_deadline()?;
        self.require(self.perms.db, "db")?;
        let sql = scope::validate_scoped_sql(&self.plugin_id, sql).map_err(failed)?;
        let conn = self.conn()?;

        let mut stmt = conn.prepare(&sql).map_err(failed)?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let mut rows_json: Vec<Map<String, Value>> = Vec::new();
        let mut rows = stmt.query([]).map_err(failed)?;
        while let Some(row) = rows.next().map_err(failed)? {
            let mut obj = Map::new();
            for (i, name) in col_names.iter().enumerate() {
                let value = match row.get_ref(i).map_err(failed)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => v.into(),
                    rusqlite::types::ValueRef::Real(v) => v.into(),
                    rusqlite::types::ValueRef::Text(t) => {
                        Value::String(String::from_utf8_lossy(t).into_owned())
                    }
                    rusqlite::types::ValueRef::Blob(b) => {
                        Value::String(format!("<blob {} bytes>", b.len()))
                    }
                };
                obj.insert(name.clone(), value);
            }
            rows_json.push(obj);
            if rows_json.len() >= MAX_QUERY_ROWS {
                break; // result cap per query
            }
        }
        Ok(rows_json)
    }

    /// Run one scoped INSERT/UPDATE/DELETE/DDL and return rows affected.
    pub fn db_execute(&mut self, sql: &str) -> HostResult<usize> {
        self.check_deadline()?;
        self.require(self.perms.db, "db")?;
        let sql = scope::validate_scoped_sql(&self.plugin_id, sql).map_err(failed)?;
        let conn = self.conn()?;
        conn.execute(&sql, []).map_err(failed)
    }

    /// Read a key from the plugin's key/value store. `None` when absent.
    pub fn kv_get(&mut self, key: &str) -> HostResult<Option<String>> {
        self.check_deadline()?;
        self.require(self.perms.kv, "kv")?;
        let plugin_id = self.plugin_id.clone();
        let conn = self.conn()?;
        let value = conn
            .query_row(
                "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                rusqlite::params![plugin_id, key],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(failed)?;
        Ok(value)
    }

    /// Write a key in the plugin's key/value store.
    pub fn kv_set(&mut self, key: &str, value: &str) -> HostResult<()> {
        self.check_deadline()?;
        self.require(self.perms.kv, "kv")?;
        Self::check_size(key.len() + value.len())?;
        let plugin_id = self.plugin_id.clone();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO plugin_kv (plugin_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(plugin_id, key) DO UPDATE SET value = excluded.value",
            rusqlite::params![plugin_id, key, value],
        )
        .map_err(failed)?;
        Ok(())
    }

    /// Delete a key from the plugin's key/value store.
    pub fn kv_del(&mut self, key: &str) -> HostResult<()> {
        self.check_deadline()?;
        self.require(self.perms.kv, "kv")?;
        let plugin_id = self.plugin_id.clone();
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            rusqlite::params![plugin_id, key],
        )
        .map_err(failed)?;
        Ok(())
    }

    /// Read a file from the plugin's storage directory. A missing file is a
    /// failure, which the guest sees as `-1`.
    pub fn storage_read(&mut self, name: &str) -> HostResult<Vec<u8>> {
        self.check_deadline()?;
        self.require(self.perms.storage, "storage")?;
        let path = tpt_home_core::files::safe_join(&self.storage_dir, &[name]).map_err(failed)?;
        let bytes = std::fs::read(path).map_err(failed)?;
        Self::check_size(bytes.len())?;
        Ok(bytes)
    }

    /// Write a UTF-8 file into the plugin's storage directory.
    pub fn storage_write(&mut self, name: &str, data: &str) -> HostResult<()> {
        self.check_deadline()?;
        self.require(self.perms.storage, "storage")?;
        Self::check_size(data.len())?;
        std::fs::create_dir_all(&self.storage_dir).map_err(failed)?;
        let path = tpt_home_core::files::safe_join(&self.storage_dir, &[name]).map_err(failed)?;
        std::fs::write(path, data).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services(perms: Permissions, deadline: Instant) -> (tempfile::TempDir, HostServices) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("test.db");
        let storage = dir.path().join("storage");
        let svc = HostServices::new("demo", &perms, &db, &storage, deadline);
        (dir, svc)
    }

    fn far_future() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn ungranted_capabilities_fail_without_trapping() {
        let (_dir, mut svc) = services(Permissions::default(), far_future());
        assert!(matches!(svc.kv_set("k", "v"), Err(HostError::Failed(_))));
        assert!(matches!(svc.kv_get("k"), Err(HostError::Failed(_))));
        assert!(matches!(
            svc.storage_write("f", "x"),
            Err(HostError::Failed(_))
        ));
        assert!(matches!(
            svc.db_query("SELECT 1"),
            Err(HostError::Failed(_))
        ));
    }

    #[test]
    fn storage_round_trip_and_traversal_is_refused() {
        let perms = Permissions {
            storage: true,
            ..Default::default()
        };
        let (_dir, mut svc) = services(perms, far_future());
        svc.storage_write("note.txt", "hello").expect("write");
        assert_eq!(svc.storage_read("note.txt").expect("read"), b"hello");
        assert!(matches!(
            svc.storage_read("../escape"),
            Err(HostError::Failed(_))
        ));
        assert!(matches!(
            svc.storage_read("missing"),
            Err(HostError::Failed(_))
        ));
    }

    #[test]
    fn calls_after_the_deadline_are_fatal() {
        let past = Instant::now();
        let perms = Permissions {
            kv: true,
            ..Default::default()
        };
        let (_dir, mut svc) = services(perms, past);
        assert!(matches!(svc.kv_set("k", "v"), Err(HostError::Deadline)));
        assert!(matches!(svc.log("x".into()), Err(HostError::Deadline)));
    }

    #[test]
    fn log_lines_are_bounded() {
        let (_dir, mut svc) = services(Permissions::default(), far_future());
        for i in 0..(MAX_LOG_LINES + 10) {
            svc.log(format!("line {i}")).expect("log");
        }
        assert_eq!(svc.logs().len(), MAX_LOG_LINES);
    }
}
