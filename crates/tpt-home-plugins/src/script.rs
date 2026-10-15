// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The JS script tier (`runtime = "js"`), built on the Boa engine.
//!
//! Compiled only with the `js` cargo feature. A script must define a global
//! `handle(req)` that returns `{status, content_type, body}`. The only host
//! surface is the frozen `tpt` global, a thin layer over the same
//! [`HostServices`] the wasm tier uses, so permission checks, scope rules,
//! size caps and the deadline are shared.
//!
//! Lockdown, applied before the plugin source runs:
//! - `eval` and `Function` are removed, and the `constructor` of every
//!   function-family prototype (plain, async, generator, async generator) is
//!   set to `undefined`, so no route to dynamic code remains.
//! - Module syntax is rejected at load, since scripts are parsed as scripts.
//! - No timers, `require`, `fetch` or filesystem: Boa provides none.
//!
//! Limits: Boa's loop-iteration and recursion limits derive from the manifest's
//! `max_fuel` (approximately; see [`call_script`]). The wall-clock deadline is
//! checked at every host call. Boa has no heap cap, so source size and the
//! host-side string caps are the memory bounds, and the docs say so.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use boa_engine::{
    Context, JsArgs, JsData, JsError, JsNativeError, JsResult, JsString, JsValue, NativeFunction,
    Script, Source,
    gc::{Finalize, Trace},
};

use crate::host::{HostError, HostResult, HostServices};
use crate::manifest::{Limits, Permissions, PluginManifest};
use crate::runtime::{PluginRequest, PluginResponse};
use crate::script_worker;

/// Largest accepted script source.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Recursion depth allowed in a script.
const RECURSION_LIMIT: usize = 256;
/// One loop iteration per this many fuel units (`max_fuel` default 50M gives
/// about one million iterations per loop).
const FUEL_PER_LOOP_ITERATION: u64 = 50;

/// A validated script, parsed once at load. Each invocation still runs in a
/// fresh realm, so the parse is repeated per call (Boa scripts are bound to
/// their realm).
#[derive(Debug)]
pub struct ScriptSource {
    source: String,
}

impl ScriptSource {
    /// Validate and wrap a script. Rejects oversize input and syntax errors,
    /// including module syntax.
    pub fn new(source: String) -> Result<Self, anyhow::Error> {
        if source.len() > MAX_SOURCE_BYTES {
            anyhow::bail!("script exceeds {MAX_SOURCE_BYTES} bytes");
        }
        let mut context = Context::default();
        Script::parse(Source::from_bytes(&source), None, &mut context)
            .map_err(|e| anyhow!("script does not parse: {e}"))?;
        Ok(Self { source })
    }

    /// Read and validate the entry file of an installed package.
    pub fn load(dir: &Path, manifest: &PluginManifest) -> Result<Self, anyhow::Error> {
        let text = std::fs::read_to_string(dir.join(&manifest.plugin.entry))
            .with_context(|| format!("read {}", manifest.plugin.entry))?;
        Self::new(text)
    }
}

/// Host data stored in the Boa context for the duration of one invocation.
#[derive(Trace, Finalize, JsData)]
struct ScriptHost {
    #[unsafe_ignore_trace]
    services: RefCell<HostServices>,
    /// Set when a host call hit the deadline, so the caller can report it.
    #[unsafe_ignore_trace]
    timed_out: Cell<bool>,
}

