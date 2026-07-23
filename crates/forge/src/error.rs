//! The facade's error type: the underlying [`processkit::Error`] the wrapper
//! clients return, plus an [`Unsupported`](Error::Unsupported) variant for an
//! operation a given forge's CLI does not provide.

use crate::ForgeKind;
use vcs_diff::Version;

/// An error from a [`Forge`](crate::Forge) operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An underlying `vcs-github` / `vcs-gitlab` / `vcs-gitea` (i.e. `processkit`)
    /// error.
    Forge(processkit::Error),
    /// The operation — or a requested **option** of it — isn't available on this
    /// forge's CLI. Two shapes share this variant: a whole operation a backend lacks
    /// (e.g. `repo_view`, `pr_mark_ready`, and `pr_checks` on Gitea, whose `tea` has no
    /// command for them), and a supported operation carrying an option the backend
    /// can't express — `pr_merge`'s `auto`/`delete_branch` and `pr_close`'s
    /// source-branch deletion (GitHub-only), or a request-changes review on GitLab
    /// (approve/revoke only). The facade rejects both **before spawning**. The
    /// `operation` is the [`ForgeApi`](crate::ForgeApi) method name. Probe any of these
    /// up front with [`Forge::supports`](crate::Forge::supports) /
    /// [`supports_review_kind`](crate::Forge::supports_review_kind) /
    /// [`supports_merge_option`](crate::Forge::supports_merge_option) /
    /// [`supports_pr_close_delete_branch`](crate::Forge::supports_pr_close_delete_branch).
    #[non_exhaustive]
    Unsupported {
        /// Which forge lacks the operation.
        forge: ForgeKind,
        /// The [`ForgeApi`](crate::ForgeApi) method that isn't supported.
        operation: &'static str,
    },
    /// The caller's input was refused by the facade before any CLI spawn —
    /// e.g. [`crate::Forge::pr_edit`] on a supported backend with both `title`
    /// and `body` set to `None`. Carries a short message naming what was wrong; surfaced by the
    /// MCP layer as `ErrorData::invalid_params` so a client can fix the call.
    InvalidInput(String),
    /// A **mutating** operation was refused **before spawning** because the
    /// installed forge CLI (`gh`/`glab`/`tea`) is **confirmed** older than the
    /// version this crate requires — the pre-flight version gate. Carries the
    /// `forge`, the [`ForgeApi`](crate::ForgeApi) method name, the `found`
    /// version the binary reported, and the `minimum` this crate needs. Only a
    /// *confirmed* below-floor version raises it: a version that couldn't be
    /// obtained or parsed is **fail-open** (the call proceeds as before), and
    /// **reading** operations are never version-gated. Distinct from
    /// [`Unsupported`](Error::Unsupported) (an operation/option a backend
    /// structurally lacks at any version): here the operation exists but the
    /// binary is too old, so the fix is to upgrade the CLI. Classify with
    /// [`Error::is_version_gated`]; the MCP layer surfaces it as
    /// `ErrorData::invalid_params` (the caller can fix it by upgrading).
    #[non_exhaustive]
    VersionUnsupported {
        /// Which forge's CLI is too old.
        forge: ForgeKind,
        /// The [`ForgeApi`](crate::ForgeApi) method that was refused.
        operation: &'static str,
        /// The version the installed CLI reported.
        found: Version,
        /// The minimum version this crate requires.
        minimum: Version,
    },
}

/// Lowercase markers identifying an **authentication** failure in a forge CLI's
/// output (`gh`/`glab`/`tea`). Phrase-based and conservative: a miss degrades to a
/// generic forge error, and the phrases are chosen to avoid a false
/// `is_unauthorized` (they don't occur in the CLIs' non-auth error text). Strictly authentication
/// (HTTP 401 / missing-or-bad token / not-logged-in) — a 403 *permission* refusal
/// stays a generic error.
const AUTH_MARKERS: &[&str] = &[
    "unauthorized",
    // Status-qualified, not a bare "401": a bare code would false-positive on a
    // PR/issue number, object id, or SHA echoed in an unrelated error message.
    "http 401",
    "bad credentials",
    "requires authentication",
    "authentication required",
    "authentication failed",
    "not logged in",
    // gh's canonical "run an operation with no auth configured" message
    // ("To get started with GitHub CLI, please run: gh auth login"). Keyed on this
    // unique phrase, NOT the bare "gh auth login" suggestion verb — gh prints that verb
    // in a wrong-remote *hint* too ("none of the git remotes … point to a known GitHub
    // host … please use `gh auth login`"), a misconfiguration, not an auth failure, so
    // matching the bare verb prompted a futile re-login (M27).
    "get started with github cli",
];

