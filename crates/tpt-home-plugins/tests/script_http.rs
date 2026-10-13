// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Outbound HTTP for the JS tier (`tpt.http`). A local TCP server stands in for
//! a device on the LAN: each test checks what the script got back and what the
//! server actually received. Also covers the manifest rules for hosts.

#![cfg(feature = "js")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use tpt_home_plugins::manifest::{self, Permissions, parse_target};
use tpt_home_plugins::runtime::PluginRequest;
use tpt_home_plugins::script::{self, ScriptSource};

/// Serve `count` requests on 127.0.0.1 and record each raw request. Returns the
/// port and the log.
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
            let body = "hello from the device";
            let reply = format!(
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (port, seen)
}

fn manifest(http: bool, hosts: &[&str]) -> manifest::PluginManifest {
    let hosts = hosts
        .iter()
        .map(|h| format!("\"{h}\""))
        .collect::<Vec<_>>()
        .join(", ");
    manifest::parse(&format!(
        r#"
[plugin]
id = "fetcher"
name = "Fetcher"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
http = {http}
http_hosts = [{hosts}]

[limits]
max_fuel = 50000000
max_memory_mb = 16
max_millis = 5000
"#
    ))
    .expect("manifest")
}

/// Run `source` as the runtime would: the manifest always requests HTTP to
/// `hosts`, and the effective grant is that request intersected with `approved`.
fn run(source: &str, approved: Permissions, hosts: &[&str]) -> String {
    let source = source.to_string();
    let hosts: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let script = ScriptSource::new(source).expect("script parses");
            let dir = tempfile::tempdir().unwrap();
            let refs: Vec<&str> = hosts.iter().map(String::as_str).collect();
            let manifest = manifest(true, &refs);
            let effective = manifest.permissions.intersect(&approved);
            let reply = script::call_script(
                &script,
                &manifest,
                &PluginRequest {
                    method: "GET".into(),
                    path: "/".into(),
                    query: String::new(),
                    body: None,
                    user: None,
                },
                "fetcher",
                &effective,
                &dir.path().join("none.db"),
                dir.path(),
            )
            .expect("script should succeed");
            reply.body
        })
        .unwrap()
        .join()
        .unwrap()
}

fn granted() -> Permissions {
    Permissions::parse_list("http").unwrap()
}

#[test]
fn get_reaches_a_declared_device_and_returns_status_and_body() {
    let (port, seen) = device(1);
    let host = format!("127.0.0.1:{port}");
    let body = run(
        &format!(
            r#"function handle() {{
                const r = tpt.http.get("http://{host}/status");
                return {{ status: 200, body: r.status + " " + r.body }};
            }}"#
        ),
        granted(),
        &[&host],
    );
    assert_eq!(body, "200 hello from the device");

    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /status HTTP/1.0\r\n"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains(&format!("Host: {host}\r\n")),
        "{}",
        requests[0]
    );
}

#[test]
fn post_sends_the_body_with_its_length() {
    let (port, seen) = device(1);
    let host = format!("127.0.0.1:{port}");
    let body = run(
        &format!(
            r#"function handle() {{
                const r = tpt.http.post("http://{host}/in", '{{"a":1}}');
                return {{ status: 200, body: r === null ? "null" : String(r.status) }};
            }}"#
        ),
        granted(),
        &[&host],
    );
    assert_eq!(body, "200");

    let requests = seen.lock().unwrap();
    assert!(
        requests[0].starts_with("POST /in HTTP/1.0\r\n"),
        "{}",
        requests[0]
    );
    assert!(
        requests[0].contains("Content-Length: 7\r\n"),
        "{}",
        requests[0]
    );
    assert!(requests[0].ends_with("{\"a\":1}"), "{}", requests[0]);
}

#[test]
fn undeclared_hosts_are_refused_without_a_connection() {
    let (port, seen) = device(1);
    let declared = "127.0.0.1:1";
    let body = run(
        &format!(
            r#"function handle() {{
                return {{ status: 200, body: String(tpt.http.get("http://127.0.0.1:{port}/")) }};
            }}"#
        ),
        granted(),
        &[declared],
    );
    assert_eq!(body, "null");
    assert!(
        seen.lock().unwrap().is_empty(),
        "the device was never contacted"
    );
}

#[test]
fn the_capability_must_be_approved() {
    let (port, seen) = device(1);
    let host = format!("127.0.0.1:{port}");
    let body = run(
        &format!(
            r#"function handle() {{
                return {{ status: 200, body: String(tpt.http.get("http://{host}/")) }};
            }}"#
        ),
        Permissions::default(),
        &[&host],
    );
    assert_eq!(body, "null", "an unapproved plugin gets nothing");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn control_characters_in_urls_are_refused() {
    let (port, seen) = device(2);
    let host = format!("127.0.0.1:{port}");
    let body = run(
        &format!(
            r#"function handle() {{
                const b = tpt.http.get("http://{host}/x\r\nhack");
                const c = tpt.http.get("http://{host}/a b");
                return {{ status: 200, body: [b, c].map(String).join(",") }};
            }}"#
        ),
        granted(),
        &[&host],
    );
    assert_eq!(body, "null,null");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn declared_targets_parse_and_reject_bad_names() {
    assert_eq!(
        parse_target("Sensor.LAN"),
        Some(("sensor.lan".into(), None))
    );
    assert_eq!(
        parse_target("10.0.0.5:8080"),
        Some(("10.0.0.5".into(), Some(8080)))
    );
    assert_eq!(parse_target("bad_host"), None, "underscore is not allowed");
    assert_eq!(parse_target("http://x"), None, "no scheme in a target");
    assert_eq!(parse_target(":80"), None, "a host is required");
    assert_eq!(parse_target("x:0"), None, "port 0 is not a port");
}

#[test]
fn hosts_need_the_capability_and_valid_names() {
    let with = |http: &str, hosts: &str| {
        format!(
            "[plugin]\nid = \"fetcher\"\nname = \"x\"\nversion = \"0.1.0\"\nentry = \"main.js\"\nruntime = \"js\"\n\
             [permissions]\n{http}\nhttp_hosts = [{hosts}]\n"
        )
    };
    assert!(manifest::parse(&with("http = true", "\"sensor.lan:8080\"")).is_ok());
    assert!(
        manifest::parse(&with("http = false", "\"sensor.lan\"")).is_err(),
        "hosts without the capability are refused"
    );
    assert!(manifest::parse(&with("http = true", "\"http://sensor.lan\"")).is_err());
}

#[test]
fn a_bare_declared_host_admits_only_the_default_port() {
    let (port, seen) = device(1);
    let body = run(
        &format!(
            r#"function handle() {{
                return {{ status: 200, body: String(tpt.http.get("http://127.0.0.1:{port}/")) }};
            }}"#
        ),
        granted(),
        &["127.0.0.1"],
    );
    assert_eq!(body, "null", "port {port} is not port 80");
    assert!(seen.lock().unwrap().is_empty());
}

#[cfg(feature = "https")]
#[test]
fn https_to_a_plaintext_device_fails_cleanly() {
    let (port, seen) = device(1);
    let host = format!("127.0.0.1:{port}");
    let body = run(
        &format!(
            r#"function handle() {{
                return {{ status: 200, body: String(tpt.http.get("https://{host}/")) }};
            }}"#
        ),
        granted(),
        &[&host],
    );
    assert_eq!(
        body, "null",
        "the TLS handshake fails against a plain server"
    );
    let _ = seen;
}
