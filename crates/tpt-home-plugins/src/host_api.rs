// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Host services for external plugin processes, over HTTP.
//!
//! `POST /api/plugin-host/<id>/<op>` runs one host operation for an enabled
//! external plugin. The operations are the same ones the wasm imports expose,
//! and they go through the same [`HostServices`], so the approved permissions,
//! table scoping, size caps and deadline apply unchanged.
//!
//! The route sits behind the normal `/api` guard. A caller must hold a token
//! scoped to `plugin:<id>` (see `auth::api_token::scope_allows`).
//!
//! | operation | body | success `value` |
//! |---|---|---|
//! | `db/query` | `{"sql"}` | array of row objects |
//! | `db/execute` | `{"sql"}` | `{"rows_affected"}` |
//! | `kv/get` | `{"key"}` | string, or `null` if absent |
//! | `kv/set` | `{"key","value"}` | `null` |
//! | `kv/del` | `{"key"}` | `null` |
//! | `storage/read` | `{"name"}` | file contents as a string |
//! | `storage/write` | `{"name","data"}` | `null` |

use std::time::{Duration, Instant};

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::dispatch::{db_path, storage_dir};
use crate::host::{HostError, HostResult, HostServices};
use crate::registry::{self, Backend};

#[derive(Deserialize)]
struct SqlBody {
    sql: String,
}

#[derive(Deserialize)]
struct KeyBody {
    key: String,
}

#[derive(Deserialize)]
struct KeyValueBody {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

#[derive(Deserialize)]
struct NameDataBody {
    name: String,
    data: String,
}

type CallError = (StatusCode, String);

/// `POST /api/plugin-host/<plugin_id>/<rest>`.
pub async fn plugin_host(
    Path((plugin_id, rest)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Response {
    let Some(plugin) = registry::get_enabled(&plugin_id) else {
        return envelope(Err((
            StatusCode::NOT_FOUND,
            format!("no enabled plugin '{plugin_id}'"),
        )));
    };
    if !matches!(plugin.backend, Backend::External(_)) {
        return envelope(Err((
            StatusCode::FORBIDDEN,
            "host services are for external plugins".into(),
        )));
    }

    let perms = plugin.effective_permissions();
    let deadline = Instant::now() + Duration::from_millis(plugin.manifest.limits.max_millis);
    let id = plugin_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let mut services = HostServices::new(&id, &perms, &db_path(), &storage_dir(&id), deadline);
        run_operation(&mut services, &rest, &body)
    })
    .await;

    envelope(match outcome {
        Ok(result) => result,
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "host operation panicked".into(),
        )),
    })
}

fn run_operation(services: &mut HostServices, op: &str, body: &[u8]) -> Result<Value, CallError> {
    match op {
        "db/query" => {
            let req: SqlBody = parse(body)?;
            to_json(settle(services.db_query(&req.sql))?)
        }
        "db/execute" => {
            let req: SqlBody = parse(body)?;
            let affected = settle(services.db_execute(&req.sql))?;
            Ok(json!({ "rows_affected": affected }))
        }
        "kv/get" => {
            let req: KeyBody = parse(body)?;
            to_json(settle(services.kv_get(&req.key))?)
        }
        "kv/set" => {
            let req: KeyValueBody = parse(body)?;
            settle(services.kv_set(&req.key, &req.value))?;
            Ok(Value::Null)
        }
        "kv/del" => {
            let req: KeyBody = parse(body)?;
            settle(services.kv_del(&req.key))?;
            Ok(Value::Null)
        }
        "storage/read" => {
            let req: NameBody = parse(body)?;
            let bytes = settle(services.storage_read(&req.name))?;
            String::from_utf8(bytes).map(Value::String).map_err(|_| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "stored file is not UTF-8".into(),
                )
            })
        }
        "storage/write" => {
            let req: NameDataBody = parse(body)?;
            settle(services.storage_write(&req.name, &req.data))?;
            Ok(Value::Null)
        }
        _ => Err((
            StatusCode::NOT_FOUND,
            format!("unknown host operation '{op}'"),
        )),
    }
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, CallError> {
    serde_json::from_slice(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad request body: {e}")))
}

/// Map a host result: denial and ordinary failures are `403` (the reason is
/// included, since the caller is the plugin's own process); a passed deadline
/// is `504`.
fn settle<T>(result: HostResult<T>) -> Result<T, CallError> {
    match result {
        Ok(value) => Ok(value),
        Err(HostError::Failed(reason)) => {
            Err((StatusCode::FORBIDDEN, format!("denied or failed: {reason}")))
        }
        Err(HostError::Deadline) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            "plugin call deadline exceeded".into(),
        )),
    }
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, CallError> {
    serde_json::to_value(value).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn envelope(result: Result<Value, CallError>) -> Response {
    match result {
        Ok(value) => (
            StatusCode::OK,
            axum::Json(json!({ "ok": true, "value": value })),
        )
            .into_response(),
        Err((status, error)) => {
            (status, axum::Json(json!({ "ok": false, "error": error }))).into_response()
        }
    }
}
