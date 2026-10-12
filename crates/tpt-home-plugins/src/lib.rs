// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home-plugins` — the WASM plugin host for tpt-homestack.
//!
//! Community plugins are `wasmi`-sandboxed modules shipped as checksummed
//! `.tptp` archives. The host grants capabilities declared in `plugin.toml`
//! and approved by the admin at install time, dispatches `/api/<id>/…` into
//! the plugin, renders manifest-declared pages, and supports hot
//! enable/disable without a restart. See `docs/PLUGINS.md`.

pub mod dispatch;
pub mod host;
pub mod lifecycle;
pub mod manifest;
pub mod package;
pub mod registry;
pub mod runtime;
pub mod scope;
#[cfg(feature = "js")]
pub mod script;

/// Link anchor for the binary: see `tpt-home-cli`'s `BLUEPRINT_LINK_ANCHORS`.
/// Never called at runtime; its address keeps the plugin host's startup and
/// route registrations in the binary.
#[inline(never)]
pub fn __link_anchor() -> &'static str {
    env!("CARGO_PKG_NAME")
}

// The plugin API catch-all: `/api/<plugin_id>/<rest>` for every method.
// Registered manually (the macro registers one method per route); static
// routes elsewhere under `/api` win over the parameter.
inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "ANY",
        path: "/api/{plugin_id}/{*rest}",
        role: None,
        register: |router| {
            use axum::routing::get;
            router.route(
                "/api/{plugin_id}/{*rest}",
                get(dispatch::api_invoke)
                    .post(dispatch::api_invoke)
                    .put(dispatch::api_invoke)
                    .delete(dispatch::api_invoke)
                    .patch(dispatch::api_invoke),
            )
        },
    }
}

// Plugin pages: `/plugins/<plugin_id>/<page-path>`, behind login.
inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "ANY",
        path: "/plugins/{plugin_id}/{*rest}",
        role: None,
        register: |router| {
            router.route(
                "/plugins/{plugin_id}/{*rest}",
                axum::routing::get(dispatch::page),
            )
        },
    }
}

// Load installed plugins (and the nav provider) at server startup.
inventory::submit! {
    tpt_home_core::StartupEntry {
        name: "plugins-load",
        spawn: |db| {
            tokio::spawn(async move {
                if let Err(e) = lifecycle::load_installed(&db).await {
                    tracing::error!(error = %e, "failed to load plugins");
                }
            });
        },
    }
}
