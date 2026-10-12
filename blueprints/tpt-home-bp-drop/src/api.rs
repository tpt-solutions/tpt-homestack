// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Drop API: streaming upload, streaming download, share links, deletion.

use std::path::PathBuf;

use axum::Json;
use axum::body::Body;
use axum::extract::Multipart;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use tpt_home_core::files;
use tpt_home_core::prelude::*;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DropRow {
    pub id: i64,
    pub filename: String,
    #[serde(skip)]
    pub stored_path: String,
    pub size: i64,
    pub mime: String,
    pub share_token: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct ShareInput {
    /// Share link lifetime in hours; `None` = no expiry.
    pub hours: Option<u64>,
    /// Optional password protecting the share link.
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SharePwQuery {
    pub pw: Option<String>,
}

fn upload_dir() -> PathBuf {
    global().uploads_dir().join("drop")
}

/// GET /api/drop
pub async fn list(db: &DbPool) -> Result<Vec<DropRow>, ApiError> {
    let rows = sqlx::query_as::<_, DropRow>(
        "SELECT id, filename, stored_path, size, mime, share_token, expires_at, created_at
         FROM drop_files ORDER BY created_at DESC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// POST /api/drop/upload — multipart form, field name `file`.
///
/// Streams chunks straight to disk; memory use stays constant regardless of
/// file size, and the per-route body limit comes from `max_upload_mb`.
pub async fn upload(
    db: &DbPool,
    user_id: i64,
    mut multipart: Multipart,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let max_bytes = (global().max_upload_mb as u64) * 1024 * 1024;
    tokio::fs::create_dir_all(upload_dir()).await?;

    let mut uploaded: Option<(String, String, u64, String)> = None; // (dest, filename, size, mime)

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart error: {e}")))?
    {
        match field.name() {
            Some("file") => {
                let original = field
                    .file_name()
                    .map(files::sanitize_filename)
                    .unwrap_or_else(|| "file".into());
                let mime = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let stored = format!("{}_{}", uuid_simple(), original);
                let dest = files::safe_join(&upload_dir(), &[&stored])?;
                let mut file = tokio::fs::File::create(&dest).await?;
                let mut total: u64 = 0;
                let mut overflow: Option<ApiError> = None;
                loop {
                    match field.chunk().await {
                        Ok(Some(chunk)) => {
                            total += chunk.len() as u64;
                            if total > max_bytes {
                                overflow = Some(ApiError::bad_request(format!(
                                    "file exceeds max_upload_mb ({})",
                                    global().max_upload_mb
                                )));
                                break;
                            }
                            if file.write_all(&chunk).await.is_err() {
                                overflow = Some(ApiError::internal("failed to write upload"));
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(_) => {
                            overflow = Some(ApiError::bad_request("upload stream failed"));
                            break;
                        }
                    }
                }
                file.flush().await.ok();
                drop(file);
                if let Some(err) = overflow {
                    let _ = tokio::fs::remove_file(&dest).await;
                    return Err(err);
                }
                uploaded = Some((stored, original, total, mime));
            }
            Some("ttl_hours") => {
                // Read and discard here; expiry is applied below via query.
                let _ = field.text().await;
            }
            _ => {
                let _ = field.text().await;
            }
        }
    }

    let (stored, filename, size, mime) =
        uploaded.ok_or_else(|| ApiError::bad_request("multipart field \"file\" is required"))?;

    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO drop_files (filename, stored_path, size, mime, created_at, owner_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id",
    )
    .bind(&filename)
    .bind(&stored)
    .bind(size as i64)
    .bind(&mime)
    .bind(unix_now())
    .bind(user_id)
    .fetch_one(db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "filename": filename, "size": size, "mime": mime })),
    ))
}

async fn drop_row_by_id(db: &DbPool, id: i64) -> Result<Option<DropRow>, ApiError> {
    Ok(sqlx::query_as::<_, DropRow>(
        "SELECT id, filename, stored_path, size, mime, share_token, expires_at, created_at
         FROM drop_files WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?)
}

/// GET /api/drop/{id}/download — authenticated, streamed.
pub async fn download(db: &DbPool, id: i64) -> Result<axum::response::Response, ApiError> {
    let row = drop_row_by_id(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;
    stream_file(&row).await
}

/// GET /api/drop/shared/{token}?pw=...
pub async fn shared(
    db: &DbPool,
    token: String,
    q: SharePwQuery,
) -> Result<axum::response::Response, ApiError> {
    let row = sqlx::query_as::<_, DropRow>(
        "SELECT id, filename, stored_path, size, mime, share_token, expires_at, created_at
         FROM drop_files WHERE share_token = ?1",
    )
    .bind(&token)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::not_found("share not found"))?;

    if let Some(exp) = row.expires_at
        && exp <= unix_now()
    {
        return Err(ApiError::not_found("share expired"));
    }

    let (hash,): (Option<String>,) =
        sqlx::query_as("SELECT share_password_hash FROM drop_files WHERE id = ?1")
            .bind(row.id)
            .fetch_one(db)
            .await?;
    if let Some(hash) = hash {
        let given = q.pw.clone().unwrap_or_default();
        // Public endpoint: never leak whether it was the password.
        if !tpt_home_core::auth::password::verify_password(&given, &hash) {
            return Err(ApiError::Unauthorized);
        }
    }
    stream_file(&row).await
}

async fn stream_file(row: &DropRow) -> Result<axum::response::Response, ApiError> {
    let path = files::safe_join(&upload_dir(), &[&row.stored_path])?;
    if !files::path_within_canonical(&upload_dir(), &path) {
        return Err(ApiError::bad_request("invalid stored path"));
    }
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::not_found("file data missing"))?;

    // Touch nothing in memory: tokio -> ReaderStream -> hyper.
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
    let mut res = Body::from_stream(stream).into_response();
    let headers = res.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        row.mime
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    let disposition = format!(
        "attachment; filename=\"{}\"",
        row.filename.replace(['"', '\\'], "_")
    );
    if let Ok(hv) = header::HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, hv);
    }
    headers.insert(
        header::CONTENT_LENGTH,
        header::HeaderValue::from_str(&row.size.to_string())
            .unwrap_or_else(|_| header::HeaderValue::from_static("0")),
    );
    Ok(res)
}

