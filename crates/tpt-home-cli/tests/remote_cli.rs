// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home remote status|setup` against fake `tailscale` and `cloudflared`
//! programs. The fakes print canned output and record what they were asked to
//! run, so the tests check the commands and the config changes without the
//! real clients or a network.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const RUNNING: &str = r#"{"BackendState":"Running","Self":{"DNSName":"laptop.tail1234.ts.net.","TailscaleIPs":["100.64.0.5"]}}"#;
const NEEDS_LOGIN: &str = r#"{"BackendState":"NeedsLogin","Self":{}}"#;

/// Write a fake client named `name` into `dir`. `body` is batch (Windows) or
/// shell (Unix) code run with the client's arguments; the fake's own folder is
/// `dir`, so it can read and write files next to itself.
#[cfg(windows)]
fn fake_client(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(format!("{name}.cmd"));
    std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
    path
}

#[cfg(unix)]
fn fake_client(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A fake `tailscale`. `status` prints `status.json` from its folder; `serve`
/// appends its arguments to `calls.log` in that folder and succeeds.
fn fake_tailscale(dir: &Path, status: &str) -> PathBuf {
    std::fs::write(dir.join("status.json"), status).unwrap();
    fake_client(dir, "tailscale", &tailscale_body())
}

#[cfg(windows)]
fn tailscale_body() -> String {
    "if \"%1\"==\"status\" (\r\n  type \"%~dp0status.json\"\r\n  exit /b 0\r\n)\r\n\
     if \"%1\"==\"serve\" (\r\n  echo %* >> \"%~dp0calls.log\"\r\n  exit /b 0\r\n)\r\nexit /b 1"
        .to_string()
}

#[cfg(unix)]
fn tailscale_body() -> String {
    "d=$(dirname \"$0\")\n\
     if [ \"$1\" = status ]; then cat \"$d/status.json\"; exit 0; fi\n\
     if [ \"$1\" = serve ]; then echo \"$@\" >> \"$d/calls.log\"; exit 0; fi\n\
     exit 1"
        .to_string()
}

fn fake_cloudflared(dir: &Path) -> PathBuf {
    fake_client(dir, "cloudflared", "echo cloudflared version 2026.1.0")
}

struct Env {
    root: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            "host = \"127.0.0.1\"\nport = 9123\n",
        )
        .unwrap();
        Env { root }
    }

    fn data(&self) -> &Path {
        self.root.path()
    }

    fn config(&self) -> toml::Table {
        toml::from_str(&std::fs::read_to_string(self.data().join("config.toml")).unwrap()).unwrap()
    }

    fn run(&self, args: &[&str], envs: &[(&str, &Path)], stdin: &str) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tpt-home"));
        cmd.arg("--data-dir")
            .arg(self.data())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, path) in envs {
            cmd.env(key, path);
        }
        let mut child = cmd.spawn().expect("run tpt-home");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn status_reports_a_signed_in_tailscale_and_missing_clients() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let ts = fake_tailscale(tools.path(), RUNNING);
    let out = env.run(
        &["remote", "status"],
        &[
            ("TPT_HOME_TAILSCALE_BIN", &ts),
            (
                "TPT_HOME_CLOUDFLARED_BIN",
                &tools.path().join("no-such-cloudflared"),
            ),
            (
                "TPT_HOME_NETBIRD_BIN",
                &tools.path().join("no-such-netbird"),
            ),
        ],
        "",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("signed in as https://laptop.tail1234.ts.net"),
        "{text}"
    );
    assert!(text.contains("cloudflared: not found"), "{text}");
    assert!(text.contains("netbird: not found"), "{text}");
    assert!(text.contains("host 127.0.0.1, port 9123"), "{text}");
}

#[test]
fn tailscale_setup_serves_and_turns_on_secure_cookies() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let ts = fake_tailscale(tools.path(), RUNNING);
    std::fs::write(tools.path().join("calls.log"), "").unwrap();

    let out = env.run(
        &["remote", "setup", "--provider", "tailscale", "--yes"],
        &[("TPT_HOME_TAILSCALE_BIN", &ts)],
        "",
    );
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    let calls = std::fs::read_to_string(tools.path().join("calls.log")).unwrap();
    assert!(calls.contains("serve --bg 9123"), "calls: {calls:?}");

    let cfg = env.config();
    assert_eq!(cfg["secure_cookies"].as_bool(), Some(true));
    // Other keys are kept and the server still listens on loopback only.
    assert_eq!(cfg["host"].as_str(), Some("127.0.0.1"));
    assert_eq!(cfg["port"].as_integer(), Some(9123));
    assert!(stdout(&out).contains("https://laptop.tail1234.ts.net"));
}

#[test]
fn declining_the_prompt_changes_nothing() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let ts = fake_tailscale(tools.path(), RUNNING);
    std::fs::write(tools.path().join("calls.log"), "").unwrap();
    let before = std::fs::read_to_string(env.data().join("config.toml")).unwrap();

    let out = env.run(
        &["remote", "setup", "--provider", "tailscale"],
        &[("TPT_HOME_TAILSCALE_BIN", &ts)],
        "n\n",
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("No changes made."));
    assert_eq!(
        std::fs::read_to_string(env.data().join("config.toml")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read_to_string(tools.path().join("calls.log")).unwrap(),
        ""
    );
}

#[test]
fn signed_out_tailscale_stops_with_a_clear_message() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let ts = fake_tailscale(tools.path(), NEEDS_LOGIN);
    let out = env.run(
        &["remote", "setup", "--provider", "tailscale", "--yes"],
        &[("TPT_HOME_TAILSCALE_BIN", &ts)],
        "",
    );
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not signed in"), "{}", stderr(&out));
    assert!(
        env.config().get("secure_cookies").is_none(),
        "config must not change"
    );
}

#[test]
fn missing_tailscale_says_how_to_install_it() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let out = env.run(
        &["remote", "setup", "--provider", "tailscale", "--yes"],
        &[(
            "TPT_HOME_TAILSCALE_BIN",
            &tools.path().join("no-such-tailscale"),
        )],
        "",
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("tailscale.com/download"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn cloudflare_setup_sets_trust_proxy_and_secure_cookies_after_confirmation() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let cf = fake_cloudflared(tools.path());

    let out = env.run(
        &["remote", "setup", "--provider", "cloudflare"],
        &[("TPT_HOME_CLOUDFLARED_BIN", &cf)],
        "y\n",
    );
    assert!(out.status.success(), "{}\n{}", stdout(&out), stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("Access"), "login warning missing: {text}");
    let cfg = env.config();
    assert_eq!(cfg["trust_proxy"].as_bool(), Some(true));
    assert_eq!(cfg["secure_cookies"].as_bool(), Some(true));
    assert_eq!(cfg["host"].as_str(), Some("127.0.0.1"));
}

#[test]
fn cloudflare_without_the_client_changes_nothing() {
    let env = Env::new();
    let tools = tempfile::tempdir().unwrap();
    let out = env.run(
        &["remote", "setup", "--provider", "cloudflare", "--yes"],
        &[(
            "TPT_HOME_CLOUDFLARED_BIN",
            &tools.path().join("no-such-cloudflared"),
        )],
        "",
    );
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("cloudflared is not installed"),
        "{}",
        stderr(&out)
    );
    assert!(env.config().get("trust_proxy").is_none());
}
