// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Blueprint registry.
//!
//! Blueprints register themselves at compile time through `inventory`
//! entries emitted by the `tpt-home-macros` attribute macros. At startup the
//! API crate walks these collections and wires routes/UI/migrations in —
//! no manual registration code anywhere.

use axum::Router;
use inventory as inv;

use crate::db::DbPool;

/// The trait implemented for every `#[blueprint]` struct. Deliberately tiny:
/// routes, migrations and UI pages register themselves separately, so a
/// blueprint needs zero boilerplate methods.
pub trait Blueprint: Send + Sync + 'static {
    /// Short id, e.g. `"myapp"`.
    fn id(&self) -> &'static str;
    /// Human-readable display name.
    fn name(&self) -> &'static str;
}

/// Metadata for one compiled-in blueprint, emitted by `#[blueprint]`.
pub struct BlueprintEntry {
    /// Short id, e.g. `"notes"` (from `tpt-home-bp-notes`).
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
}

inv::collect!(BlueprintEntry);

/// One API route contributed by `#[bp_route]`.
///
/// `register` receives the shared `Router<DbPool>` and returns it with the
/// route attached, so any axum combinator stays available.
pub struct RouteEntry {
    /// Crate that contributed the route (i.e. the blueprint crate name).
    pub blueprint: &'static str,
    /// HTTP method, uppercase (`"GET"`, `"POST"`, ...).
    pub method: &'static str,
    /// Full path; must start with `/api/`.
    pub path: &'static str,
    /// Minimum role required, `Some("admin")` etc.
    pub role: Option<&'static str>,
    /// Adds this route to the router.
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(RouteEntry);

/// One *unauthenticated* API route contributed by a blueprint (e.g. the
/// drop blueprint's public share links). Emitted by
/// `#[bp_route(..., public)]`.
pub struct PublicRouteEntry {
    pub blueprint: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(PublicRouteEntry);

/// One UI page contributed by `#[bp_ui_route]`: a server-rendered page plus a
/// sidebar entry. Paths live outside `/api/`.
pub struct UiEntry {
    pub blueprint: &'static str,
    /// Page path, e.g. `"/notes"`.
    pub path: &'static str,
    /// Sidebar label.
    pub label: &'static str,
    pub register: fn(Router<DbPool>) -> Router<DbPool>,
}

inv::collect!(UiEntry);

/// A background task contributed by a blueprint (e.g. the drop cleanup
/// cron). `spawn` is called once at server startup with the shared pool.
pub struct StartupEntry {
    /// Unique task name for logs.
    pub name: &'static str,
    /// Spawns the task onto the current tokio runtime.
    pub spawn: fn(DbPool),
}

inv::collect!(StartupEntry);

/// The future a scheduled job returns. Boxed so jobs can be plain fn pointers.
pub type JobFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::ApiError>> + Send>>;

/// A periodic job contributed by `#[bp_job(every = "...")]`. The server runs
/// `run` once per `every`, starting one period after boot. A failed or
/// panicking run is logged and the next run still happens. A run that takes
/// longer than `timeout` is aborted and logged; the next run waits for the
/// previous one to finish, so runs never overlap.
pub struct JobEntry {
    pub blueprint: &'static str,
    /// Job name for logs, e.g. `"nightly-export"`.
    pub name: &'static str,
    pub every: std::time::Duration,
    pub timeout: std::time::Duration,
    pub run: fn(DbPool) -> JobFuture,
}

inv::collect!(JobEntry);

/// Live state of one scheduled job, as shown on the admin jobs page.
#[derive(Clone, Debug, serde::Serialize)]
pub struct JobStatus {
    pub blueprint: String,
    pub name: String,
    pub every_secs: u64,
    pub timeout_secs: u64,
    /// Unix seconds when the next run is due.
    pub next_run_at: i64,
    pub last_started_at: Option<i64>,
    pub last_finished_at: Option<i64>,
    /// `ok`, `failed`, `panicked` or `timed out`.
    pub last_outcome: Option<String>,
    pub last_error: Option<String>,
    pub runs: u64,
    pub failures: u64,
}

/// One entry per job, keyed by blueprint and name. Kept in memory: it resets
/// on restart, which is fine for "what happened lately".
static JOB_STATUS: std::sync::Mutex<Vec<JobStatus>> = std::sync::Mutex::new(Vec::new());

/// Update (creating on first use) the status entry for `job`.
fn with_status<R>(job: &JobEntry, update: impl FnOnce(&mut JobStatus) -> R) -> R {
    let mut all = JOB_STATUS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let index = match all
        .iter()
        .position(|s| s.blueprint == job.blueprint && s.name == job.name)
    {
        Some(index) => index,
        None => {
            all.push(JobStatus {
                blueprint: job.blueprint.to_string(),
                name: job.name.to_string(),
                every_secs: job.every.as_secs(),
                timeout_secs: job.timeout.as_secs(),
                next_run_at: unix_now() + job.every.as_secs() as i64,
                last_started_at: None,
                last_finished_at: None,
                last_outcome: None,
                last_error: None,
                runs: 0,
                failures: 0,
            });
            all.len() - 1
        }
    };
    update(&mut all[index])
}

/// Snapshot of every job's status, in registration order.
pub fn job_statuses() -> Vec<JobStatus> {
    JOB_STATUS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Start every registered job. Called once at server startup.
pub fn spawn_jobs(db: DbPool) {
    for job in inv::iter::<JobEntry>() {
        with_status(job, |_| ());
        tokio::spawn(run_job_loop(job, db.clone()));
        tracing::debug!(
            job = job.name,
            every_secs = job.every.as_secs(),
            "scheduled job"
        );
    }
}

/// Run one job forever. Each run is its own task, so a panic surfaces as a
/// `JoinError` here instead of ending the loop, and a timed-out run is
/// aborted before the next tick is awaited.
async fn run_job_loop(job: &'static JobEntry, db: DbPool) {
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + job.every, job.every);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        with_status(job, |s| s.last_started_at = Some(unix_now()));
        let handle = tokio::spawn((job.run)(db.clone()));
        let abort = handle.abort_handle();
        let (outcome, error) = match tokio::time::timeout(job.timeout, handle).await {
            Ok(Ok(Ok(()))) => {
                tracing::debug!(job = job.name, "job finished");
                ("ok", None)
            }
            Ok(Ok(Err(e))) => {
                tracing::warn!(job = job.name, error = %e, "job failed");
                ("failed", Some(e.to_string()))
            }
            Ok(Err(e)) => {
                tracing::error!(job = job.name, error = %e, "job panicked");
                ("panicked", Some(e.to_string()))
            }
            Err(_) => {
                abort.abort();
                tracing::warn!(
                    job = job.name,
                    timeout_secs = job.timeout.as_secs(),
                    "job timed out"
                );
                ("timed out", None)
            }
        };
        let now = unix_now();
        with_status(job, |s| {
            s.runs += 1;
            if outcome != "ok" {
                s.failures += 1;
            }
            s.last_finished_at = Some(now);
            s.last_outcome = Some(outcome.to_string());
            s.last_error = error;
            s.next_run_at = now + job.every.as_secs() as i64;
        });
    }
}

/// All registered blueprints in registration order.
pub fn blueprints() -> impl Iterator<Item = &'static BlueprintEntry> {
    inv::iter::<BlueprintEntry>()
}

