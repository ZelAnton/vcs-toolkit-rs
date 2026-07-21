//! The facade's error type: a thin wrapper that adds repo-detection failures on
//! top of the underlying [`processkit::Error`] the per-tool clients return.
//!
//! The [`ErrorKind::Vcs`] variant carries a [`processkit::Error`] verbatim — re-exported
//! at the crate root (`vcs_core::processkit`) so you can match it without a direct
//! `processkit` dependency. Prefer the `is_*` classifiers ([`is_merge_conflict`](Error::is_merge_conflict)
//! / [`is_nothing_to_commit`](Error::is_nothing_to_commit) /
//! [`is_transient_fetch_error`](Error::is_transient_fetch_error) /
//! [`is_transient`](Error::is_transient) / [`is_not_found`](Error::is_not_found))
//! to branch on intent rather than matching the wrapped error's internals.

use std::fmt::Formatter;
use std::path::PathBuf;

/// An error from a [`Repo`](crate::Repo) operation.
#[derive(Debug)]
pub struct Error {
    kind: Box<ErrorKind>,
}
impl Error {
    /// Construct an [`Error`] from an [`ErrorKind`],
    /// without providing any additional information.
    #[cold]
    pub fn from_kind(kind: ErrorKind) -> Self {
        Error {
            kind: Box::new(kind),
        }
    }

    /// The kind of error that occurred.
    #[inline]
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Convert this error into an owned [`ErrorKind`].
    #[inline]
    pub fn into_kind(self) -> ErrorKind {
        *self.kind
    }

    /// Whether this wraps a merge/rebase **conflict** from the backend. Forwards to
    /// [`ErrorKind::is_merge_conflict`].
    pub fn is_merge_conflict(&self) -> bool {
        self.kind().is_merge_conflict()
    }

    /// Whether this is a benign "nothing to commit". Forwards to
    /// [`ErrorKind::is_nothing_to_commit`].
    pub fn is_nothing_to_commit(&self) -> bool {
        self.kind().is_nothing_to_commit()
    }

    /// Whether this is a **transient** fetch/network failure worth retrying.
    /// Forwards to [`ErrorKind::is_transient_fetch_error`].
    pub fn is_transient_fetch_error(&self) -> bool {
        self.kind().is_transient_fetch_error()
    }

    /// Whether the underlying error is a **transient io/spawn** failure. Forwards
    /// to [`ErrorKind::is_transient`].
    pub fn is_transient(&self) -> bool {
        self.kind().is_transient()
    }

    /// Whether the underlying CLI binary (`git`/`jj`) **wasn't found**. Forwards
    /// to [`ErrorKind::is_not_found`].
    pub fn is_not_found(&self) -> bool {
        self.kind().is_not_found()
    }

    /// Whether this is an **input rejection**. Forwards to
    /// [`ErrorKind::is_invalid_input`].
    pub fn is_invalid_input(&self) -> bool {
        self.kind().is_invalid_input()
    }

    /// Whether a **resource the operation named doesn't exist**. Forwards to
    /// [`ErrorKind::is_resource_not_found`].
    pub fn is_resource_not_found(&self) -> bool {
        self.kind().is_resource_not_found()
    }

    /// Whether this is an [`Unsupported`](ErrorKind::Unsupported) action. Forwards
    /// to [`ErrorKind::is_unsupported`].
    pub fn is_unsupported(&self) -> bool {
        self.kind().is_unsupported()
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.kind(), f)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.kind().source()
    }
}
impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self::from_kind(kind)
    }
}

