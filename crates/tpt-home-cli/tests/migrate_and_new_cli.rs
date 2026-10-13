// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `migrate`, and the refusal paths of `new`, run as the real binary.
//! `new` writes into the repository, so only its failure modes are tested here.

use std::path::Path;
use std::process::{Command, Output};

const ADMIN_PASSWORD: &str = "cli-test-password-1";

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

#[test]
fn migrate_is_idempotent_after_init() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let init = tpt(&data_dir, &["init", "--admin-password", ADMIN_PASSWORD]);
    assert!(init.status.success(), "init failed: {}", stderr(&init));

    // Init already applied every migration, so a second run changes nothing.
    let first = tpt(&data_dir, &["migrate"]);
    assert!(first.status.success(), "migrate failed: {}", stderr(&first));
    assert!(
        stdout(&first).contains("database is up to date"),
        "unexpected output: {}",
        stdout(&first)
    );

    let second = tpt(&data_dir, &["migrate"]);
    assert!(
        second.status.success(),
        "migrate failed: {}",
        stderr(&second)
    );
    assert!(stdout(&second).contains("database is up to date"));
}

#[test]
fn new_rejects_an_invalid_blueprint_id() {
    let cwd = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(cwd.path())
        .args(["new", "Bad-Name"])
        .output()
        .expect("run tpt-home new");
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("invalid blueprint id"),
        "unexpected stderr: {}",
        stderr(&out)
    );
}

#[test]
fn new_refuses_to_run_outside_the_workspace() {
    let cwd = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(cwd.path())
        .args(["new", "valid_name"])
        .output()
        .expect("run tpt-home new");
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("not inside a tpt-homestack workspace"),
        "unexpected stderr: {}",
        stderr(&out)
    );
    assert!(
        !cwd.path().join("blueprints").exists(),
        "nothing should be scaffolded outside a workspace"
    );
}
