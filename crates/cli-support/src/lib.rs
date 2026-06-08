#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-cli-support` — the [`processkit`]-coupled plumbing the CLI wrappers reuse.
//!
//! `vcs-git` / `vcs-jj` / `vcs-github` all drive a CLI through [`processkit`], so
//! they share three concerns that *touch* [`processkit::Error`]: an argv injection
//! guard, a fetch-retry policy, and a set of [`Error`] classifiers. Extracting them
//! here keeps the std-only `vcs-diff` clean of the `processkit` dependency, and —
//! more to the point — keeps the marker lists and classifier logic from drifting
//! between backends. The wrapper crates re-export these items (so you reach them
//! as `vcs_git::is_merge_conflict`, not via this crate's name) and rarely name
//! `vcs-cli-support` directly.
//!
//! # The surface
//!
//! - **[`reject_flag_like`]** — the injection guard for bare positional argv slots.
//!   A caller value that is empty/whitespace, or starts with `-`, is refused before
//!   spawning (the CLI would parse it as a flag); flag-*value* slots (`-m <msg>`)
//!   are consumed verbatim and skip the check. Wrappers call it with their own
//!   binary name so the surfaced [`Error::Spawn`] names the right `program`.
//! - **[`FETCH_ATTEMPTS`] / [`FETCH_BACKOFF`]** — the shared transient-retry policy
//!   for `fetch` (one try plus two retries, fixed backoff between them).
//! - **[`is_merge_conflict`] / [`is_nothing_to_commit`] / [`is_transient_fetch_error`]**
//!   — classify a returned [`Error`] so callers branch on *intent* ("conflict,
//!   resolve it"; "nothing to commit, no-op"; "transient, retry") instead of
//!   matching on error internals. They inspect captured [`Error::Exit`] output
//!   against fixed marker lists (and treat a [`processkit`] [`Error::Timeout`] as
//!   transient); any unfamiliar `#[non_exhaustive]` variant falls through to "no".
//!
//! # Recipes
//!
//! Classify a failed `fetch` to drive a retry decision — branch on intent, not on
//! the error's internals:
//!
//! ```no_run
//! use vcs_cli_support::{is_transient_fetch_error, FETCH_ATTEMPTS, FETCH_BACKOFF};
//! # fn run() -> Result<(), processkit::Error> { todo!() }
//! # fn demo() -> Result<(), processkit::Error> {
//! for attempt in 1..=FETCH_ATTEMPTS {
//!     match run() {
//!         Ok(()) => break,
//!         Err(e) if is_transient_fetch_error(&e) && attempt < FETCH_ATTEMPTS => {
//!             std::thread::sleep(FETCH_BACKOFF); // DNS/timeout — worth a retry
//!         }
//!         Err(e) => return Err(e),               // anything else: give up
//!     }
//! }
//! # Ok(()) }
//! ```

use std::time::Duration;

use processkit::{Error, Result};

/// Injection guard for bare positional argv slots: a caller-supplied value with a
/// leading `-` would be parsed by the CLI as a *flag* (verified: `git checkout
/// -evil` → "unknown switch"; jj likewise), and an empty (or whitespace-only)
/// value silently changes most commands' meaning. Refuse both before anything
/// spawns, surfacing an [`Error::Spawn`] naming `program`. Flag-VALUE positions
/// (`-m <msg>`, `--branch <b>`) don't need this — the CLI consumes the next
/// token verbatim there.
pub fn reject_flag_like(program: &str, what: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.starts_with('-') {
        return Err(Error::Spawn {
            program: program.to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} would be parsed as a flag (or is empty) — \
                     refusing to pass it as a positional argument"
                ),
            ),
        });
    }
    Ok(())
}

/// Total attempts for a transient-retried `fetch` (1 try + 2 retries).
pub const FETCH_ATTEMPTS: u32 = 3;
/// Fixed backoff between fetch retries.
pub const FETCH_BACKOFF: Duration = Duration::from_millis(500);

/// Lower-case substrings marking a merge that stopped on conflicts.
const CONFLICT_MARKERS: &[&str] = &["conflict (", "automatic merge failed"];
/// Lower-case substrings marking a commit that found nothing to record.
const NOTHING_TO_COMMIT_MARKERS: &[&str] = &["nothing to commit", "nothing added to commit"];
/// Lower-case substrings marking a transient (retryable) network/fetch failure.
const TRANSIENT_FETCH_MARKERS: &[&str] = &[
    "could not resolve host",
    "couldn't resolve host",
    "temporary failure in name resolution",
    "connection timed out",
    "connection refused",
    "operation timed out",
    "timed out",
    "network is unreachable",
    "failed to connect",
    "could not read from remote repository",
    "the remote end hung up",
    "early eof",
    "rpc failed",
];

/// Whether `err` is an [`Error::Exit`] whose captured output contains any marker.
fn exit_output_matches(err: &Error, markers: &[&str]) -> bool {
    let Error::Exit { stdout, stderr, .. } = err else {
        return false;
    };
    let out = stdout.to_ascii_lowercase();
    let errt = stderr.to_ascii_lowercase();
    markers.iter().any(|m| out.contains(m) || errt.contains(m))
}

