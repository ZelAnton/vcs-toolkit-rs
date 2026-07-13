#![no_main]
//! Fuzz `vcs_git::parse_porcelain_v2`: `git status` output from an untrusted
//! repository must never panic on arbitrary UTF-8 input.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = vcs_git::parse_porcelain_v2(text);
});