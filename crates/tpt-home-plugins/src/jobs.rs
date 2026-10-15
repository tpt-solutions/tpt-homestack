// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Scheduled jobs for plugins (`[[jobs]]` in `plugin.toml`).
//!
//! The host calls a job's `path` with `POST` and no caller, once every
//! `every`. One scheduler task serves every enabled plugin, so enabling,
//! disabling or removing a plugin takes effect on the next tick with no
//! per-plugin task to manage.
//!
//! Runs are sequential: a job due while another is running waits its turn,
//! so jobs never overlap. Each run is bounded by the plugin's `max_millis`
//! (the call deadline) plus one second. A job is first due one interval after
//! the scheduler first sees it, the same as `#[bp_job]`, and is due again one
//! interval after each run.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::{dispatch, manifest, registry};

/// How often the scheduler looks for due jobs. Jobs are at least 10s apart,
/// so this bounds how late a run can start.
const TICK: Duration = Duration::from_secs(5);

/// One finished job run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    pub plugin: String,
    pub job: String,
    /// `ok`, `timed out`, or `failed: <reason>`.
    pub outcome: String,
}

/// Tracks when each enabled job is next due.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// Unix seconds when each `(plugin, job)` is next due.
    due: HashMap<(String, String), i64>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run every enabled job that is due at `now` (Unix seconds), one after
    /// another, and return what ran.
    pub async fn run_due(&mut self, now: i64) -> Vec<JobRun> {
        let mut jobs = Vec::new();
        for (plugin, enabled) in registry::all() {
            if !enabled {
                continue;
            }
            for job in &plugin.manifest.jobs {
                let Ok(every) = manifest::job_interval_secs(&job.every) else {
                    continue;
                };
                jobs.push((
                    plugin.manifest.plugin.id.clone(),
                    job.clone(),
                    every as i64,
                    plugin.manifest.limits.max_millis,
                ));
            }
        }

        // Forget jobs that are disabled, removed or no longer declared, so a
        // later re-enable starts a fresh interval.
        let live: HashSet<(String, String)> = jobs
            .iter()
            .map(|(plugin, job, _, _)| (plugin.clone(), job.name.clone()))
            .collect();
        self.due.retain(|key, _| live.contains(key));

        let mut runs = Vec::new();
        for (plugin, job, every, max_millis) in jobs {
            let key = (plugin.clone(), job.name.clone());
            let due = *self.due.entry(key.clone()).or_insert(now + every);
            if now < due {
                continue;
            }
            let outcome = run_one(&plugin, &job.path, max_millis).await;
            if outcome == "ok" {
                tracing::debug!(plugin = %plugin, job = %job.name, "plugin job ran");
            } else {
                tracing::warn!(plugin = %plugin, job = %job.name, %outcome, "plugin job did not finish cleanly");
            }
            self.due.insert(key, now + every);
            runs.push(JobRun {
                plugin,
                job: job.name,
                outcome,
            });
        }
        runs
    }
}

/// Call one job handler and describe the result.
async fn run_one(plugin: &str, path: &str, max_millis: u64) -> String {
    let rest = path.trim_start_matches('/');
    let limit = Duration::from_millis(max_millis) + Duration::from_secs(1);
    match tokio::time::timeout(
        limit,
        dispatch::invoke_plugin(None, plugin, "POST", rest, "", None),
    )
    .await
    {
        Err(_) => "timed out".to_string(),
        Ok(Err(e)) => format!("failed: {e}"),
        Ok(Ok(response)) if response.status().is_success() => "ok".to_string(),
        Ok(Ok(response)) => format!("failed: status {}", response.status()),
    }
}

/// Run the scheduler for the life of the server.
pub async fn run_forever() {
    let mut scheduler = Scheduler::new();
    let mut ticker = tokio::time::interval(TICK);
    loop {
        ticker.tick().await;
        scheduler.run_due(unix_now()).await;
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// Start the scheduler at server startup, next to the plugin loader.
inventory::submit! {
    tpt_home_core::StartupEntry {
        name: "plugin-jobs",
        spawn: |_db| {
            tokio::spawn(run_forever());
        },
    }
}