/// Lowercase markers identifying a **rate-limit** failure. Keyed on the message
/// (not the ambiguous 403 status, which GitHub also uses for permission errors).
const RATE_LIMIT_MARKERS: &[&str] = &[
    "rate limit",
    // Status-qualified (see AUTH_MARKERS) — a bare "429" would match a stray number.
    "http 429",
    "too many requests",
    "retry-after",
    "abuse detection",
];

/// Lowercase markers identifying **a named resource that doesn't exist** (a PR/MR,
/// issue, repository, or release the operation referenced). Deliberately narrow and
/// high-precision — the forge CLIs report this inconsistently, so these are the
/// specific phrasings `gh`/`glab` emit and the facade's own Gitea "no such PR"
/// message; a miss degrades to a generic error rather than a false positive.
const RESOURCE_NOT_FOUND_MARKERS: &[&str] = &[
    "could not resolve to", // gh: "Could not resolve to a PullRequest/Issue/Repository …"
    "404 not found",        // glab (note: GitLab also returns 404 for a *hidden* resource
    // the caller can't access, so a hit can mean "forbidden" as well as "absent")
    "no pull request", // the Gitea `pr_view` miss (surfaced as a facade parse error)
    "release not found", // gh: `release view <missing-tag>`
];

impl Error {
    /// Build an [`Unsupported`](Error::Unsupported) error naming the `forge` and the
    /// `operation` it lacks. The stable construction path — the variant is
    /// `#[non_exhaustive]`, so an external [`ForgeApi`](crate::ForgeApi) impl (a custom
    /// backend) must build it through this rather than a struct literal.
    pub fn unsupported(forge: ForgeKind, operation: &'static str) -> Self {
        Error::Unsupported { forge, operation }
    }

    /// Build a [`VersionUnsupported`](Error::VersionUnsupported) error — the
    /// pre-spawn version gate's refusal of a mutating `operation` on a `forge`
    /// whose CLI is confirmed `found` (below the crate's `minimum`). The stable
    /// construction path — the variant is `#[non_exhaustive]`, so an external
    /// [`ForgeApi`](crate::ForgeApi) impl gating its own backend must build it
    /// through this rather than a struct literal.
    pub fn version_unsupported(
        forge: ForgeKind,
        operation: &'static str,
        found: Version,
        minimum: Version,
    ) -> Self {
        Error::VersionUnsupported {
            forge,
            operation,
            found,
            minimum,
        }
    }

    /// Lowercased `stdout`+`stderr` of an underlying non-zero `Exit` — the CLI's
    /// message body, for marker classification. `None` for non-`Exit` errors
    /// (spawn/timeout/signal/not-found) and the facade's own variants, which carry
    /// no CLI message to classify.
    fn cli_output(&self) -> Option<String> {
        match self {
            Error::Forge(processkit::Error::Exit { stdout, stderr, .. }) => {
                Some(format!("{stdout}\n{stderr}").to_ascii_lowercase())
            }
            _ => None,
        }
    }

    /// Whether the forge CLI reported an **authentication** failure — a missing,
    /// expired, or invalid token, or "not logged in" — as opposed to a transient
    /// network error or a generic non-zero exit. Lets a caller (or a language
    /// binding) surface a dedicated auth error and prompt a re-login.
    ///
    /// Note: this classifies an auth failure *raised by an operation*. The separate
    /// [`Forge::auth_status`](crate::Forge::auth_status) probe returns `Ok(false)`
    /// for "not authenticated" rather than an error.
    pub fn is_unauthorized(&self) -> bool {
        self.cli_output()
            .is_some_and(|out| AUTH_MARKERS.iter().any(|m| out.contains(m)))
    }

