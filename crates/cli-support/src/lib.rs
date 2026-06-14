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
/// Grace period for a timed-out fetch: on the deadline processkit signals the
/// process tree (terminate), waits this long for it to exit cleanly — flush, close
/// the connection, drop any lock — then hard-kills. Only takes effect when a
/// per-client timeout is set (`Git::default_timeout` / `Jj::default_timeout`); a
/// fetch with no deadline is unaffected.
pub const FETCH_TIMEOUT_GRACE: Duration = Duration::from_secs(2);

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
    // A processkit-level timeout (a `.timeout()`-bounded run that expired) is
    // inherently transient; treat it as retryable too, regardless of any partial
    // output it captured before the deadline (as of processkit 0.10 a `Timeout`
    // carries the partial `stdout`/`stderr`, but the retry decision doesn't depend
    // on it). So is an io-level transient from the spawn itself (interrupted /
    // would-block / busy), which processkit classifies via `Error::is_transient()`
    // (it covers `Spawn`/`Io`, not `Exit`, so it composes cleanly with the marker
    // scan below).
    matches!(err, Error::Timeout { .. })
        || err.is_transient()
        || exit_output_matches(err, TRANSIENT_FETCH_MARKERS)
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

        // A processkit timeout is transient too. (As of processkit 0.10 a `Timeout`
        // carries whatever partial `stdout`/`stderr` was captured before the
        // deadline; we still treat it as unconditionally retryable regardless.)
        let timeout = Error::Timeout {
            program: "git".into(),
            timeout: Duration::from_secs(10),
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(is_transient_fetch_error(&timeout));
    }

    // R9: an io-level transient from the spawn (EINTR / EAGAIN / busy) is fetch-
    // retryable too, via processkit's `Error::is_transient()`.
    #[test]
    fn classifies_io_transient_as_fetch_retryable() {
        let interrupted = Error::Spawn {
            program: "git".into(),
            source: std::io::Error::from(std::io::ErrorKind::Interrupted),
        };
        assert!(
            interrupted.is_transient(),
            "processkit treats Interrupted as a transient io error"
        );
        assert!(is_transient_fetch_error(&interrupted));
        // A non-transient io error (e.g. NotFound — the binary is missing) is not retried.
        let missing = Error::Spawn {
            program: "git".into(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert!(!is_transient_fetch_error(&missing));
    }

    // R2: regression for the processkit 0.9.1 untruncated-`Error::Exit` fix. A large
    // output (well past the old 4 KiB cap) with the decisive marker near the END must
    // still classify — proving the classifiers see the whole captured stream.
    #[test]
    fn classifies_on_large_output_past_the_old_4kib_cap() {
        let padding = "noise line that says nothing\n".repeat(500); // ~14 KiB
        let conflict = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: format!("{padding}CONFLICT (content): Merge conflict in late.rs"),
            stderr: String::new(),
        };
        assert!(
            is_merge_conflict(&conflict),
            "a conflict marker past 4 KiB must still classify"
        );

        let transient = Error::Exit {
            program: "git".into(),
            code: 128,
            stdout: String::new(),
            stderr: format!("{padding}fatal: unable to access: Could not resolve host: x"),
        };
        assert!(is_transient_fetch_error(&transient));
    }

    // processkit's `Error` is `#[non_exhaustive]` and grows variants over time
    // (`NotReady`/`Unsupported`/`CassetteMiss`/`NotFound`/`Signalled`/`Cancelled`/
    // `ResourceLimit`). Unfamiliar variants must fall through every classifier to
    // "no" — a not-ready or unsupported run is neither a conflict, nor a clean
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

    // `Error::Cancelled` (a client-level `default_cancel_on` killing an in-flight
    // run; always available since cancellation became core in processkit 0.10) must
    // fall through every classifier to "no" — a cancelled fetch was *deliberately*
    // stopped, so replaying it would fight the cancellation. (Behaviour already held
    // via the `#[non_exhaustive]` fall-through above; this pins it as a first-class
    // assertion.)
    #[test]
    fn cancelled_is_not_transient_or_otherwise_classified() {
        let cancelled = Error::Cancelled {
            program: "git".into(),
        };
        assert!(!is_transient_fetch_error(&cancelled));
        assert!(!is_merge_conflict(&cancelled));
        assert!(!is_nothing_to_commit(&cancelled));
    }

    // `Error::Signalled` (a process killed by a signal — e.g. an external SIGTERM/
    // SIGKILL, surfaced first-class since processkit 0.9.2 and carrying partial
    // `stdout`/`stderr` since 0.10) is *terminal*, not transient: a deliberate kill
    // should not be auto-retried, and a signal death is neither a merge conflict nor
    // a clean tree. processkit's own `is_transient()` agrees (false for `Signalled`),
    // so it falls through every classifier to "no" — pinned here, including the case
    // where the captured stderr happens to contain an otherwise-transient marker (a
    // killed fetch is still not ours to silently replay).
    #[test]
    fn signalled_is_terminal_not_transient() {
        let signalled = Error::Signalled {
            program: "git".into(),
            signal: Some(15),
            stdout: String::new(),
            stderr: "fatal: unable to access: Could not resolve host: x".into(),
        };
        assert!(!signalled.is_transient());
        assert!(!is_transient_fetch_error(&signalled));
        assert!(!is_merge_conflict(&signalled));
        assert!(!is_nothing_to_commit(&signalled));
    }
}
