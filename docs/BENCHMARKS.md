# Benchmarks

Measured against the targets in `todo.md` (Phase 8). Re-run with:

```sh
cargo build --release -p tpt-home-cli
pwsh scripts/bench.ps1
```

## Results

Measured 2026-10-13 on Windows 11 Pro (x86_64), release build with fat LTO,
empty server, no plugins, default settings.

| Target | Limit | Measured | Result |
|---|---|---|---|
| Binary size | < 25 MB | 22.3 MB (`tpt-home.exe`); 22.5 MB after the JS worker change | pass |
| Startup to first healthy response | < 0.5 s | 247 ms | pass |
| Idle RAM (working set) | < 30 MB | 11.3 MB | pass |
| Idle CPU | ~0% | 0% over 10 s | pass |

## How they are measured

- **Binary size:** file length of the release binary.
- **Startup:** from starting the process to the first `200` from
  `/healthz`. The probe runs in PowerShell, so the figure includes a few
  milliseconds of probe overhead.
- **Idle RAM:** working set after 2 s of settling, then the value at the end
  of the idle window. Peak working set is reported too.
- **Idle CPU:** processor time used during a 10 s window with no requests,
  divided by the window length.

## Load and soak

Run with `scripts/load.ps1` against the same release build. Each request is an
authenticated `GET /api/notes` over a keep-alive connection, after 50 notes are
created.

| Run | Requests | Concurrency | Statuses | Req/s | p50 | p95 | p99 | RAM before → after |
|---|---|---|---|---|---|---|---|---|
| Plain, one client address | 2,000 | 16 | 200 ×189, 429 ×1,811 | 190 | 5.6 ms | 9 ms | 12 ms | 12.4 → 13.2 MB |
| Rotated addresses, short | 3,000 | 16 | 200 ×3,000 | 392 | 6.2 ms | 9.6 ms | 11.3 ms | 12.4 → 13.9 MB |
| Rotated addresses, soak (120 s) | 22,000 | 16 | 200 ×22,000 | 169 | 8.9 ms | 14.1 ms | 17.4 ms | 12.4 → 14.1 MB |

- The plain run is limited by the server's rate limit (240 requests a minute
  per client address). 189 reads plus the 50 writes match that budget. This is
  the designed behaviour, not a fault.
- The rotated runs send a different forwarded address per request with
  `trust_proxy` on, so they measure the server rather than the limiter.
- Throughput fell from about 390 to 170 requests a second between the short run
  and the soak. The cause is not isolated. Candidates are the client (the
  script starts a new parallel batch every 2,000 requests) and the machine
  itself. The server's memory did not grow beyond 1.7 MB over 22,000 requests,
  and no request failed.
- The client is PowerShell, so its own overhead limits throughput. These numbers
  are a floor for the server, not a measure of its maximum.

### Thirty-minute soak

Release build with rotated addresses, 16 workers, memory sampled every 60 s
(`-SampleFile`). Run on the build that has the rate-limiter pruning fix.

| Measure | Result |
|---|---|
| Requests | 2,925,982, all 200 |
| Throughput | 1,626 req/s average |
| Latency p50 / p95 / p99 | 9.2 / 17.7 / 23.1 ms |
| Latency max | 8,397 ms (one event; see below) |
| Server working set | 15.4 MB at start, 24.8 MB by 2 min, 25.2–26.1 MB for the rest of the run |
| Client private memory | 74 MB at start, 122 MB at end (the client keeps one latency per request, about 8 bytes each) |

The server's memory rose for the first two minutes and then stayed level. That
rise is the rate limiter's table of tracked addresses filling up with 62,000
distinct addresses. It does not keep growing under a steady mix of addresses.

**Stalls.** Requests over one second are rare: about one or two in ten minutes
with PowerShell. In a PowerShell run the slow requests start together across
all 16 workers, which points at a client pause. A Node client
(a throwaway experiment, not in the repo: 6 minutes, 1.36 million requests)
saw one 5.1 s stall on a single connection. Neither run shows a server stall that can be
confirmed. The cause on this machine is not known.

## Not yet measured

- **ARM64.** The target was written for ARM64 hardware (Raspberry Pi class).
  This machine is x86_64. The figures above do not show the ARM64 targets
  are met.
- **Soaks of hours.** The longest run is thirty minutes.
- **Writes under load.** The load is reads. Concurrent writes to notes, track
  and plugin tables were not measured.
- **Allocator.** The Phase 8 allocator choice has not been evaluated.

## Allocator: system heap vs mimalloc (2026-10-15)

Same machine, same release settings (fat LTO), same command:
`scripts/load.ps1 -Requests 20000 -Concurrency 32 -RotateClients`. Only the global
allocator differs: `mimalloc` (MIT) in `tpt-home-cli`.

| Measure | System heap | mimalloc |
|---|---|---|
| Requests per second | 704 | 946 |
| p50 | 41 ms | 32 ms |
| p95 | 64 ms | 53 ms |
| p99 | 98 ms | 67 ms |
| Slowest request | 3,322 ms | 1,293 ms |
| Working set after the run | 20.9 MB | 29.4 MB |
| Idle working set (bench) | 12.1 MB | 13.1 MB |
| Binary | 24.99 MB | 24.98 MB |

Throughput and latency improved. The price is memory: mimalloc holds more after
a burst (about 8 MB more here) and about 1 MB more at idle. Both figures stay under
the idle target of 30 MB. The "RAM before" figure in the load run read 33 MB with
mimalloc against 13 MB before; that number is not explained yet, so treat it with
care. Startup measured 158 ms on a warm run; an earlier cold run read 751 ms.

The binary is 24.98 MB against the 25 MB limit, so there is almost no room left.
