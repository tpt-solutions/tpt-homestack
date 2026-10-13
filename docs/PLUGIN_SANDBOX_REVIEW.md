# Plugin sandbox review

Status: **code review and targeted tests, 2026-10-13.** Not an independent review.

Plugins come in three tiers. Each tier is reviewed against what it promises.

| Tier | Where it runs | Isolation |
|---|---|---|
| WASM (`wasm`) | In-process, wasmi | Memory cap, fuel, wall-clock, trap isolation |
| JavaScript (`js`) | A worker process per call (CLI build), under an OS memory limit | Fuel, recursion, wall-clock, grants, memory cap, process isolation |
| External (`external`) | Separate process run by the user | The process boundary; host services over HTTP with scoped tokens |

Installing a plugin is an admin action, and the admin approves the permissions
it asks for. The review asks what a plugin can do once installed, including a
plugin that is buggy or hostile.

## Controls and the tests that cover them

| Control | Tier | Test |
|---|---|---|
| Instruction budget (`max_fuel`) | WASM | `sandbox.rs::infinite_loop_exhausts_fuel` |
| Memory cap (`max_memory_mb`) | WASM | `sandbox.rs::memory_growth_past_the_cap_traps` |
| Memory cap (`max_memory_mb` plus a 16 MB base) | JS, worker | `tpt-home-cli/tests/e2e_server.rs::a_script_that_exhausts_memory_fails_alone` |
| Process isolation: a script that aborts fails one call | JS, worker | `e2e_server.rs::a_script_that_exhausts_memory_fails_alone` |
| Wall-clock budget (`max_millis`) at each host call | WASM, JS | `sandbox.rs::host_call_loop_hits_the_wall_clock_deadline`, `script_sandbox.rs::host_call_loop_hits_the_deadline` |
| Trap isolation (a crash fails one call) | WASM | `sandbox.rs::a_trapping_guest_fails_its_own_call_and_nothing_else` |
| Loop budget | JS | `script_sandbox.rs::infinite_loop_hits_the_loop_limit` |
| Recursion cap (256) | JS | `script_sandbox.rs::deep_recursion_is_stopped` |
| No `eval`, `Function` or constructor escapes | JS | `script_sandbox.rs::eval_is_removed`, `function_constructor_escapes_are_closed` |
| No modules, timers or host globals | JS | `script_sandbox.rs::module_syntax_is_rejected_at_load`, `no_timers_modules_or_host_leaks` |
| Source size cap (1 MiB) | JS | `script_sandbox.rs::oversize_source_is_rejected_at_load` |
| Ungranted capabilities return null or false | JS, WASM | `script_sandbox.rs::ungranted_kv_read_is_null`, `ungranted_write_reports_false`; `sandbox.rs::ungranted_host_write_returns_minus_one_without_trapping` |
| SQL limited to `<id>_*` tables, one statement | Both | `script_sandbox.rs::out_of_scope_sql_is_null_even_when_granted`; `tests/boundary.rs::random_sql_is_accepted_only_as_a_scoped_select` |
| HTTP only to declared hosts, no redirects, no header injection | JS | `script_http.rs::undeclared_hosts_are_refused_without_a_connection`, `redirects_are_not_followed`, `control_characters_in_urls_are_refused` |
| Response size caps (4 MiB WASM, 1 MiB HTTP body) | WASM, JS | `sandbox.rs::response_length_over_cap_is_an_error`; HTTP cap in `script_http.rs` (per the todo) |
| Package unpack refuses traversal and foreign files | All | `package.rs::traversal_and_foreign_files_rejected`; `tests/boundary.rs::damaged_packages_are_refused_without_panicking_or_escaping` |

## JavaScript memory: from gap to contained

**The gap.** Boa has no heap limit, and JavaScript plugins ran inside the server
process. A probe during the earlier review showed the problem:

```js
function handle() {
  let s = 'x';
  for (let i = 0; i < 28; i++) { s = s + s; }   // 28 loop steps
  return { status: 200, body: String(s.length) };
}
```

It returned `268435456` (256 MB) with the default fuel. The loop budget was not
reached. Thirty-two doublings would need 4 GB and could take the whole server
down.

**The fix.** With a worker configured (the CLI always sets one), each JavaScript
call runs in a short-lived child process of the same binary (`script_worker.rs`):

- The child gets an operating-system memory limit. On Windows, a Job Object with
  a process memory limit and kill-on-close. On Linux, `RLIMIT_DATA` set before
  exec. The limit is `WORKER_BASE_MB` (16) plus the manifest's `max_memory_mb`.
- The parent kills the child at the call deadline plus 2 s. A crash, abort or
  memory stop is an error for that one call. The server keeps running.
- The child opens the same database and storage files as the host services do,
  so host calls need no extra channel. Permissions are checked in the child with
  the same rules as before.
- The protocol is one JSON line in and one JSON line out.

**Evidence (Windows).** `a_script_that_exhausts_memory_fails_alone` installs a
script that doubles a string 40 times. Its call returns 500, the message names
the memory limit, and afterwards `/healthz`, notes, and a normal script call
(200) still work. A small script with the same plugin returns 200 first, so
the limit is not simply refusing everything.

**Measurements (Windows, release build).**

- A trivial job peaked at about 3.6 MB of private memory, sampled every
  millisecond, so that is a lower bound. The 16 MB base leaves room for the
  engine to grow during a real call.
- A worker call costs about 16 ms at the median (p90 22 ms), mostly process
  creation. Plugin calls that used to cost well under a millisecond now pay
  that.

**Not yet verified or limited:**

- **Linux.** The `RLIMIT_DATA` code type-checks against the Linux target (in a
  scratch crate, because SQLite's C build needs a Linux compiler this machine
  lacks). It has not run. Confirm in CI before relying on it. `RLIMIT_DATA`
  counts private writable mappings, including thread stacks, so the 32 MB
  worker stack uses part of the budget.
- **Library use.** Without `set_worker_exe`, scripts run in the calling process
  with no memory cap. The CLI sets the worker; any other embedder must do the
  same.
- **Worker stderr** is discarded. The error text returned to the caller is the
  only diagnostic.
- **Stack.** The worker runs the script on a 32 MB thread because Boa's evaluator
  is recursive and overflows Windows' 1 MB main-thread stack.
- **Unexplained flake.** Once during this work, a plugin install request was
  reset by the server during a test run (`10054`). It did not recur in five
  later runs of the same file. The cause is not known.

## Other observations

- **Wall-clock limits on WASM are checked at host calls.** A pure WASM loop is
  stopped by fuel only. `docs/PLUGINS.md` already says so.
- **Panics in the JS tier.** A panic in the worker is a crash of that process,
  so it fails one call. In-process use (library embedders) relies on
  `spawn_blocking`, which catches panics in its task.
- **External plugins** run as their own process, so their memory is outside
  the server's. Their trust depends on the process the user runs and on the
  scoped token they hold.
- **Plugin jobs and hooks** use the same call path, so they are isolated in the
  same way.

## Re-running the checks

```sh
cargo test -p tpt-home-plugins --features js
cargo test -p tpt-home-cli --test e2e_server
```