/// Builds the frozen `tpt` global from the raw natives, deletes the raw
/// natives, and applies the lockdown. Runs before the plugin source.
const PRELUDE: &str = r#"
(function () {
  "use strict";
  const raw = {
    log: __tpt_log,
    dbQuery: __tpt_db_query,
    dbExecute: __tpt_db_execute,
    kvGet: __tpt_kv_get,
    kvSet: __tpt_kv_set,
    kvDel: __tpt_kv_del,
    storageRead: __tpt_storage_read,
    storageWrite: __tpt_storage_write,
    now: __tpt_now,
    http: __tpt_http,
    notify: __tpt_notify,
  };
  for (const name of ["__tpt_log", "__tpt_db_query", "__tpt_db_execute", "__tpt_kv_get",
      "__tpt_kv_set", "__tpt_kv_del", "__tpt_storage_read", "__tpt_storage_write", "__tpt_now",
      "__tpt_http", "__tpt_notify"]) {
    delete globalThis[name];
  }
  const parse = (text) => (text === null ? null : JSON.parse(text));
  const tpt = Object.freeze({
    log: (...parts) => raw.log(parts.map((p) => String(p)).join(" ")),
    now: () => parse(raw.now()),
    notify: (channel, message) => raw.notify(String(channel), String(message)),
    http: Object.freeze({
      get: (url) => parse(raw.http("GET", String(url), undefined)),
      post: (url, body) => parse(raw.http("POST", String(url), String(body))),
    }),
    db: Object.freeze({
      query: (sql) => parse(raw.dbQuery(String(sql))),
      execute: (sql) => raw.dbExecute(String(sql)),
    }),
    kv: Object.freeze({
      get: (key) => parse(raw.kvGet(String(key))),
      set: (key, value) => raw.kvSet(String(key), String(value)),
      del: (key) => raw.kvDel(String(key)),
    }),
    storage: Object.freeze({
      read: (name) => raw.storageRead(String(name)),
      write: (name, data) => raw.storageWrite(String(name), String(data)),
    }),
  });
  Object.defineProperty(globalThis, "tpt", { value: tpt, writable: false, configurable: false });

  delete globalThis.eval;
  delete globalThis.Function;
  const protos = [
    Object.getPrototypeOf(function () {}),
    Object.getPrototypeOf(async function () {}),
    Object.getPrototypeOf(function* () {}),
    Object.getPrototypeOf(async function* () {}),
  ];
  for (const proto of protos) {
    Object.defineProperty(proto, "constructor", { value: undefined, writable: false, configurable: false });
  }
})();
"#;

/// Evaluates to a function that runs `handle` on a JSON request and returns
/// the JSON response. Evaluated after the plugin source.
const ENTRY: &str = r#"
(function () {
  return (requestJson) => JSON.stringify(handle(JSON.parse(requestJson)));
})();
"#;

/// Run a script plugin's `handle` for `request`.
///
/// When a worker executable is configured (see
/// [`script_worker::set_worker_exe`]), the call runs in a child process under
/// an operating-system memory limit. Otherwise it runs in this process, where
/// memory is not capped.
///
/// Every failure is an `Err`: syntax, runtime errors, exceeded loop or
/// recursion limits, a passed deadline, and in a worker a crash or memory
/// stop. The deadline message is the one case the caller can rely on.
pub fn call_script(
    script: &ScriptSource,
    manifest: &PluginManifest,
    request: &PluginRequest,
    plugin_id: &str,
    perms: &Permissions,
    db_path: &Path,
    storage_dir: &Path,
) -> Result<PluginResponse, anyhow::Error> {
    if let Some(exe) = script_worker::worker_exe() {
        let job = script_worker::Job {
            source: script.source.clone(),
            limits: manifest.limits.clone(),
            plugin_id: plugin_id.to_string(),
            perms: perms.clone(),
            request: request.clone(),
            db_path: db_path.to_path_buf(),
            storage_dir: storage_dir.to_path_buf(),
        };
        return script_worker::run_isolated(exe, &job);
    }
    call_in_process(
        script,
        &manifest.limits,
        request,
        plugin_id,
        perms,
        db_path,
        storage_dir,
    )
}

