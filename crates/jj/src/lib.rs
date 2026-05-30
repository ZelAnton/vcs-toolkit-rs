//! `vcs-jj` — automate Jujutsu (`jj`) from Rust through CLI process execution.
//!
//! Thin wrappers that shell out to the `jj` binary and capture its output.
//! Commands run inside an OS job (via [`vcs_process`]) so a `jj` subprocess is
//! never orphaned. This is the starting skeleton; add command wrappers (status,
//! log, describe, …) as the toolkit grows.

use std::ffi::OsStr;
use std::io;

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "jj";

/// Run `jj <args>` and return trimmed stdout on success.
///
/// Fails if the process can't be spawned (e.g. `jj` not on `PATH`) or exits
/// with a non-zero status — stderr is surfaced in the error message.
pub fn run<I, S>(args: I) -> io::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    vcs_process::run(BINARY, args)
}

/// Return the installed Jujutsu version (`jj --version`).
pub fn version() -> io::Result<String> {
    run(["--version"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_is_jj() {
        assert_eq!(BINARY, "jj");
    }

    // Requires the `jj` binary on PATH, so it's ignored by default and not
    // exercised in CI. Run locally with `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires the jj binary to be installed"]
    fn version_mentions_jj() {
        let v = version().expect("jj should be installed");
        assert!(v.to_lowercase().contains("jj"), "unexpected output: {v}");
    }
}
