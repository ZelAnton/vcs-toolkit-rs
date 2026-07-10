//! The crate's error type: filesystem-watcher setup failures plus the underlying
//! `vcs-core` re-query errors.
//!
//! The filesystem-watch backend (`notify`) is a **private** dependency: its
//! failures surface as the opaque [`WatchError`], classified through this crate's
//! own stable methods rather than by matching the third-party error type. So a
//! consumer reads and source-chains a watch failure through `vcs-watch` alone —
//! no direct `notify` dependency to keep version-matched — and a `notify` major
//! bump stays an *internal*, non-breaking change here (the backend is not part of
//! this crate's stability contract).

use std::path::PathBuf;

/// An error from setting up or running a [`RepoWatcher`](crate::RepoWatcher).
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The filesystem watcher failed to start, or to register/deregister a
    /// watched path. Opaque over the private backend — inspect it through
    /// [`WatchError`]'s classifiers (reachable directly, or via
    /// [`Error::watch_error`]) instead of the backend's own error type.
    Notify(WatchError),
    /// A `vcs-core` query (detection / `snapshot` / `local_branches`) failed —
    /// chiefly while *building* the watcher (capturing the baseline state). A
    /// re-query failure *during* watching is skipped and retried, not surfaced
    /// here (see [`RepoWatcher`](crate::RepoWatcher)).
    Vcs(vcs_core::Error),
    /// A filesystem operation failed (e.g. resolving a worktree gitlink).
    Io(std::io::Error),
}

impl Error {
    /// Whether this wraps a **transient** failure worth retrying — an
    /// interrupted / would-block / resource-busy io/spawn failure from the
    /// underlying `vcs-core` query (delegates to
    /// [`vcs_core::Error::is_transient`]), **or** a baseline-snapshot **timeout**
    /// (`Io` `TimedOut`, raised when the startup snapshot exceeds
    /// `requery_timeout`) — a wedged repo may un-wedge, and the loop already treats
    /// a re-query timeout as a transient skip, so `build()` agrees. Other `Io` and
    /// `Notify` errors are `false` (a failed OS watch registration won't fix itself
    /// on a blind retry — classify it via [`WatchError`] and act on the cause).
    /// Mirrors the classifier family on the other facades.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Vcs(e) => e.is_transient(),
            Error::Io(e) => e.kind() == std::io::ErrorKind::TimedOut,
            _ => false,
        }
    }

    /// Whether the underlying VCS binary (`git`/`jj`) **wasn't found** — a setup
    /// problem (not installed / not on `PATH`), surfaced while building the
    /// watcher's baseline. Delegates to [`vcs_core::Error::is_not_found`].
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Vcs(e) if e.is_not_found())
    }

    /// The opaque [`WatchError`] when this is a filesystem-watch backend failure;
    /// `None` for a `Vcs`/`Io` error. A stable accessor (mirroring
    /// [`processkit_error`](Self::processkit_error)) so a caller — or a language
    /// binding — can reach the watch classifiers without matching the enum
    /// variant by hand.
    pub fn watch_error(&self) -> Option<&WatchError> {
        match self {
            Error::Notify(e) => Some(e),
            _ => None,
        }
    }

    /// The structured underlying [`processkit::Error`], if this error came from a
    /// VCS subprocess — flattening the two-level
    /// `Vcs(`[`vcs_core::Error::Vcs`]`(_))` nesting so a caller (or a language
    /// binding) can read its structured fields (`program`, plus `code`/`stdout`/
    /// `stderr` on an `Exit`) without hand-walking it. `None` for a `Notify`/`Io`
    /// failure or a non-subprocess `vcs-core` error (e.g. "not a repository").
    pub fn processkit_error(&self) -> Option<&processkit::Error> {
        match self {
            Error::Vcs(vcs_core::Error::Vcs(e)) => Some(e),
            _ => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Notify(e) => write!(f, "filesystem watch failed: {e}"),
            Error::Vcs(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Notify(e) => Some(e),
            Error::Vcs(e) => Some(e),
            Error::Io(e) => Some(e),
        }
    }
}

impl From<notify::Error> for Error {
    fn from(e: notify::Error) -> Self {
        Error::Notify(WatchError(e))
    }
}

