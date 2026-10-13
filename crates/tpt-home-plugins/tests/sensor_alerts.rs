// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The shipped `plugins/sensor-alerts` plugin, installed for real and driven
//! through the same dispatch the hook and the job use. A local device stands in
//! for the admin's "alerts" channel and records what it receives.
//!
//! Each test installs its own copy under a unique id and the tests take turns,
//! as in the other plugin tests.

#![cfg(feature = "js")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::to_bytes;
use chrono::{Local, Timelike};
use tpt_home_core::auth::user::{AuthUser, Role};
use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::{DbPool, connect};
use tpt_home_plugins::dispatch::invoke_plugin;
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::Permissions;

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

/// Serve `count` requests with 200, recording each raw request.
fn device(count: usize) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).into_owned());
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    (port, seen)
}

/// Install the shipped plugin under a unique id, with the `alerts` channel
/// pointing at `channel_url`. The plugin id is rewritten in the manifest and in
/// the script's table names.
async fn install(channel_url: &str) -> (String, DbPool) {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect(&config().db_path()).await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    let id = format!("sa{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/sensor-alerts");
    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["plugin.toml", "main.js"] {
        let text = std::fs::read_to_string(source.join(file))
            .unwrap()
            .replace("sensor_alerts", &id);
        std::fs::write(dir.join(file), text).unwrap();
    }
    let grant = Permissions::parse_list("db,kv,notify").unwrap();
    lifecycle::install_from_dir(&db, &dir, true, &grant)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO notification_channels (name, url, created_at) VALUES ('alerts', ?1, 0)
         ON CONFLICT (name) DO UPDATE SET url = excluded.url, last_error = NULL, last_sent_at = NULL",
    )
    .bind(channel_url)
    .execute(&db)
    .await
    .unwrap();
    (id, db)
}

fn admin() -> AuthUser {
    AuthUser {
        id: 1,
        username: "admin".into(),
        role: Role::Admin,
    }
}

fn member() -> AuthUser {
    AuthUser {
        id: 2,
        username: "ana".into(),
        role: Role::User,
    }
}

/// One call to the plugin. Returns the status and the body text.
async fn call(
    user: Option<&AuthUser>,
    id: &str,
    method: &str,
    rest: &str,
    body: Option<&str>,
) -> (u16, String) {
    let response = invoke_plugin(user, id, method, rest, "", body.map(String::from))
        .await
        .unwrap_or_else(|e| panic!("invoke {method} {rest}: {e:?}"));
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Quiet hours around the current local hour: a two-hour window that covers
/// it, so the test does not depend on the exact minute.
fn quiet_window_now() -> (i64, i64) {
    let hour = Local::now().hour() as i64;
    ((hour + 23) % 24, (hour + 1) % 24)
}

async fn set_settings(id: &str, from: i64, to: i64, hours: i64) {
    let body =
        format!(r#"{{"quiet_from":"{from}","quiet_to":"{to}","silent_after_hours":"{hours}"}}"#);
    let (status, text) = call(Some(&admin()), id, "POST", "settings", Some(&body)).await;
    assert_eq!(status, 200, "{text}");
}

async fn count(db: &DbPool, table: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM \"{table}\""))
        .fetch_one(db)
        .await
        .unwrap();
    n
}

#[tokio::test]
async fn quiet_openings_and_silent_sensors_are_sent_once_each() {
    let _turn = turn().await;
    let (port, requests) = device(2);
    let (id, db) = install(&format!("http://127.0.0.1:{port}/hook")).await;
    let (from, to) = quiet_window_now();
    set_settings(&id, from, to, 1).await;

    // A door opens in quiet hours: recorded, not yet sent.
    let (status, text) = call(
        None,
        &id,
        "POST",
        "hooks/feed",
        Some(r#"{"sensor":"front-door","state":"open"}"#),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    // A close is recorded but raises nothing.
    call(
        None,
        &id,
        "POST",
        "hooks/feed",
        Some(r#"{"sensor":"front-door","state":"closed"}"#),
    )
    .await;
    assert_eq!(count(&db, &format!("{id}_alerts")).await, 1);

    // The job sends the quiet alert.
    let (_, text) = call(None, &id, "POST", "check", None).await;
    assert!(text.contains("\"sent\":1"), "{text}");

    // A garage sensor that last reported two hours ago has gone silent.
    let two_hours_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 7_200;
    sqlx::query(&format!(
        "INSERT INTO \"{id}_events\" (sensor, state, seen_at) VALUES ('garage', 'closed', ?1)"
    ))
    .bind(two_hours_ago)
    .execute(&db)
    .await
    .unwrap();
    let (_, text) = call(None, &id, "POST", "check", None).await;
    assert!(text.contains("\"sent\":1"), "{text}");

    // Nothing new: no repeat alert and no repeat send.
    let (_, text) = call(None, &id, "POST", "check", None).await;
    assert!(text.contains("\"sent\":0"), "{text}");

    let kinds: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT kind, notified FROM \"{id}_alerts\" ORDER BY id"
    ))
    .fetch_all(&db)
    .await
    .unwrap();
    assert_eq!(
        kinds,
        vec![("quiet-open".to_string(), 1), ("silent".to_string(), 1)]
    );

    let sent = requests.lock().unwrap();
    assert_eq!(sent.len(), 2, "exactly one send per alert");
    assert!(
        sent[0].contains("front-door opened during quiet hours"),
        "{}",
        sent[0]
    );
    assert!(
        sent[1].contains("garage has not reported for 2 hours"),
        "{}",
        sent[1]
    );
}

#[tokio::test]
async fn only_admins_change_settings_and_the_values_are_checked() {
    let _turn = turn().await;
    let (id, _db) = install("http://127.0.0.1:9/unused").await;

    let good = r#"{"quiet_from":"22","quiet_to":"7","silent_after_hours":"12"}"#;
    assert_eq!(
        call(Some(&member()), &id, "POST", "settings", Some(good))
            .await
            .0,
        403
    );
    assert_eq!(call(None, &id, "POST", "settings", Some(good)).await.0, 403);

    for bad in [
        r#"{"quiet_from":"25","quiet_to":"7","silent_after_hours":"12"}"#,
        r#"{"quiet_from":"","quiet_to":"7","silent_after_hours":"12"}"#,
        r#"{"quiet_from":"22","quiet_to":"7","silent_after_hours":"0"}"#,
        r#"{"quiet_from":"22","quiet_to":"7","silent_after_hours":"721"}"#,
    ] {
        assert_eq!(
            call(Some(&admin()), &id, "POST", "settings", Some(bad))
                .await
                .0,
            400,
            "{bad}"
        );
    }
    assert_eq!(
        call(Some(&admin()), &id, "POST", "settings", Some(good))
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn malformed_events_are_accepted_but_not_recorded() {
    let _turn = turn().await;
    let (id, db) = install("http://127.0.0.1:9/unused").await;

    for body in [
        "not json",
        r#"{"sensor":"","state":"open"}"#,
        r#"{"state":"open"}"#,
    ] {
        let (status, text) = call(None, &id, "POST", "hooks/feed", Some(body)).await;
        assert_eq!(status, 200, "{body}");
        assert!(text.contains("\"recorded\":false"), "{text}");
    }
    assert_eq!(count(&db, &format!("{id}_events")).await, 0);
}
