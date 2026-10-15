// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home mcp --token ...` as a spawned process fed a scripted stdin
//! session, the way an AI agent's host runs it. Checks the protocol lines on
//! stdout, that a bad line does not end the session, and that a bad token
//! refuses to start.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_cli(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run tpt-home")
}

/// A data directory with an admin and an API token for that admin.
fn setup() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let init = run_cli(
        &data_dir,
        &["init", "--admin-password", "cli-test-password-1"],
    );
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let made = run_cli(
        &data_dir,
        &[
            "token", "create", "admin", "--name", "agent", "--scope", "api",
        ],
    );
    assert!(
        made.status.success(),
        "token: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    let stdout = String::from_utf8_lossy(&made.stdout).into_owned();
    let secret = stdout
        .lines()
        .find_map(|l| l.strip_prefix("secret (shown once, store it now): "))
        .expect("secret line")
        .trim()
        .to_string();
    (root, data_dir, secret)
}

/// Run `mcp` with `session` written to its stdin, then close stdin.
fn mcp_session(data_dir: &Path, token: &str, session: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(["mcp", "--token", token])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tpt-home mcp");
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in session {
            writeln!(stdin, "{line}").unwrap();
        }
    }
    child.wait_with_output().expect("wait for mcp")
}

#[test]
fn scripted_session_gets_one_protocol_reply_per_request() {
    let (_root, data_dir, token) = setup();
    let out = mcp_session(
        &data_dir,
        &token,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "this is not json",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let replies: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("stdout line is not JSON: {l} ({e})"))
        })
        .collect();
    // The notification gets no reply; the bad line gets a parse error; the
    // session carries on to tools/list.
    assert_eq!(replies.len(), 3, "replies: {stdout}");
    assert_eq!(replies[0]["id"], 1);
    assert!(replies[0]["result"].is_object(), "initialize result");
    assert!(
        replies[1]["error"].is_object(),
        "parse error for the bad line"
    );
    assert_eq!(replies[2]["id"], 2);
    assert!(replies[2]["result"]["tools"].is_array(), "tools list");
}

#[test]
fn a_bad_token_refuses_to_start() {
    let (_root, data_dir, _token) = setup();
    let out = mcp_session(&data_dir, "not-a-real-token", &[]);
    assert!(
        !out.status.success(),
        "a bad token must not start a session"
    );
    assert!(
        out.stdout.is_empty(),
        "no protocol output without a session"
    );
}