/// The type of [`Error`] that has occurred.
#[derive(Debug)]
#[non_exhaustive]
pub enum ErrorKind {
    /// [`Repo::discover`](crate::Repo::discover) found no `.git`/`.jj` from the
    /// start dir up to the filesystem root, or [`Repo::open`](crate::Repo::open)
    /// found no `.git`/`.jj` marker in the exact directory it was given.
    NotARepository(PathBuf),
    /// [`Repo::discover`](crate::Repo::discover) walked up to a **bare** git
    /// repository (created with `git init --bare`, or an equivalent bare clone)
    /// — a directory holding `HEAD`/`config`/`objects`/`refs` directly, with no
    /// `.git` subdirectory and no worktree. This is distinct from
    /// [`NotARepository`](ErrorKind::NotARepository): a bare repository *is* a
    /// valid git repository, just one this facade doesn't drive (it has no
    /// working tree for the CLI wrappers to operate against). See
    /// <https://github.com/ZelAnton/vcs-toolkit-rs/issues/6>.
    BareRepository(PathBuf),
    /// A worktree/workspace lookup by path matched no attached worktree.
    WorktreeNotFound(PathBuf),
    /// A filesystem operation failed (e.g. removing a workspace directory).
    Io(std::io::Error),
    /// An underlying `vcs-git` / `vcs-jj` (i.e. `processkit`) error.
    Vcs(processkit::Error),
    /// A concurrency-safe op-log rollback could not restore the repository to its
    /// captured pre-operation state: the `op restore` failed, or a **concurrent** jj
    /// process advanced the operation log so reverting would have clobbered its work
    /// (see [`vcs_jj::Rollback`]). Raised by
    /// [`Repo::try_merge`](crate::Repo::try_merge) on the jj backend when its
    /// trial-merge rollback cannot complete cleanly — the trial merge may remain
    /// materialized, so the probe result would be untrustworthy. The structured
    /// [`vcs_jj::Rollback`] carries which case it was (and, for a failed restore, the
    /// underlying cause).
    Rollback(vcs_jj::Rollback),
    /// The requested action has no meaningful mapping for the repository's current
    /// in-progress state, so it is refused **explicitly** rather than performed as a
    /// misleading success. Currently raised by
    /// [`Repo::continue_in_progress`](crate::Repo::continue_in_progress) during a
    /// `git bisect`: a bisect advances by marking commits good/bad, not by a
    /// `--continue` step, so "continue" cannot be honoured. Carries a short message
    /// naming the situation. Classified by
    /// [`is_unsupported`](Error::is_unsupported); a language binding maps it to an
    /// `unsupported`/`ValueError`-style error.
    Unsupported(String),
}

