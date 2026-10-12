// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP surface for plugins: request dispatch into the sandbox, declarative
//! pages, and the admin lifecycle API.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;
use sqlx::{Column, Row};

use tpt_home_core::auth::user::AuthUser;
use tpt_home_core::prelude::*;
use tpt_home_ui as ui;

use crate::lifecycle;
use crate::manifest::Permissions;
use crate::registry::Backend;
use crate::runtime::{self, PluginRequest};

pub(crate) fn db_path() -> std::path::PathBuf {
    tpt_home_core::config::global().db_path()
}

pub(crate) fn storage_dir(id: &str) -> std::path::PathBuf {
    tpt_home_core::config::global()
        .data_dir
        .join("plugins")
        .join("_data")
        .join(id)
}

/// Dispatch `/api/<plugin_id>/<rest>` into the sandbox. Registered as a
/// `RouteEntry` (protected); static routes elsewhere in `/api` win over the
/// parameter, so this only sees plugin traffic.
pub async fn api_invoke(
    State(_db): State<DbPool>,
    Path((plugin_id, rest)): Path<(String, String)>,
    method: axum::http::Method,
    Query(query): Query<std::collections::HashMap<String, String>>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let query_string = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let body = (!body.is_empty()).then(|| String::from_utf8_lossy(&body).into_owned());
    invoke_plugin(&plugin_id, method.as_str(), &rest, &query_string, body).await
}