impl From<vcs_core::Error> for Error {
    fn from(e: vcs_core::Error) -> Self {
        Error::Vcs(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// An opaque filesystem-watch backend failure — the watcher couldn't start, or
/// couldn't register/deregister a watched path.
///
/// It wraps the crate's **private** watch backend so a consumer can *classify*
/// and *source-chain* the failure without naming — or depending on — that
/// third-party crate:
///
/// - [`is_path_not_found`](Self::is_path_not_found) — the watched `.git`/`.jj`
///   directory does not exist (removed, or never present);
/// - [`is_watch_limit`](Self::is_watch_limit) — the OS watch-descriptor limit was
///   reached (e.g. Linux inotify's `max_user_watches`);
/// - [`io_error`](Self::io_error) — the raw [`std::io::Error`] when the backend
///   failure was an I/O one (also reachable via
///   [`std::error::Error::source`]);
/// - [`paths`](Self::paths) — the paths the backend blamed, if any.
///
/// Because the backend type never appears in a public signature, a backend major
/// bump is an internal change here — not a breaking one downstream. Obtain one
/// from [`Error::watch_error`] or by matching [`Error::Notify`].
#[derive(Debug)]
pub struct WatchError(notify::Error);

impl WatchError {
    /// The watched path does not exist — e.g. the `.git`/`.jj` state directory
    /// was removed (or never existed), so the OS watch can't be registered until
    /// the path is present.
    pub fn is_path_not_found(&self) -> bool {
        matches!(self.0.kind, notify::ErrorKind::PathNotFound)
    }

    /// The OS watch-descriptor limit was reached (e.g. Linux inotify's
    /// `max_user_watches`) — the watch can't be registered until the limit is
    /// raised or other watches are released. Best-effort / platform-dependent.
    pub fn is_watch_limit(&self) -> bool {
        matches!(self.0.kind, notify::ErrorKind::MaxFilesWatch)
    }

    /// The underlying [`std::io::Error`] when the backend failure was an I/O
    /// error (`None` otherwise) — inspect its [`kind`](std::io::Error::kind)
    /// without walking the [`source`](std::error::Error::source) chain.
    pub fn io_error(&self) -> Option<&std::io::Error> {
        match &self.0.kind {
            notify::ErrorKind::Io(e) => Some(e),
            _ => None,
        }
    }

    /// The paths the backend associated with this failure — empty for a general,
    /// path-less failure.
    pub fn paths(&self) -> &[PathBuf] {
        &self.0.paths
    }
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for WatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // The backend only overrides the deprecated `cause()`, so re-expose the
        // underlying io error via the modern `source()` for a caller walking the
        // chain (`Error` -> `WatchError` -> `io::Error`).
        match &self.0.kind {
            notify::ErrorKind::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// `Result` specialised to the watcher [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The classifiers delegate through the `Vcs(vcs_core::Error)` layer and the
    /// accessor flattens the two-level nesting; non-VCS errors are inert.
    #[test]
    fn classifiers_and_accessor_reach_through_the_vcs_layer() {
        // A transient io/spawn hiccup from the underlying vcs-core query.
        let transient = Error::Vcs(vcs_core::Error::Vcs(processkit::Error::spawn(
            "git",
            std::io::Error::from(std::io::ErrorKind::Interrupted),
        )));
        assert!(transient.is_transient(), "interrupted spawn is transient");
        assert!(!transient.is_not_found());
        assert!(
            transient.processkit_error().is_some(),
            "reaches the inner error"
        );
        assert!(
            transient.watch_error().is_none(),
            "a vcs-core error is not a watch error"
        );

        // The VCS binary wasn't found (setup problem), not transient.
        let missing = Error::Vcs(vcs_core::Error::Vcs(processkit::Error::not_found(
            "jj", None,
        )));
        assert!(missing.is_not_found(), "missing binary is not-found");
        assert!(!missing.is_transient());
        assert!(missing.processkit_error().is_some());

        // A generic filesystem (`Io`) failure is neither, and carries no process error.
        let io = Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(!io.is_transient() && !io.is_not_found());
        assert!(
            io.processkit_error().is_none(),
            "no subprocess behind an Io error"
        );

        // ...but a baseline-snapshot timeout (`Io` `TimedOut`, R4) IS transient — a
        // wedged repo may un-wedge, so `build()` is worth retrying.
        let baseline_timeout = Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert!(
            baseline_timeout.is_transient(),
            "a baseline TimedOut is transient (retryable)"
        );
    }

    /// The opaque `WatchError` classifies each backend failure kind and
    /// source-chains its io cause — all without re-exposing the backend type.
    #[test]
    fn watch_error_classifies_backend_kinds() {
        // PathNotFound: the watched state dir is gone. `paths` carries the blame.
        let e: Error = notify::Error::path_not_found()
            .add_path(PathBuf::from("/repo/.git"))
            .into();
        let w = e.watch_error().expect("a Notify error exposes its WatchError");
        assert!(w.is_path_not_found());
        assert!(!w.is_watch_limit());
        assert!(w.io_error().is_none());
        assert_eq!(w.paths(), [PathBuf::from("/repo/.git")]);
        // No io cause for a path-not-found, and the enum classifiers stay inert.
        assert!(std::error::Error::source(w).is_none());
        assert!(!e.is_transient() && !e.is_not_found());
        assert!(e.processkit_error().is_none());

        // MaxFilesWatch: the inotify / descriptor limit.
        let limit: Error = notify::Error::new(notify::ErrorKind::MaxFilesWatch).into();
        let w = limit.watch_error().expect("WatchError");
        assert!(w.is_watch_limit() && !w.is_path_not_found());

        // Io: the raw error is reachable directly and via `source()`.
        let io: Error =
            notify::Error::io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)).into();
        let w = io.watch_error().expect("WatchError");
        assert_eq!(
            w.io_error().map(|e| e.kind()),
            Some(std::io::ErrorKind::PermissionDenied)
        );
        let src = std::error::Error::source(w).expect("io cause is source-chained");
        assert!(src.downcast_ref::<std::io::Error>().is_some());
    }

    /// The top-level `Error` source chain reaches the io cause *through* the
    /// opaque wrapper: `Error` -> `WatchError` -> `io::Error`.
    #[test]
    fn top_level_source_chain_reaches_io_through_watch_error() {
        let e: Error = notify::Error::io(std::io::Error::from(std::io::ErrorKind::NotFound)).into();
        let first = std::error::Error::source(&e).expect("WatchError is the first source");
        assert!(
            first.downcast_ref::<WatchError>().is_some(),
            "the opaque wrapper is the immediate source"
        );
        let second = first.source().expect("io::Error is the next link");
        assert!(second.downcast_ref::<std::io::Error>().is_some());
    }
}
