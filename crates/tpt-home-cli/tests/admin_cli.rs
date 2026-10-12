// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `user`, `token`, `backup` and `restore`, run as the real binary against a
//! fresh data directory.

use std::path::{Path, PathBuf};
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

/// A fresh data directory with the `admin` user created.
fn setup() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let init = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .arg("--data-dir")
        .arg(&data_dir)
        .args(["init", "--admin-password", ADMIN_PASSWORD])
        .output()
        .expect("run init");
    assert!(init.status.success(), "init failed: {}", stderr(&init));
    (root, data_dir)
}

/// The only `tpt-*.db` file in a backup directory.
fn backup_file(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("tpt-") && n.ends_with(".db"))
        })
        .collect();
    assert_eq!(found.len(), 1, "expected one backup in {}", dir.display());
    found.remove(0)
}

#[test]
fn user_add_list_reset_and_remove() {
    let (_root, data_dir) = setup();

    let out = tpt(
        &data_dir,
        &["user", "add", "bob", "--password", "bob-password-1"],
    );
    assert!(out.status.success(), "add failed: {}", stderr(&out));
    assert!(
        stdout(&out).contains("created user 'bob'"),
        "{}",
        stdout(&out)
    );

    let list = stdout(&tpt(&data_dir, &["user", "list"]));
    assert!(list.contains("admin") && list.contains("bob"), "{list}");

    let out = tpt(
        &data_dir,
        &[
            "user",
            "reset-password",
            "bob",
            "--password",
            "bob-password-2",
        ],
    );
    assert!(out.status.success(), "reset failed: {}", stderr(&out));

    let out = tpt(&data_dir, &["user", "remove", "bob", "--yes"]);
    assert!(out.status.success(), "remove failed: {}", stderr(&out));
    assert!(!stdout(&tpt(&data_dir, &["user", "list"])).contains("bob"));
}

#[test]
fn user_add_as_admin_is_listed_as_admin() {
    let (_root, data_dir) = setup();
    let out = tpt(
        &data_dir,
        &[
            "user",
            "add",
            "carol",
            "--password",
            "carol-password-1",
            "--admin",
        ],
    );
    assert!(out.status.success(), "add failed: {}", stderr(&out));
    let list = stdout(&tpt(&data_dir, &["user", "list"]));
    let carol = list
        .lines()
        .find(|l| l.contains("carol"))
        .expect("carol listed");
    assert!(carol.contains("admin"), "{carol}");
}

#[test]
fn token_create_list_and_revoke() {
    let (_root, data_dir) = setup();

    let out = tpt(
        &data_dir,
        &[
            "token", "create", "admin", "--name", "agent", "--scope", "api", "--days", "30",
        ],
    );
    assert!(out.status.success(), "create failed: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("created token 1 'agent' for admin (scopes: api)"),
        "{text}"
    );
    assert!(text.contains("secret (shown once"), "{text}");

    let list = stdout(&tpt(&data_dir, &["token", "list", "--username", "admin"]));
    assert!(list.contains("agent"), "{list}");

    let out = tpt(&data_dir, &["token", "revoke", "1"]);
    assert!(out.status.success(), "revoke failed: {}", stderr(&out));
    assert!(stdout(&out).contains("revoked token 1"));

    let out = tpt(&data_dir, &["token", "revoke", "999"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("no token with id 999"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn backup_then_restore_brings_back_the_saved_users() {
    let (root, data_dir) = setup();
    let backups = root.path().join("backups");

    let out = tpt(
        &data_dir,
        &["backup", "--out-dir", backups.to_str().unwrap()],
    );
    assert!(out.status.success(), "backup failed: {}", stderr(&out));
    let saved = backup_file(&backups);

    // Change the live data after the backup, then restore.
    assert!(
        tpt(
            &data_dir,
            &["user", "add", "dave", "--password", "dave-password-1"]
        )
        .status
        .success()
    );
    assert!(stdout(&tpt(&data_dir, &["user", "list"])).contains("dave"));

    let out = tpt(&data_dir, &["restore", saved.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "restore failed: {}", stderr(&out));
    assert!(stdout(&out).contains("restored from"));
    assert!(!stdout(&tpt(&data_dir, &["user", "list"])).contains("dave"));
}

#[test]
fn restore_refuses_a_file_that_is_not_a_database_and_keeps_the_live_one() {
    let (root, data_dir) = setup();
    let junk = root.path().join("junk.db");
    std::fs::write(&junk, b"this is not a sqlite database at all").unwrap();

    let out = tpt(&data_dir, &["restore", junk.to_str().unwrap(), "--yes"]);
    assert!(!out.status.success(), "a non-database must not restore");
    assert!(
        stderr(&out).contains("is not a SQLite database"),
        "{}",
        stderr(&out)
    );
    // The live database still works and still has its admin.
    assert!(stdout(&tpt(&data_dir, &["user", "list"])).contains("admin"));
}

#[tokio::test]
async fn restore_refuses_a_sqlite_file_that_is_not_a_tpt_home_backup() {
    use sqlx::Connection;

    let (root, data_dir) = setup();
    let other = root.path().join("other.db");
    let url = format!("sqlite://{}?mode=rwc", other.display());
    let mut conn = sqlx::SqliteConnection::connect(&url).await.unwrap();
    sqlx::query("CREATE TABLE widgets (id INTEGER PRIMARY KEY)")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();

    let out = tpt(&data_dir, &["restore", other.to_str().unwrap(), "--yes"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("not a tpt-home backup"),
        "{}",
        stderr(&out)
    );
    assert!(stdout(&tpt(&data_dir, &["user", "list"])).contains("admin"));
}

#[test]
fn restore_of_a_missing_file_fails() {
    let (root, data_dir) = setup();
    let missing = root.path().join("nope.db");
    let out = tpt(&data_dir, &["restore", missing.to_str().unwrap(), "--yes"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("backup not found"),
        "{}",
        stderr(&out)
    );
}