    /// Whether the forge CLI was **rate-limited** (HTTP 429, "API rate limit
    /// exceeded", or a secondary/abuse limit) — a back-off-and-retry-later signal,
    /// distinct from [`is_transient_fetch_error`](Error::is_transient_fetch_error)
    /// (transient network blips). Lets a caller honour the limit instead of
    /// hammering the API.
    pub fn is_rate_limited(&self) -> bool {
        self.cli_output()
            .is_some_and(|out| RATE_LIMIT_MARKERS.iter().any(|m| out.contains(m)))
    }

    /// Whether this is a **transient** network failure worth retrying — DNS, a
    /// dropped connection, a fast blip — so a network-bound forge command can be
    /// retried. A **timeout is not** classified transient (it already spent the full
    /// deadline; retrying would multiply the wall-clock — see
    /// [`vcs_cli_support::is_transient_fetch_error`], which this delegates to).
    pub fn is_transient_fetch_error(&self) -> bool {
        matches!(self, Error::Forge(e) if vcs_cli_support::is_transient_fetch_error(e))
    }

    /// Whether the underlying error is a **transient io/spawn** failure
    /// (interrupted / would-block / resource-busy) — delegates to
    /// [`processkit::Error::is_transient`]. Narrower than
    /// [`is_transient_fetch_error`](Error::is_transient_fetch_error) (which also
    /// treats the network markers as retryable — but not a timeout). Mirrors
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
    /// than a forge/network failure). Covers **both** the facade's own variant
    /// (an operation — or a requested option — a backend can't do, e.g. `pr_checks`
    /// on Gitea, or `auto`/`delete_branch` on `pr_merge` and source-branch deletion on
    /// `pr_close` off GitHub, which the facade rejects before spawning) **and** a
    /// wrapper-level [`processkit::Error::Unsupported`] bubbling up through
    /// [`Forge`](crate::Error::Forge) — the same "this backend can't do that" signal,
    /// whether the facade caught it up front or a wrapper reported it. A caller acts on
    /// either identically.
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            Error::Unsupported { .. } | Error::Forge(processkit::Error::Unsupported { .. })
        )
    }

    /// Whether this is the pre-spawn **version gate** refusal
    /// ([`VersionUnsupported`](Error::VersionUnsupported)) — a mutating
    /// operation declined because the installed `gh`/`glab`/`tea` is
    /// **confirmed** below the version this crate requires. Deliberately
    /// **distinct** from [`is_unsupported`](Error::is_unsupported): there the
    /// backend structurally can't do the operation (at any version); here the
    /// operation exists but the binary is too old, so the fix is to upgrade the
    /// CLI, not to avoid the operation. A binding maps a hit to a "please
    /// upgrade the CLI" error; the MCP layer surfaces it as `invalid_params`
    /// (the caller can fix it). Reading operations never raise it (they aren't
    /// version-gated), and an unknown/unparseable version is fail-open (no
    /// error at all).
    pub fn is_version_gated(&self) -> bool {
        matches!(self, Error::VersionUnsupported { .. })
    }

    /// Whether this is an **input rejection** — a bad argument the facade refused
    /// before/without a useful CLI call: the facade's own
    /// [`InvalidInput`](Error::InvalidInput) (e.g. `pr_edit` with nothing to change on
    /// a supported backend),
    /// or a wrapper argument guard (`reject_flag_like`, e.g. a flag-like Gitea
    /// comment body). A caller bug, distinct from a forge/network failure; a binding
    /// maps it to a `ValueError`.
    pub fn is_invalid_input(&self) -> bool {
        match self {
            Error::InvalidInput(_) => true,
            Error::Forge(e) => vcs_cli_support::is_invalid_input(e),
            _ => false,
        }
    }

    /// Whether a **named resource doesn't exist** — the PR/MR, issue, repository, or
    /// release the call referenced (`pr_view(9999)`, etc.). Distinct from
    /// [`is_not_found`](Error::is_not_found), which means the `gh`/`glab`/`tea`
    /// **binary** wasn't found. Best-effort and high-precision (a small marker set):
    /// the CLIs phrase this inconsistently, so a miss degrades to a generic error. A
    /// binding maps a hit to a `NotFoundError`. Caveat: GitLab returns HTTP 404 for a
    /// resource the caller *can't access* as well as one that's absent, so a `glab`
    /// hit can mean "forbidden" — indistinguishable from the CLI text.
    pub fn is_resource_not_found(&self) -> bool {
        let hay = match self {
            Error::Forge(processkit::Error::Exit { stdout, stderr, .. }) => {
                format!("{stdout}\n{stderr}").to_ascii_lowercase()
            }
            // The Gitea `pr_view` miss is a facade *parse* error (an absent list row),
            // not a non-zero exit — check its message too.
            Error::Forge(processkit::Error::Parse { message, .. }) => message.to_ascii_lowercase(),
            _ => return false,
        };
        RESOURCE_NOT_FOUND_MARKERS.iter().any(|m| hay.contains(m))
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
            Error::VersionUnsupported {
                forge,
                operation,
                found,
                minimum,
            } => write!(
                f,
                "{}'s CLI is too old for `{operation}`: found {found}, need >= {minimum}",
                forge.as_str()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Forge(e) => Some(e),
            Error::Unsupported { .. }
            | Error::InvalidInput(_)
            | Error::VersionUnsupported { .. } => None,
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
        let missing = Error::Forge(processkit::Error::not_found("gh", None));
        assert!(missing.is_not_found());
        // An ordinary non-zero exit (e.g. no such PR) is not a "binary not found".
        let exit = Error::Forge(processkit::Error::exit(
            "gh",
            1,
            "",
            "no pull requests found",
        ));
        assert!(!exit.is_not_found());
        // The facade's own variants are never "not found".
        assert!(!Error::InvalidInput("x".into()).is_not_found());
    }

    #[test]
    fn is_transient_only_for_an_io_transient() {
        let interrupted = Error::Forge(processkit::Error::spawn(
            "glab",
            std::io::Error::from(std::io::ErrorKind::Interrupted),
        ));
        assert!(interrupted.is_transient());
        // A missing binary is NOT transient (retrying won't install it).
        let missing = Error::Forge(processkit::Error::spawn(
            "glab",
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ));
        assert!(!missing.is_transient());
        assert!(
            !Error::Unsupported {
                forge: ForgeKind::Gitea,
                operation: "pr_checks",
            }
            .is_transient()
        );
    }

    #[test]
    fn classifies_auth_and_rate_limit_from_cli_output() {
        let exit = |stderr: &str| Error::Forge(processkit::Error::exit("gh", 1, "", stderr));
        // Authentication failures (representative gh / glab phrasings) — auth, not rate-limit.
        for msg in [
            "HTTP 401: Bad credentials (https://api.github.com/graphql)",
            "error: 401 Unauthorized",
            "401 Unauthorized (could not authenticate, run `glab auth login`)",
            "you are not logged in. Run gh auth login to authenticate",
            "GraphQL: requires authentication",
            // gh's canonical no-auth operation failure — carries only the "get started"
            // phrase, not the other markers (M27: this must still classify).
            "To get started with GitHub CLI, please run:  gh auth login",
        ] {
            assert!(
                exit(msg).is_unauthorized(),
                "{msg:?} should be unauthorized"
            );
            assert!(!exit(msg).is_rate_limited(), "{msg:?} is not rate-limited");
        }
        // M27: gh's wrong-remote hint SUGGESTS `gh auth login` but is a
        // misconfiguration, not an auth failure — matching the bare "auth login"
        // suggestion verb would prompt a futile re-login, so it must NOT classify.
        let wrong_host = "none of the git remotes configured for this repository point \
                          to a known GitHub host. To tell gh about a new GitHub host, \
                          please use `gh auth login`";
        assert!(
            !exit(wrong_host).is_unauthorized(),
            "a wrong-remote hint is a misconfiguration, not an auth failure"
        );
        // Rate limits (incl. the secondary/abuse limit) — rate-limit, not auth.
        for msg in [
            "API rate limit exceeded for user ID 123",
            "HTTP 429: Too Many Requests",
            "You have exceeded a secondary rate limit (abuse detection mechanism)",
        ] {
            assert!(
                exit(msg).is_rate_limited(),
                "{msg:?} should be rate-limited"
            );
            assert!(!exit(msg).is_unauthorized(), "{msg:?} is not an auth error");
        }
        // A generic non-zero exit is neither — crucially including a not-found that
        // merely *echoes a number* like 401/429 (markers are status-qualified
        // `http 401`/`http 429`, not bare integers, so no false positive).
        assert!(!exit("no pull requests found").is_unauthorized());
        assert!(!exit("no pull requests found").is_rate_limited());
        assert!(
            !exit("Could not resolve to a PullRequest with the number of 401.").is_unauthorized()
        );
        assert!(!exit("Could not resolve to an Issue with the number of 429.").is_rate_limited());
        // Empty / message-less output is neither.
        assert!(!exit("").is_unauthorized() && !exit("").is_rate_limited());
        // Non-`Exit` errors and the facade's own variants carry no CLI body → neither.
        let spawn = Error::Forge(processkit::Error::spawn(
            "gh",
            std::io::Error::from(std::io::ErrorKind::Interrupted),
        ));
        assert!(!spawn.is_unauthorized() && !spawn.is_rate_limited());
        assert!(!Error::InvalidInput("x".into()).is_unauthorized());
        assert!(
            !Error::Unsupported {
                forge: ForgeKind::Gitea,
                operation: "pr_checks",
            }
            .is_rate_limited()
        );
    }

    #[test]
    fn classifies_invalid_input_and_resource_not_found() {
        let exit = |stderr: &str| Error::Forge(processkit::Error::exit("gh", 1, "", stderr));
        // Invalid input: the facade's own variant + a wrapper guard rejection.
        assert!(Error::InvalidInput("nothing to edit".into()).is_invalid_input());
        assert!(
            Error::Forge(processkit::Error::spawn(
                "tea",
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "flag-like body"),
            ))
            .is_invalid_input()
        );
        assert!(!exit("boom").is_invalid_input());

        // Resource not found: gh/glab exit phrasings + the Gitea parse-miss.
        assert!(
            exit("GraphQL: Could not resolve to a PullRequest with the number of 9999")
                .is_resource_not_found()
        );
        assert!(exit("404 Not Found").is_resource_not_found());
        assert!(exit("release not found").is_resource_not_found()); // gh release miss
        assert!(
            Error::Forge(processkit::Error::parse(
                "tea",
                "no pull request #9999 in `tea pr list`",
            ))
            .is_resource_not_found()
        );
        // A missing binary is `is_not_found`, NOT resource-not-found; and neither
        // classifies a generic error.
        let missing = Error::Forge(processkit::Error::not_found("gh", None));
        assert!(missing.is_not_found() && !missing.is_resource_not_found());
        assert!(!exit("some other error").is_resource_not_found());
        assert!(!exit("some other error").is_invalid_input());
    }

    #[test]
    fn version_gated_is_its_own_class_distinct_from_unsupported() {
        let found = Version {
            major: 1,
            minor: 14,
            patch: 0,
        };
        let minimum = Version {
            major: 2,
            minor: 0,
            patch: 0,
        };
        let err = Error::version_unsupported(ForgeKind::GitHub, "pr_create", found, minimum);
        // The dedicated classifier fires…
        assert!(err.is_version_gated());
        // …and it is deliberately NOT `is_unsupported` — the operation exists,
        // the binary is just too old (a distinct "upgrade the CLI" signal).
        assert!(!err.is_unsupported(), "version-gate is not `Unsupported`");
        // It is none of the CLI-output classifiers either (it carries no CLI body).
        assert!(!err.is_invalid_input());
        assert!(!err.is_unauthorized());
        assert!(!err.is_rate_limited());
        assert!(!err.is_resource_not_found());
        assert!(!err.is_not_found());
        assert!(!err.is_transient());
        // It has no error source (a facade-own variant, like `Unsupported`).
        assert!(std::error::Error::source(&err).is_none());
        // Display names the operation, the found version, and the required floor.
        let msg = err.to_string();
        assert!(msg.contains("pr_create"), "{msg}");
        assert!(msg.contains("1.14.0"), "{msg}");
        assert!(msg.contains("2.0.0"), "{msg}");

        // The inverse: a structural `Unsupported` is NOT a version gate.
        assert!(
            !Error::unsupported(ForgeKind::Gitea, "pr_diff").is_version_gated(),
            "a structural Unsupported is not a version gate"
        );
    }
}
