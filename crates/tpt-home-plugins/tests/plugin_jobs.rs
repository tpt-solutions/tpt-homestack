// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plugin scheduled jobs (`[[jobs]]`): a JavaScript plugin with a counting job
//! and a failing job, installed for real and driven through
//! [`Scheduler::run_due`] with explicit times. Also checks manifest rules.
//!
//! Like the records tests, these share one database file, so they take turns.

#![cfg(feature = "js")]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tpt_home_core::config::{Config, set_global_for_tests};
use tpt_home_core::db::connect;
use tpt_home_plugins::jobs::{JobRun, Scheduler};
use tpt_home_plugins::lifecycle;
use tpt_home_plugins::manifest::{self, Permissions};

const MANIFEST: &str = r#"
[plugin]
id = "ID"
name = "Job test"
version = "0.1.0"
entry = "main.js"
runtime = "js"

[permissions]
db = true

[[tables]]
name = "ticks"
columns = [{ name = "note", kind = "text" }]

[[jobs]]
name = "tick"
every = "10s"
path = "/tick"

[[jobs]]
name = "boom"
every = "10s"
path = "/boom"
"#;

const SCRIPT: &str = r#"
function handle(req) {
  if (req.method === "POST" && req.path === "/tick") {
    const ok = tpt.db.execute("INSERT INTO ID_ticks (note) VALUES ('tick')");
    return { status: ok ? 200 : 500, content_type: "application/json", body: "{}" };
  }
  if (req.method === "POST" && req.path === "/boom") {
    return { status: 500, content_type: "application/json", body: "{}" };
  }
  return { status: 404, content_type: "application/json", body: "{}" };
}
"#;

fn data_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::mem::forget(dir);
        set_global_for_tests(Config {
            data_dir: root.clone(),
            ..Config::default()
        });
        root
    })
}

/// One test at a time over the shared database file.
async fn turn() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

/// Migrate the server database file and install a fresh copy under `id`.
async fn install(id: &str) -> tpt_home_core::db::DbPool {
    data_root();
    std::fs::create_dir_all(lifecycle::plugins_root()).unwrap();
    let db = connect(
        &Config {
            data_dir: data_root().to_path_buf(),
            ..Config::default()
        }
        .db_path(),
    )
    .await
    .unwrap();
    tpt_home_core::migrate::run_all(&db).await.unwrap();

    let dir = data_root()
        .join("staging")
        .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), MANIFEST.replace("ID", id)).unwrap();
    std::fs::write(dir.join("main.js"), SCRIPT.replace("ID", id)).unwrap();

    let grant = Permissions::parse_list("db").unwrap();
    lifecycle::install_from_dir(&db, &dir, true, &grant)
        .await
        .unwrap();
    db
}

/// A unique plugin id: lowercase letters and digits, starting with a letter.
fn fresh_id() -> String {
    format!("job{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

async fn tick_rows(db: &tpt_home_core::db::DbPool, id: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM \"{id}_ticks\""))
        .fetch_one(db)
        .await
        .unwrap();
    n
}

/// Runs of one plugin only. Plugins installed by earlier tests stay enabled in
/// the process-wide registry, so their jobs run too.
async fn run_ours(scheduler: &mut Scheduler, now: i64, id: &str) -> Vec<JobRun> {
    scheduler
        .run_due(now)
        .await
        .into_iter()
        .filter(|r| r.plugin == id)
        .collect()
}

#[tokio::test]
async fn jobs_run_once_per_interval_and_only_when_due() {
    let _turn = turn().await;
    let id = fresh_id();
    let db = install(&id).await;
    let t0 = 1_000_000;
    let mut scheduler = Scheduler::new();

    // First sight arms the interval: nothing runs until one period has passed.
    assert!(run_ours(&mut scheduler, t0, &id).await.is_empty());
    assert!(run_ours(&mut scheduler, t0 + 9, &id).await.is_empty());

    let runs = run_ours(&mut scheduler, t0 + 10, &id).await;
    let tick = runs.iter().find(|r| r.job == "tick").expect("tick is due");
    assert_eq!(tick.outcome, "ok");
    assert_eq!(tick_rows(&db, &id).await, 1);

    // Not due again until another full interval after the run.
    assert!(
        run_ours(&mut scheduler, t0 + 15, &id)
            .await
            .iter()
            .all(|r| r.job != "tick"),
        "tick must not run early"
    );
    run_ours(&mut scheduler, t0 + 20, &id).await;
    assert_eq!(tick_rows(&db, &id).await, 2);
}