/// Apply every blueprint API route to `router`.
pub fn apply_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<RouteEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Apply every blueprint public route to the *unauthenticated* router.
pub fn apply_public_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<PublicRouteEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Apply every blueprint UI page to `router`.
pub fn apply_ui_routes(router: Router<DbPool>) -> Router<DbPool> {
    let mut router = router;
    for entry in inv::iter::<UiEntry>() {
        router = (entry.register)(router);
    }
    router
}

/// Spawn every blueprint background task.
pub fn spawn_startup_tasks(db: DbPool) {
    for entry in inv::iter::<StartupEntry>() {
        (entry.spawn)(db.clone());
        tracing::debug!(task = entry.name, "started blueprint task");
    }
}

/// Sidebar navigation derived from registered UI pages, plus any extra
/// entries contributed at runtime (e.g. enabled WASM plugins install a nav
/// provider at startup).
pub fn nav_entries() -> Vec<(String, String)> {
    let mut nav: Vec<(String, String)> = inv::iter::<UiEntry>()
        .map(|e| (e.label.to_string(), e.path.to_string()))
        .collect();
    if let Some(provider) = NAV_PROVIDER.get() {
        nav.extend(provider());
    }
    nav
}

/// A nav entry: `(label, path)`.
type NavEntry = (String, String);

static NAV_PROVIDER: std::sync::OnceLock<fn() -> Vec<NavEntry>> = std::sync::OnceLock::new();

/// Install a runtime nav provider (one extra sidebar entry per enabled
/// plugin). Returns `false` if one was already installed.
pub fn set_nav_provider(provider: fn() -> Vec<NavEntry>) -> bool {
    NAV_PROVIDER.set(provider).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collections_are_iterable_without_entries() {
        // Core alone registers no blueprints; iteration must not panic and
        // nav derives from the same collection.
        assert_eq!(nav_entries().len(), blueprints().count());
    }
}

