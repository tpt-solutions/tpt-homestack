// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Core engine for tpt-homestack.
//!
//! Contains everything a blueprint needs to plug into the platform:
//! SQLite access ([`db`]), configuration ([`config`]), the migration runner
//! ([`migrate`]), the blueprint registry ([`registry`]), authentication and
//! sessions ([`auth`]), safe file I/O ([`files`]) and shared HTTP plumbing
//! ([`error`], [`middleware`]).
//!
//! Blueprints normally only need `use tpt_home_core::prelude::*;`.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod files;
pub mod middleware;
pub mod migrate;
pub mod migrations;
pub mod prelude;
pub mod registry;
pub mod settings;
pub mod webhook;

// Convenience re-exports at the crate root.
pub use error::ApiError;
/// Re-exported so blueprint code never needs a direct `inventory` dep: the
/// macros emit `tpt_home_core::inventory::submit!`.
pub use inventory;
pub use middleware::RateLimiter;
pub use migrate::MigrationEntry;
pub use registry::{
    Blueprint, BlueprintEntry, JobEntry, PublicRouteEntry, RouteEntry, StartupEntry, UiEntry,
};

/// Crate version, as reported by `/healthz`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
