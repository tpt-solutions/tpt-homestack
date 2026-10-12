// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Admin-only user management API. Mounted under `/api/admin/users` with a
//! role guard; safe-guards: you cannot delete yourself, and you cannot strip
//! or delete the last remaining admin.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use tpt_home_core::auth::user::{self, AuthUser, Role};
use tpt_home_core::prelude::*;

#[derive(serde::Serialize)]
pub struct UserView {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: i64,
}

fn view(u: &user::User) -> UserView {
    UserView {
        id: u.id,
        username: u.username.clone(),
        role: u.role().as_str().to_string(),
        disabled: u.disabled,
        created_at: u.created_at,
    }
}

/// GET /api/admin/users
pub async fn list(State(db): State<DbPool>) -> Result<Json<Vec<UserView>>, ApiError> {
    let users = user::list_users(&db).await?;
    Ok(Json(users.iter().map(view).collect()))
}

#[derive(Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub password: String,
    pub role: String,
}

/// POST /api/admin/users
pub async fn create(
    State(db): State<DbPool>,
    Json(body): Json<CreateUser>,
) -> Result<impl IntoResponse, ApiError> {
    let role = Role::parse(&body.role)
        .ok_or_else(|| ApiError::bad_request("role must be 'admin' or 'user'"))?;
    let created = user::create_user(&db, &body.username, &body.password, role).await?;
    Ok((axum::http::StatusCode::CREATED, Json(view(&created))))
}

#[derive(Deserialize, Default)]
pub struct UpdateUser {
    pub role: Option<String>,
    pub disabled: Option<bool>,
    pub password: Option<String>,
}

async fn count_admins(db: &DbPool) -> Result<i64, ApiError> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin' AND disabled = 0")
            .fetch_one(db)
            .await?;
    Ok(n)
}

/// PATCH /api/admin/users/{id}
pub async fn update(
    State(db): State<DbPool>,
    Extension(caller): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUser>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let target = user::get_user_by_id(&db, id)
        .await
        .map_err(|_| ApiError::not_found("user not found"))?;

    if let Some(role) = &body.role {
        let new_role = Role::parse(role)
            .ok_or_else(|| ApiError::bad_request("role must be 'admin' or 'user'"))?;
        if target.role() == Role::Admin && new_role == Role::User && count_admins(&db).await? <= 1 {
            return Err(ApiError::conflict("cannot demote the last admin"));
        }
        user::set_role(&db, id, new_role).await?;
    }

    if let Some(disabled) = body.disabled {
        if disabled {
            if target.id == caller.id {
                return Err(ApiError::conflict("you cannot disable yourself"));
            }
            if target.role() == Role::Admin && count_admins(&db).await? <= 1 {
                return Err(ApiError::conflict("cannot disable the last admin"));
            }
        }
        user::set_disabled(&db, id, disabled).await?;
    }

    if let Some(pw) = &body.password {
        user::set_password(&db, id, pw).await?;
    }

    let updated = user::get_user_by_id(&db, id).await?;
    Ok(Json(json!(view(&updated))))
}

/// DELETE /api/admin/users/{id}
pub async fn delete(
    State(db): State<DbPool>,
    Extension(caller): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if id == caller.id {
        return Err(ApiError::conflict("you cannot delete yourself"));
    }
    let target = user::get_user_by_id(&db, id)
        .await
        .map_err(|_| ApiError::not_found("user not found"))?;
    if target.role() == Role::Admin && count_admins(&db).await? <= 1 {
        return Err(ApiError::conflict("cannot delete the last admin"));
    }
    user::delete_user(&db, id).await?;
    Ok(Json(json!({ "ok": true })))
}

/// GET /api/admin/jobs: each scheduled job's last run, outcome and next run.
pub async fn jobs() -> Json<Vec<registry::JobStatus>> {
    Json(registry::job_statuses())
}