#[cfg(test)]
mod job_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static OK_RUNS: AtomicUsize = AtomicUsize::new(0);
    static ERR_RUNS: AtomicUsize = AtomicUsize::new(0);
    static PANIC_RUNS: AtomicUsize = AtomicUsize::new(0);

    fn ok_job(_db: DbPool) -> JobFuture {
        Box::pin(async {
            OK_RUNS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn err_job(_db: DbPool) -> JobFuture {
        Box::pin(async {
            ERR_RUNS.fetch_add(1, Ordering::SeqCst);
            Err(crate::ApiError::bad_request("expected failure"))
        })
    }

    fn panic_job(_db: DbPool) -> JobFuture {
        Box::pin(async {
            PANIC_RUNS.fetch_add(1, Ordering::SeqCst);
            panic!("expected panic");
        })
    }

    static HANG_RUNS: AtomicUsize = AtomicUsize::new(0);

    /// A run that never finishes on its own; only the timeout can end it.
    fn hang_job(_db: DbPool) -> JobFuture {
        Box::pin(async {
            HANG_RUNS.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
            Ok(())
        })
    }

    fn leak_job(name: &'static str, run: fn(DbPool) -> JobFuture) -> &'static JobEntry {
        leak_job_with_timeout(name, run, std::time::Duration::from_secs(5))
    }

    fn leak_job_with_timeout(
        name: &'static str,
        run: fn(DbPool) -> JobFuture,
        timeout: std::time::Duration,
    ) -> &'static JobEntry {
        Box::leak(Box::new(JobEntry {
            blueprint: "test",
            name,
            every: std::time::Duration::from_millis(20),
            timeout,
            run,
        }))
    }

    #[tokio::test]
    async fn hanging_run_is_aborted_and_the_job_keeps_running() {
        let db = crate::db::connect_in_memory().await.unwrap();
        let job = leak_job_with_timeout("hang", hang_job, std::time::Duration::from_millis(30));
        tokio::spawn(run_job_loop(job, db));

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        // Without the timeout the first run would block the loop forever and
        // the counter would stay at 1.
        assert!(
            HANG_RUNS.load(Ordering::SeqCst) >= 3,
            "hang job ran {}",
            HANG_RUNS.load(Ordering::SeqCst)
        );

        let status = job_statuses()
            .into_iter()
            .find(|s| s.blueprint == "test" && s.name == "hang")
            .expect("status recorded for the hang job");
        assert_eq!(status.last_outcome.as_deref(), Some("timed out"));
        assert!(status.failures >= 1 && status.runs >= status.failures);
        assert!(status.next_run_at >= status.last_finished_at.unwrap_or(0));
    }

    #[tokio::test]
    async fn jobs_repeat_and_survive_errors_and_panics() {
        let db = crate::db::connect_in_memory().await.unwrap();
        let jobs = [
            leak_job("ok", ok_job),
            leak_job("err", err_job),
            leak_job("panic", panic_job),
        ];
        for job in jobs {
            tokio::spawn(run_job_loop(job, db.clone()));
        }

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Each job runs several times; the failing ones keep running after
        // their first error or panic.
        assert!(
            OK_RUNS.load(Ordering::SeqCst) >= 3,
            "ok job ran {}",
            OK_RUNS.load(Ordering::SeqCst)
        );
        assert!(
            ERR_RUNS.load(Ordering::SeqCst) >= 3,
            "err job ran {}",
            ERR_RUNS.load(Ordering::SeqCst)
        );
        assert!(
            PANIC_RUNS.load(Ordering::SeqCst) >= 3,
            "panic job ran {}",
            PANIC_RUNS.load(Ordering::SeqCst)
        );
    }
}

/// A webhook delivery accepted by the server, handed to blueprint code by
/// `#[bp_webhook("name")]`. `body` is the UTF-8 text the sender posted.
#[derive(Clone, Debug)]
pub struct WebhookDelivery {
    pub id: i64,
    pub webhook: String,
    pub received_at: i64,
    pub body: String,
}

/// The future a webhook handler returns. Boxed so handlers can be plain fn pointers.
pub type WebhookFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::ApiError>> + Send>>;

/// A handler contributed by `#[bp_webhook("name")]`. The server calls it once
/// for each accepted delivery to that webhook, after the sender has been answered.
pub struct WebhookEntry {
    pub blueprint: &'static str,
    /// The webhook name it listens to, as created on the Webhooks page.
    pub webhook: &'static str,
    pub run: fn(DbPool, WebhookDelivery) -> WebhookFuture,
}

inv::collect!(WebhookEntry);

/// Longest a webhook handler may run for one delivery.
pub const WEBHOOK_HANDLER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Hand a delivery to every handler registered for its webhook. Each handler
/// runs in its own task, so the caller does not wait, a failure is logged, a
/// panic is contained, and a run over `WEBHOOK_HANDLER_TIMEOUT` is abandoned.
pub fn dispatch_webhook(db: &DbPool, delivery: WebhookDelivery) {
    for entry in inv::iter::<WebhookEntry>() {
        if entry.webhook != delivery.webhook {
            continue;
        }
        let db = db.clone();
        let delivery = delivery.clone();
        let blueprint = entry.blueprint;
        tokio::spawn(async move {
            let run = tokio::spawn((entry.run)(db, delivery.clone()));
            match tokio::time::timeout(WEBHOOK_HANDLER_TIMEOUT, run).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => tracing::warn!(
                    blueprint,
                    webhook = %delivery.webhook,
                    delivery = delivery.id,
                    error = %e,
                    "webhook handler failed"
                ),
                Ok(Err(_)) => tracing::error!(
                    blueprint,
                    webhook = %delivery.webhook,
                    delivery = delivery.id,
                    "webhook handler panicked"
                ),
                Err(_) => tracing::warn!(
                    blueprint,
                    webhook = %delivery.webhook,
                    delivery = delivery.id,
                    "webhook handler timed out"
                ),
            }
        });
    }
}
