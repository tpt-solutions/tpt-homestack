#![no_main]

// `.tptp` packages are uploaded archives: a corrupt or hostile archive must be
// refused with an error. It must never panic, and it must never write outside
// the destination directory.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(dir) = tempfile::tempdir() else {
        return;
    };
    let _ = tpt_home_plugins::package::unpack_verify(data, dir.path());
});
