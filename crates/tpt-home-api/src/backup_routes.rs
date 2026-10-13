// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Admin backup API under `/api/admin/backups`: list, take a backup now,
//! download a backup's database, and upload a database as a new backup. All
//! routes sit behind the admin role guard. Restoring is not done here: it
//! needs the server stopped, so the page points at `tpt-home restore`.

use std::path::Path;
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::io::AsyncWriteExt;

use tpt_home_core::config::Config;
use tpt_home_core::db::DbPool;
use tpt_home_core::prelude::*;

use crate::backup::{self, BackupInfo};

/// GET /api/admin/backups
pub async fn list(Extension(cfg): Extension<Arc<Config>>) -> Json<Vec<BackupInfo>> {
    Json(backup::list(&backup::default_dir(&cfg)))
}

/// POST /api/admin/backups: take a backup now.
pub async fn create(
    State(db): State<DbPool>,
    Extension(cfg): Extension<Arc<Config>>,
) -> Result<impl IntoResponse, ApiError> {
    let info = backup::create(&db, &cfg, &backup::default_dir(&cfg))
        .await
        .map_err(|e| ApiError::internal(format!("backup failed: {e}")))?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// GET /api/admin/backups/{name}/download: the database file. Uploads are not
/// included; the file is for `tpt-home restore` or for keeping off-site.
pub async fn download(
    Extension(cfg): Extension<Arc<Config>>,
    UrlPath(name): UrlPath<String>,
) -> Result<Response, ApiError> {
    let path = backup::db_file_for(&backup::default_dir(&cfg), &name)
        .ok_or_else(|| ApiError::not_found("no such backup"))?;
    let file = tokio::fs::File::open(&path).await?;
    let len = file.metadata().await?.len();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_LENGTH, len.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"tpt-{name}.db\""),
            ),
        ],
        body,
    )
        .into_response())
}

/// POST /api/admin/backups/upload: a multipart form with one field `file`.
/// The file is streamed to a staging file (size capped by the route's body
/// limit), checked as a tpt-home database, and kept as a new backup.
pub async fn upload(
    Extension(cfg): Extension<Arc<Config>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let dir = backup::default_dir(&cfg);
    std::fs::create_dir_all(&dir)?;
    let staged = dir.join(format!(".upload-{}.tmp", uuid::Uuid::new_v4()));

    let result = receive_and_import(&mut multipart, &dir, &staged).await;
    // The import moves the file on success; on any failure, remove it.
    let _ = tokio::fs::remove_file(&staged).await;
    Ok((StatusCode::CREATED, Json(result?)))
}

async fn receive_and_import(
    multipart: &mut Multipart,
    dir: &Path,
    staged: &Path,
) -> Result<BackupInfo, ApiError> {
    let mut found = false;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("bad upload: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let mut out = tokio::fs::File::create(staged).await?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::bad_request(format!("bad upload: {e}")))?
        {
            out.write_all(&chunk).await?;
        }
        out.flush().await?;
        found = true;
        break;
    }
    if !found {
        return Err(ApiError::bad_request(
            "send the database as form field 'file'",
        ));
    }
    backup::import(dir, staged).await.map_err(|e| {
        // The detail names server paths, so it goes to the log, not the client.
        tracing::warn!(error = %e, "uploaded backup refused");
        ApiError::bad_request("not a tpt-home backup database")
    })
}
