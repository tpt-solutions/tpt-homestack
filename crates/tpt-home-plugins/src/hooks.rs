// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Public signed endpoints for plugins (`[[hooks]]` in `plugin.toml`).
//!
//! A sender that holds a plugin's secret posts to
//! `POST /api/plugin-hooks/<plugin-id>/<name>` with `X-Tpt-Timestamp` and
//! `X-Tpt-Signature` (HMAC-SHA256 of `"<timestamp>.<body>"`, the same scheme
//! as the core webhook receiver). The host verifies the request and then calls
//! the plugin at `POST /api/<plugin-id>/hooks/<name>` with the verified body
//! as text. The plugin never sees the signature or the secret.
//!
//! An admin creates or rotates a secret with
//! `POST /api/plugins/<plugin-id>/hooks/<name>/secret`. The secret is returned
//! once and stored as plain text, because HMAC needs the key. Uninstalling the
//! plugin deletes its secrets.
//!
//! Unknown plugin, disabled plugin, undeclared hook, missing secret and bad
//! signature all get the same `401`, so the endpoint reveals nothing to a
//! caller without a secret. Replays inside the 5-minute window are not caught;
//! see `tpt_home_core::webhook`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;
use tpt_home_core::db::DbPool;
use tpt_home_core::error::ApiError;
use tpt_home_core::webhook;

use crate::dispatch;
use crate::registry;

/// Largest accepted body, the same cap as the core webhook receiver.
const MAX_BODY: usize = 64 * 1024;
/// Allowed clock difference between the sender and the server, in seconds.
const WINDOW_SECS: i64 = 300;

inventory::submit! {
    tpt_home_core::PublicRouteEntry {
        blueprint: "tpt-home-plugins",
        method: "POST",
        path: "/api/plugin-hooks/{plugin_id}/{name}",
        register: |router| {
            router.route(
                "/api/plugin-hooks/{plugin_id}/{name}",
                axum::routing::post(receive),
            )
        },
    }
}

inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "POST",
        path: "/api/plugins/{plugin_id}/hooks/{name}/secret",
        role: Some("admin"),
        register: |router| {
            router.route(
                "/api/plugins/{plugin_id}/hooks/{name}/secret",
                axum::routing::post(rotate_secret).layer(tpt_home_core::admin_only!()),
            )
        },
    }
}

/// `POST /api/plugin-hooks/<plugin_id>/<name>` (public).
pub async fn receive(
    State(db): State<DbPool>,
    Path((plugin_id, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    if body.len() > MAX_BODY {
        return Err(ApiError::bad_request("body is larger than 64 KiB"));
    }
    let declared = registry::get_enabled(&plugin_id)
        .is_some_and(|plugin| plugin.manifest.hooks.iter().any(|h| h.name == name));
    if !declared {
        return Err(ApiError::Unauthorized);
    }
    let row: Option<(String,)> =
        sqlx::query_as("SELECT secret FROM plugin_hooks WHERE plugin_id = ?1 AND name = ?2")
            .bind(&plugin_id)
            .bind(&name)
            .fetch_optional(&db)
            .await?;
    let Some((secret,)) = row else {
        return Err(ApiError::Unauthorized);
    };

    let header = |key: &str| {
        headers
            .get(key)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    };
    if let Err(reason) = webhook::verify(
        secret.as_bytes(),
        header("x-tpt-timestamp"),
        header("x-tpt-signature"),
        &body,
        unix_now(),
        WINDOW_SECS,
    ) {
        tracing::debug!(plugin = %plugin_id, hook = %name, %reason, "plugin hook rejected");
        return Err(ApiError::Unauthorized);
    }

    let text = std::str::from_utf8(&body)
        .map_err(|_| ApiError::bad_request("body must be UTF-8 text"))?
        .to_string();
    sqlx::query("UPDATE plugin_hooks SET last_received_at = ?3 WHERE plugin_id = ?1 AND name = ?2")
        .bind(&plugin_id)
        .bind(&name)
        .bind(unix_now())
        .execute(&db)
        .await?;

    let response = dispatch::invoke_plugin(
        None,
        &plugin_id,
        "POST",
        &format!("hooks/{name}"),
        "",
        Some(text),
    )
    .await?;
    if !response.status().is_success() {
        tracing::warn!(plugin = %plugin_id, hook = %name, status = %response.status(), "plugin hook handler failed");
        return Err(ApiError::internal(
            "the plugin could not handle this delivery",
        ));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true }))))
}

/// `POST /api/plugins/<plugin_id>/hooks/<name>/secret` (admin). Creates the
/// secret or replaces it. The response is the only time it is shown.
pub async fn rotate_secret(
    State(db): State<DbPool>,
    Path((plugin_id, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let Some(plugin) = registry::get(&plugin_id) else {
        return Err(ApiError::not_found(format!("no plugin '{plugin_id}'")));
    };
    if !plugin.manifest.hooks.iter().any(|h| h.name == name) {
        return Err(ApiError::not_found(format!(
            "plugin '{plugin_id}' declares no hook '{name}'"
        )));
    }
    let secret = new_secret();
    sqlx::query(
        "INSERT INTO plugin_hooks (plugin_id, name, secret, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (plugin_id, name) DO UPDATE SET
             secret = excluded.secret,
             created_at = excluded.created_at,
             last_received_at = NULL",
    )
    .bind(&plugin_id)
    .bind(&name)
    .bind(&secret)
    .bind(unix_now())
    .execute(&db)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "secret": secret,
            "url": format!("/api/plugin-hooks/{plugin_id}/{name}"),
        })),
    ))
}

/// 64 hex characters from two random v4 UUIDs (OS randomness).
fn new_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
