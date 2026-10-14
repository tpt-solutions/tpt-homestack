// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! tpt-home-api — the HTTP surface of tpt-homestack.
//!
//! Assembles the Axum router from three sources, in the spirit of the spec:
//! core routes (auth, admin, pages), plus whatever routes, UI pages and
//! public endpoints the compiled-in blueprints registered through the
//! `tpt-home-macros` macros. No blueprint is ever hand-wired here.

pub mod admin_routes;
pub mod auth_routes;
pub mod backup;
pub mod backup_routes;
pub mod lan;
pub mod pages;
pub mod remote_routes;
pub mod safety;
pub mod settings_routes;
pub mod update_routes;
pub mod webhook_routes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};
use tower_http::cors::CorsLayer;

use tpt_home_core::RateLimiter;
use tpt_home_core::config::Config;
use tpt_home_core::db::{self, DbPool};
use tpt_home_core::middleware as mw;
use tpt_home_core::{config, migrate, registry};

/// Global body cap for JSON endpoints; the drop blueprint overrides it on
/// its upload route.
const DEFAULT_BODY_LIMIT: usize = 1024 * 1024;

/// Install the tracing subscriber (no-op if already installed, e.g. tests).
pub fn init_tracing(cfg: &Config) {
    let filter = cfg
        .log_level
        .clone()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| "info".to_string());
    // Logs go to stderr so that stdio protocols (the `mcp` command) keep stdout clean.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

/// Prepare everything that is shared between `serve` and CLI commands that
/// touch the database: dirs, global config, pool, migrations.
pub async fn init_server(cfg: Config) -> anyhow::Result<DbPool> {
    cfg.ensure_dirs()?;
    if !config::init_global(cfg.clone()) {
        anyhow::bail!("config already initialised");
    }
    let db = db::connect(&cfg.db_path()).await?;
    let report = prepare_database(&db, &cfg, false).await?;
    if let Some(copy) = &report.snapshot {
        tracing::info!(copy = %copy.display(), "saved a copy of the database before upgrading");
    }
    Ok(db)
}

/// What [`migrate_database`] found and did.
#[derive(Debug)]
pub struct MigrateReport {
    /// Migrations this binary would apply (applied now, unless dry run).
    pub pending: Vec<String>,
    /// Migrations applied by this call, in order.
    pub applied: Vec<String>,
    /// The copy of the database taken before changing it, if there was one.
    pub snapshot: Option<std::path::PathBuf>,
}

/// Bring the database in `cfg` up to date: refuse a database from a newer
/// version, keep a copy of an existing database before changing it, then
/// apply the pending migrations. With `dry_run`, only report what is pending.
pub async fn migrate_database(cfg: &Config, dry_run: bool) -> anyhow::Result<MigrateReport> {
    cfg.ensure_dirs()?;
    let db = db::connect(&cfg.db_path()).await?;
    prepare_database(&db, cfg, dry_run).await
}

async fn prepare_database(
    db: &DbPool,
    cfg: &Config,
    dry_run: bool,
) -> anyhow::Result<MigrateReport> {
    migrate::check_compatible(db).await?;
    let pending = migrate::pending_keys(db).await?;
    if dry_run || pending.is_empty() {
        return Ok(MigrateReport {
            pending,
            applied: Vec::new(),
            snapshot: None,
        });
    }
    let snapshot = if migrate::has_history(db).await? {
        Some(backup::upgrade_snapshot(db, cfg).await?)
    } else {
        None
    };
    let applied = migrate::run_all(db).await?;
    Ok(MigrateReport {
        pending,
        applied,
        snapshot,
    })
}

