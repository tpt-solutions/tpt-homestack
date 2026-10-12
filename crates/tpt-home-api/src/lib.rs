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
pub mod pages;

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post};
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
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_target(false)
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
    migrate::run_all(&db).await?;
    Ok(db)
}

/// Build the full application router. Public for integration tests.
pub fn build_router(db: DbPool, cfg: &Config) -> Router {
    // --- unauthenticated branch -------------------------------------------
    let public = Router::<DbPool>::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/login", post(auth_routes::login))
        .route("/api/setup", post(auth_routes::setup))
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
    let protected = Router::<DbPool>::new()
        .route("/", get(pages::root))
        .route("/dashboard", get(pages::dashboard_page))
        .route("/settings", get(pages::settings_page))
        .route("/admin/users", get(pages::users_page))
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
        .merge(registry::apply_routes(Router::new()))
        .merge(registry::apply_ui_routes(Router::new()))
        .layer(axum::middleware::from_fn_with_state(
            db.clone(),
            mw::auth_resolver,
        ));

    // --- global stack -------------------------------------------------------
    let limiter = RateLimiter::new(240, Duration::from_secs(60));
    let mut app = Router::new()
        .merge(public)
        .merge(protected)
        .layer(axum::middleware::from_fn(mw::security_headers))
        .layer(axum::middleware::from_fn(mw::csrf_guard))
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            mw::rate_limit_fn,
        ))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT));

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
    tokio::spawn(session_purge_loop(db.clone()));

    let app = build_router(db, &cfg);
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
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
