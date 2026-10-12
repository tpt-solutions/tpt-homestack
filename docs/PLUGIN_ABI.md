# Plugin WASM ABI (version 1)

This is the contract between a WebAssembly plugin and the tpt-homestack host.
Any language that can produce a wasm module with the exports and imports
below can write a plugin. Rust authors should use `tpt-home-plugin-sdk`, which
wraps this ABI. A minimal module in WAT is in
[`examples/minimal.wat`](examples/minimal.wat), and the test suite compiles
that exact file, so it stays correct.

The ABI version is `api_version = 1` in `plugin.toml`. The host rejects any
other value.

## Module requirements

- The module imports **only** from the `env` module. Anything else, including
  WASI, fails to instantiate.
- The module exports `memory`, `tpt_alloc` and `tpt_entry`.
- No `start` function that depends on host state. The host instantiates the
  module per call, so globals reset between requests.

## Exports

| export | signature | meaning |
|---|---|---|
| `memory` | linear memory | The host reads and writes request and result buffers here. |
| `tpt_alloc` | `(len: i32) -> i32` | Returns a pointer to `len` bytes of guest memory the host may write into. |
| `tpt_entry` | `(ptr: i32, len: i32) -> i64` | Handles one request. `ptr`/`len` locate the request JSON. |

**`tpt_alloc` must be re-entrant.** The host calls it from inside a host
import, while `tpt_entry` is still running, to place a database row set, a
stored value or a file in guest memory. Allocate from a heap that is safe to
extend mid-request. A bump allocator (as in the example) is correct because
each request gets a fresh instance.

### Packed results

`tpt_entry` returns a packed `i64`:

- `-1` (any negative value) means the plugin failed. The host returns a 500.
- Otherwise `(ret_ptr << 32) | ret_len` names a UTF-8 JSON response at
  `ret_ptr` of `ret_len` bytes, in guest memory.

Imports that return data use the same packing. The host allocates the buffer
with the guest's `tpt_alloc`, so the guest keeps ownership of its heap.

## Request JSON

The host passes this object to `tpt_entry`:

```json
{ "method": "GET", "path": "/events", "query": "from=2026-01-01", "body": null }
```

- `path` is relative to `/api/<id>`.
- `query` is the raw query string without the `?`. It is empty when absent.
- `body` is the request body as a string, or `null`. It is not decoded.

## Response JSON

```json
{ "status": 200, "content_type": "application/json", "body": "[]" }
```

- `status` is a `u16` HTTP status. An invalid status becomes 500.
- `content_type` defaults to `text/plain`.
- `body` defaults to the empty string.

A response that is not valid JSON fails the call.

## Host imports (module `env`)

Every capability-gated import returns `-1` when the capability is not granted
or the operation fails (missing file, out-of-scope SQL, oversize value). The
host logs the reason at debug level. Only two things trap the guest: the
wall-clock deadline (`max_millis`) and fuel exhaustion (`max_fuel`).

| import | capability | returns |
|---|---|---|
| `tpt_log(ptr: i32, len: i32)` | always | nothing. At most 100 lines per call are kept. |
| `tpt_db_query(ptr: i32, len: i32) -> i64` | `db` | packed JSON array of row objects, or `-1`. At most 1000 rows. |
| `tpt_db_execute(ptr: i32, len: i32) -> i64` | `db` | packed JSON `{"rows_affected": n}`, or `-1`. |
| `tpt_kv_get(ptr: i32, len: i32) -> i64` | `kv` | packed JSON string, or packed JSON `null` if the key is absent. `-1` when denied. |
| `tpt_kv_set(kp: i32, kl: i32, vp: i32, vl: i32) -> i32` | `kv` | `0` on success, `-1` otherwise. |
| `tpt_kv_del(ptr: i32, len: i32) -> i32` | `kv` | `0` on success, `-1` otherwise. |
| `tpt_storage_read(ptr: i32, len: i32) -> i64` | `storage` | packed file bytes, or `-1` when denied or the file is absent. |
| `tpt_storage_write(np: i32, nl: i32, dp: i32, dl: i32) -> i32` | `storage` | `0` on success, `-1` otherwise. |

Strings passed to imports are UTF-8 and read from guest memory at `(ptr, len)`.
Lengths above 4 MiB are refused.

Table rows that a `db` query returns are JSON objects keyed by column name.
Blob values are shown as the string `<blob N bytes>`.

## Limits

Each invocation runs under limits from the manifest's `[limits]` section:

| key | default | range | enforced by |
|---|---|---|---|
| `max_fuel` | 50,000,000 | at least 1,000 | wasm instruction fuel; exhaustion traps |
| `max_memory_mb` | 16 | 1–256 | linear memory growth; growing past the cap traps |
| `max_millis` | 5,000 | 1–60,000 | wall clock, checked at every host call |

Fuel bounds guest computation. `max_millis` bounds time spent across host
calls. A guest that loops without calling the host is bounded by fuel alone,
because wasmi has no preemptive interrupt.

## Traps and failures

These end the call with an error, which the host reports as a 500:

- fuel exhausted, or the deadline passed (the deadline is raised from a host
  call, so a pure guest loop is stopped by fuel instead)
- memory access out of bounds, or memory growth past the cap
- `tpt_alloc` missing, or with the wrong signature
- `tpt_entry` returns a negative value, or a response outside memory or over
  4 MiB
- the response is not valid JSON

A missing `tpt_entry` or `memory` export also fails the call.

## Not part of v1

- WASI, sockets, clocks or randomness beyond what the host provides. The
  module cannot read the time directly.
- Multiple entry points. Everything goes through `tpt_entry`.
- Typed host structures. Imports exchange JSON strings and packed pointers.

## Writing a module by hand

[`examples/minimal.wat`](examples/minimal.wat) is the smallest module that
meets the contract. Build it with `wat2wasm` or any WAT-aware tool, then
package it with a `plugin.toml` (see [`PLUGINS.md`](PLUGINS.md)).
