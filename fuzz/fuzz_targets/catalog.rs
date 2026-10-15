#![no_main]

// Catalog indexes are fetched from an address the admin set, so their content
// is untrusted. Parsing and checking must not panic on any input.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = tpt_home_plugins::catalog::parse_index(text);
    }
});
