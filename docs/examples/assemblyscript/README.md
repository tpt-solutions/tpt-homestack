# Example plugin in AssemblyScript

A small plugin written in AssemblyScript, a TypeScript-like language that compiles
to WebAssembly. It follows the contract in [PLUGIN_ABI.md](../../PLUGIN_ABI.md), so
it runs in the same host as the Rust SDK plugins.

It answers under `/api/hello_as`:

- `GET /api/hello_as/hello?name=Ada` returns `{"greeting":"Hello, Ada"}`
- `POST /api/hello_as/hello` returns `{"received_bytes":N}`

The test suite (`crates/tpt-home-plugins/tests/assemblyscript.rs`) runs the committed
`plugin.wasm` through the host, so this example is checked on every test run.

## Build

You need Node.js 18 or later.

```sh
npm install        # once: AssemblyScript 0.27 (pinned)
npm run build      # writes plugin.wasm
```

`plugin.wasm` is committed, so you can skip the build to try the plugin.

## Pack and install

From the repository root:

```sh
tpt-home plugin build docs/examples/assemblyscript
tpt-home plugin install hello_as-0.1.0.tptp --enable
```

Nothing is requested in `[permissions]`, so the plugin gets no database, key/value
or file access. It can still log and answer requests.

## Notes for writing your own

- Build with the `stub` runtime and route `abort` to a function that traps
  (`asconfig.json`). The host provides no `abort` import, and the ABI allows
  imports only from `env`.
- `tpt_alloc` is a plain `heap.alloc`. Each request runs in a fresh instance, so a
  bump allocator is enough.
- AssemblyScript has no JSON parser in its standard library. `assembly/index.ts`
  reads the request's few fields by hand. For real work, add a JSON library
  that compiles with the stub runtime, or keep the parsing simple.
