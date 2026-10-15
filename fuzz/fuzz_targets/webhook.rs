#![no_main]

// Webhook headers and bodies come from anyone on the network. Verification must
// refuse every malformed header without panicking.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Split the input into four fields: timestamp, signature, secret and body.
    let mut parts = data.splitn(4, |&b| b == b'\n');
    let timestamp = String::from_utf8_lossy(parts.next().unwrap_or_default());
    let signature = String::from_utf8_lossy(parts.next().unwrap_or_default());
    let secret = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();
    let _ = tpt_home_core::webhook::verify(secret, &timestamp, &signature, body, 1_800_000_000, 300);
});
