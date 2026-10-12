// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Host-boundary tests: hostile or broken guest modules must produce `Err`
//! from `call_plugin`, never a panic, a hang, or a host-memory read. The
//! guests are small WAT modules compiled by wasmi's `wat` feature.

use tpt_home_plugins::manifest::{self, Permissions, PluginManifest};
use tpt_home_plugins::runtime::{self, PluginRequest, PluginResponse};

fn manifest_with_fuel(max_fuel: u64) -> PluginManifest {
    manifest::parse(&format!(
        r#"
[plugin]
id = "bad"
name = "Bad"
version = "0.0.1"
entry = "plugin.wasm"

[limits]
max_fuel = {max_fuel}
max_memory_mb = 16
"#
    ))
    .expect("manifest")
}

fn request() -> PluginRequest {
    PluginRequest {
        method: "GET".into(),
        path: "/".into(),
        query: String::new(),
        body: None,
    }
}

/// Instantiate `wat` and call it with the default permissions. Runs on a
/// roomy thread: debug builds of the wasmi interpreter need more stack than
/// the default test thread gets.
fn call(wat: &str, max_fuel: u64) -> Result<PluginResponse, anyhow::Error> {
    let wat = wat.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || call_on_this_thread(&wat, max_fuel))
        .expect("spawn")
        .join()
        .expect("guest call panicked")
}

fn call_on_this_thread(wat: &str, max_fuel: u64) -> Result<PluginResponse, anyhow::Error> {
    let module = wasmi::Module::new(runtime::engine(), wat.as_bytes())
        .map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    let dir = tempfile::tempdir().unwrap();
    runtime::call_plugin(
        &module,
        &manifest_with_fuel(max_fuel),
        &request(),
        "bad",
        &Permissions::default(),
        &dir.path().join("none.db"),
        dir.path(),
    )
}

/// Build a guest whose `tpt_entry` returns `ret` and whose linear memory
/// holds `data` at offset 0.
fn guest_returning(data: &[u8], ret: &str) -> String {
    let escaped: String = data.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{escaped}")
            ;; Allocate past the data segment so the request does not overwrite it.
            (func (export "tpt_alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tpt_entry") (param i32 i32) (result i64) {ret})
        )"#
    )
}

const RESPONSE_JSON: &str = r#"{"status":200,"content_type":"text/plain","body":"hi"}"#;

#[test]
fn well_formed_guest_round_trips() {
    let wat = guest_returning(
        RESPONSE_JSON.as_bytes(),
        &format!("i64.const {}", RESPONSE_JSON.len()),
    );
    let r = call(&wat, 1_000_000).expect("valid guest");
    assert_eq!(r.status, 200);
    assert_eq!(r.body, "hi");
}

#[test]
fn infinite_loop_exhausts_fuel() {
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "tpt_alloc") (param i32) (result i32) i32.const 0)
        (func (export "tpt_entry") (param i32 i32) (result i64)
            (loop $spin (br $spin))
            i64.const 0))"#;
    assert!(call(wat, 100_000).is_err(), "fuel limit must stop the loop");
}

#[test]
fn reported_failure_sentinel_is_an_error() {
    let wat = guest_returning(b"", "i64.const -1");
    let err = call(&wat, 1_000_000).unwrap_err().to_string();
    assert!(err.contains("failure"), "{err}");
}

#[test]
fn response_pointer_outside_memory_is_an_error() {
    // ptr = 0xFFFF_FFF0 (far beyond the single 64 KiB page), len = 16.
    let wat = guest_returning(b"", "i64.const 0xFFFFFFF000000010");
    assert!(call(&wat, 1_000_000).is_err());
}

#[test]
fn response_length_over_cap_is_an_error() {
    // len = 0xFFFF_FFFF (over the 4 MiB result cap).
    let wat = guest_returning(b"", "i64.const 0xFFFFFFFF");
    let err = call(&wat, 1_000_000).unwrap_err().to_string();
    assert!(err.contains("too large"), "{err}");
}

#[test]
fn non_json_response_is_an_error() {
    let garbage = b"<html>not json</html>";
    let wat = guest_returning(garbage, &format!("i64.const {}", garbage.len()));
    let err = call(&wat, 1_000_000).unwrap_err().to_string();
    assert!(err.contains("invalid response JSON"), "{err}");
}

#[test]
fn missing_memory_export_is_an_error() {
    let wat = r#"(module
        (func (export "tpt_alloc") (param i32) (result i32) i32.const 0)
        (func (export "tpt_entry") (param i32 i32) (result i64) i64.const 0))"#;
    let err = call(wat, 1_000_000).unwrap_err().to_string();
    assert!(err.contains("does not export memory"), "{err}");
}

#[test]
fn missing_entry_point_is_an_error() {
    let wat = r#"(module (memory (export "memory") 1))"#;
    assert!(call(wat, 1_000_000).is_err());
}

#[test]
fn memory_growth_past_the_cap_traps() {
    // 100 000 pages is 6.4 GB, far above the 16 MiB manifest cap.
    let wat = r#"(module
        (memory (export "memory") 1)
        (func (export "tpt_alloc") (param i32) (result i32) i32.const 0)
        (func (export "tpt_entry") (param i32 i32) (result i64)
            (drop (memory.grow (i32.const 100000)))
            i64.const -1))"#;
    assert!(
        call(wat, 1_000_000).is_err(),
        "growth beyond the cap must fail"
    );
}
