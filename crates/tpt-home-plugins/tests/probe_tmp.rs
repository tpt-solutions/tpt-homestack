use std::path::PathBuf;
use tpt_home_plugins::manifest::{self, Permissions};
use tpt_home_plugins::runtime::{self, PluginRequest};

#[test]
#[ignore]
fn probe() {
    std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let wasm = std::fs::read(root.join("target/wasm32-unknown-unknown/release/tpt_home_bp_calendar.wasm")).unwrap();
        eprintln!("A: read wasm {}", wasm.len());
        let m = manifest::from_dir(&root.join("plugins/tpt-home-bp-calendar")).unwrap();
        eprintln!("B: manifest");
        let module = wasmi::Module::new(runtime::engine(), wasm.as_slice()).unwrap();
        eprintln!("C: module");
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("a.db");
        rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TABLE calendar_events (id INTEGER PRIMARY KEY, title TEXT, starts_at INTEGER);").unwrap();
        eprintln!("D: db");
        let r = runtime::call_plugin(&module, &m, &PluginRequest{method:"GET".into(),path:"/nope".into(),query:String::new(),body:None}, "calendar", &Permissions::default(), &db, dir.path());
        eprintln!("E: unknown route -> {:?}", r.map(|r| r.status));
        let r = runtime::call_plugin(&module, &m, &PluginRequest{method:"GET".into(),path:"/events".into(),query:String::new(),body:None}, "calendar", &m.permissions, &db, dir.path());
        eprintln!("F: list -> {:?}", r.map(|r| r.status));
    }).unwrap().join().unwrap();
}