/// Run a script in this process. Used directly, and by the worker process.
pub(crate) fn call_in_process(
    script: &ScriptSource,
    limits: &Limits,
    request: &PluginRequest,
    plugin_id: &str,
    perms: &Permissions,
    db_path: &Path,
    storage_dir: &Path,
) -> Result<PluginResponse, anyhow::Error> {
    let deadline = Instant::now() + Duration::from_millis(limits.max_millis);
    let mut context = Context::default();
    {
        let runtime = context.runtime_limits_mut();
        runtime.set_loop_iteration_limit((limits.max_fuel / FUEL_PER_LOOP_ITERATION).max(1));
        runtime.set_recursion_limit(RECURSION_LIMIT);
    }
    context.insert_data(ScriptHost {
        services: RefCell::new(HostServices::new(
            plugin_id,
            perms,
            db_path,
            storage_dir,
            deadline,
        )),
        timed_out: Cell::new(false),
    });
    install_natives(&mut context).map_err(|e| anyhow!("host setup failed: {e}"))?;

    let entry = run(&mut context, PRELUDE)
        .and_then(|_| run(&mut context, &script.source))
        .and_then(|_| run(&mut context, ENTRY))
        .map_err(|e| failure(&context, e))?;

    let callable = entry
        .as_callable()
        .ok_or_else(|| anyhow!("script entry is not callable"))?;
    let request_json = serde_json::to_string(request)?;
    let out = callable
        .call(
            &JsValue::undefined(),
            &[JsValue::from(JsString::from(request_json))],
            &mut context,
        )
        .map_err(|e| failure(&context, e))?;
    let text = out
        .to_string(&mut context)
        .map_err(|e| failure(&context, e))?
        .to_std_string_escaped();
    serde_json::from_str(&text).map_err(|e| anyhow!("script returned invalid response JSON: {e}"))
}

fn run(context: &mut Context, source: &str) -> Result<JsValue, JsError> {
    context.eval(Source::from_bytes(source))
}

/// Turn a Boa error into the plugin error, reporting a deadline from the
/// host flag rather than from the message text.
fn failure(context: &Context, error: JsError) -> anyhow::Error {
    let timed_out = context
        .get_data::<ScriptHost>()
        .is_some_and(|host| host.timed_out.get());
    if timed_out {
        anyhow!("plugin call deadline exceeded")
    } else {
        // Boa appends a multi-line stack trace; the first line is the error.
        let message = error.to_string();
        anyhow!(
            "script error: {}",
            message.lines().next().unwrap_or_default()
        )
    }
}

fn install_natives(context: &mut Context) -> JsResult<()> {
    let natives: [(&str, usize, NativeFunction); 11] = [
        ("__tpt_log", 1, NativeFunction::from_fn_ptr(native_log)),
        (
            "__tpt_notify",
            2,
            NativeFunction::from_fn_ptr(native_notify),
        ),
        ("__tpt_http", 3, NativeFunction::from_fn_ptr(native_http)),
        ("__tpt_now", 0, NativeFunction::from_fn_ptr(native_now)),
        (
            "__tpt_db_query",
            1,
            NativeFunction::from_fn_ptr(native_db_query),
        ),
        (
            "__tpt_db_execute",
            1,
            NativeFunction::from_fn_ptr(native_db_execute),
        ),
        (
            "__tpt_kv_get",
            1,
            NativeFunction::from_fn_ptr(native_kv_get),
        ),
        (
            "__tpt_kv_set",
            2,
            NativeFunction::from_fn_ptr(native_kv_set),
        ),
        (
            "__tpt_kv_del",
            1,
            NativeFunction::from_fn_ptr(native_kv_del),
        ),
        (
            "__tpt_storage_read",
            1,
            NativeFunction::from_fn_ptr(native_storage_read),
        ),
        (
            "__tpt_storage_write",
            2,
            NativeFunction::from_fn_ptr(native_storage_write),
        ),
    ];
    for (name, length, function) in natives {
        context.register_global_builtin_callable(JsString::from(name), length, function)?;
    }
    Ok(())
}

fn host_of(context: &Context) -> JsResult<&ScriptHost> {
    context.get_data::<ScriptHost>().ok_or_else(|| {
        JsNativeError::typ()
            .with_message("host services missing")
            .into()
    })
}

fn string_arg(args: &[JsValue], index: usize, context: &mut Context) -> JsResult<String> {
    Ok(args
        .get_or_undefined(index)
        .to_string(context)?
        .to_std_string_escaped())
}

