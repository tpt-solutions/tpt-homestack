// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! HTTP API for the file inbox: list, upload, download and delete.

use std::path::Path as FsPath;

use axum::extract::{Multipart, Path};
use axum::extract::multipart::Field;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use tokio::io::AsyncWriteExt;
use tpt_home_core::config::global;
use tpt_home_core::files;
use tpt_home_core::prelude::*;
use tpt_home_macros::bp_route;

/// Files are kept in `data/uploads/<NAME>/`, outside the web root.
fn upload_dir() -> std::path::PathBuf {
    global().uploads_dir().join("<NAME>")
}

#[derive(serde::Serialize, sqlx::FromRow)]
pub struct FileRow {
    pub id: i64,
    pub original_name: String,
    pub size: i64,
    pub mime: String,
    pub created_at: i64,
}

#[derive(sqlx::FromRow)]
struct StoredFile {
    stored_name: String,
    original_name: String,
    mime: String,
}

#[bp_route(get, "/api/<NAME>/files")]
async fn list_files(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let rows: Vec<FileRow> = sqlx::query_as(
        "SELECT id, original_name, size, mime, created_at FROM <NAME>_files
         ORDER BY id DESC LIMIT 500",
    )
    .fetch_all(&db)
    .await?;
    Ok(Json(rows))
}

/// `POST /api/<NAME>/files`: multipart form with one field named `file`.
#[bp_route(post, "/api/<NAME>/files")]
async fn upload_file(
    State(db): State<DbPool>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let max_bytes = global().max_upload_mb.saturating_mul(1024 * 1024);
    tokio::fs::create_dir_all(upload_dir()).await?;

    let mut saved: Option<(String, String, u64, String)> = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart error: {e}")))?
    {
        if field.name() != Some("file") {
            let _ = field.bytes().await;
            continue;
        }
        let original = field
            .file_name()
            .map(files::sanitize_filename)
            .unwrap_or_else(|| "file".into());
        let mime = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        // Random-enough prefix so two uploads with the same name never clash.
        let stored = format!("{}_{}", unix_nanos(), original);
        let dest = files::safe_join(&upload_dir(), &[&stored])?;
        let size = match write_field(&mut field, &dest, max_bytes).await {
            Ok(size) => size,
            Err(err) => {
                let _ = tokio::fs::remove_file(&dest).await;
                return Err(err);
            }
        };
        saved = Some((stored, original, size, mime));
        break;
    }

    let Some((stored, original, size, mime)) = saved else {
        return Err(ApiError::bad_request("send a multipart field named 'file'"));
    };
    let inserted: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "INSERT INTO <NAME>_files (original_name, stored_name, size, mime, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
    )
    .bind(&original)
    .bind(&stored)
    .bind(size as i64)
    .bind(&mime)
    .bind(unix_secs())
    .fetch_one(&db)
    .await;
    match inserted {
        Ok((id,)) => Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id, "size": size })))),
        Err(err) => {
            let _ = tokio::fs::remove_file(upload_dir().join(&stored)).await;
            Err(err.into())
        }
    }
}

/// Stream the field to `dest` in chunks, so memory stays flat for any size.
async fn write_field(
    field: &mut Field<'_>,
    dest: &FsPath,
    max_bytes: u64,
) -> Result<u64, ApiError> {
    let mut file = tokio::fs::File::create(dest).await?;
    let mut total: u64 = 0;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) => return Err(ApiError::bad_request("upload stream failed")),
        };
        total += chunk.len() as u64;
        if total > max_bytes {
            return Err(ApiError::bad_request(format!(
                "file exceeds max_upload_mb ({})",
                global().max_upload_mb
            )));
        }
        file.write_all(&chunk)
            .await
            .map_err(|_| ApiError::internal("failed to write upload"))?;
    }
    file.flush().await?;
    Ok(total)
}

#[bp_route(get, "/api/<NAME>/files/{id}")]
async fn download_file(
    State(db): State<DbPool>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let row = stored_file(&db, id).await?;
    let path = files::safe_join(&upload_dir(), &[&row.stored_name])?;
    if !files::path_within_canonical(&upload_dir(), &path) {
        return Err(ApiError::bad_request("invalid stored path"));
    }
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::not_found("file data missing"))?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
    let mut response = axum::body::Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        row.mime
            .parse()
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    let disposition = format!(
        "attachment; filename=\"{}\"",
        row.original_name.replace(['"', '\\'], "_")
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        disposition
            .parse()
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(response)
}

#[bp_route(delete, "/api/<NAME>/files/{id}")]
async fn delete_file(
    State(db): State<DbPool>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let row = stored_file(&db, id).await?;
    if let Ok(path) = files::safe_join(&upload_dir(), &[&row.stored_name]) {
        // A missing data file should not block removing the row.
        let _ = tokio::fs::remove_file(path).await;
    }
    sqlx::query("DELETE FROM <NAME>_files WHERE id = ?1")
        .bind(id)
        .execute(&db)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn stored_file(db: &DbPool, id: i64) -> Result<StoredFile, ApiError> {
    let row: Option<StoredFile> = sqlx::query_as(
        "SELECT stored_name, original_name, mime FROM <NAME>_files WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    row.ok_or_else(|| ApiError::not_found("file not found"))
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