impl ErrorKind {
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
        matches!(self, ErrorKind::Vcs(e) if vcs_cli_support::is_merge_conflict(e))
    }

    /// Whether this is a benign "nothing to commit" — an empty commit attempt the
    /// caller likely wants to treat as a no-op.
    pub fn is_nothing_to_commit(&self) -> bool {
        matches!(self, ErrorKind::Vcs(e) if vcs_cli_support::is_nothing_to_commit(e))
    }

    /// Whether this is a **transient** fetch/network failure worth retrying — DNS, a
    /// dropped connection, a fast blip. A **timeout is not** transient (it already
    /// spent the full deadline; retrying would multiply the wall-clock — see
    /// [`vcs_cli_support::is_transient_fetch_error`]). The underlying clients already
    /// retry their own fetches; this is for retrying higher-level flows.
    pub fn is_transient_fetch_error(&self) -> bool {
        matches!(self, ErrorKind::Vcs(e) if vcs_cli_support::is_transient_fetch_error(e))
    }

    /// Whether the underlying error is a **transient io/spawn** failure
    /// (interrupted / would-block / resource-busy) — delegates to
    /// [`processkit::Error::is_transient`]. Narrower than
    /// [`is_transient_fetch_error`](Error::is_transient_fetch_error) (which also
    /// treats the network markers as retryable — but not a timeout); use this to retry
    /// *any* operation past a momentary io hiccup. The facade's own
    /// [`Io`](ErrorKind::Io)/[`NotARepository`](ErrorKind::NotARepository)/
    /// [`BareRepository`](ErrorKind::BareRepository)/
    /// [`WorktreeNotFound`](ErrorKind::WorktreeNotFound) variants are never transient.
    pub fn is_transient(&self) -> bool {
        matches!(self, ErrorKind::Vcs(e) if e.is_transient())
    }

    /// Whether the underlying CLI binary (`git`/`jj`) **wasn't found** — a setup
    /// problem (the tool isn't installed or isn't on `PATH`), not a repository or
    /// usage error. Delegates to [`processkit::Error::is_not_found`]; lets a caller
    /// surface a "please install git/jj" hint instead of a raw spawn failure.
    pub fn is_not_found(&self) -> bool {
        matches!(self, ErrorKind::Vcs(e) if e.is_not_found())
    }

    /// Whether this is an **input rejection** — a value the facade refused *before*
    /// spawning, because it was a bad argument: a flag-like/empty/NUL-containing
    /// value in a guarded positional slot (via the wrapper guards), or a facade-level
    /// precondition on the arguments (an empty file set for `commit_paths`, removing
    /// the main workspace). This is a **caller bug**, distinct from a real IO or
    /// backend failure — a language binding maps it to a `ValueError`. Completes the
    /// `is_*` classifier family alongside [`is_not_found`](Error::is_not_found).
    pub fn is_invalid_input(&self) -> bool {
        match self {
            ErrorKind::Io(e) => e.kind() == std::io::ErrorKind::InvalidInput,
            ErrorKind::Vcs(e) => vcs_cli_support::is_invalid_input(e),
            _ => false,
        }
    }

    /// Whether a **resource the operation named doesn't exist** — currently a
    /// worktree/workspace lookup by path that matched no attached worktree
    /// ([`WorktreeNotFound`](ErrorKind::WorktreeNotFound)). Distinct from
    /// [`is_not_found`](Error::is_not_found), which means the `git`/`jj` **binary**
    /// wasn't found (a setup problem), and from [`is_invalid_input`](Error::is_invalid_input)
    /// (a bad argument). A binding maps this to a `NotFoundError`.
    ///
    /// Note the backend asymmetry: only the **jj** backend raises the typed
    /// `WorktreeNotFound`; git's missing-worktree removal surfaces as a generic
    /// backend `Exit`, which this does not classify. (Likewise the main-workspace
    /// refusal that [`is_invalid_input`](Error::is_invalid_input) recognizes is a
    /// typed error only on jj.)
    pub fn is_resource_not_found(&self) -> bool {
        matches!(self, ErrorKind::WorktreeNotFound(_))
    }

    /// Whether this is an [`Unsupported`](ErrorKind::Unsupported) action — the caller
    /// asked for something the repository's current in-progress state cannot
    /// honour (e.g. `continue_in_progress` during a `git bisect`). Distinct from
    /// [`is_invalid_input`](Error::is_invalid_input) (a *bad argument*): the
    /// argument was fine, the *state* just has no such step. Mirrors
    /// `vcs_forge::Error::is_unsupported`, so the two facades name the concept the
    /// same way for a language binding.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, ErrorKind::Unsupported(_))
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::NotARepository(p) => {
                // Deliberately doesn't say "at or above": `Repo::open` returns this
                // for a strict check of exactly `p` (no walking up), while
                // `Repo::discover` returns it after walking up from `p` and finding
                // nothing — a single wording that's accurate for both callers.
                write!(f, "no git or jj repository found at {}", p.display())
            }
            ErrorKind::BareRepository(p) => {
                write!(f, "bare git repositories are unsupported ({})", p.display())
            }
            ErrorKind::WorktreeNotFound(p) => {
                write!(f, "no worktree found at {}", p.display())
            }
            ErrorKind::Io(e) => write!(f, "{e}"),
            ErrorKind::Vcs(e) => write!(f, "{e}"),
            ErrorKind::Rollback(r) => {
                write!(f, "operation rollback did not complete cleanly: {r}")
            }
            ErrorKind::Unsupported(what) => write!(f, "unsupported operation: {what}"),
        }
    }
}

impl std::error::Error for ErrorKind {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ErrorKind::Io(e) => Some(e),
            ErrorKind::Vcs(e) => Some(e),
            // A failed restore carries the underlying cause; a divergence-skip has
            // no wrapped error to chain.
            ErrorKind::Rollback(r) => r.failure().map(|e| e as &(dyn std::error::Error + 'static)),
            _ => None,
        }
    }
}

