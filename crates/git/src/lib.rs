//! `vcs-git` — automate Git from Rust through CLI process execution.
//!
//! Thin wrappers that shell out to the `git` binary and capture its output.
//! Commands run inside an OS job (via [`vcs_process`]) so a `git` subprocess is
//! never orphaned. This is the starting skeleton; add command wrappers (status,
//! log, commit, …) as the toolkit grows.

use std::ffi::OsStr;
use std::io;

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "git";

/// Run `git <args>` and return trimmed stdout on success.
///
/// Fails if the process can't be spawned (e.g. `git` not on `PATH`) or exits
/// with a non-zero status — stderr is surfaced in the error message.
pub fn run<I, S>(args: I) -> io::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    vcs_process::run(BINARY, args)
}

/// Return the installed Git version (`git --version`).
pub fn version() -> io::Result<String> {
    run(["--version"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_is_git() {
        assert_eq!(BINARY, "git");
    }

    // Requires the `git` binary on PATH, so it's ignored by default and not
    // exercised in CI. Run locally with `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires the git binary to be installed"]
    fn version_mentions_git() {
        let v = version().expect("git should be installed");
        assert!(v.to_lowercase().contains("git"), "unexpected output: {v}");
    }
}
