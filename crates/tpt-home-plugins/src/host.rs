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

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::manifest::{Permissions, parse_target};
use crate::scope;
use rusqlite::OptionalExtension as _;

/// Largest response body an outbound request may return.
pub const MAX_HTTP_BODY: usize = 1024 * 1024;
/// Longest message `tpt.notify` sends.
pub const MAX_NOTIFY_CHARS: usize = 2_000;
/// Upper bound on connect and each read or write of one outbound request.
const HTTP_IO_LIMIT: Duration = Duration::from_secs(10);

/// A finished outbound request: the status code and the body text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

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

/// The clock as a plugin sees it: Unix seconds, plus the server's local date,
/// hour, minute and UTC offset (quiet hours and date grouping need the local
/// view).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Now {
    pub unix: i64,
    /// `YYYY-MM-DD` in the server's local zone.
    pub local_date: String,
    pub local_hour: u32,
    pub local_minute: u32,
    pub utc_offset_minutes: i32,
}

/// A parsed outbound URL: the scheme, the host and port (defaults applied),
/// and the path with its leading slash.
struct Target {
    secure: bool,
    host: String,
    port: u16,
    path: String,
}

/// Parse an `http://` or `https://` URL. Control characters and spaces are
/// refused, so a script cannot write its own request headers.
fn parse_url(url: &str) -> HostResult<Target> {
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(failed("URL contains a control character or space"));
    }
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(failed("only http:// and https:// URLs are supported"));
    };
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let default_port = if secure { 443 } else { 80 };
    let (host, port) = parse_target(authority)
        .map(|(host, port)| (host, port.unwrap_or(default_port)))
        .ok_or_else(|| failed("URL host is not valid"))?;
    Ok(Target {
        secure,
        host,
        port,
        path: path.to_string(),
    })
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

    /// `tpt.now()`: the clock, in UTC and in the server's local zone. Always
    /// allowed; it reveals nothing a plugin cannot already see.
    pub fn now(&mut self) -> HostResult<Now> {
        self.check_deadline()?;
        let utc = chrono::Utc::now();
        let local = chrono::Local::now();
        Ok(Now {
            unix: utc.timestamp(),
            local_date: local.format("%Y-%m-%d").to_string(),
            local_hour: local.format("%H").to_string().parse().unwrap_or(0),
            local_minute: local.format("%M").to_string().parse().unwrap_or(0),
            utc_offset_minutes: local.offset().local_minus_utc() / 60,
        })
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

    /// `tpt.http`: one request to a declared host, over `http://` or, with the
    /// `https` feature, `https://`. Redirects are not followed, so a 3xx comes
    /// back as is.
    pub fn http_request(
        &mut self,
        method: &str,
        url: &str,
        body: Option<&str>,
    ) -> HostResult<HttpReply> {
        self.check_deadline()?;
        self.require(self.perms.http, "http")?;
        if method != "GET" && method != "POST" {
            return Err(failed(format!("method '{method}' is not allowed")));
        }
        let target = parse_url(url)?;
        // A bare declared host admits the scheme's default port only; an
        // explicit declared port must match exactly.
        let default_port = if target.secure { 443 } else { 80 };
        let declared = self.perms.http_hosts.iter().any(|entry| {
            parse_target(entry)
                .is_some_and(|(h, p)| h == target.host && p.unwrap_or(default_port) == target.port)
        });
        if !declared {
            return Err(failed(format!(
                "host '{}:{}' is not declared by the plugin",
                target.host, target.port
            )));
        }
        self.send(method, &target, body, "text/plain; charset=utf-8")
    }

    /// `tpt.notify`: POST `{"text": message}` to the admin's channel. The
    /// channel URL is the approval itself, so it is not checked against
    /// `http_hosts`. The outcome is recorded on the channel. `Ok(false)` when
    /// the channel is unknown, the message is too long, or the target did not
    /// answer 2xx.
    pub fn notify(&mut self, channel: &str, message: &str) -> HostResult<bool> {
        self.check_deadline()?;
        self.require(self.perms.notify, "notify")?;
        if !self.perms.notify_channels.iter().any(|c| c == channel) {
            return Err(failed(format!(
                "channel '{channel}' is not declared by the plugin"
            )));
        }
        if message.chars().count() > MAX_NOTIFY_CHARS {
            return Err(failed("message is too long"));
        }
        let url: Option<String> = self
            .conn()?
            .query_row(
                "SELECT url FROM notification_channels WHERE name = ?1",
                [channel],
                |row| row.get(0),
            )
            .optional()
            .map_err(failed)?;
        let Some(url) = url else {
            return Err(failed(format!("channel '{channel}' is not configured")));
        };
        let target = parse_url(&url)?;
        let payload = serde_json::json!({ "text": message }).to_string();
        let outcome = self.send("POST", &target, Some(&payload), "application/json");
        let (sent, error) = match outcome {
            Ok(reply) if (200..300).contains(&reply.status) => (true, None),
            Ok(reply) => (false, Some(format!("status {}", reply.status))),
            Err(HostError::Failed(reason)) => (false, Some(reason)),
            Err(HostError::Deadline) => return Err(HostError::Deadline),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn()?
            .execute(
                "UPDATE notification_channels SET last_sent_at = ?2, last_error = ?3 WHERE name = ?1",
                rusqlite::params![channel, now, error],
            )
            .map_err(failed)?;
        Ok(sent)
    }

    /// Send one request, over plain TCP or TLS, and read the reply.
    fn send(
        &mut self,
        method: &str,
        target: &Target,
        body: Option<&str>,
        content_type: &str,
    ) -> HostResult<HttpReply> {
        self.check_deadline()?;
        let timeout = self
            .deadline
            .saturating_duration_since(Instant::now())
            .min(HTTP_IO_LIMIT);
        if target.secure {
            return self.send_https(method, target, body, content_type, timeout);
        }
        self.send_plain(method, target, body, content_type, timeout)
    }

    /// Plain `http://`: HTTP/1.0 over TCP, body read to EOF.
    fn send_plain(
        &mut self,
        method: &str,
        target: &Target,
        body: Option<&str>,
        content_type: &str,
        timeout: Duration,
    ) -> HostResult<HttpReply> {
        let addr = format!("{}:{}", target.host, target.port)
            .to_socket_addrs()
            .map_err(failed)?
            .next()
            .ok_or_else(|| failed("host did not resolve"))?;
        let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(failed)?;
        stream.set_read_timeout(Some(timeout)).map_err(failed)?;
        stream.set_write_timeout(Some(timeout)).map_err(failed)?;

        let payload = body.unwrap_or("");
        let request = format!(
            "{method} {path} HTTP/1.0\r\nHost: {host}:{port}\r\nUser-Agent: tpt-homestack-plugin\r\n\
             Connection: close\r\nContent-Type: {content_type}\r\nContent-Length: {len}\r\n\r\n{payload}",
            path = target.path,
            host = target.host,
            port = target.port,
            len = payload.len(),
        );
        stream.write_all(request.as_bytes()).map_err(failed)?;
        let mut raw = Vec::new();
        (&mut stream)
            .take(MAX_HTTP_BODY as u64 + 16 * 1024)
            .read_to_end(&mut raw)
            .map_err(failed)?;
        self.check_deadline()?;

        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| failed("response has no header end"))?;
        let head = String::from_utf8_lossy(&raw[..split]);
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| failed("response has no status code"))?;
        let body = &raw[split + 4..];
        if body.len() > MAX_HTTP_BODY {
            return Err(failed("response body exceeds 1 MiB"));
        }
        Ok(HttpReply {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        })
    }

    /// `https://` through ureq and rustls, with the OS trust store.
    #[cfg(feature = "https")]
    fn send_https(
        &mut self,
        method: &str,
        target: &Target,
        body: Option<&str>,
        content_type: &str,
        timeout: Duration,
    ) -> HostResult<HttpReply> {
        let url = format!("https://{}:{}{}", target.host, target.port, target.path);
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let reply = if method == "POST" {
            agent
                .post(&url)
                .header("Content-Type", content_type)
                .send(body.unwrap_or(""))
        } else {
            agent.get(&url).call()
        };
        let mut response = reply.map_err(failed)?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_HTTP_BODY as u64)
            .read_to_string()
            .map_err(failed)?;
        self.check_deadline()?;
        Ok(HttpReply { status, body: text })
    }

    /// Without the `https` feature there is no TLS stack, so the call fails.
    #[cfg(not(feature = "https"))]
    fn send_https(
        &mut self,
        _method: &str,
        _target: &Target,
        _body: Option<&str>,
        _content_type: &str,
        _timeout: Duration,
    ) -> HostResult<HttpReply> {
        Err(failed("https needs a build with the https feature"))
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