/// Whether a failed `merge`/`merge_commit` stopped on a merge conflict. (jj
/// surfaces conflicts as state rather than as errors, so this only fires on git
/// output — see `vcs_core::Error::is_merge_conflict`.)
pub fn is_merge_conflict(err: &Error) -> bool {
    exit_output_matches(err, CONFLICT_MARKERS)
}

/// Whether a failed `commit`/`commit_paths` reported nothing to commit (a clean
/// tree), as opposed to a real error.
pub fn is_nothing_to_commit(err: &Error) -> bool {
    exit_output_matches(err, NOTHING_TO_COMMIT_MARKERS)
}

/// Whether a failed `fetch`/`fetch_remote_branch`/`remote_branch_exists` looks
/// transient (DNS, timeout, dropped connection) and is worth retrying.
pub fn is_transient_fetch_error(err: &Error) -> bool {
    // A processkit-level timeout (a `.timeout()`-bounded run that expired) carries
    // no captured output but is inherently transient; treat it as retryable too.
    matches!(err, Error::Timeout { .. }) || exit_output_matches(err, TRANSIENT_FETCH_MARKERS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_leading_dash() {
        assert!(reject_flag_like("git", "branch name", "-evil").is_err());
        assert!(reject_flag_like("git", "branch name", "").is_err());
        // Whitespace-only is as meaning-changing as empty — refuse it too.
        assert!(reject_flag_like("git", "branch name", "  ").is_err());
        assert!(reject_flag_like("git", "branch name", "\t").is_err());
        assert!(reject_flag_like("git", "branch name", "feature").is_ok());
        // The error names the program and surfaces as a spawn-side refusal.
        let err = reject_flag_like("jj", "revset", "--remote").unwrap_err();
        assert!(matches!(err, Error::Spawn { program, .. } if program == "jj"));
    }

    #[test]
    fn classifies_merge_conflict() {
        let on_stdout = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: "CONFLICT (content): Merge conflict in a.rs".into(),
            stderr: String::new(),
        };
        let on_stderr = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: String::new(),
            stderr: "Automatic merge failed; fix conflicts and then commit".into(),
        };
        let unrelated = Error::Exit {
            program: "git".into(),
            code: 128,
            stdout: String::new(),
            stderr: "fatal: not a git repository".into(),
        };
        assert!(is_merge_conflict(&on_stdout));
        assert!(is_merge_conflict(&on_stderr));
        assert!(!is_merge_conflict(&unrelated));
        assert!(!is_nothing_to_commit(&on_stdout));
    }

    #[test]
    fn classifies_nothing_to_commit_and_transient_fetch() {
        let nothing = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: "nothing to commit, working tree clean".into(),
            stderr: String::new(),
        };
        assert!(is_nothing_to_commit(&nothing));

        let dns = Error::Exit {
            program: "git".into(),
            code: 128,
            stdout: String::new(),
            stderr: "fatal: unable to access 'https://x/': Could not resolve host: x".into(),
        };
        assert!(is_transient_fetch_error(&dns));
        assert!(!is_transient_fetch_error(&nothing));

        // A processkit timeout (no captured output) is transient too.
        let timeout = Error::Timeout {
            program: "git".into(),
            timeout: Duration::from_secs(10),
        };
        assert!(is_transient_fetch_error(&timeout));
    }

    // processkit 0.7's `Error` is `#[non_exhaustive]` and grows variants over
    // time (`NotReady`/`Unsupported` now; `Cancelled`/`ResourceLimit` behind
    // features). Unfamiliar variants must fall through every classifier to
    // "no" — a cancelled or unsupported run is neither a conflict, nor a clean
    // tree, nor worth a fetch retry.
    #[test]
    fn unfamiliar_error_variants_are_not_classified() {
        let not_ready = Error::NotReady {
            program: "git".into(),
            timeout: Duration::from_secs(5),
        };
        let unsupported = Error::Unsupported {
            operation: "suspend".into(),
        };
        for err in [&not_ready, &unsupported] {
            assert!(!is_merge_conflict(err));
            assert!(!is_nothing_to_commit(err));
            assert!(!is_transient_fetch_error(err));
        }
    }

    // processkit 0.8's `cancellation` feature makes `Error::Cancelled` reachable
    // (a client-level `default_cancel_on` killing an in-flight run). It must fall
    // through every classifier to "no" — a cancelled fetch was *deliberately*
    // stopped, so replaying it would fight the cancellation. (Behaviour already
    // held via the `#[non_exhaustive]` fall-through above; this pins it as a
    // first-class assertion now that the variant can be constructed.)
    #[cfg(feature = "cancellation")]
    #[test]
    fn cancelled_is_not_transient_or_otherwise_classified() {
        let cancelled = Error::Cancelled {
            program: "git".into(),
        };
        assert!(!is_transient_fetch_error(&cancelled));
        assert!(!is_merge_conflict(&cancelled));
        assert!(!is_nothing_to_commit(&cancelled));
    }
}
