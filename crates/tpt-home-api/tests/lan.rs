// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The LAN name: the settings page shows the address and a QR code when the
//! server listens beyond loopback, and the mDNS announcement can be browsed
//! back on this machine.

use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use tower::ServiceExt;

use tpt_home_api::{build_router, lan};
use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};

/// A lan name unlikely to clash with a real device on the network.
fn unique_name() -> String {
    format!("tpt-test-{}", std::process::id())
}

#[test]
fn announcement_is_browsable_on_this_machine() {
    let cfg = Config {
        host: "0.0.0.0".into(),
        port: 8765,
        lan_name: unique_name(),
        ..Config::default()
    };
    let announced = lan::announce(&cfg)
        .expect("announce")
        .expect("announcement");

    let browser = ServiceDaemon::new().expect("browser daemon");
    let receiver = browser.browse(lan::SERVICE_TYPE).expect("browse");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut found = false;
    while Instant::now() < deadline && !found {
        if let Ok(ServiceEvent::ServiceResolved(info)) =
            receiver.recv_timeout(Duration::from_millis(500))
        {
            found = info.get_fullname().contains(&cfg.lan_name) && info.get_port() == 8765;
        }
    }
    let _ = browser.shutdown();
    drop(announced);
    assert!(
        found,
        "the server's announcement was not seen on the local network"
    );
}

#[tokio::test]
async fn settings_page_shows_the_lan_address_and_a_code() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        host: "0.0.0.0".into(),
        port: 8765,
        lan_name: "family".into(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg.clone());
    let db: DbPool = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    let app = build_router(db, &cfg);

    let setup = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"username":"admin","password":"hunter2hunter2"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = setup
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html =
        String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).into_owned();
    assert!(html.contains("http://family.local:8765"), "address shown");
    assert!(html.contains("<svg"), "QR code inline");
}

#[test]
fn loopback_settings_explain_how_to_open_the_server() {
    let card = tpt_home_ui::phone_link_unavailable_card();
    assert!(card.contains("0.0.0.0"));
    assert!(
        tpt_home_ui::qr_svg("http://homestack.local")
            .unwrap()
            .contains("<svg")
    );
}