/// POST /api/drop/{id}/share — create (or replace) the share link.
pub async fn share(
    db: &DbPool,
    id: i64,
    body: ShareInput,
) -> Result<Json<serde_json::Value>, ApiError> {
    drop_row_by_id(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;

    let token = uuid_simple();
    let expires_at = body.hours.map(|h| unix_now() + (h * 3600) as i64);
    let hash = match &body.password {
        Some(pw) if !pw.is_empty() => Some(tpt_home_core::auth::password::hash_password(pw)?),
        _ => None,
    };
    sqlx::query(
        "UPDATE drop_files SET share_token = ?1, share_password_hash = ?2, expires_at = ?3 WHERE id = ?4",
    )
    .bind(&token)
    .bind(&hash)
    .bind(expires_at)
    .bind(id)
    .execute(db)
    .await?;

    Ok(Json(serde_json::json!({
        "url": format!("/api/drop/shared/{token}"),
        "expires_at": expires_at,
        "password_protected": hash.is_some(),
    })))
}

/// DELETE /api/drop/{id} — removes the row and the file.
pub async fn delete(db: &DbPool, id: i64) -> Result<(), ApiError> {
    let row = drop_row_by_id(db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("file not found"))?;
    sqlx::query("DELETE FROM drop_files WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    if let Ok(path) = files::safe_join(&upload_dir(), &[&row.stored_path]) {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
}

/// POST /api/drop/cleanup — manual trigger for the expiry sweep.
pub async fn cleanup_now(db: &DbPool) -> Result<u32, ApiError> {
    super::cleanup::sweep(db).await
}

fn uuid_simple() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
