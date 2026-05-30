//! `vcs-github` — automate GitHub from Rust through the `gh` CLI.
//!
//! Thin wrappers that shell out to the GitHub CLI (`gh`) and capture its output.
//! Commands run inside an OS job (via [`vcs_process`]) so a `gh` subprocess is
//! never orphaned. This is the starting skeleton; add command wrappers (pr,
//! issue, repo, api, …) as the toolkit grows.

use std::ffi::OsStr;
use std::io;

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "gh";

/// Run `gh <args>` and return trimmed stdout on success.
///
/// Fails if the process can't be spawned (e.g. `gh` not on `PATH`) or exits
/// with a non-zero status — stderr is surfaced in the error message.
pub fn run<I, S>(args: I) -> io::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    vcs_process::run(BINARY, args)
}

/// Return the installed GitHub CLI version (`gh --version`).
pub fn version() -> io::Result<String> {
    run(["--version"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_name_is_gh() {
        assert_eq!(BINARY, "gh");
    }

    // Requires the `gh` binary on PATH, so it's ignored by default and not
    // exercised in CI. Run locally with `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires the gh binary to be installed"]
    fn version_mentions_gh() {
        let v = version().expect("gh should be installed");
        assert!(v.to_lowercase().contains("gh"), "unexpected output: {v}");
    }
}
