// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home plugin` subcommands, run as the real binary against a fresh data
//! directory. An external plugin is used because it needs no wasm build.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MANIFEST: &str = r#"
[plugin]
id = "ticker"
name = "Ticker"
version = "0.1.0"
runtime = "external"
entry = "http://127.0.0.1:8790"

[permissions]
db = true

[[tables]]
name = "items"
[[tables.columns]]
name = "label"
kind = "text"
"#;

fn tpt(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .output()
        .expect("run tpt-home")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A fresh data directory with the admin user created, plus a `.tptp` package
/// of the ticker plugin next to it.
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");

    let init = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(&data_dir)
        .args(["init", "--admin-password", "cli-test-password-1"])
        .output()
        .expect("run init");
    assert!(init.status.success(), "init failed: {}", stderr(&init));

    let src = root.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("plugin.toml"), MANIFEST).unwrap();
    let package = tpt_home_plugins::package::pack(&src).expect("pack ticker");
    let tptp = root.path().join("ticker-0.1.0.tptp");
    std::fs::write(&tptp, &package.bytes).unwrap();

    (root, data_dir, tptp)
}

#[test]
fn install_list_disable_enable_and_remove_round_trip() {
    let (_root, data_dir, tptp) = setup();
    let tptp = tptp.to_str().unwrap();

    // Install approves db and enables in one step.
    let out = tpt(
        &data_dir,
        &["plugin", "install", tptp, "--approve", "db", "--enable"],
    );
    assert!(out.status.success(), "install failed: {}", stderr(&out));
    assert!(stdout(&out).contains("installed ticker 0.1.0 (enabled: true"));

    let out = tpt(&data_dir, &["plugin", "list"]);
    assert!(
        stdout(&out).contains("ticker 0.1.0 (Ticker) enabled"),
        "{}",
        stdout(&out)
    );

    assert!(
        tpt(&data_dir, &["plugin", "disable", "ticker"])
            .status
            .success()
    );
    let out = tpt(&data_dir, &["plugin", "list"]);
    assert!(stdout(&out).contains("ticker 0.1.0 (Ticker) disabled"));

    assert!(
        tpt(&data_dir, &["plugin", "enable", "ticker"])
            .status
            .success()
    );

    // Removing without --yes or --keep-data is refused, and the plugin stays.
    let out = tpt(&data_dir, &["plugin", "remove", "ticker"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("drops its data"), "{}", stderr(&out));
    assert!(stdout(&tpt(&data_dir, &["plugin", "list"])).contains("ticker"));

    let out = tpt(&data_dir, &["plugin", "remove", "ticker", "--yes"]);
    assert!(out.status.success(), "remove failed: {}", stderr(&out));
    assert!(
        stdout(&out).contains("dropped 1 table(s)"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&tpt(&data_dir, &["plugin", "list"])).contains("no plugins installed"));
}

#[test]
fn install_without_approve_grants_nothing_and_stays_disabled() {
    let (_root, data_dir, tptp) = setup();
    let out = tpt(&data_dir, &["plugin", "install", tptp.to_str().unwrap()]);
    assert!(out.status.success(), "install failed: {}", stderr(&out));
    assert!(stdout(&out).contains("enabled: false"));
    assert!(stdout(&out).contains("approved: []"), "{}", stdout(&out));
}

#[test]
fn keep_data_removal_leaves_the_tables_in_place() {
    let (_root, data_dir, tptp) = setup();
    let tptp = tptp.to_str().unwrap();
    assert!(
        tpt(&data_dir, &["plugin", "install", tptp, "--approve", "db"])
            .status
            .success()
    );

    let out = tpt(&data_dir, &["plugin", "remove", "ticker", "--keep-data"]);
    assert!(out.status.success(), "remove failed: {}", stderr(&out));
    assert!(stdout(&out).contains("data kept"));
}

#[test]
fn enable_of_a_missing_plugin_fails() {
    let (_root, data_dir, _tptp) = setup();
    let out = tpt(&data_dir, &["plugin", "enable", "ghost"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not loaded"), "{}", stderr(&out));
}
