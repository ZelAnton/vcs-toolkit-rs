#![no_main]
//! Fuzz `vcs_diff::parse_diff`: forge-provided PR diffs are untrusted text, so
//! the parser must never panic on arbitrary UTF-8 input.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = vcs_diff::parse_diff(text);
});