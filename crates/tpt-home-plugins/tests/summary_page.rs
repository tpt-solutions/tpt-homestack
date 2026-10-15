// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `summary` pages: metric cards computed from scoped SELECTs. Renders a page
//! of an installed plugin through the real page handler, and checks that the
//! manifest refuses metrics that are not single scoped SELECTs.
//!
//! Like the other plugin tests, these share one database file and take turns.

#![cfg(feature = "js")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use axum::Extension;
use axum::extract::{Path as AxPath, Query, State};
use tpt_home_core::auth::user::{AuthUser, Role};
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::{DbPool, connect};
use tpt_home_plugins::dispatch;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::{self, Permissions};

const MANIFEST: &str = r#"
[plugin]
id = "ID"
name = "Summary test"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
db = true

[[tables]]
name = "items"
columns = [{ name = "label", kind = "text" }, { name = "qty", kind = "integer" }]

[[pages]]
path = "/summary"
title = "Summary"
kind = "summary"
metrics = [
    { label = "Items", sql = "SELECT COUNT(*) FROM ID_items" },
    { label = "Quantity", sql = "SELECT SUM(qty) FROM ID_items" },
    { label = "Average", sql = "SELECT AVG(qty) FROM ID_items" },
    { label = "Nothing", sql = "SELECT qty FROM ID_items WHERE qty > 1000" },
]
"#;

fn data_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::mem::forget(dir);
        set_global_for_tests(Config {
            data_dir: root.clone(),
            ..Config::default()
        });
        root
    })
}

fn config() -> Config {
    Config {
        data_dir: data_root().to_path_buf(),
        ..Config::default()
    }
}

/// One test at a time over the shared database file.
async fn turn() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Install a fresh copy under `id` with two rows in its table.
async fn install(id: &str) -> DbPool {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect(&config().db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), MANIFEST.replace("ID", id)).unwrap();
    std::fs::write(
        dir.join("main.js"),
        "function handle() { return { status: 404, body: '' }; }",
    )
    .unwrap();
    let grant = Permissions::parse_list("db").unwrap();
    lifecycle::install_from_dir(&db, &dir, true, &grant)
        .await
        .unwrap();

    for (label, qty) in [("apples", 2), ("pears", 3)] {
        sqlx::query(&format!(
            "INSERT INTO \"{id}_items\" (label, qty) VALUES (?1, ?2)"
        ))
        .bind(label)
        .bind(qty)
        .execute(&db)
        .await
        .unwrap();
    }
    db
}

/// A unique plugin id: lowercase letters and digits, starting with a letter.
fn fresh_id() -> String {
    format!("sm{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

fn viewer() -> AuthUser {
    AuthUser {
        id: 1,
        username: "ana".into(),
        role: Role::User,
    }
}

#[tokio::test]
async fn summary_page_shows_each_metric() {
    let _turn = turn().await;
    let id = fresh_id();
    let db = install(&id).await;

    let html = dispatch::page(
        State(db),
        Extension(viewer()),
        AxPath((id.clone(), "summary".to_string())),
        Query(HashMap::new()),
    )
    .await
    .unwrap()
    .0;

    // Each card is a label then a value.
    for (label, value) in [
        ("Items", "2"),
        ("Quantity", "5"),
        ("Average", "2.50"),
        ("Nothing", "\u{2014}"),
    ] {
        let card = format!(">{label}</div>");
        assert!(html.contains(&card), "missing label {label}");
        let shown = format!(">{value}</div>");
        assert!(
            html.contains(&shown),
            "missing value {value} for {label}: {html}"
        );
    }
}

#[test]
fn metrics_must_be_single_scoped_selects() {
    let page_with = |sql: &str| {
        format!(
            "[plugin]\nid = \"sum_bad\"\nname = \"x\"\nversion = \"0.1.0\"\nentry = \"main.js\"\nruntime = \"js\"\n\
             [[pages]]\npath = \"/s\"\ntitle = \"S\"\nkind = \"summary\"\n\
             metrics = [{{ label = \"m\", sql = \"{sql}\" }}]\n"
        )
    };
    assert!(manifest::parse(&page_with("SELECT COUNT(*) FROM sum_bad_items")).is_ok());

    for (label, sql) in [
        ("write", "DELETE FROM sum_bad_items"),
        ("other table", "SELECT COUNT(*) FROM someone_else"),
        ("two statements", "SELECT 1 FROM sum_bad_items; SELECT 2"),
    ] {
        assert!(
            manifest::parse(&page_with(sql)).is_err(),
            "{label} should be refused"
        );
    }

    let no_metrics = "[plugin]\nid = \"sum_bad\"\nname = \"x\"\nversion = \"0.1.0\"\nentry = \"main.js\"\nruntime = \"js\"\n\
        [[pages]]\npath = \"/s\"\ntitle = \"S\"\nkind = \"summary\"\n";
    assert!(
        manifest::parse(no_metrics).is_err(),
        "a summary page needs metrics"
    );
}
