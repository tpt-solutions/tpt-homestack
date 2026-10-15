// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The plugin catalog end to end against a local HTTP server: a good install,
//! and the refusals for a checksum mismatch, a manifest id that differs from
//! the index, and a signature from a key the index does not name.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;
use sha2::{Digest, Sha256};

use tpt_home_core::config::Config;
use tpt_home_core::db::{DbPool, connect_in_memory};
use tpt_home_plugins::catalog::{CatalogInstall, install_from_catalog, list_catalog};
use tpt_home_plugins::signing;

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// Serve the files in `files` by path, at the start of the test's own port.
fn serve(files: Files) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            let body = files.lock().unwrap().get(&path).cloned();
            let response = match body {
                Some(body) => {
                    let mut out = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    out
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    format!("http://{addr}")
}

fn package(id: &str) -> Vec<u8> {
    package_at(id, "0.1.0")
}

fn package_at(id: &str, version: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("plugin.toml"),
        format!(
            "[plugin]\nid = \"{id}\"\nname = \"Entry\"\nversion = \"{version}\"\nruntime = \"external\"\nentry = \"http://127.0.0.1:9\"\n\n[permissions]\ndb = true\n\n[[tables]]\nname = \"items\"\n[[tables.columns]]\nname = \"label\"\nkind = \"text\"\n"
        ),
    )
    .unwrap();
    tpt_home_plugins::package::pack(&src).unwrap().bytes
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn index(
    base: &str,
    id: &str,
    sha256: &str,
    signature: Option<&str>,
    signed_by: Option<&str>,
) -> Vec<u8> {
    let mut entry = json!({
        "id": id,
        "name": "Entry",
        "version": "0.1.0",
        "description": "A test entry.",
        "package": format!("{base}/pkg.tptp"),
        "sha256": sha256,
        "permissions": ["db"],
    });
    if let Some(sig) = signature {
        entry["signature"] = json!(format!("{base}{sig}"));
    }
    if let Some(key) = signed_by {
        entry["signed_by"] = json!(key);
    }
    json!({"format": 1, "name": "Test catalog", "plugins": [entry]})
        .to_string()
        .into_bytes()
}

async fn db() -> DbPool {
    let db = connect_in_memory().await.unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();
    db
}

async fn install(db: &DbPool, id: &str) -> u16 {
    let result = install_from_catalog(
        State(db.clone()),
        Json(CatalogInstall {
            id: id.into(),
            approve: vec!["db".into()],
            enable: Some(true),
        }),
    )
    .await;
    match result {
        Ok(ok) => ok.into_response().status().as_u16(),
        Err(err) => err.into_response().status().as_u16(),
    }
}

#[tokio::test]
async fn the_catalog_installs_only_what_checks_out() {
    let files: Files = Arc::new(Mutex::new(HashMap::new()));
    let base = serve(files.clone());
    let (secret, public) = signing::generate_keypair();
    let (other_secret, other_public) = signing::generate_keypair();

    let pkg = package("logbook");
    let sig = signing::sign(&pkg, &secret).unwrap();
    {
        let mut f = files.lock().unwrap();
        f.insert("/pkg.tptp".into(), pkg.clone());
        f.insert("/pkg.sig".into(), sig.clone().into_bytes());
        f.insert(
            "/index.json".into(),
            index(&base, "logbook", &sha(&pkg), None, None),
        );
    }
    let cfg = Config {
        catalog_url: format!("{base}/index.json"),
        trusted_plugin_keys: vec![public.clone(), other_public],
        data_dir: tempfile::tempdir().unwrap().keep(),
        ..Config::default()
    };
    tpt_home_core::config::set_global_for_tests(cfg);

    let db = db().await;
    // The listing shows the entry as not installed yet.
    let Json(listed) = list_catalog().await.unwrap();
    assert_eq!(listed["configured"], true);
    assert_eq!(listed["plugins"][0]["status"], "not installed");
    let first_ability = listed["plugins"][0]["permissions_plain"][0]
        .as_str()
        .unwrap();
    assert!(first_ability.contains("records"), "{first_ability}");

    // A checksum that does not match what the index names is refused.
    files
        .lock()
        .unwrap()
        .insert("/pkg.tptp".into(), package_at("logbook", "0.1.9"));
    assert_eq!(install(&db, "logbook").await, 400);
    assert!(tpt_home_plugins::registry::get("logbook").is_none());

    // The right package from the index installs.
    files
        .lock()
        .unwrap()
        .insert("/pkg.tptp".into(), pkg.clone());
    assert_eq!(install(&db, "logbook").await, 201);
    assert!(tpt_home_plugins::registry::get("logbook").is_some());

    // An index that names an id the manifest does not carry is refused.
    let wrong_id = package("ticker");
    files
        .lock()
        .unwrap()
        .insert("/wrong.tptp".into(), wrong_id.clone());
    files.lock().unwrap().insert(
        "/index.json".into(),
        String::from_utf8(index(&base, "logbook", &sha(&wrong_id), None, None))
            .unwrap()
            .replace("/pkg.tptp", "/wrong.tptp")
            .into_bytes(),
    );
    assert_eq!(install(&db, "logbook").await, 400);

    // A package signed by a key the index does not name is refused.
    let other_sig = signing::sign(&pkg, &other_secret).unwrap();
    files
        .lock()
        .unwrap()
        .insert("/other.sig".into(), other_sig.into_bytes());
    files.lock().unwrap().insert(
        "/index.json".into(),
        index(
            &base,
            "logbook",
            &sha(&pkg),
            Some("/other.sig"),
            Some(&public),
        ),
    );
    assert_eq!(install(&db, "logbook").await, 400);
}
