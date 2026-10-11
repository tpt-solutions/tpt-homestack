// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Authoring SDK for tpt-homestack WASM plugins (ABI version 1).
//!
//! A plugin is a `cdylib` built for `wasm32-unknown-unknown` that calls
//! [`export!`] once with a handler `fn(Request) -> Response`. The macro
//! generates the guest exports the host expects (`tpt_alloc`, `tpt_entry`).
//! Host services (SQL, key/value, file storage, logging) are wrapped as plain
//! functions; each returns `Err` when the admin did not grant the matching
//! permission, so plugins can degrade instead of trapping.
//!
//! ```ignore
//! use tpt_home_plugin_sdk::{export, Request, Response};
//!
//! fn handle(req: Request) -> Response {
//!     Response::text(200, format!("hello from {}", req.path))
//! }
//!
//! export!(handle);
//! ```

use std::ptr;

pub use serde_json;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One request forwarded by the host. `path` is below `/api/<id>`.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub body: Option<String>,
}

impl Request {
    /// Deserialize the request body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, String> {
        let body = self.body.as_deref().ok_or("request has no body")?;
        serde_json::from_str(body).map_err(|e| format!("invalid JSON body: {e}"))
    }

    /// Look up a raw (not percent-decoded) query parameter.
    pub fn query_param(&self, key: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
}

/// One response returned to the host.
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

impl Response {
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            body: body.into(),
        }
    }

    pub fn json<T: Serialize>(status: u16, value: &T) -> Self {
        Self {
            status,
            content_type: "application/json".into(),
            body: serde_json::to_string(value).unwrap_or_else(|_| "null".into()),
        }
    }

    /// `{"error": message}` with the given status.
    pub fn error(status: u16, message: impl AsRef<str>) -> Self {
        Self::json(status, &serde_json::json!({ "error": message.as_ref() }))
    }
}

/// Quote a string as an SQL literal (single quotes doubled). The host's SQL
/// API has no bound parameters in ABI v1, so every interpolated value must go
/// through this.
pub fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// Host imports. On wasm32 these are the `env` imports; elsewhere they are
// inert stubs so that native builds of plugin crates (workspace checks, unit
// tests) link cleanly.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        pub fn tpt_log(ptr: i32, len: i32);
        pub fn tpt_db_query(ptr: i32, len: i32) -> i64;
        pub fn tpt_db_execute(ptr: i32, len: i32) -> i64;
        pub fn tpt_kv_get(ptr: i32, len: i32) -> i64;
        pub fn tpt_kv_set(kp: i32, kl: i32, vp: i32, vl: i32);
        pub fn tpt_kv_del(ptr: i32, len: i32);
        pub fn tpt_storage_read(ptr: i32, len: i32) -> i64;
        pub fn tpt_storage_write(np: i32, nl: i32, dp: i32, dl: i32);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod host {
    pub unsafe fn tpt_log(_: i32, _: i32) {}
    pub unsafe fn tpt_db_query(_: i32, _: i32) -> i64 {
        -1
    }
    pub unsafe fn tpt_db_execute(_: i32, _: i32) -> i64 {
        -1
    }
    pub unsafe fn tpt_kv_get(_: i32, _: i32) -> i64 {
        -1
    }
    pub unsafe fn tpt_kv_set(_: i32, _: i32, _: i32, _: i32) {}
    pub unsafe fn tpt_kv_del(_: i32, _: i32) {}
    pub unsafe fn tpt_storage_read(_: i32, _: i32) -> i64 {
        -1
    }
    pub unsafe fn tpt_storage_write(_: i32, _: i32, _: i32, _: i32) {}
}

// ---------------------------------------------------------------------------
// Guest heap plumbing. Every buffer that crosses the boundary is a boxed
// slice of exact length, so freeing only needs `(ptr, len)`.
// ---------------------------------------------------------------------------

/// Allocate `len` bytes on the guest heap. Backs the `tpt_alloc` export.
pub fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// Copy `len` bytes out of guest memory at `ptr` and free the buffer.
///
/// # Safety
/// `ptr`/`len` must describe a buffer returned by [`alloc`] that nothing else
/// still references.
pub unsafe fn take_bytes(ptr: usize, len: usize) -> Vec<u8> {
    let slice = ptr::slice_from_raw_parts_mut(ptr as *mut u8, len);
    // SAFETY: caller guarantees the buffer came from `alloc` and is unused.
    unsafe { Box::from_raw(slice) }.into_vec()
}

/// Unpack a host result `(ptr << 32) | len`, copy it out and free it.
/// Returns `None` for the `-1` failure sentinel.
fn unpack_host(packed: i64) -> Option<Vec<u8>> {
    if packed < 0 {
        return None;
    }
    let ptr = (packed as u64 >> 32) as usize;
    let len = (packed as u64 & 0xFFFF_FFFF) as usize;
    // SAFETY: the host wrote this buffer through our `tpt_alloc` export.
    Some(unsafe { take_bytes(ptr, len) })
}

