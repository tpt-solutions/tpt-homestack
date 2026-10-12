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
//! Guest imports (module `env`). Every capability-gated import returns `-1`
//! when the capability is not granted or the operation fails; the host logs
//! the reason. Only a passed deadline or exhausted fuel traps the guest.
//!
//! | import | capability | returns |
//! |---|---|---|
//! | `tpt_log(ptr, len)` | always | nothing |
//! | `tpt_db_query(ptr, len) -> i64` | `db` | packed JSON rows, or `-1` |
//! | `tpt_db_execute(ptr, len) -> i64` | `db` | packed JSON `{rows_affected}`, or `-1` |
//! | `tpt_kv_get(ptr, len) -> i64` | `kv` | packed JSON value (`null` if absent), or `-1` |
//! | `tpt_kv_set(kp, kl, vp, vl) -> i32` | `kv` | `0` on success, `-1` otherwise |
//! | `tpt_kv_del(ptr, len) -> i32` | `kv` | `0` on success, `-1` otherwise |
//! | `tpt_storage_read(ptr, len) -> i64` | `storage` | packed bytes, or `-1` if absent |
//! | `tpt_storage_write(p1, l1, p2, l2) -> i32` | `storage` | `0` on success, `-1` otherwise |
//!
//! String-returning imports allocate the result buffer with the guest's own
//! `tpt_alloc` and return the packed pointer, so the guest keeps ownership
//! of its heap.
//!
//! # Limits
//!
//! `max_fuel` bounds guest instructions. `max_millis` is a wall-clock budget
//! for the whole invocation, checked at every host call; wasmi offers no
//! preemptive interrupt, so a guest that loops without calling the host is
//! bounded by fuel alone.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use wasmi::{AsContext, Caller, Engine, Linker, Module, Store};

use tpt_home_core::auth::user::AuthUser;

use crate::host::{HostError, HostResult, HostServices, MAX_STRING_RESULT};
use crate::manifest::PluginManifest;

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
    /// The signed-in caller, when there is one. Omitted from the JSON when
    /// absent, so guests that ignore it see the same shape as before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<PluginUser>,
}

/// The caller as a plugin sees it. Identity only: no credential or session.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginUser {
    pub id: i64,
    pub username: String,
    /// `admin` or `user`.
    pub role: String,
}

impl From<&AuthUser> for PluginUser {
    fn from(user: &AuthUser) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            role: user.role.as_str().to_string(),
        }
    }
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

/// Per-invocation host state, reachable from every import through the
/// store's data.
pub struct HostCtx {
    pub services: HostServices,
    /// Memory growth limits for wasmi (installed via `Store::limiter`).
    pub limits: wasmi::StoreLimits,
}

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
/// or a hang: fuel exhaustion, the deadline, traps, bad ABIs and protocol
/// violations all surface as errors.
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
    let deadline = Instant::now() + Duration::from_millis(manifest.limits.max_millis);
    let mut ctx = HostCtx {
        services: HostServices::new(plugin_id, perms, db_path, storage_dir, deadline),
        limits: wasmi::StoreLimitsBuilder::new()
            .memory_size(max_memory_bytes)
            .trap_on_grow_failure(true)
            .build(),
    };

    let mut store = Store::new(engine(), &mut ctx);
    store.limiter(|ctx| &mut ctx.limits);
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
fn guest_str(
    caller: &Caller<'_, &mut HostCtx>,
    ptr: u32,
    len: u32,
) -> Result<String, wasmi::Error> {
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

/// Map a host-service result onto the guest ABI. A failure becomes `None`
/// (the guest sees `-1`); a passed deadline traps the invocation.
fn gate<T>(result: HostResult<T>) -> Result<Option<T>, wasmi::Error> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(HostError::Failed(reason)) => {
            tracing::debug!(target: "plugin", %reason, "host call failed");
            Ok(None)
        }
        Err(HostError::Deadline) => Err(wasmi::Error::new("plugin call deadline exceeded")),
    }
}

/// Status for write imports: `0` on success, `-1` on failure.
fn status(result: HostResult<()>) -> Result<i32, wasmi::Error> {
    Ok(if gate(result)?.is_some() { 0 } else { -1 })
}

fn register_imports(linker: &mut Linker<&mut HostCtx>) -> Result<(), wasmi::Error> {
    linker.func_wrap(
        "env",
        "tpt_log",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
            let msg = guest_str(&caller, ptr as u32, len as u32).unwrap_or_default();
            gate(caller.data_mut().services.log(msg))?;
            Ok(())
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_db_query",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            let sql = guest_str(&caller, ptr as u32, len as u32)?;
            let Some(rows) = gate(caller.data_mut().services.db_query(&sql))? else {
                return Ok(-1);
            };
            let bytes = serde_json::to_vec(&rows).map_err(err_to_wasmi)?;
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_db_execute",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            let sql = guest_str(&caller, ptr as u32, len as u32)?;
            let Some(affected) = gate(caller.data_mut().services.db_execute(&sql))? else {
                return Ok(-1);
            };
            let bytes = serde_json::to_vec(&serde_json::json!({ "rows_affected": affected }))
                .map_err(err_to_wasmi)?;
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_kv_get",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            let key = guest_str(&caller, ptr as u32, len as u32)?;
            let Some(value) = gate(caller.data_mut().services.kv_get(&key))? else {
                return Ok(-1);
            };
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
         -> Result<i32, wasmi::Error> {
            let key = guest_str(&caller, kptr as u32, klen as u32)?;
            let value = guest_str(&caller, vptr as u32, vlen as u32)?;
            status(caller.data_mut().services.kv_set(&key, &value))
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_kv_del",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let key = guest_str(&caller, ptr as u32, len as u32)?;
            status(caller.data_mut().services.kv_del(&key))
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_storage_read",
        |mut caller: Caller<'_, &mut HostCtx>, ptr: i32, len: i32| -> Result<i64, wasmi::Error> {
            let name = guest_str(&caller, ptr as u32, len as u32)?;
            let Some(bytes) = gate(caller.data_mut().services.storage_read(&name))? else {
                return Ok(-1);
            };
            guest_alloc(caller, &bytes)
        },
    )?;

    linker.func_wrap(
        "env",
        "tpt_storage_write",
        |mut caller: Caller<'_, &mut HostCtx>,
         nptr: i32,
         nlen: i32,
         dptr: i32,
         dlen: i32|
         -> Result<i32, wasmi::Error> {
            let name = guest_str(&caller, nptr as u32, nlen as u32)?;
            let data = guest_str(&caller, dptr as u32, dlen as u32)?; // UTF-8 only in v1
            status(caller.data_mut().services.storage_write(&name, &data))
        },
    )?;

    Ok(())
}

fn err_to_wasmi<E: std::fmt::Display>(e: E) -> wasmi::Error {
    wasmi::Error::new(format!("host fn failed: {e}"))
}
