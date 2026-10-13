// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Server settings for the first-run wizard: the server's name and whether
//! the wizard is finished. Any signed-in user may read them; only admins may
//! change them.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

use tpt_home_core::db::DbPool;
use tpt_home_core::prelude::*;
use tpt_home_core::settings;

#[derive(Serialize)]
pub struct ServerSettings {
    pub name: String,
    pub wizard_done: bool,
}

/// GET /api/settings/server
pub async fn get(State(db): State<DbPool>) -> Json<ServerSettings> {
    Json(read(&db).await)
}

#[derive(Deserialize, Default)]
pub struct UpdateServer {
    pub name: Option<String>,
    pub wizard_done: Option<bool>,
}

/// PUT /api/admin/server: change the name and/or finish the wizard.
pub async fn update(
    State(db): State<DbPool>,
    Json(body): Json<UpdateServer>,
) -> Result<Json<ServerSettings>, ApiError> {
    if let Some(name) = &body.name {
        let valid = settings::valid_server_name(name).ok_or_else(|| {
            ApiError::bad_request("the name must be 1 to 64 characters with no control characters")
        })?;
        settings::set(&db, settings::SERVER_NAME, &valid).await?;
    }
    if let Some(done) = body.wizard_done {
        settings::set(&db, settings::WIZARD_DONE, if done { "1" } else { "0" }).await?;
    }
    Ok(Json(read(&db).await))
}

async fn read(db: &DbPool) -> ServerSettings {
    ServerSettings {
        name: settings::server_name(db).await,
        wizard_done: settings::wizard_done(db).await,
    }
}