/// Serialize a response into guest memory and return the packed pointer the
/// host expects from `tpt_entry`.
pub fn pack_response(response: &Response) -> i64 {
    let bytes = serde_json::to_vec(response)
        .unwrap_or_else(|_| br#"{"status":500,"content_type":"text/plain","body":"serialize failed"}"#.to_vec());
    let ptr = alloc(bytes.len());
    // SAFETY: `ptr` has room for `bytes.len()` bytes and does not overlap.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    (((ptr as u64) << 32) | bytes.len() as u64) as i64
}

// ---------------------------------------------------------------------------
// Host services.
// ---------------------------------------------------------------------------

/// Write a line to the host log (bounded per request).
pub fn log(message: &str) {
    // SAFETY: the pointer and length describe a live `str` for the call.
    unsafe { host::tpt_log(message.as_ptr() as i32, message.len() as i32) };
}

/// Run a single scoped SELECT. Rows come back as JSON objects.
pub fn db_query(sql: &str) -> Result<Vec<Map<String, Value>>, String> {
    // SAFETY: as in `log`.
    let packed = unsafe { host::tpt_db_query(sql.as_ptr() as i32, sql.len() as i32) };
    let bytes = unpack_host(packed).ok_or("database access denied or query failed")?;
    serde_json::from_slice(&bytes).map_err(|e| format!("bad host response: {e}"))
}

/// Run a single scoped INSERT/UPDATE/DELETE/DDL. Returns rows affected.
pub fn db_execute(sql: &str) -> Result<u64, String> {
    // SAFETY: as in `log`.
    let packed = unsafe { host::tpt_db_execute(sql.as_ptr() as i32, sql.len() as i32) };
    let bytes = unpack_host(packed).ok_or("database access denied or statement failed")?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("bad host response: {e}"))?;
    Ok(value["rows_affected"].as_u64().unwrap_or(0))
}

/// Read a key from the plugin's key/value store.
pub fn kv_get(key: &str) -> Result<Option<String>, String> {
    // SAFETY: as in `log`.
    let packed = unsafe { host::tpt_kv_get(key.as_ptr() as i32, key.len() as i32) };
    let bytes = unpack_host(packed).ok_or("key/value access denied")?;
    serde_json::from_slice(&bytes).map_err(|e| format!("bad host response: {e}"))
}

/// Write a key in the plugin's key/value store. Silently ignored when the
/// permission is not granted (the host does not trap).
pub fn kv_set(key: &str, value: &str) {
    // SAFETY: as in `log`.
    unsafe {
        host::tpt_kv_set(
            key.as_ptr() as i32,
            key.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        )
    };
}

/// Delete a key from the plugin's key/value store.
pub fn kv_del(key: &str) {
    // SAFETY: as in `log`.
    unsafe { host::tpt_kv_del(key.as_ptr() as i32, key.len() as i32) };
}

/// Read a UTF-8 file from the plugin's storage directory. `Ok(None)` when the
/// file is absent.
pub fn storage_read(name: &str) -> Result<Option<String>, String> {
    // SAFETY: as in `log`.
    let packed = unsafe { host::tpt_storage_read(name.as_ptr() as i32, name.len() as i32) };
    match unpack_host(packed) {
        None => Ok(None),
        Some(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "stored file is not UTF-8".into()),
    }
}

/// Write a UTF-8 file into the plugin's storage directory.
pub fn storage_write(name: &str, data: &str) {
    // SAFETY: as in `log`.
    unsafe {
        host::tpt_storage_write(
            name.as_ptr() as i32,
            name.len() as i32,
            data.as_ptr() as i32,
            data.len() as i32,
        )
    };
}

/// Export the ABI entry points for a handler `fn(Request) -> Response`.
///
/// Invoke exactly once, at the crate root of the plugin.
#[macro_export]
macro_rules! export {
    ($handler:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn tpt_alloc(len: u32) -> u32 {
            $crate::alloc(len as usize) as u32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn tpt_entry(ptr: i32, len: i32) -> i64 {
            // SAFETY: the host wrote the request through `tpt_alloc`.
            let raw = unsafe { $crate::take_bytes(ptr as u32 as usize, len as usize) };
            let response = match $crate::serde_json::from_slice::<$crate::Request>(&raw) {
                Ok(request) => $handler(request),
                Err(e) => $crate::Response::error(400, format!("bad request envelope: {e}")),
            };
            $crate::pack_response(&response)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_quote_doubles_quotes() {
        assert_eq!(sql_quote("it's"), "'it''s'");
        assert_eq!(sql_quote("x'); DROP TABLE t; --"), "'x''); DROP TABLE t; --'");
    }

    #[test]
    fn query_param_lookup() {
        let req = Request {
            method: "GET".into(),
            path: "/events".into(),
            query: "id=7&tag=work".into(),
            body: None,
        };
        assert_eq!(req.query_param("id"), Some("7"));
        assert_eq!(req.query_param("missing"), None);
    }

    #[test]
    fn json_body_round_trip() {
        #[derive(Deserialize, PartialEq, Debug)]
        struct Body {
            title: String,
        }
        let req = Request {
            method: "POST".into(),
            path: "/events".into(),
            query: String::new(),
            body: Some(r#"{"title":"hi"}"#.into()),
        };
        assert_eq!(req.json::<Body>().unwrap(), Body { title: "hi".into() });
    }
}