/// Build the full application router. Public for integration tests.
pub fn build_router(db: DbPool, cfg: &Config) -> Router {
    // --- unauthenticated branch -------------------------------------------
    let public = Router::<DbPool>::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/login", post(auth_routes::login))
        .route("/api/setup", post(auth_routes::setup))
        .route("/api/hooks/{name}", post(webhook_routes::receive))
        .route("/login", get(pages::login_page))
        .route("/setup", get(pages::setup_page))
        .merge(registry::apply_public_routes(Router::new()));

    // --- authenticated branch ---------------------------------------------
    // Admin-only routes get a role guard on top of the session resolver.
    let admin_role = || {
        axum::middleware::from_fn(
            |req: axum::extract::Request, next: axum::middleware::Next| async move {
                mw::role_guard(req, next, "admin").await
            },
        )
    };
    let upload_limit = (cfg.max_upload_mb as usize).saturating_mul(1024 * 1024);
    let shared_cfg = std::sync::Arc::new(cfg.clone());
    let protected = Router::<DbPool>::new()
        .route("/", get(pages::root))
        .route("/dashboard", get(pages::dashboard_page))
        .route("/settings", get(pages::settings_page))
        .route("/admin/users", get(pages::users_page))
        .route("/admin/jobs", get(pages::jobs_page))
        .route("/admin/plugins", get(pages::plugins_page))
        .route("/admin/remote", get(pages::remote_page))
        .route("/admin/status", get(pages::status_page))
        .route("/welcome", get(pages::welcome_page))
        .route("/api/settings/server", get(settings_routes::get))
        .route(
            "/api/admin/server",
            put(settings_routes::update).layer(admin_role()),
        )
        .route(
            "/api/admin/jobs",
            get(admin_routes::jobs).layer(admin_role()),
        )
        .route("/api/auth/logout", post(auth_routes::logout))
        .route("/api/auth/me", get(auth_routes::me))
        .route(
            "/api/auth/change-password",
            post(auth_routes::change_password),
        )
        .route(
            "/api/admin/users",
            get(admin_routes::list)
                .post(admin_routes::create)
                .layer(admin_role()),
        )
        .route(
            "/api/admin/users/{id}",
            patch(admin_routes::update)
                .delete(admin_routes::delete)
                .layer(admin_role()),
        )
        .route(
            "/api/webhooks",
            get(webhook_routes::list)
                .post(webhook_routes::create)
                .layer(admin_role()),
        )
        .route(
            "/api/webhooks/{id}",
            delete(webhook_routes::delete).layer(admin_role()),
        )
        .route(
            "/api/webhooks/{id}/deliveries",
            get(webhook_routes::deliveries).layer(admin_role()),
        )
        .route(
            "/api/admin/health",
            get(admin_routes::health).layer(admin_role()),
        )
        .route(
            "/api/admin/remote",
            get(remote_routes::status).layer(admin_role()),
        )
        .route(
            "/api/admin/update",
            get(update_routes::status).layer(admin_role()),
        )
        .route(
            "/api/admin/backups",
            get(backup_routes::list)
                .post(backup_routes::create)
                .layer(admin_role()),
        )
        .route(
            "/api/admin/backups/{name}/download",
            get(backup_routes::download).layer(admin_role()),
        )
        .route(
            "/api/admin/backups/upload",
            post(backup_routes::upload)
                .layer(DefaultBodyLimit::max(upload_limit))
                .layer(admin_role()),
        )
        .merge(registry::apply_routes(Router::new()))
        .merge(registry::apply_ui_routes(Router::new()))
        .layer(Extension(shared_cfg))
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(safety::Safety {
                db: db.clone(),
                exposed: cfg.exposed(),
            }),
            safety::guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            db.clone(),
            mw::auth_resolver,
        ));

    // --- global stack -------------------------------------------------------
    let limiter = RateLimiter::new(240, Duration::from_secs(60)).trust_proxy(cfg.trust_proxy);
    let mut app = Router::new()
        .merge(public)
        .merge(protected)
        .layer(axum::middleware::from_fn(mw::security_headers))
        .layer(axum::middleware::from_fn(mw::csrf_guard))
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            mw::rate_limit_fn,
        ))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        // Outermost, so plain-HTTP requests are answered before anything else runs.
        .layer(axum::middleware::from_fn_with_state(
            mw::HttpsPolicy {
                required: cfg.require_https,
                trust_proxy: cfg.trust_proxy,
            },
            mw::https_required,
        ));

    if !cfg.cors_origins.is_empty() {
        let origins: Vec<_> = cfg
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        app = app.layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_credentials(true)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );
    }

    app.with_state(db)
}

/// GET /healthz — liveness probe, no auth.
async fn healthz() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "ok": true,
        "service": "tpt-homestack",
        "version": tpt_home_core::VERSION,
    }))
}

/// Periodic session purge (runs for the process lifetime).
async fn session_purge_loop(db: DbPool) {
    let mut ticker = tokio::time::interval(Duration::from_secs(3600));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match tpt_home_core::auth::session::purge_expired(&db).await {
            Ok(n) if n > 0 => tracing::debug!(purged = n, "expired sessions removed"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = ?e, "session purge failed"),
        }
    }
}

/// Ctrl-C / SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl-c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

/// Load config, run migrations, start blueprint tasks and serve HTTP until
/// shutdown. This is `tpt-home run`.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    init_tracing(&cfg);
    let db = init_server(cfg.clone()).await?;
    registry::spawn_startup_tasks(db.clone());
    registry::spawn_jobs(db.clone());
    admin_routes::started_at();
    tokio::spawn(backup::schedule_loop(db.clone(), cfg.clone()));
    tokio::spawn(session_purge_loop(db.clone()));

    let app = build_router(db, &cfg);
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    // Held until the server stops; dropping it withdraws the LAN name.
    let _lan_name = match lan::announce(&cfg) {
        Ok(announcement) => {
            if announcement.is_some() {
                tracing::info!(url = %lan::lan_url(&cfg), "announced on the local network");
            }
            announcement
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not announce the server on the local network");
            None
        }
    };
    tracing::info!(
        version = tpt_home_core::VERSION,
        %addr,
        blueprints = registry::blueprints().count(),
        "tpt-homestack listening"
    );
    if registry::nav_entries().is_empty() {
        tracing::info!("no blueprint UI pages registered yet — the dashboard will be empty");
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}