/// Run one call against an enabled plugin, whatever its runtime. `rest` is the
/// path below `/api/<plugin_id>` without its leading slash. Shared by the HTTP
/// route and the MCP tools, so both get the same runtime and permission rules.
pub async fn invoke_plugin(
    plugin_id: &str,
    method: &str,
    rest: &str,
    query: &str,
    body: Option<String>,
) -> Result<Response, ApiError> {
    let Some(plugin) = crate::registry::get_enabled(plugin_id) else {
        return Err(ApiError::not_found(format!(
            "no enabled plugin '{plugin_id}'"
        )));
    };

    if let Backend::External(base) = &plugin.backend {
        let timeout = std::time::Duration::from_millis(plugin.manifest.limits.max_millis);
        return Ok(crate::external::forward(
            base,
            method,
            &format!("/{rest}"),
            query,
            body,
            timeout,
        )
        .await);
    }
    let request = PluginRequest {
        method: method.to_string(),
        path: format!("/{rest}"),
        query: query.to_string(),
        body,
    };

    let granted = plugin.effective_permissions();
    let result = tokio::task::spawn_blocking(move || match &plugin.backend {
        Backend::External(_) => Err(anyhow::anyhow!("external plugins are forwarded, not run")),
        Backend::Wasm(module) => runtime::call_plugin(
            module,
            &plugin.manifest,
            &request,
            &plugin.manifest.plugin.id,
            &granted,
            &db_path(),
            &storage_dir(&plugin.manifest.plugin.id),
        ),
        #[cfg(feature = "js")]
        Backend::Script(script) => crate::script::call_script(
            script,
            &plugin.manifest,
            &request,
            &plugin.manifest.plugin.id,
            &granted,
            &db_path(),
            &storage_dir(&plugin.manifest.plugin.id),
        ),
    })
    .await
    .map_err(|e| ApiError::internal(format!("plugin task failed: {e}")))?
    .map_err(|e| ApiError::internal(format!("plugin error: {e}")))?;

    let status = StatusCode::from_u16(result.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let content_type = result
        .content_type
        .parse()
        .unwrap_or_else(|_| header::HeaderValue::from_static("text/plain"));
    Ok((status, [(header::CONTENT_TYPE, content_type)], result.body).into_response())
}

/// Render a declarative plugin page: `/plugins/<plugin_id>/<page-path>`.
pub async fn page(
    State(db): State<DbPool>,
    Extension(user): Extension<AuthUser>,
    Path((plugin_id, rest)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Html<String>, ApiError> {
    let Some(plugin) = crate::registry::get_enabled(&plugin_id) else {
        return Err(ApiError::not_found(format!(
            "no enabled plugin '{plugin_id}'"
        )));
    };
    let wanted = format!("/{rest}");
    let Some(page_def) = plugin.manifest.pages.iter().find(|p| {
        p.path == wanted || format!("/{rest}").trim_end_matches('/') == p.path.trim_end_matches('/')
    }) else {
        return Err(ApiError::not_found(format!("page '{wanted}' not found")));
    };

    let api_base = format!("/api/{plugin_id}");
    let link = |page: &crate::manifest::PageDef| {
        page.link
            .as_deref()
            .map(|l| lifecycle::plugin_page_path(&plugin_id, l))
    };
    let content = match page_def.kind.as_str() {
        "table" => {
            let Some(table) = &page_def.table else {
                return Err(ApiError::bad_request("page has no table"));
            };
            let rows = load_rows(&db, &plugin.scoped_table(table), None).await?;
            let columns = page_columns(&page_def.columns, rows.first());
            ui::plugin_table_page(&columns, &rows, &api_base, link(page_def).as_deref())
        }
        "list" => {
            let Some(table) = &page_def.table else {
                return Err(ApiError::bad_request("page has no table"));
            };
            let rows = load_rows(&db, &plugin.scoped_table(table), None).await?;
            ui::plugin_list_page(&page_def.columns, &rows, link(page_def).as_deref())
        }
        "detail" => {
            let Some(table) = &page_def.table else {
                return Err(ApiError::bad_request("page has no table"));
            };
            let id: i64 = params
                .get("id")
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| ApiError::bad_request("detail pages need an integer ?id="))?;
            let rows = load_rows(&db, &plugin.scoped_table(table), Some(id)).await?;
            let columns = page_columns(&page_def.columns, rows.first());
            ui::plugin_detail_page(&columns, rows.first())
        }
        "form" => ui::plugin_form_page(
            &page_def.fields,
            &format!("{api_base}{}", page_def.action.as_deref().unwrap_or("/")),
        ),
        other => ui::plugin_error_page(&format!("unknown page kind '{other}'")),
    };

    let nav = tpt_home_core::registry::nav_entries();
    let view = ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    Ok(Html(ui::page(
        &page_def.title,
        &nav,
        &lifecycle::plugin_page_path(&plugin_id, &page_def.path),
        &view,
        &content,
    )))
}

/// Columns a page shows: the manifest's list, or every column of the first row
/// when the manifest names none.
fn page_columns(declared: &[String], first: Option<&serde_json::Value>) -> Vec<String> {
    if !declared.is_empty() {
        return declared.to_vec();
    }
    first
        .and_then(serde_json::Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// Read rows from a plugin's scoped table as JSON objects. `Some(id)` selects a
/// single row; `None` returns the newest 200.
async fn load_rows(
    db: &DbPool,
    scoped_table: &str,
    id: Option<i64>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let rows = match id {
        Some(id) => {
            sqlx::query(&format!("SELECT * FROM \"{scoped_table}\" WHERE id = ?1"))
                .bind(id)
                .fetch_all(db)
                .await?
        }
        None => {
            sqlx::query(&format!(
                "SELECT * FROM \"{scoped_table}\" ORDER BY id DESC LIMIT 200"
            ))
            .fetch_all(db)
            .await?
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut obj = serde_json::Map::new();
        for (i, column) in row.columns().iter().enumerate() {
            let value: serde_json::Value = if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                v.map(|v| json!(v)).unwrap_or(json!(null))
            } else if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
                v.map(|v| json!(v)).unwrap_or(json!(null))
            } else if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                v.map(|v| json!(v)).unwrap_or(json!(null))
            } else {
                json!(null)
            };
            obj.insert(column.name().to_string(), value);
        }
        out.push(serde_json::Value::Object(obj));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Admin lifecycle API (role = "admin")
// ---------------------------------------------------------------------------

/// GET /api/plugins — installed plugins + status.
#[tpt_home_macros::bp_route(get, "/api/plugins", role = "admin")]
pub async fn list_plugins(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let mut out = Vec::new();
    for (plugin, enabled) in crate::registry::all() {
        let (installed_at,): (i64,) =
            sqlx::query_as("SELECT installed_at FROM plugins WHERE id = ?1")
                .bind(&plugin.manifest.plugin.id)
                .fetch_optional(&db)
                .await?
                .unwrap_or((0,));
        out.push(json!({
            "id": plugin.manifest.plugin.id,
            "name": plugin.manifest.plugin.name,
            "version": plugin.manifest.plugin.version,
            "enabled": enabled,
            "permissions": plugin.manifest.permissions.to_json(),
            "approved_permissions": plugin.approved.intersect(&plugin.manifest.permissions).to_json(),
            "pages": plugin.manifest.pages.iter().map(|p| lifecycle::plugin_page_path(&plugin.manifest.plugin.id, &p.path)).collect::<Vec<_>>(),
            "installed_at": installed_at,
        }));
    }
    Ok(Json(out))
}

/// POST /api/plugins/install — multipart field `package` (a `.tptp`).
/// Optional `enable` form field (`true`) enables immediately; permissions are approved as
/// declared in the manifest (the admin chose to install it).
#[tpt_home_macros::bp_route(post, "/api/plugins/install", role = "admin")]
pub async fn install_plugin(
    State(db): State<DbPool>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut package: Option<Vec<u8>> = None;
    let mut enable: Option<bool> = None;
    let mut granted = Permissions::default();
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart error: {e}")))?
    {
        match field.name() {
            Some("package") => {
                let mut bytes = Vec::new();
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("multipart error: {e}")))?
                {
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() > 64 * 1024 * 1024 {
                        return Err(ApiError::bad_request("package too large"));
                    }
                }
                package = Some(bytes);
            }
            Some("enable") => {
                enable = Some(matches!(
                    field.text().await.unwrap_or_default().as_str(),
                    "true" | "1" | "on"
                ));
            }
            Some("approve") => {
                let list = field.text().await.unwrap_or_default();
                granted = Permissions::parse_list(&list).map_err(ApiError::bad_request)?;
            }
            _ => {
                let _ = field.text().await;
            }
        }
    }
    let archive =
        package.ok_or_else(|| ApiError::bad_request("multipart field \"package\" is required"))?;

    let (staged, manifest) = lifecycle::stage_package(&archive)
        .map_err(|e| ApiError::bad_request(format!("invalid package: {e}")))?;
    // Without an explicit `enable`, an upgrade keeps the plugin's current state
    // rather than taking a running plugin offline.
    let enable = match enable {
        Some(enable) => enable,
        None => lifecycle::is_enabled(&db, &manifest.plugin.id)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?,
    };
    let installed = lifecycle::install_from_dir(&db, &staged, enable, &granted)
        .await
        .map_err(|e| ApiError::internal(format!("install failed: {e}")))?;
    let effective = granted.intersect(&installed.permissions);

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": installed.plugin.id,
            "version": installed.plugin.version,
            "requested_permissions": installed.permissions.to_json(),
            "approved_permissions": effective.to_json(),
        })),
    ))
}

#[derive(Deserialize)]
pub struct KeepDataQuery {
    #[serde(default)]
    pub keep_data: bool,
}

/// POST /api/plugins/{id}/enable
#[tpt_home_macros::bp_route(post, "/api/plugins/{id}/enable", role = "admin")]
pub async fn enable_plugin(
    State(db): State<DbPool>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    lifecycle::enable(&db, &id)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "enabled": true })))
}

/// POST /api/plugins/{id}/disable
#[tpt_home_macros::bp_route(post, "/api/plugins/{id}/disable", role = "admin")]
pub async fn disable_plugin(
    State(db): State<DbPool>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    lifecycle::disable(&db, &id)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "enabled": false })))
}

/// DELETE /api/plugins/{id}?keep_data=true — uninstall; by default the
/// plugin's tables and kv data are removed too.
#[tpt_home_macros::bp_route(delete, "/api/plugins/{id}", role = "admin")]
pub async fn uninstall_plugin(
    State(db): State<DbPool>,
    Path(id): Path<String>,
    Query(q): Query<KeepDataQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let dropped = lifecycle::uninstall(&db, &id, q.keep_data)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "dropped_tables": dropped })))
}
