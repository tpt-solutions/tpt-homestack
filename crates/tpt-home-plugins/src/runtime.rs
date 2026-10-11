// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The plugin sandbox: wasmi engine, ABI, capability-gated host functions
//! and resource limits.
//!
//! # ABI (api_version 1)
//!
//! Guest exports: `memory`, `tpt_alloc(len: u32) -> u32`,
//! `tpt_entry(ptr: i32, len: i32) -> i64`.
//!
//! `tpt_entry` receives the request JSON (method/path/query/body) and
//! returns `(ret_ptr << 32) | ret_len` pointing at the response JSON
//! (`{"status":200,"content_type":"…","body":"…"}`), or `-1` on failure.
//!
//! Guest imports (module `env`), all returning `-1` on error:
//!
//! | import | capability |
//! |---|---|
//! | `tpt_log(ptr, len)` | always |
//! | `tpt_db_query(ptr, len) -> i64` | `db` |
//! | `tpt_db_execute(ptr, len) -> i64` | `db` |
//! | `tpt_kv_get(ptr, len) -> i64` | `kv` |
//! | `tpt_kv_set(kp, kl, vp, vl)` | `kv` |
//! | `tpt_kv_del(ptr, len)` | `kv` |
//! | `tpt_storage_read(ptr, len) -> i64` | `storage` |
//! | `tpt_storage_write(p1, l1, p2, l2)` | `storage` |
//!
//! String-returning imports allocate the result buffer with the guest's own
//! `tpt_alloc` and return the packed pointer, so the guest keeps ownership
//! of its heap.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use wasmi::{AsContext, Caller, Engine, Linker, Module, Store};

use crate::manifest::PluginManifest;
use crate::scope;

/// The plugin's request, built by the dispatcher.
#[derive(Debug, Serialize)]
pub struct PluginRequest {
    pub method: String,
    /// Path *below* `/api/<id>`, e.g. `/events` for `/api/calendar/events`.
    pub path: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub body: Option<String>,
}

/// The plugin's response, unpacked by the dispatcher.
#[derive(Debug, Deserialize)]
pub struct PluginResponse {
    pub status: u16,
    #[serde(default = "default_content_type")]
    pub content_type: String,
    #[serde(default)]
    pub body: String,
}

fn default_content_type() -> String {
    "text/plain".to_string()
}

/// Capability context + services handed to host functions.
pub struct HostCtx {
    pub plugin_id: String,
    pub perms: crate::manifest::Permissions,
    /// Path to the shared SQLite file (the host opens its own sync handle).
    pub db_path: std::path::PathBuf,
    /// Plugin-scoped storage directory.
    pub storage_dir: std::path::PathBuf,
    /// Captured `tpt_log` lines (bounded).
    pub logs: Vec<String>,
    /// Service state that must survive across host calls within one
    /// invocation.
    pub state: HostState,
}

pub struct HostState {
    /// Memory/table growth limits for wasmi (installed via `Store::limiter`).
    pub limits: wasmi::StoreLimits,
    /// Lazily-created SQLite connection (opened on first host DB call).
    conn: Option<rusqlite::Connection>,
}

impl HostState {
    fn new(max_memory_bytes: usize) -> Self {
        Self {
            limits: wasmi::StoreLimitsBuilder::new()
                .memory_size(max_memory_bytes)
                .trap_on_grow_failure(true)
                .build(),
            conn: None,
        }
    }

    fn conn(
        &mut self,
        db_path: &std::path::Path,
    ) -> Result<&mut rusqlite::Connection, rusqlite::Error> {
        if self.conn.is_none() {
            let conn = rusqlite::Connection::open(db_path)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            self.conn = Some(conn);
        }
        Ok(self.conn.as_mut().expect("just set"))
    }
}

const MAX_LOG_LINES: usize = 100;
const MAX_STRING_RESULT: u32 = 4 * 1024 * 1024;
const MAX_QUERY_ROWS: usize = 1_000;

/// The shared plugin engine (fuel metering enabled).
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        Engine::new(&config)
    })
}

