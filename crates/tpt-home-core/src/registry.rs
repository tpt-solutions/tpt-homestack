// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Blueprint registry.
//!
//! Blueprints register themselves at compile time through `inventory`
//! entries emitted by the `tpt-home-macros` attribute macros. At startup the
//! API crate walks these collections and wires routes/UI/migrations in —
//! no manual registration code anywhere.

use axum::Router;
use inventory as inv;

use crate::db::DbPool;

/// The trait implemented for every `#[blueprint]` struct. Deliberately tiny:
/// routes, migrations and UI pages register themselves separately, so a
/// blueprint needs zero boilerplate methods.
pub trait Blueprint: Send + Sync + 'static {
    /// Short id, e.g. `"myapp"`.
    fn id(&self) -> &'static str;
    /// Human-readable display name.
    fn name(&self) -> &'static str;
}

/// Metadata for one compiled-in blueprint, emitted by `#[blueprint]`.
pub struct BlueprintEntry {
    /// Short id, e.g. `"notes"` (from `tpt-home-bp-notes`).
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
}

inv::collect!(BlueprintEntry);

/// One API route contributed by `#[bp_route]`.
///
/// `register` receives the shared `Router<DbPool>` and returns it with the
/// route attached, so any axum combinator stays available.
pub struct RouteEntry {
    /// Crate that contributed the route (i.e. the blueprint crate name).
    pub blueprint: &'static str,
    /// HTTP method, uppercase (`"GET"`, `"POST"`, ...).
    pub method: &'static str,
    /// Full path; must start with `/api/`.
    pub path: &'static str,
    /// Minimum role required, `Some("admin")` etc.
    pub role: Option<&'static str>,
    /// Adds this route to the router.
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(RouteEntry);

/// One *unauthenticated* API route contributed by a blueprint (e.g. the
/// drop blueprint's public share links). Emitted by
/// `#[bp_route(..., public)]`.
pub struct PublicRouteEntry {
    pub blueprint: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(PublicRouteEntry);

/// One UI page contributed by `#[bp_ui_route]`: a server-rendered page plus a
/// sidebar entry. Paths live outside `/api/`.
pub struct UiEntry {
    pub blueprint: &'static str,
    /// Page path, e.g. `"/notes"`.
    pub path: &'static str,
    /// Sidebar label.
    pub label: &'static str,
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(UiEntry);

/// A background task contributed by a blueprint (e.g. the drop cleanup
/// cron). `spawn` is called once at server startup with the shared pool.
pub struct StartupEntry {
    /// Unique task name for logs.
    pub name: &'static str,
    /// Spawns the task onto the current tokio runtime.
    pub spawn: fn(DbPool),
}

inv::collect!(StartupEntry);

/// All registered blueprints in registration order.
pub fn blueprints() -> impl Iterator<Item = &'static BlueprintEntry> {
    inv::iter::<BlueprintEntry>()
}

/// Apply every blueprint API route to `router`.
pub fn apply_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<RouteEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Apply every blueprint public route to the *unauthenticated* router.
pub fn apply_public_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<PublicRouteEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Apply every blueprint UI page to `router`.
pub fn apply_ui_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<UiEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Spawn every blueprint background task.
pub fn spawn_startup_tasks(db: DbPool) {
    for entry in inv::iter::<StartupEntry>() {
        (entry.spawn)(db.clone());
        tracing::debug!(task = entry.name, "started blueprint task");
    }
}

/// Sidebar navigation derived from registered UI pages.
pub fn nav_entries() -> Vec<(&'static str, &'static str)> {
    inv::iter::<UiEntry>().map(|e| (e.label, e.path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collections_are_iterable_without_entries() {
        // Core alone registers no blueprints; iteration must not panic and
        // nav derives from the same collection.
        assert_eq!(nav_entries().len(), blueprints().count());
    }
}
