// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Compile-fail tests: each invalid blueprint attribute must stop the build
//! with a message that names the problem. Expected output lives beside each
//! case as `<name>.stderr`; regenerate with `TRYBUILD=overwrite` and review.

#[test]
fn invalid_attributes_are_rejected_with_clear_messages() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
