// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Runs JavaScript plugin calls in a child process, so a script cannot take the
//! server down.
//!
//! The JS engine runs in the server process by default, and Boa has no heap
//! limit, so one script could exhaust the server's memory. With a worker
//! configured, each call runs in a short-lived child of the same binary:
//!
//! - the child gets an operating-system memory limit (Windows: a Job Object;
//!   Linux: `RLIMIT_DATA` set before exec), so a script that allocates without
//!   bound is stopped at the limit instead of exhausting the machine;
//! - the parent kills the child at the call deadline, and a crash or abort in
//!   the child is an error for that one call;
//! - the child opens the same database and storage files as the host services
//!   do, so its host calls need no extra channel.
//!
//! The protocol is one JSON line in on stdin and one JSON line out on stdout.
//! The worker is started with [`WORKER_ARG`] as its only argument; the binary
//! that serves plugins must check for it before anything else (see
//! [`run_worker`]).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use serde::{Deserialize, Serialize};

use crate::manifest::{Limits, Permissions};
use crate::runtime::{PluginRequest, PluginResponse};
use crate::script::{self, ScriptSource};

/// The argument that makes a binary act as a script worker.
pub const WORKER_ARG: &str = "__script-worker";

/// Extra time the parent waits past the call's own deadline, so the child can
/// report its own deadline error first.
const GRACE: Duration = Duration::from_millis(2_000);

/// Memory the worker needs beyond the script's own budget: the Rust runtime,
/// the Boa engine and the request. On Windows a trivial job peaked at about
/// 3.6 MB of private memory (sampled every millisecond, so a lower bound).
/// The base is about four times that, to leave room for the engine to grow
/// during a real call. The script's own budget is `max_memory_mb` on top.
pub const WORKER_BASE_MB: u64 = 16;

static WORKER_EXE: OnceLock<PathBuf> = OnceLock::new();

/// Use `exe` (the binary that serves plugins) to run script calls in worker
/// processes. Set once at startup; later calls are ignored. Without it, scripts
/// run in the server process.
pub fn set_worker_exe(exe: PathBuf) {
    let _ = WORKER_EXE.set(exe);
}

pub(crate) fn worker_exe() -> Option<&'static Path> {
    WORKER_EXE.get().map(PathBuf::as_path)
}

/// Everything one call needs, as sent to the worker.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Job {
    pub source: String,
    pub limits: Limits,
    pub plugin_id: String,
    pub perms: Permissions,
    pub request: PluginRequest,
    pub db_path: PathBuf,
    pub storage_dir: PathBuf,
}

/// What the worker sends back. A script error is `error`, not a process
/// failure, so the parent can report it exactly as the in-process path does.
#[derive(Debug, Serialize, Deserialize)]
struct Outcome {
    response: Option<PluginResponse>,
    error: Option<String>,
}

/// Stack for the thread that runs the script. Boa's evaluator is recursive and
/// overflows the default 1 MB main-thread stack on Windows.
const WORKER_STACK_BYTES: usize = 32 * 1024 * 1024;

/// Worker entry point. Reads one job from stdin, writes one outcome to stdout,
/// and returns the process exit code.
pub fn run_worker() -> i32 {
    std::thread::Builder::new()
        .stack_size(WORKER_STACK_BYTES)
        .spawn(run_job)
        .map_or(2, |handle| handle.join().unwrap_or(2))
}

fn run_job() -> i32 {
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("script worker: cannot read job: {e}");
        return 2;
    }
    let job: Job = match serde_json::from_str(input.trim()) {
        Ok(job) => job,
        Err(e) => {
            eprintln!("script worker: bad job: {e}");
            return 2;
        }
    };
    // The parent validated the source at load; validating again here keeps the
    // worker safe if it is ever run on its own.
    let result = ScriptSource::new(job.source.clone()).and_then(|script| {
        script::call_in_process(
            &script,
            &job.limits,
            &job.request,
            &job.plugin_id,
            &job.perms,
            &job.db_path,
            &job.storage_dir,
        )
    });
    let outcome = match result {
        Ok(response) => Outcome {
            response: Some(response),
            error: None,
        },
        Err(e) => Outcome {
            response: None,
            error: Some(format!("{e:#}")),
        },
    };
    let mut out = std::io::stdout().lock();
    match serde_json::to_string(&outcome) {
        Ok(line) => {
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                return 2;
            }
            0
        }
        Err(_) => 2,
    }
}

