// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `migrate`, and the refusal paths of `new`, run as the real binary.
//! `new` writes into the repository, so its blueprint success path is not run here.
//! `new --plugin` is tested in a scratch workspace.

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

/// A directory that looks like the repo root to `new`: a workspace manifest
/// with an empty members list.
fn scratch_workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\n]\n",
    )
    .unwrap();
    root
}

fn new_plugin(cwd: &Path, name: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(cwd)
        .args(["new", "--plugin", name])
        .output()
        .expect("run tpt-home new --plugin")
}

#[test]
fn new_plugin_scaffolds_a_registered_plugin_crate() {
    let root = scratch_workspace();
    let out = new_plugin(root.path(), "demo_plugin");
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let dir = root.path().join("plugins").join("demo_plugin");
    let manifest = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    assert!(manifest.contains("id = \"demo_plugin\""), "{manifest}");
    assert!(manifest.contains("name = \"Demo Plugin\""), "{manifest}");
    let lib = std::fs::read_to_string(dir.join("src").join("lib.rs")).unwrap();
    assert!(lib.contains("demo_plugin_items"), "{lib}");
    assert!(lib.contains("export!(handle);"), "{lib}");
    let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("name = \"tpt-home-plugin-demo_plugin\""),
        "{cargo}"
    );
    assert!(
        cargo.contains("crate-type = [\"cdylib\", \"rlib\"]"),
        "{cargo}"
    );

    let root_manifest = std::fs::read_to_string(root.path().join("Cargo.toml")).unwrap();
    assert!(
        root_manifest.contains("\"plugins/demo_plugin\","),
        "{root_manifest}"
    );
    // Plugins are loaded from packages, so the CLI is not touched.
    assert!(!root.path().join("crates").exists());
}

#[test]
fn new_plugin_refuses_an_existing_directory() {
    let root = scratch_workspace();
    assert!(new_plugin(root.path(), "demo_plugin").status.success());
    let again = new_plugin(root.path(), "demo_plugin");
    assert!(!again.status.success());
    assert!(
        stderr(&again).contains("already exists"),
        "{}",
        stderr(&again)
    );
}

#[test]
fn new_plugin_rejects_a_blueprint_template_and_a_bad_id() {
    let root = scratch_workspace();
    let conflict = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(root.path())
        .args(["new", "--plugin", "demo", "--template", "records"])
        .output()
        .expect("run tpt-home new");
    assert!(!conflict.status.success());
    assert!(
        stderr(&conflict).contains("cannot be used with"),
        "{}",
        stderr(&conflict)
    );

    let bad = new_plugin(root.path(), "Bad-Name");
    assert!(!bad.status.success());
    assert!(!root.path().join("plugins").exists());
}

#[test]
fn migrate_reports_what_it_applied_and_keeps_a_copy_first() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let init = tpt(&data, &["init", "--admin-password", ADMIN_PASSWORD]);
    assert!(init.status.success(), "{}", stderr(&init));
    assert!(stdout(&tpt(&data, &["migrate"])).contains("database is up to date"));

    // Forget one core migration, as an older version would have.
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let pool = tpt_home_core::db::connect(&data.join("tpt.db"))
            .await
            .unwrap();
        sqlx::query("DELETE FROM _tpt_schema_migrations WHERE key = ?")
            .bind("tpt-home-core:004_api_tokens")
            .execute(&pool)
            .await
            .unwrap();
    });

    let dry = tpt(&data, &["migrate", "--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(
        stdout(&dry).contains("1 migration(s) would be applied"),
        "{}",
        stdout(&dry)
    );
    assert!(stdout(&dry).contains("004_api_tokens"));

    let run = tpt(&data, &["migrate"]);
    assert!(run.status.success(), "{}", stderr(&run));
    let text = stdout(&run);
    assert!(text.contains("copy saved before upgrading"), "{text}");
    assert!(
        text.contains("applied tpt-home-core:004_api_tokens"),
        "{text}"
    );
    assert!(data.join("backups").join("upgrade").is_dir());
}

/// A scratch repo laid out like the real one where `new` edits it: the root
/// workspace, the CLI's manifest with one blueprint dependency, and its
/// `main.rs` with one link anchor.
fn scratch_repo_with_cli() -> tempfile::TempDir {
    let root = scratch_workspace();
    let cli = root.path().join("crates").join("tpt-home-cli");
    std::fs::create_dir_all(cli.join("src")).unwrap();
    std::fs::write(
        cli.join("Cargo.toml"),
        "[package]\nname = \"tpt-home-cli\"\n\n[dependencies]\n\
         tpt-home-bp-track = { path = \"../../blueprints/tpt-home-bp-track\", version = \"0.1.0\" }\n\
         tpt-home-plugins = { path = \"../tpt-home-plugins\", version = \"0.1.0\" }\n",
    )
    .unwrap();
    std::fs::write(
        cli.join("src").join("main.rs"),
        "static BLUEPRINT_LINK_ANCHORS: &[fn() -> &'static str] = &[\n    \
         tpt_home_bp_track::__link_anchor,\n    \
         tpt_home_plugins::__link_anchor,\n];\n",
    )
    .unwrap();
    root
}

#[test]
fn new_blueprint_scaffolds_registers_and_links_the_crate() {
    let root = scratch_repo_with_cli();
    let out = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(root.path())
        .args(["new", "demo_notes", "--template", "basic"])
        .output()
        .expect("run tpt-home new");
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let dir = root
        .path()
        .join("blueprints")
        .join("tpt-home-bp-demo_notes");
    let lib = std::fs::read_to_string(dir.join("src").join("lib.rs")).unwrap();
    assert!(lib.contains("demo_notes"), "name filled in: {lib}");
    assert!(!lib.contains("<NAME>") && !lib.contains("<TITLE>"), "{lib}");
    assert!(dir.join("Cargo.toml").exists());

    let root_manifest = std::fs::read_to_string(root.path().join("Cargo.toml")).unwrap();
    assert!(
        root_manifest.contains("\"blueprints/tpt-home-bp-demo_notes\","),
        "{root_manifest}"
    );
    let cli_manifest =
        std::fs::read_to_string(root.path().join("crates/tpt-home-cli/Cargo.toml")).unwrap();
    assert!(
        cli_manifest.contains(
            "tpt-home-bp-demo_notes = { path = \"../../blueprints/tpt-home-bp-demo_notes\""
        ),
        "{cli_manifest}"
    );
    let main =
        std::fs::read_to_string(root.path().join("crates/tpt-home-cli/src/main.rs")).unwrap();
    assert!(
        main.contains("tpt_home_bp_demo_notes::__link_anchor,"),
        "{main}"
    );

    // A second run refuses rather than overwriting the crate.
    let again = Command::new(env!("CARGO_BIN_EXE_tpt-home"))
        .current_dir(root.path())
        .args(["new", "demo_notes"])
        .output()
        .unwrap();
    assert!(!again.status.success());
    assert!(
        stderr(&again).contains("already exists"),
        "{}",
        stderr(&again)
    );
}
