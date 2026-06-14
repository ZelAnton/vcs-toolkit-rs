//! The facade's error type: a thin wrapper that adds repo-detection failures on
//! top of the underlying [`processkit::Error`] the per-tool clients return.
//!
//! The [`Error::Vcs`] variant carries a [`processkit::Error`] verbatim — re-exported
//! at the crate root (`vcs_core::processkit`) so you can match it without a direct
//! `processkit` dependency. Prefer the `is_*` classifiers ([`is_merge_conflict`](Error::is_merge_conflict)
//! / [`is_nothing_to_commit`](Error::is_nothing_to_commit) /
//! [`is_transient_fetch_error`](Error::is_transient_fetch_error) /
//! [`is_transient`](Error::is_transient) / [`is_not_found`](Error::is_not_found))
//! to branch on intent rather than matching the wrapped error's internals.

use std::path::PathBuf;

/// An error from a [`Repo`](crate::Repo) operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// [`Repo::open`](crate::Repo::open) found no `.git`/`.jj` from the start dir
    /// up to the filesystem root.
    NotARepository(PathBuf),
    /// A worktree/workspace lookup by path matched no attached worktree.
    WorktreeNotFound(PathBuf),
    /// A filesystem operation failed (e.g. removing a workspace directory).
    Io(std::io::Error),
    /// An underlying `vcs-git` / `vcs-jj` (i.e. `processkit`) error.
    Vcs(processkit::Error),
}

impl Error {
    /// Whether this wraps a merge/rebase **conflict** from the backend — so a
    /// caller can branch on "conflict, resolve it" vs. a hard failure without
    /// matching on [`processkit::Error`] internals. (Recognises git's conflict
    /// markers; jj surfaces conflicts as state, not errors — see
    /// [`Repo::in_progress_state`](crate::Repo::in_progress_state).)
    ///
    /// Named to match the wrapper classifiers
    /// ([`vcs_cli_support::is_merge_conflict`]) — one name per concept across the
    /// workspace.
    pub fn is_merge_conflict(&self) -> bool {
        matches!(self, Error::Vcs(e) if vcs_cli_support::is_merge_conflict(e))
    }

    /// Whether this is a benign "nothing to commit" — an empty commit attempt the
    /// caller likely wants to treat as a no-op.
    pub fn is_nothing_to_commit(&self) -> bool {
        matches!(self, Error::Vcs(e) if vcs_cli_support::is_nothing_to_commit(e))
    }

    /// Whether this is a **transient** fetch/network failure worth retrying
    /// (DNS, connection reset, timeout). The underlying clients already retry
    /// their own fetches; this is for retrying higher-level flows.
    pub fn is_transient_fetch_error(&self) -> bool {
        matches!(self, Error::Vcs(e) if vcs_cli_support::is_transient_fetch_error(e))
    }

    /// Whether the underlying error is a **transient io/spawn** failure
    /// (interrupted / would-block / resource-busy) — delegates to
    /// [`processkit::Error::is_transient`]. Narrower than
    /// [`is_transient_fetch_error`](Error::is_transient_fetch_error) (which also
    /// treats a timeout and the network markers as retryable); use this to retry
    /// *any* operation past a momentary io hiccup. The facade's own
    /// [`Io`](Error::Io)/[`NotARepository`](Error::NotARepository)/
    /// [`WorktreeNotFound`](Error::WorktreeNotFound) variants are never transient.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Vcs(e) if e.is_transient())
    }

    /// Whether the underlying CLI binary (`git`/`jj`) **wasn't found** — a setup
    /// problem (the tool isn't installed or isn't on `PATH`), not a repository or
    /// usage error. Delegates to [`processkit::Error::is_not_found`]; lets a caller
    /// surface a "please install git/jj" hint instead of a raw spawn failure.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Vcs(e) if e.is_not_found())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotARepository(p) => {
                write!(
                    f,
                    "no git or jj repository found at or above {}",
                    p.display()
                )
            }
            Error::WorktreeNotFound(p) => {
                write!(f, "no worktree found at {}", p.display())
            }
            Error::Io(e) => write!(f, "{e}"),
            Error::Vcs(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Vcs(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<processkit::Error> for Error {
    fn from(e: processkit::Error) -> Self {
        Error::Vcs(e)
    }
}

/// `Result` specialised to the facade [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transient_delegates_to_processkit_and_excludes_facade_variants() {
        // An interrupted spawn is a transient io failure.
        let interrupted = Error::Vcs(processkit::Error::Spawn {
            program: "git".into(),
            source: std::io::Error::from(std::io::ErrorKind::Interrupted),
        });
        assert!(interrupted.is_transient());
        // A missing binary is NOT transient (retrying won't install it).
        let missing = Error::Vcs(processkit::Error::Spawn {
            program: "git".into(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        assert!(!missing.is_transient());
        // The facade's own io/detection variants are never transient.
        assert!(!Error::Io(std::io::Error::from(std::io::ErrorKind::Interrupted)).is_transient());
        assert!(!Error::NotARepository("/x".into()).is_transient());
    }

    #[test]
    fn is_not_found_only_for_a_missing_binary() {
        let not_found = Error::Vcs(processkit::Error::NotFound {
            program: "jj".into(),
            searched: None,
        });
        assert!(not_found.is_not_found());
        // An ordinary non-zero exit is not a "binary not found".
        let exit = Error::Vcs(processkit::Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: String::new(),
            stderr: "fatal: not a git repository".into(),
        });
        assert!(!exit.is_not_found());
        assert!(!Error::NotARepository("/x".into()).is_not_found());
    }
}
