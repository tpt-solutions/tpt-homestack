#![no_main]

// The release manifest is fetched from the update address, so its content is
// untrusted. Parsing and checking must not panic on any input.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = tpt_home_core::update::parse_manifest(text);
        let _ = tpt_home_core::update::is_newer(text, "0.1.0");
    }
});
