#![no_main]

// Plugin manifests come from uploaded packages, so any bytes may arrive here.
// The parser must return an error, never panic.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = tpt_home_plugins::manifest::parse(text);
    }
});