/// Call a plugin's `tpt_entry` with `request`, enforcing the manifest's
/// permissions and limits. Every failure mode is a `Result`, never a panic
/// or a hang: fuel exhaustion, traps, bad ABIs and protocol violations all
/// surface as errors.
pub fn call_plugin(
    module: &Module,
    manifest: &PluginManifest,
    request: &PluginRequest,
    plugin_id: &str,
    perms: &crate::manifest::Permissions,
    db_path: &std::path::Path,
    storage_dir: &std::path::Path,
) -> Result<PluginResponse, anyhow::Error> {
    let max_memory_bytes = (manifest.limits.max_memory_mb as usize) * 1024 * 1024;
    let mut ctx = HostCtx {
        plugin_id: plugin_id.to_string(),
        perms: perms.clone(),
        db_path: db_path.to_path_buf(),
        storage_dir: storage_dir.to_path_buf(),
        logs: Vec::new(),
        state: HostState::new(max_memory_bytes),
    };

    let mut store = Store::new(engine(), &mut ctx);
    store.limiter(|ctx| &mut ctx.state.limits);
    store.set_fuel(manifest.limits.max_fuel)?;

    let mut linker: Linker<&mut HostCtx> = Linker::new(engine());
    register_imports(&mut linker)?;

    let instance = linker.instantiate_and_start(&mut store, module)?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| anyhow::anyhow!("plugin does not export memory"))?;

    let request_json = serde_json::to_vec(request)?;
    if request_json.len() > MAX_STRING_RESULT as usize {
        anyhow::bail!("request too large for plugin ABI");
    }

    let alloc = instance.get_typed_func::<u32, u32>(&store, "tpt_alloc")?;
    let entry = instance.get_typed_func::<(i32, i32), i64>(&store, "tpt_entry")?;

    let ptr = alloc.call(&mut store, request_json.len() as u32)?;
    memory.write(&mut store, ptr as usize, &request_json)?;

    let packed = entry.call(&mut store, (ptr as i32, request_json.len() as i32))?;
    if packed < 0 {
        anyhow::bail!("plugin returned failure (-1)");
    }
    let (ret_ptr, ret_len) = ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32);
    if ret_len as u64 > MAX_STRING_RESULT as u64 {
        anyhow::bail!("plugin response too large");
    }
    let mut out = vec![0u8; ret_len as usize];
    memory.read(&store, ret_ptr as usize, &mut out)?;

    let response: PluginResponse = serde_json::from_slice(&out)
        .map_err(|e| anyhow::anyhow!("plugin returned invalid response JSON: {e}"))?;
    Ok(response)
}

/// Read a raw UTF-8 string from guest memory.
fn guest_str(caller: &Caller<'_, &mut HostCtx>, ptr: u32, len: u32) -> Result<String, wasmi::Error> {
    if len > MAX_STRING_RESULT {
        return Err(wasmi::Error::new("host string length exceeds cap"));
    }
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("plugin does not export memory"))?;
    let mut buf = vec![0u8; len as usize];
    memory.read(caller, ptr as usize, &mut buf)?;
    String::from_utf8(buf).map_err(|_| wasmi::Error::new("invalid UTF-8 from plugin"))
}

