//! The facade's error type: the underlying [`processkit::Error`] the wrapper
//! clients return, plus an [`Unsupported`](Error::Unsupported) variant for an
//! operation a given forge's CLI does not provide.

use crate::ForgeKind;

/// An error from a [`Forge`](crate::Forge) operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An underlying `vcs-github` / `vcs-gitlab` / `vcs-gitea` (i.e. `processkit`)
    /// error.
    Forge(processkit::Error),
    /// The operation isn't available on this forge's CLI — e.g. `repo_view`,
    /// `pr_mark_ready`, and `pr_checks` on Gitea, whose `tea` has no command for
    /// them. The `operation` is the [`ForgeApi`](crate::ForgeApi) method name.
    Unsupported {
        /// Which forge lacks the operation.
        forge: ForgeKind,
        /// The [`ForgeApi`](crate::ForgeApi) method that isn't supported.
        operation: &'static str,
    },
    /// The caller's input was refused by the facade before any CLI spawn —
    /// e.g. [`crate::Forge::pr_edit`] with both `title` and `body` set to
    /// `None`. Carries a short message naming what was wrong; surfaced by the
    /// MCP layer as `ErrorData::invalid_params` so a client can fix the call.
    InvalidInput(String),
}

impl Error {
    /// Whether this is a **transient** network failure worth retrying (DNS,
    /// connection reset, timeout) — forge commands are network-bound, so a higher
    /// flow may want to retry. Named to match the wrapper classifiers
    /// ([`vcs_cli_support::is_transient_fetch_error`]).
    pub fn is_transient_fetch_error(&self) -> bool {
        matches!(self, Error::Forge(e) if vcs_cli_support::is_transient_fetch_error(e))
    }

    /// Whether the underlying error is a **transient io/spawn** failure
    /// (interrupted / would-block / resource-busy) — delegates to
    /// [`processkit::Error::is_transient`]. Narrower than
    /// [`is_transient_fetch_error`](Error::is_transient_fetch_error) (which also
    /// treats a timeout and the network markers as retryable). Mirrors
    /// [`vcs_core::Error::is_transient`](https://docs.rs/vcs-core/latest/vcs_core/enum.Error.html#method.is_transient)
    /// so the classifier family is the same on both facades.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Forge(e) if e.is_transient())
    }

    /// Whether the underlying forge CLI binary (`gh`/`glab`/`tea`) **wasn't found** —
    /// a setup problem (the tool isn't installed or isn't on `PATH`), not a usage or
    /// network error. Delegates to [`processkit::Error::is_not_found`]; lets a caller
    /// surface a "please install gh/glab/tea" hint. Mirrors
    /// [`vcs_core::Error::is_not_found`](https://docs.rs/vcs-core/latest/vcs_core/enum.Error.html#method.is_not_found).
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Forge(e) if e.is_not_found())
    }

    /// Whether this is an [`Unsupported`](Error::Unsupported) operation (rather
    /// than a forge/network failure).
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Error::Unsupported { .. })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Forge(e) => write!(f, "{e}"),
            Error::Unsupported { forge, operation } => {
                write!(f, "{} does not support `{operation}`", forge.as_str())
            }
            Error::InvalidInput(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Forge(e) => Some(e),
            Error::Unsupported { .. } | Error::InvalidInput(_) => None,
        }
    }
}

impl From<processkit::Error> for Error {
    fn from(e: processkit::Error) -> Self {
        Error::Forge(e)
    }
}

/// `Result` specialised to the facade [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_not_found_only_for_a_missing_cli_binary() {
        let missing = Error::Forge(processkit::Error::NotFound {
            program: "gh".into(),
            searched: None,
        });
        assert!(missing.is_not_found());
        // An ordinary non-zero exit (e.g. no such PR) is not a "binary not found".
        let exit = Error::Forge(processkit::Error::Exit {
            program: "gh".into(),
            code: 1,
            stdout: String::new(),
            stderr: "no pull requests found".into(),
        });
        assert!(!exit.is_not_found());
        // The facade's own variants are never "not found".
        assert!(!Error::InvalidInput("x".into()).is_not_found());
    }

    #[test]
    fn is_transient_only_for_an_io_transient() {
        let interrupted = Error::Forge(processkit::Error::Spawn {
            program: "glab".into(),
            source: std::io::Error::from(std::io::ErrorKind::Interrupted),
        });
        assert!(interrupted.is_transient());
        // A missing binary is NOT transient (retrying won't install it).
        let missing = Error::Forge(processkit::Error::Spawn {
            program: "glab".into(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        assert!(!missing.is_transient());
        assert!(
            !Error::Unsupported {
                forge: ForgeKind::Gitea,
                operation: "pr_checks",
            }
            .is_transient()
        );
    }
}