/// Map a host result onto the script view: a failure becomes `None` (the
/// script gets `null` or `false`); a passed deadline sets the flag and throws.
fn settle<T>(host: &ScriptHost, result: HostResult<T>) -> JsResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(HostError::Failed(reason)) => {
            tracing::debug!(target: "plugin", %reason, "script host call failed");
            Ok(None)
        }
        Err(HostError::Deadline) => {
            host.timed_out.set(true);
            Err(JsNativeError::error()
                .with_message("plugin call deadline exceeded")
                .into())
        }
    }
}

fn native_log(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let message = string_arg(args, 0, context)?;
    settle(
        host_of(context)?,
        host_of(context)?.services.borrow_mut().log(message),
    )?;
    Ok(JsValue::undefined())
}

fn native_http(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let method = string_arg(args, 0, context)?;
    let url = string_arg(args, 1, context)?;
    let body = match args.get_or_undefined(2) {
        value if value.is_undefined() => None,
        _ => Some(string_arg(args, 2, context)?),
    };
    let host = host_of(context)?;
    let reply = settle(
        host,
        host.services
            .borrow_mut()
            .http_request(&method, &url, body.as_deref()),
    )?;
    json_or_null(reply)
}

fn native_notify(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let channel = string_arg(args, 0, context)?;
    let message = string_arg(args, 1, context)?;
    let host = host_of(context)?;
    let sent = settle(host, host.services.borrow_mut().notify(&channel, &message))?;
    Ok(JsValue::from(sent.unwrap_or(false)))
}

fn native_now(_this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = host_of(context)?;
    let now = settle(host, host.services.borrow_mut().now())?;
    json_or_null(now)
}

fn native_db_query(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let sql = string_arg(args, 0, context)?;
    let host = host_of(context)?;
    let rows = settle(host, host.services.borrow_mut().db_query(&sql))?;
    json_or_null(rows)
}

fn native_db_execute(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let sql = string_arg(args, 0, context)?;
    let host = host_of(context)?;
    let done = settle(host, host.services.borrow_mut().db_execute(&sql))?;
    Ok(JsValue::from(done.is_some()))
}

fn native_kv_get(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string_arg(args, 0, context)?;
    let host = host_of(context)?;
    let value = settle(host, host.services.borrow_mut().kv_get(&key))?.flatten();
    json_or_null(value)
}

fn native_kv_set(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string_arg(args, 0, context)?;
    let value = string_arg(args, 1, context)?;
    let host = host_of(context)?;
    let done = settle(host, host.services.borrow_mut().kv_set(&key, &value))?;
    Ok(JsValue::from(done.is_some()))
}

fn native_kv_del(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let key = string_arg(args, 0, context)?;
    let host = host_of(context)?;
    let done = settle(host, host.services.borrow_mut().kv_del(&key))?;
    Ok(JsValue::from(done.is_some()))
}

fn native_storage_read(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?;
    let host = host_of(context)?;
    let bytes = settle(host, host.services.borrow_mut().storage_read(&name))?;
    match bytes.map(String::from_utf8) {
        Some(Ok(text)) => Ok(JsValue::from(JsString::from(text))),
        _ => Ok(JsValue::null()),
    }
}

fn native_storage_write(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = string_arg(args, 0, context)?;
    let data = string_arg(args, 1, context)?;
    let host = host_of(context)?;
    let done = settle(host, host.services.borrow_mut().storage_write(&name, &data))?;
    Ok(JsValue::from(done.is_some()))
}

/// Rows and values cross into the script as JSON text, which the prelude
/// parses. `None` becomes `null`.
fn json_or_null<T: serde::Serialize>(value: Option<T>) -> JsResult<JsValue> {
    match value {
        Some(value) => {
            let text = serde_json::to_string(&value)
                .map_err(|e| JsNativeError::typ().with_message(e.to_string()))?;
            Ok(JsValue::from(JsString::from(text)))
        }
        None => Ok(JsValue::null()),
    }
}