#[tokio::test]
async fn a_failing_handler_is_reported_and_the_schedule_goes_on() {
    let _turn = turn().await;
    let id = fresh_id();
    install(&id).await;
    let mut scheduler = Scheduler::new();
    run_ours(&mut scheduler, 0, &id).await;

    let runs = run_ours(&mut scheduler, 10, &id).await;
    let boom = runs.iter().find(|r| r.job == "boom").expect("boom is due");
    assert_eq!(boom.outcome, "failed: status 500 Internal Server Error");

    let runs = run_ours(&mut scheduler, 20, &id).await;
    assert!(
        runs.iter().any(|r| r.job == "boom"),
        "boom runs again next interval"
    );
}

#[tokio::test]
async fn disabled_jobs_stop_and_re_enabling_starts_a_fresh_interval() {
    let _turn = turn().await;
    let id = fresh_id();
    let db = install(&id).await;
    let mut scheduler = Scheduler::new();
    run_ours(&mut scheduler, 100, &id).await;
    run_ours(&mut scheduler, 110, &id).await;
    assert_eq!(tick_rows(&db, &id).await, 1);

    lifecycle::disable(&db, &id).await.unwrap();
    assert!(
        run_ours(&mut scheduler, 500, &id).await.is_empty(),
        "disabled plugins are not run"
    );

    lifecycle::enable(&db, &id).await.unwrap();
    assert!(
        run_ours(&mut scheduler, 600, &id).await.is_empty(),
        "re-enabled jobs wait one interval before the first run"
    );
    run_ours(&mut scheduler, 610, &id).await;
    assert_eq!(tick_rows(&db, &id).await, 2);
}

#[test]
fn intervals_follow_the_bp_job_rules() {
    assert_eq!(manifest::job_interval_secs("30s"), Ok(30));
    assert_eq!(manifest::job_interval_secs(" 5m "), Ok(300));
    assert_eq!(manifest::job_interval_secs("1d"), Ok(86_400));
    assert!(manifest::job_interval_secs("5").is_err());
    assert!(manifest::job_interval_secs("5w").is_err());
    assert!(manifest::job_interval_secs("9s").is_err());
    assert!(manifest::job_interval_secs("").is_err());
    assert!(manifest::job_interval_secs("xm").is_err());
}

#[test]
fn bad_job_declarations_are_refused_at_parse() {
    let with_job = |job: &str| {
        format!(
            "[plugin]\nid = \"jobs_bad\"\nname = \"x\"\nversion = \"0.1.0\"\nentry = \"main.js\"\nruntime = \"js\"\n{job}"
        )
    };
    let ok = with_job("[[jobs]]\nname = \"sweep\"\nevery = \"1m\"\npath = \"/sweep\"\n");
    assert!(manifest::parse(&ok).is_ok());

    for (label, job) in [
        (
            "short interval",
            "[[jobs]]\nname = \"a\"\nevery = \"5s\"\npath = \"/a\"\n",
        ),
        (
            "bad path",
            "[[jobs]]\nname = \"a\"\nevery = \"1m\"\npath = \"/../x\"\n",
        ),
        (
            "no leading slash",
            "[[jobs]]\nname = \"a\"\nevery = \"1m\"\npath = \"a\"\n",
        ),
        (
            "bad name",
            "[[jobs]]\nname = \"Bad Name\"\nevery = \"1m\"\npath = \"/a\"\n",
        ),
        (
            "duplicate name",
            "[[jobs]]\nname = \"a\"\nevery = \"1m\"\npath = \"/a\"\n[[jobs]]\nname = \"a\"\nevery = \"1m\"\npath = \"/b\"\n",
        ),
    ] {
        assert!(
            manifest::parse(&with_job(job)).is_err(),
            "{label} should be refused"
        );
    }
}