macro_rules! simple_from {
    ($($src:ty => $variant:ident),+ $(,)?) => {
        $(
        impl From<$src> for Error {
            fn from(cause: $src) -> Self {
                Error::from_kind(cause.into())
            }
        }
        impl From<$src> for ErrorKind {
            fn from(cause: $src) -> Self {
                ErrorKind::$variant(cause)
            }
        }
        )*
    };
}
simple_from! {
    std::io::Error => Io,
    processkit::Error => Vcs,
}

/// `Result` specialised to the facade [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transient_delegates_to_processkit_and_excludes_facade_variants() {
        // An interrupted spawn is a transient io failure.
        let interrupted = ErrorKind::Vcs(processkit::Error::spawn(
            "git",
            std::io::Error::from(std::io::ErrorKind::Interrupted),
        ));
        assert!(interrupted.is_transient());
        // A missing binary is NOT transient (retrying won't install it).
        let missing = ErrorKind::Vcs(processkit::Error::spawn(
            "git",
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ));
        assert!(!missing.is_transient());
        // The facade's own io/detection variants are never transient.
        assert!(
            !ErrorKind::Io(std::io::Error::from(std::io::ErrorKind::Interrupted)).is_transient()
        );
        assert!(!ErrorKind::NotARepository("/x".into()).is_transient());
    }

    #[test]
    fn is_not_found_only_for_a_missing_binary() {
        let not_found = ErrorKind::Vcs(processkit::Error::not_found("jj", None));
        assert!(not_found.is_not_found());
        // An ordinary non-zero exit is not a "binary not found".
        let exit = ErrorKind::Vcs(processkit::Error::exit(
            "git",
            1,
            "",
            "fatal: not a git repository",
        ));
        assert!(!exit.is_not_found());
        assert!(!ErrorKind::NotARepository("/x".into()).is_not_found());
    }

    #[test]
    fn is_invalid_input_for_guard_rejections_and_facade_input_errors() {
        // A wrapper guard rejection (flag-like positional) surfaces as invalid input.
        let guarded = ErrorKind::Vcs(processkit::Error::spawn(
            "git",
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "flag-like"),
        ));
        assert!(guarded.is_invalid_input());
        // The facade's own `Io(InvalidInput)` guard (e.g. an empty commit set) too.
        assert!(
            ErrorKind::Io(std::io::Error::from(std::io::ErrorKind::InvalidInput))
                .is_invalid_input()
        );
        // A real spawn failure, a detection error, and a generic io error are NOT.
        assert!(
            !ErrorKind::Vcs(processkit::Error::spawn(
                "git",
                std::io::Error::from(std::io::ErrorKind::NotFound),
            ))
            .is_invalid_input()
        );
        assert!(!ErrorKind::NotARepository("/x".into()).is_invalid_input());
        assert!(!ErrorKind::Io(std::io::Error::other("disk full")).is_invalid_input());
    }

    #[test]
    fn is_unsupported_only_for_the_unsupported_variant() {
        let unsupported = ErrorKind::Unsupported("continue during a bisect".into());
        assert!(unsupported.is_unsupported());
        assert!(unsupported.to_string().contains("bisect"));
        // Not conflated with a bad-argument rejection or any other variant.
        assert!(!unsupported.is_invalid_input());
        assert!(
            !ErrorKind::Io(std::io::Error::from(std::io::ErrorKind::InvalidInput)).is_unsupported()
        );
        assert!(!ErrorKind::NotARepository("/x".into()).is_unsupported());
    }

    #[test]
    fn is_resource_not_found_only_for_a_worktree_lookup() {
        assert!(ErrorKind::WorktreeNotFound("/wt".into()).is_resource_not_found());
        // The *binary* missing is a different classifier (is_not_found), and a bad
        // repo path is neither.
        let missing_bin = ErrorKind::Vcs(processkit::Error::not_found("jj", None));
        assert!(missing_bin.is_not_found() && !missing_bin.is_resource_not_found());
        assert!(!ErrorKind::NotARepository("/x".into()).is_resource_not_found());
    }
}
