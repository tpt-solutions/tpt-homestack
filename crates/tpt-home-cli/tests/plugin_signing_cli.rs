// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plugin signing through the real binary: keygen, sign, and install with the
//! trusted-key and required-signature rules from config.toml.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MANIFEST: &str = r#"
[plugin]
id = "signed"
name = "Signed"
version = "0.1.0"
runtime = "external"
entry = "http://127.0.0.1:8791"

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

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

/// A data directory with an admin, a packed plugin, and its path.
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("data");
    let init = tpt(
        &data_dir,
        &["init", "--admin-password", "cli-test-password-1"],
    );
    assert!(init.status.success(), "init: {}", text(&init.stderr));
    let src = root.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("plugin.toml"), MANIFEST).unwrap();
    let package = tpt_home_plugins::package::pack(&src).unwrap();
    let tptp = root.path().join("signed-0.1.0.tptp");
    std::fs::write(&tptp, &package.bytes).unwrap();
    (root, data_dir, tptp)
}

/// Create a key pair under `root`, returning the secret path and public hex.
fn keygen(root: &Path, data_dir: &Path) -> (PathBuf, String) {
    let key = root.join("publisher.key");
    let out = tpt(
        data_dir,
        &["plugin", "keygen", "--out", key.to_str().unwrap()],
    );
    assert!(out.status.success(), "keygen: {}", text(&out.stderr));
    let public = std::fs::read_to_string(root.join("publisher.key.pub")).unwrap();
    (key, public.trim().to_string())
}

fn write_config(data_dir: &Path, keys: &[String], require: bool) {
    let quoted: Vec<String> = keys.iter().map(|k| format!("\"{k}\"")).collect();
    std::fs::write(
        data_dir.join("config.toml"),
        format!(
            "trusted_plugin_keys = [{}]\nrequire_signed_plugins = {require}\n",
            quoted.join(", ")
        ),
    )
    .unwrap();
}

#[test]
fn a_signed_package_installs_only_with_a_trusted_key() {
    let (root, data_dir, tptp) = setup();
    let (key, public) = keygen(root.path(), &data_dir);
    let signed = tpt(
        &data_dir,
        &[
            "plugin",
            "sign",
            tptp.to_str().unwrap(),
            "--key",
            key.to_str().unwrap(),
        ],
    );
    assert!(signed.status.success(), "sign: {}", text(&signed.stderr));
    let sig = PathBuf::from(format!("{}.sig", tptp.display()));
    assert!(sig.exists());

    // Before the key is trusted, the signature is refused.
    let refused = tpt(
        &data_dir,
        &[
            "plugin",
            "install",
            tptp.to_str().unwrap(),
            "--signature",
            sig.to_str().unwrap(),
        ],
    );
    assert!(!refused.status.success());
    assert!(
        text(&refused.stderr).contains("no trusted plugin keys"),
        "{}",
        text(&refused.stderr)
    );

    write_config(&data_dir, std::slice::from_ref(&public), false);
    let installed = tpt(
        &data_dir,
        &[
            "plugin",
            "install",
            tptp.to_str().unwrap(),
            "--signature",
            sig.to_str().unwrap(),
        ],
    );
    assert!(
        installed.status.success(),
        "install: {}",
        text(&installed.stderr)
    );
    assert!(
        text(&installed.stdout).contains("signature verified"),
        "{}",
        text(&installed.stdout)
    );
}

#[test]
fn a_changed_package_is_refused_even_with_its_old_signature() {
    let (root, data_dir, tptp) = setup();
    let (key, public) = keygen(root.path(), &data_dir);
    tpt(
        &data_dir,
        &[
            "plugin",
            "sign",
            tptp.to_str().unwrap(),
            "--key",
            key.to_str().unwrap(),
        ],
    );
    let sig = PathBuf::from(format!("{}.sig", tptp.display()));
    write_config(&data_dir, &[public], false);

    let mut bytes = std::fs::read(&tptp).unwrap();
    bytes.push(0);
    std::fs::write(&tptp, &bytes).unwrap();
    let out = tpt(
        &data_dir,
        &[
            "plugin",
            "install",
            tptp.to_str().unwrap(),
            "--signature",
            sig.to_str().unwrap(),
        ],
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("does not match"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn required_signatures_refuse_an_unsigned_package() {
    let (root, data_dir, tptp) = setup();
    let (_key, public) = keygen(root.path(), &data_dir);
    write_config(&data_dir, &[public], true);
    let out = tpt(&data_dir, &["plugin", "install", tptp.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("only signed plugins"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn unsigned_installs_still_work_when_signatures_are_optional() {
    let (root, data_dir, tptp) = setup();
    let (_key, public) = keygen(root.path(), &data_dir);
    write_config(&data_dir, &[public], false);
    let out = tpt(&data_dir, &["plugin", "install", tptp.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("unsigned package"));
}

#[test]
fn keygen_never_overwrites_an_existing_secret() {
    let (root, data_dir, _tptp) = setup();
    let (key, _public) = keygen(root.path(), &data_dir);
    let before = std::fs::read(&key).unwrap();
    let again = tpt(
        &data_dir,
        &["plugin", "keygen", "--out", key.to_str().unwrap()],
    );
    assert!(!again.status.success());
    assert_eq!(
        std::fs::read(&key).unwrap(),
        before,
        "the secret is untouched"
    );
}
