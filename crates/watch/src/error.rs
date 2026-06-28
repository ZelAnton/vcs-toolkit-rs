//! The crate's error type: filesystem-watcher setup failures plus the underlying
//! `vcs-core` re-query errors.

/// An error from setting up or running a [`RepoWatcher`](crate::RepoWatcher).
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The `notify` filesystem watcher failed to start or register a path.
    Notify(notify::Error),
    /// A `vcs-core` query (detection / `snapshot` / `local_branches`) failed —
    /// chiefly while *building* the watcher (capturing the baseline state). A
    /// re-query failure *during* watching is skipped and retried, not surfaced
    /// here (see [`RepoWatcher`](crate::RepoWatcher)).
    Vcs(vcs_core::Error),
    /// A filesystem operation failed (e.g. resolving a worktree gitlink).
    Io(std::io::Error),
}

impl Error {
    /// Whether this wraps a **transient** io/spawn failure (interrupted /
    /// would-block / resource-busy) from the underlying `vcs-core` query —
    /// delegates to [`vcs_core::Error::is_transient`]. Mirrors the classifier
    /// family on the other facades. `Notify`/`Io` and non-transient errors are
    /// `false`.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Vcs(e) if e.is_transient())
    }

    /// Whether the underlying VCS binary (`git`/`jj`) **wasn't found** — a setup
    /// problem (not installed / not on `PATH`), surfaced while building the
    /// watcher's baseline. Delegates to [`vcs_core::Error::is_not_found`].
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Vcs(e) if e.is_not_found())
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
        Error::Notify(e)
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
        let transient = Error::Vcs(vcs_core::Error::Vcs(processkit::Error::Spawn {
            program: "git".into(),
            source: std::io::Error::from(std::io::ErrorKind::Interrupted),
        }));
        assert!(transient.is_transient(), "interrupted spawn is transient");
        assert!(!transient.is_not_found());
        assert!(
            transient.processkit_error().is_some(),
            "reaches the inner error"
        );

        // The VCS binary wasn't found (setup problem), not transient.
        let missing = Error::Vcs(vcs_core::Error::Vcs(processkit::Error::NotFound {
            program: "jj".into(),
            searched: None,
        }));
        assert!(missing.is_not_found(), "missing binary is not-found");
        assert!(!missing.is_transient());
        assert!(missing.processkit_error().is_some());

        // A filesystem (`Io`) failure is neither, and carries no process error.
        let io = Error::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(!io.is_transient() && !io.is_not_found());
        assert!(
            io.processkit_error().is_none(),
            "no subprocess behind an Io error"
        );
    }
}
