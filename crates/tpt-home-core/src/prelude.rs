// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The one import blueprint authors need:
//!
//! ```ignore
//! use tpt_home_core::prelude::*;
//! ```

pub use crate::auth::session as session_store;
pub use crate::auth::user::{AuthUser, Role};
pub use crate::auth::{AuthError, password};
pub use crate::config::{Config, global};
pub use crate::db::DbPool;
pub use crate::error::ApiError;
pub use crate::middleware;
pub use crate::migrate::{self, MigrationEntry};
pub use crate::registry::{
    self, Blueprint, BlueprintEntry, JobEntry, RouteEntry, StartupEntry, UiEntry,
};

// axum pieces blueprint handlers touch daily.
pub use axum::Json;
pub use axum::Router;
pub use axum::extract::State;
pub use axum::response::{Html, IntoResponse, Redirect, Response};

// Re-export sqlx so blueprint code can write `sqlx::query_as` with a single
// `use tpt_home_core::prelude::*;`.
pub use sqlx;