/// Allocate in guest memory and copy `bytes` there, returning the packed
/// `(ptr << 32) | len` for the guest.
fn guest_alloc(mut caller: Caller<'_, &mut HostCtx>, bytes: &[u8]) -> Result<i64, wasmi::Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmi::Error::new("plugin does not export memory"))?;
    let alloc = caller
        .get_export("tpt_alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| wasmi::Error::new("plugin does not export tpt_alloc"))?;
    let alloc = alloc
        .typed::<u32, u32>(caller.as_context())
        .map_err(|e| wasmi::Error::new(format!("tpt_alloc has the wrong signature: {e}")))?;
    let ptr = alloc.call(&mut caller, bytes.len() as u32)?;
    memory.write(&mut caller, ptr as usize, bytes)?;
    Ok((((ptr as u64) << 32) | bytes.len() as u64) as i64)
}

fn register_imports(linker: &mut Linker<&mut HostCtx>) -> Result<(), wasmi::Error> {
    linker.func_wrap(
        "env",
        "tpt_log",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| {
            let msg = guest_str(&caller, ptr as u32, len as u32).unwrap_or_default();
            if caller.data().logs.len() < MAX_LOG_LINES {
                caller.data_mut().logs.push(msg.clone());
            }
            tracing::debug!(target: "plugin", "{msg}");
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_db_query",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            if !caller.data().perms.db {
                return Ok(-1);
            }
            let plugin_id = caller.data().plugin_id.clone();
            let sql = scope::validate_scoped_sql(
                &plugin_id,
                &guest_str(&caller, ptr as u32, len as u32)?,
            )
            .map_err(|e| wasmi::Error::new(e.to_string()))?;
            let db_path = caller.data().db_path.clone();
            // Scope the DB borrow so `caller` is free again for `guest_alloc`.
            let bytes = {
                let conn = caller
                    .data_mut()
                    .state
                    .conn(&db_path)
                    .map_err(err_to_wasmi)?;

                let mut stmt = conn.prepare(&sql).map_err(err_to_wasmi)?;
                let col_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let mut rows_json: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
                let mut rows = stmt.query([]).map_err(err_to_wasmi)?;
                while let Some(row) = rows.next().map_err(err_to_wasmi)? {
                    let mut obj = serde_json::Map::new();
                    for (i, name) in col_names.iter().enumerate() {
                        let value = match row.get_ref(i).map_err(err_to_wasmi)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(v) => v.into(),
                            rusqlite::types::ValueRef::Real(v) => v.into(),
                            rusqlite::types::ValueRef::Text(t) => {
                                serde_json::Value::String(String::from_utf8_lossy(t).into_owned())
                            }
                            rusqlite::types::ValueRef::Blob(b) => {
                                serde_json::Value::String(format!("<blob {} bytes>", b.len()))
                            }
                        };
                        obj.insert(name.clone(), value);
                    }
                    rows_json.push(obj);
                    if rows_json.len() >= MAX_QUERY_ROWS {
                        break; // result cap per query
                    }
                }
                serde_json::to_vec(&rows_json).map_err(err_to_wasmi)?
            };
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_db_execute",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            if !caller.data().perms.db {
                return Ok(-1);
            }
            let plugin_id = caller.data().plugin_id.clone();
            let sql = scope::validate_scoped_sql(
                &plugin_id,
                &guest_str(&caller, ptr as u32, len as u32)?,
            )
            .map_err(|e| wasmi::Error::new(e.to_string()))?;
            let db_path = caller.data().db_path.clone();
            let conn = caller
                .data_mut()
                .state
                .conn(&db_path)
                .map_err(err_to_wasmi)?;
            let affected = conn.execute(&sql, []).map_err(err_to_wasmi)?;
            let bytes = serde_json::to_vec(&serde_json::json!({ "rows_affected": affected }))
                .map_err(err_to_wasmi)?;
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_kv_get",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            if !caller.data().perms.kv {
                return Ok(-1);
            }
            let plugin_id = caller.data().plugin_id.clone();
            let key = guest_str(&caller, ptr as u32, len as u32)?;
            let db_path = caller.data().db_path.clone();
            let conn = caller.data_mut().state.conn(&db_path).map_err(err_to_wasmi)?;
            let value: Option<String> = conn
                .query_row(
                    "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                    [&plugin_id, &key],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
                .map_err(err_to_wasmi)?;
            let bytes = serde_json::to_vec(&value).map_err(err_to_wasmi)?;
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_kv_set",
        |mut caller: Caller<'_, &mut HostCtx>,
         kptr: i32,
         klen: i32,
         vptr: i32,
         vlen: i32|
         -> Result<(), wasmi::Error> {
            if !caller.data().perms.kv {
                return Ok(());
            }
            let plugin_id = caller.data().plugin_id.clone();
            let key = guest_str(&caller, kptr as u32, klen as u32)?;
            let value = guest_str(&caller, vptr as u32, vlen as u32)?;
            let db_path = caller.data().db_path.clone();
            let conn = caller.data_mut().state.conn(&db_path).map_err(err_to_wasmi)?;
            conn.execute(
                "INSERT INTO plugin_kv (plugin_id, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(plugin_id, key) DO UPDATE SET value = excluded.value",
                rusqlite::params![plugin_id, key, value],
            )
            .map_err(err_to_wasmi)?;
            Ok(())
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_kv_del",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
            if !caller.data().perms.kv {
                return Ok(());
            }
            let plugin_id = caller.data().plugin_id.clone();
            let key = guest_str(&caller, ptr as u32, len as u32)?;
            let db_path = caller.data().db_path.clone();
            let conn = caller.data_mut().state.conn(&db_path).map_err(err_to_wasmi)?;
            conn.execute(
                "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                rusqlite::params![plugin_id, key],
            )
            .map_err(err_to_wasmi)?;
            Ok(())
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_storage_read",
        |caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            if !caller.data().perms.storage {
                return Ok(-1);
            }
            let name = guest_str(&caller, ptr as u32, len as u32)?;
            let dir = caller.data().storage_dir.clone();
            let path = tpt_home_core::files::safe_join(&dir, &[&name]).map_err(err_to_wasmi)?;
            // Missing files are -1 (data absent), not a trap.
            match std::fs::read(&path) {
                Ok(bytes) if bytes.len() <= MAX_STRING_RESULT as usize => {
                    guest_alloc(caller, &bytes)
                }
                _ => Ok(-1),
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_storage_write",
        |caller: Caller<'_, &mut HostCtx>,
         nptr: i32,
         nlen: i32,
         dptr: i32,
         dlen: i32|
         -> Result<(), wasmi::Error> {
            if !caller.data().perms.storage {
                return Ok(());
            }
            let name = guest_str(&caller, nptr as u32, nlen as u32)?;
            let data = guest_str(&caller, dptr as u32, dlen as u32)?; // UTF-8 only in v1
            let dir = caller.data().storage_dir.clone();
            std::fs::create_dir_all(&dir).map_err(err_to_wasmi)?;
            let path = tpt_home_core::files::safe_join(&dir, &[&name]).map_err(err_to_wasmi)?;
            std::fs::write(path, data).map_err(err_to_wasmi)
        },
    )?;

    Ok(())
}

fn err_to_wasmi<E: std::fmt::Display>(e: E) -> wasmi::Error {
    wasmi::Error::new(format!("host fn failed: {e}"))
}