/// Run one call in a worker process under the memory and time limits.
pub(crate) fn run_isolated(exe: &Path, job: &Job) -> Result<PluginResponse, anyhow::Error> {
    let memory_bytes = (WORKER_BASE_MB + job.limits.max_memory_mb) * 1024 * 1024;
    let deadline = Instant::now() + Duration::from_millis(job.limits.max_millis) + GRACE;

    let mut command = Command::new(exe);
    command
        .arg(WORKER_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    apply_memory_limit_before_exec(&mut command, memory_bytes);

    let mut child = command.spawn().context("start script worker")?;
    // Windows applies the limit to the running process, before it gets any
    // input, so no allocation can happen first.
    let _memory = attach_memory_limit(&child, memory_bytes)?;

    let line = format!("{}\n", serde_json::to_string(job)?);
    let mut stdin = child.stdin.take().context("worker stdin")?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(line.as_bytes());
        // Dropping stdin tells the worker the job is complete.
    });
    let mut stdout = child.stdout.take().context("worker stdout")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });

    let status = loop {
        if let Some(status) = child.try_wait().context("wait for script worker")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let _ = reader.join();
            return Err(anyhow!("plugin call deadline exceeded"));
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let _ = writer.join();
    let text = reader.join().unwrap_or_default();

    if !status.success() {
        // An allocation past the memory limit aborts the worker; the server is
        // unaffected and only this call fails.
        return Err(anyhow!(
            "script worker stopped without a result ({status}); a script may have exceeded its memory limit"
        ));
    }
    let outcome: Outcome = serde_json::from_str(text.trim())
        .map_err(|e| anyhow!("script worker returned invalid output: {e}"))?;
    match (outcome.response, outcome.error) {
        (Some(response), _) => Ok(response),
        (None, Some(error)) => Err(anyhow!(error)),
        (None, None) => Err(anyhow!("script worker returned no result")),
    }
}

#[cfg(unix)]
fn apply_memory_limit_before_exec(command: &mut Command, bytes: u64) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure runs in the forked child before exec and calls only
    // async-signal-safe `setrlimit`.
    unsafe {
        command.pre_exec(move || {
            let limit = libc::rlimit {
                rlim_cur: bytes as libc::rlim_t,
                rlim_max: bytes as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_DATA, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_memory_limit_before_exec(_command: &mut Command, _bytes: u64) {}

/// Owns the Job Object that limits the worker. Closing it kills the worker if
/// it is still running, so a leftover child cannot outlive the call.
#[cfg(windows)]
struct MemoryGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for MemoryGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn attach_memory_limit(child: &Child, bytes: u64) -> Result<MemoryGuard, anyhow::Error> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    // SAFETY: plain Win32 calls with valid, owned handles and a correctly sized
    // information struct. Failures are checked below.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(anyhow!(
                "create job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        let guard = MemoryGuard(job);

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        info.ProcessMemoryLimit = bytes as usize;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err(anyhow!(
                "set job memory limit: {}",
                std::io::Error::last_os_error()
            ));
        }
        let ok = AssignProcessToJobObject(job, child.as_raw_handle().cast());
        if ok == 0 {
            return Err(anyhow!(
                "assign worker to job: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(guard)
    }
}

#[cfg(not(windows))]
struct MemoryGuard;

#[cfg(not(windows))]
fn attach_memory_limit(_child: &Child, _bytes: u64) -> Result<MemoryGuard, anyhow::Error> {
    Ok(MemoryGuard)
}
