# Fuzz targets

Coverage-guided fuzzing (libFuzzer through `cargo-fuzz`) for the parsers that read
input from outside the machine:

| target | input |
|---|---|
| `manifest` | `plugin.toml` text (`tpt_home_plugins::manifest::parse`) |
| `package` | a `.tptp` archive (`package::unpack_verify`) |
| `catalog` | a plugin catalog index (`catalog::parse_index`) |
| `update` | a release manifest (`update::parse_manifest`) |
| `webhook` | webhook timestamp, signature, secret and body (`webhook::verify`) |

The property tested is that no input panics. Errors are expected and fine.

## Running

Fuzzing needs a nightly toolchain and `cargo-fuzz`:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run manifest -- -max_total_time=60
```

Run each target for a few minutes before a release. A crash is saved under
`fuzz/artifacts/<target>/`; reproduce it with `cargo +nightly fuzz run <target> <file>`,
fix the parser, and keep the file as a regression test in the crate that owns the parser.

`corpus/` and `artifacts/` are not committed.

## Results

Run on 2026-10-15 on Windows 11 (x86_64, MSVC, nightly 2026-08-05), 60 seconds per
target, about 6.9 million runs in total: no crashes, no panics.

On Windows the AddressSanitizer runtime DLL must be on `PATH` before a target starts,
for example from the Visual Studio install: `.../VC/Tools/MSVC/<version>/bin/Hostx64/x64`.
