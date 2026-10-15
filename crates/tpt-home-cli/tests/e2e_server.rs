// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! End to end: the real `tpt-home` binary serves HTTP on a loopback port and
//! the test talks to it over a plain TCP socket. Covers login and sessions,
//! blueprint use (notes, track), a plugin install through the multipart
//! endpoint, and refusals for anonymous callers.
//!
//! Each test starts its own server with a fresh data directory.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const ADMIN_PASSWORD: &str = "e2e-test-password-1";

/// A running server; the process is killed when this is dropped.
struct Server {
    child: Child,
    port: u16,
    root: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn log(&self) -> String {
        std::fs::read_to_string(self.root.path().join("server.log")).unwrap_or_default()
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start() -> Server {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let init = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(&data_dir)
        .args(["init", "--admin-password", ADMIN_PASSWORD])
        .output()
        .expect("run init");
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let port = free_port();
    let log = std::fs::File::create(root.path().join("server.log")).unwrap();
    let log_err = log.try_clone().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(&data_dir)
        .args(["run", "--host", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .expect("start server");
    let mut server = Server { child, port, root };
    wait_until_ready(&mut server);
    server
}

fn wait_until_ready(server: &mut Server) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            panic!("server exited early ({status}):\n{}", server.log());
        }
        if let Ok(res) = try_send(server.port, "GET", "/healthz", &[], &[])
            && res.status == 200
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "server did not become ready:\n{}",
            server.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// One HTTP/1.1 request over a fresh connection (`Connection: close`).
fn try_send(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> std::io::Result<Response> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    Ok(parse_response(&raw))
}

fn send(
    server: &Server,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> Response {
    try_send(server.port, method, path, headers, body)
        .unwrap_or_else(|e| panic!("{method} {path}: {e}"))
}

fn parse_response(raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response has a header block");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("status line");
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    assert!(
        !chunked,
        "chunked responses are not expected from these routes"
    );
    Response {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

/// `tpt_session=...` from a login response, ready for a Cookie header.
fn login(server: &Server) -> String {
    let body = json!({"username": "admin", "password": ADMIN_PASSWORD}).to_string();
    let res = send(
        server,
        "POST",
        "/api/auth/login",
        &[("Content-Type", "application/json".into())],
        body.as_bytes(),
    );
    assert_eq!(res.status, 200, "login: {}", res.text());
    res.header("set-cookie")
        .and_then(|c| c.split(';').next())
        .expect("session cookie")
        .to_string()
}

fn json_call(server: &Server, method: &str, path: &str, cookie: &str, body: &Value) -> Response {
    send(
        server,
        method,
        path,
        &[
            ("Content-Type", "application/json".into()),
            ("Cookie", cookie.into()),
        ],
        body.to_string().as_bytes(),
    )
}

fn get(server: &Server, path: &str, cookie: &str) -> Response {
    send(server, "GET", path, &[("Cookie", cookie.into())], &[])
}

/// A `multipart/form-data` body: plain fields and one file part.
fn multipart(boundary: &str, fields: &[(&str, &str)], file: (&str, &[u8])) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
            file.0
        )
        .as_bytes(),
    );
    body.extend_from_slice(file.1);
    body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn records_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/records")
}

#[test]
fn login_notes_and_tracks_over_http() {
    let server = start();

    // Health is public; the API is not.
    assert_eq!(send(&server, "GET", "/healthz", &[], &[]).status, 200);
    assert_eq!(send(&server, "GET", "/api/notes", &[], &[]).status, 401);

    let cookie = login(&server);
    let me = get(&server, "/api/auth/me", &cookie);
    assert_eq!(me.status, 200, "{}", me.text());
    assert_eq!(me.json()["username"], "admin");

    // A bad password is refused and does not create a session.
    let bad = json_call(
        &server,
        "POST",
        "/api/auth/login",
        "",
        &json!({"username": "admin", "password": "wrong-password"}),
    );
    assert_eq!(bad.status, 401);

    // Notes: create, then list and read back.
    let created = json_call(
        &server,
        "POST",
        "/api/notes",
        &cookie,
        &json!({"title": "Shopping", "body": "milk and **bread**", "tags": ["home"]}),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let id = created.json()["id"].as_i64().expect("note id");
    let list = get(&server, "/api/notes", &cookie);
    assert!(list.text().contains("Shopping"), "{}", list.text());
    let rendered = get(&server, &format!("/api/notes/{id}/render"), &cookie);
    assert_eq!(rendered.status, 200);
    assert!(
        rendered.text().contains("<strong>bread</strong>"),
        "{}",
        rendered.text()
    );

    // Track: a table, a row, and the CSV export that includes the row.
    let table = json_call(
        &server,
        "POST",
        "/api/track/tables",
        &cookie,
        &json!({"name": "Expenses", "fields": [{"name": "label", "kind": "text"}]}),
    );
    assert_eq!(table.status, 201, "{}", table.text());
    let table_id = table.json()["id"].as_i64().expect("table id");
    let row = json_call(
        &server,
        "POST",
        &format!("/api/track/tables/{table_id}/rows"),
        &cookie,
        &json!({"label": "coffee"}),
    );
    assert_eq!(row.status, 201, "{}", row.text());
    let csv = get(
        &server,
        &format!("/api/track/tables/{table_id}/export.csv"),
        &cookie,
    );
    assert!(csv.text().contains("coffee"), "{}", csv.text());
}

#[test]
fn plugin_install_and_use_over_http() {
    let server = start();
    let cookie = login(&server);

    // Build the shipped records plugin with the CLI, as a user would.
    let scratch = tempfile::tempdir().unwrap();
    let package = scratch.path().join("records.tptp");
    let built = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("plugin")
        .arg("build")
        .arg(records_dir())
        .arg("--out")
        .arg(&package)
        .output()
        .expect("run plugin build");
    assert!(
        built.status.success(),
        "plugin build: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let bytes = std::fs::read(&package).unwrap();

    // Install through the multipart endpoint: approve db, enable at once.
    let boundary = "e2eBoundary7d1f";
    let body = multipart(
        boundary,
        &[("approve", "db"), ("enable", "true")],
        ("records.tptp", &bytes),
    );
    let installed = try_send(
        server.port,
        "POST",
        "/api/plugins/install",
        &[
            ("Cookie", cookie.clone()),
            ("X-TPT-Request", "1".into()),
            (
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ),
        ],
        &body,
    )
    .unwrap_or_else(|e| {
        panic!(
            "install connection failed: {e}\nserver log:\n{}",
            server.log()
        )
    });
    assert!(
        (200..300).contains(&installed.status),
        "install: {}",
        installed.text()
    );

    // The plugin's routes now answer, for signed-in users only.
    let created = json_call(
        &server,
        "POST",
        "/api/records/records",
        &cookie,
        &json!({"title": "Torch", "notes": "in the hall"}),
    );
    assert_eq!(created.status, 201, "{}", created.text());
    let list = get(&server, "/api/records/records", &cookie);
    assert_eq!(list.status, 200);
    assert!(list.text().contains("Torch"), "{}", list.text());
    assert_eq!(
        send(&server, "GET", "/api/records/records", &[], &[]).status,
        401
    );

    // A request without the multipart header is refused before the install
    // handler runs.
    // The server answers 415 without reading the body, then closes. A client
    // still sending a large body can see the close as a reset instead of the
    // answer. Either way the install did not run.
    let refused = try_send(
        server.port,
        "POST",
        "/api/plugins/install",
        &[
            ("Cookie", cookie),
            (
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ),
        ],
        &body,
    );
    match refused {
        Ok(response) => assert_eq!(response.status, 415),
        Err(e) => assert!(
            matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            ),
            "unexpected error: {e}\nserver log:\n{}",
            server.log()
        ),
    }
}

/// Build a plugin from `dir` with the CLI and install it with no permissions,
/// enabled at once. Returns the install response.
fn build_and_install(server: &Server, cookie: &str, dir: &Path) -> Response {
    let scratch = tempfile::tempdir().unwrap();
    let package = scratch.path().join("plugin.tptp");
    let built = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("plugin")
        .arg("build")
        .arg(dir)
        .arg("--out")
        .arg(&package)
        .output()
        .expect("run plugin build");
    assert!(
        built.status.success(),
        "plugin build: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let bytes = std::fs::read(&package).unwrap();
    let boundary = "e2eBoundaryBomb9";
    let body = multipart(
        boundary,
        &[("approve", ""), ("enable", "true")],
        ("plugin.tptp", &bytes),
    );
    send(
        server,
        "POST",
        "/api/plugins/install",
        &[
            ("Cookie", cookie.into()),
            ("X-TPT-Request", "1".into()),
            (
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            ),
        ],
        &body,
    )
}

#[test]
fn a_script_that_exhausts_memory_fails_alone() {
    let server = start();
    let cookie = login(&server);

    // Doubling a string 40 times asks for terabytes. Without a memory limit on
    // the script worker this would take the server down.
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path().join("bomb");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        r#"[plugin]
id = "bomb"
name = "Bomb"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[limits]
max_fuel = 50000000
max_memory_mb = 16
max_millis = 5000
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("main.js"),
        "function handle(req) { const steps = req.query.indexOf('big') >= 0 ? 40 : 10; let s = 'x'; for (let i = 0; i < steps; i++) { s = s + s; } return { status: 200, body: 'done' }; }",
    )
    .unwrap();

    let installed = build_and_install(&server, &cookie, &dir);
    assert!(
        (200..300).contains(&installed.status),
        "install: {}",
        installed.text()
    );

    // A small allocation (1 KB) is far under the cap and must succeed, so the
    // limit is not simply refusing every script.
    let small = get(&server, "/api/bomb/go", &cookie);
    assert_eq!(
        small.status,
        200,
        "a small script must run: {}",
        small.text()
    );
    assert_eq!(small.text(), "done");

    let blown = get(&server, "/api/bomb/go?big", &cookie);
    assert_eq!(
        blown.status,
        500,
        "the bomb call must fail: {}",
        blown.text()
    );

    // Every call fails the same way, and the rest of the server is unaffected.
    let again = get(&server, "/api/bomb/go?big", &cookie);
    assert_eq!(again.status, 500, "{}", again.text());
    assert_eq!(send(&server, "GET", "/healthz", &[], &[]).status, 200);
    assert_eq!(get(&server, "/api/notes", &cookie).status, 200);

    // The cause is the memory cap, not some other crash.
    assert!(
        server.log().contains("exceeded its memory limit"),
        "the failure should name the memory limit:\n{}",
        server.log()
    );
}
