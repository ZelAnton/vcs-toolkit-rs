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
//! - **[`is_merge_conflict`] / [`is_nothing_to_commit`] / [`is_transient_fetch_error`]
//!   / [`is_lock_contention`]** — classify a returned [`Error`] so callers branch on
//!   *intent* ("conflict, resolve it"; "nothing to commit, no-op"; "transient,
//!   retry"; "another process holds the lock, retry") instead of matching on error
//!   internals. They inspect captured [`Error::Exit`] output against fixed marker
//!   lists; a [`processkit`] [`Error::Timeout`] is **not** treated as a transient
//!   fetch error (it already spent the full deadline — see
//!   [`is_transient_fetch_error`]); any unfamiliar `#[non_exhaustive]` variant falls
//!   through to "no".
//! - **[`RetryPolicy`] / [`retry_async`] / [`ManagedClient`]** — an opt-in retry
//!   strategy (attempts + exponential, jittered backoff) for **lock-contention**
//!   failures. `ManagedClient` wraps a [`processkit`] `CliClient` and applies the
//!   policy to every command, so the `vcs-git`/`vcs-jj` clients gain retry via
//!   `with_retry(...)` without changing a call site. Lock-acquisition failures are
//!   pre-execution, so retrying is safe even for mutating commands.
//! - **[`CredentialProvider`] / [`Credential`] / [`Secret`]** — an opt-in seam for
//!   supplying a secret *per operation* (a CI token, a vault lookup) instead of
//!   relying on ambient CLI auth. `ManagedClient` injects the resolved token into
//!   each command (the forge `GH_TOKEN`/`GITLAB_TOKEN` env); git uses
//!   [`git_credential_helper`] to keep the secret out of `argv`. Default is no
//!   provider → ambient auth, unchanged. See the [`credentials`](mod@credentials)
//!   module for the full picture.
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
//!             std::thread::sleep(FETCH_BACKOFF); // DNS / dropped connection — worth a retry
//!         }
//!         Err(e) => return Err(e),               // anything else: give up
//!     }
//! }
//! # Ok(()) }
//! ```

use std::ffi::OsStr;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use processkit::{
    CliClient, Command, Error, IntoCommand, JobRunner, ProcessResult, ProcessRunner, Result,
};

pub mod credentials;
pub use credentials::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, FnProvider,
    GitCredentialHelper, Secret, StaticCredential, git_credential_helper, https_host, provider_fn,
};

/// JSON helpers shared by the forge wrappers, behind the `serde` feature — so the
/// three forge parsers share one `null -> ""` and parse-error convention.
#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod json {
    use processkit::{Error, Result};
    use serde::Deserialize;
    use serde::de::DeserializeOwned;

    /// Deserialize a `String` a forge CLI may send as JSON `null` for an empty
    /// optional value: `null` -> empty string, same as an absent key. `#[serde(default)]`
    /// alone covers only an absent key; a present `null` would fail the whole-object
    /// parse. Use as `#[serde(deserialize_with = "vcs_cli_support::json::null_to_empty")]`.
    pub fn null_to_empty<'de, D>(deserializer: D) -> ::core::result::Result<String, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
    }

    /// Deserialize a forge CLI's `--json` output into `T`, mapping a parse failure to
    /// [`Error::Parse`] tagged with `program` (the CLI's binary name).
    pub fn from_json<T: DeserializeOwned>(program: &str, json: &str) -> Result<T> {
        serde_json::from_str(json).map_err(|e| Error::parse(program, e.to_string()))
    }
}

/// Generate the cwd-bound forwarders for a CLI wrapper's `…At` view.
///
/// Each CLI wrapper (`vcs-git`, `vcs-jj`, `vcs-github`, `vcs-gitlab`, `vcs-gitea`)
/// exposes a cwd-bound view — `GitAt`, `JjAt`, `GitHubAt`, `GitLabAt`, `GiteaAt` —
/// that holds a reference to the client plus a pre-bound `dir`, and re-exposes the
/// client's methods with `dir` already supplied. The forwarder bodies are
/// byte-identical across the five backends but for a handful of names, so they live
/// here once instead of as a copied `macro_rules!` per crate:
///
/// - `$view` — the bound view type (e.g. `GitAt`). It must be generic over
///   `<'a, R: ProcessRunner>` and have a field named `$field` holding the client
///   plus a `dir: &'a Path` field.
/// - `$field` — the inner field naming the client (e.g. `git`, `gh`, `glab`,
///   `tea`).
/// - `$client` — a **string literal** naming the client type, used in the
///   generated doc strings and rendered as an intra-doc link (e.g. `"Git"` →
///   ``[`Git`]``).
/// - `bare { … }` — methods forwarded verbatim to `self.$field`. Reserve this for
///   the genuinely dir-*independent* calls (`version`, `capabilities`, a
///   `clone`/`git_clone` that names its own destination): the view drops `dir`
///   entirely, so a `bare` method never touches it.
/// - `dir  { … }` — methods that take `self.dir` as their first argument.
/// - `raw  { fn view(args…) -> Ret => target; … }` — the **raw escape hatches**
///   (`run`/`run_raw`/`run_args`/`run_raw_args`). These used to sit in `bare`, so
///   `git.at(dir).run(…)` silently ran in the *process* cwd, not the bound `dir` —
///   a bound handle whose raw call could hit a different repository (M15/T-035).
///   They are now **bound**: the view method `view` forwards to the client's
///   dir-taking `target` (`self.$field.target(self.dir, args…)`), so a raw call
///   *through the view* runs in `dir` like every other `…At` method. The
///   **process-cwd** escape hatch is still there — call `run`/`run_raw`/… on the
///   client itself (`git.run(…)`), not through `.at(dir)`.
///
/// The argument and return types in the method lists resolve in the **calling**
/// crate, so they are written exactly as that wrapper's own methods are. The
/// `ProcessRunner` bound is fully qualified (`::processkit::ProcessRunner`) so the
/// expansion compiles regardless of which items the caller has imported.
///
/// ```ignore
/// vcs_cli_support::at_forwarders! {
///     GitAt, git, "Git",
///     bare { fn version() -> Result<String>; }
///     dir  { fn status() -> Result<Vec<StatusEntry>>; }
///     raw  { fn run(args: &[String]) -> Result<String> => run_in; }
/// }
/// ```
#[macro_export]
macro_rules! at_forwarders {
    (
        $view:ident, $field:ident, $client:literal,
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
        $( raw  { $( fn $rn:ident( $($ra:ident: $rt:ty),* $(,)? ) -> $rr:ty => $rtgt:ident; )* } )?
    ) => {
        impl<'a, R: ::processkit::ProcessRunner> $view<'a, R> {
            $(
                #[doc = concat!("Bound form of [`", $client, "`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.$field.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`", $client, "`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.$field.$dn(self.dir, $($da),*).await
                }
            )*
            $($(
                #[doc = concat!(
                    "Bound form of [`", $client, "`]'s `", stringify!($rn),
                    "` raw escape hatch — runs the given argv **in the bound `dir`** \
                     (forwards to the client's `", stringify!($rtgt), "`). For the \
                     process-cwd escape hatch, call `", stringify!($rn),
                    "` on [`", $client, "`] directly."
                )]
                pub async fn $rn(&self, $($ra: $rt),*) -> $rr {
                    self.$field.$rtgt(self.dir, $($ra),*).await
                }
            )*)?
        }
    };
}

/// Emit the common client scaffold every CLI wrapper hand-writes around a
/// [`ManagedClient`].
///
/// `vcs-git`, `vcs-jj`, `vcs-github`, and `vcs-gitlab` each wrap a
/// [`ManagedClient`] in a thin newtype that re-exposes the same handful of
/// constructors and default-applying builders — `new` / `Default` /
/// `with_runner` / `default_timeout` / `default_env` / `default_env_remove` /
/// `default_cancel_on` — with byte-identical bodies and doc strings. This macro
/// generates that shared part so it can't drift between backends; each wrapper
/// keeps its *capability* builders (`with_retry`, `with_credentials`, every verb,
/// the `…At` view, …) hand-written in a separate `impl` block.
///
/// The generated newtype is `struct $name<R: ProcessRunner = JobRunner>` with a
/// single private `core: ManagedClient<R>` field — accessible to the rest of the
/// wrapper crate (same module). All paths are fully qualified, so the expansion
/// compiles regardless of what the caller has imported.
///
/// - `$name` — the wrapper type (e.g. `Git`). The struct-level doc comment (and
///   any other attributes) written before `struct` are attached to it verbatim.
/// - `$binary` — the program the client drives (an expression, typically the
///   crate's `BINARY` const).
/// - `token_env = ($svc, $var)` — *optional*. When given, `new`/`with_runner`
///   chain [`ManagedClient::with_token_env`] so a resolved credential is injected
///   into the `$var` environment variable for service `$svc` (the forge case:
///   `GH_TOKEN`, `GITLAB_TOKEN`). Omit it for the ambient-auth backends (git, jj).
/// - `scrub_env = [ $var, … ]` — *optional*. When given, `new`/`with_runner`
///   chain [`ManagedClient::default_env_remove`] for each var, so **every** client
///   the macro generates drops those inherited environment variables by default
///   (`vcs-git` uses it to scrub the repo-redirector vars — `GIT_DIR`, … — so a
///   value leaking from the parent process can't retarget commands). Must come
///   *after* `token_env` when both are present.
///
/// ```ignore
/// vcs_cli_support::managed_client! {
///     /// The real GitHub client.
///     pub struct GitHub => BINARY, token_env = (CredentialService::GitHub, "GH_TOKEN")
/// }
/// vcs_cli_support::managed_client! {
///     /// The real Git client — scrubs the repo-redirector env vars by default.
///     pub struct Git => BINARY, scrub_env = ["GIT_DIR", "GIT_WORK_TREE"]
/// }
/// ```
#[macro_export]
macro_rules! managed_client {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident => $binary:expr
        $(, token_env = ($svc:expr, $var:expr) )?
        $(, scrub_env = [ $($scrub:expr),* $(,)? ] )?
        $(,)?
    ) => {
        $(#[$meta])*
        $vis struct $name<R: ::processkit::ProcessRunner = ::processkit::JobRunner> {
            core: $crate::ManagedClient<R>,
        }

        // Manual Debug: no `R: Debug` bound (matches `ManagedClient`'s own impl),
        // delegating straight to `core` — `ManagedClient::fmt` already redacts any
        // configured credential provider / token-env binding, so nothing secret
        // reaches `{:?}` here either.
        impl<R: ::processkit::ProcessRunner> ::core::fmt::Debug for $name<R> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("core", &self.core)
                    .finish()
            }
        }

        impl $name<::processkit::JobRunner> {
            /// Create a client driving the real job-backed runner.
            pub fn new() -> Self {
                Self { core: $crate::ManagedClient::new($binary)
                    $(.with_token_env($svc, $var))?
                    $($(.default_env_remove($scrub))*)?
                }
            }
        }

        impl ::core::default::Default for $name<::processkit::JobRunner> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<R: ::processkit::ProcessRunner> $name<R> {
            /// Create a client driving `runner` — inject a fake in tests.
            pub fn with_runner(runner: R) -> Self {
                Self {
                    core: $crate::ManagedClient::with_runner($binary, runner)
                        $(.with_token_env($svc, $var))?
                        $($(.default_env_remove($scrub))*)?,
                }
            }

            /// Apply a default timeout to every command this client builds.
            pub fn default_timeout(mut self, timeout: ::core::time::Duration) -> Self {
                self.core = self.core.default_timeout(timeout);
                self
            }

            /// Set an environment variable on every command this client builds.
            pub fn default_env(
                mut self,
                key: impl ::core::convert::AsRef<::std::ffi::OsStr>,
                value: impl ::core::convert::AsRef<::std::ffi::OsStr>,
            ) -> Self {
                self.core = self.core.default_env(key, value);
                self
            }

            /// Remove an inherited environment variable on every command this client builds.
            pub fn default_env_remove(
                mut self,
                key: impl ::core::convert::AsRef<::std::ffi::OsStr>,
            ) -> Self {
                self.core = self.core.default_env_remove(key);
                self
            }

            /// Cancel every command this client builds when `token` fires.
            pub fn default_cancel_on(mut self, token: ::processkit::CancellationToken) -> Self {
                self.core = self.core.default_cancel_on(token);
                self
            }
        }
    };
}

/// Injection guard for bare positional argv slots: a caller-supplied value with a
/// leading `-` would be parsed by the CLI as a *flag* (verified: `git checkout
/// -evil` → "unknown switch"; jj likewise), and an empty (or whitespace-only)
/// value silently changes most commands' meaning. Refuse both before anything
/// spawns, surfacing an [`Error::Spawn`] naming `program`. An interior NUL is
/// refused too (it can't be passed in argv and otherwise surfaces as an opaque
/// OS spawn error). Flag-VALUE positions (`-m <msg>`, `--branch <b>`) don't need
/// this — the CLI consumes the next token verbatim there.
///
/// The leading-`-` test is applied to the **trimmed** value, so a value like
/// `" --upload-pack=…"` (leading whitespace) is still refused — the empty-check
/// and the flag-check now agree on what "the value" is.
pub fn reject_flag_like(program: &str, what: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || value.contains('\0') {
        return Err(Error::spawn(
            program,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} would be parsed as a flag (or is empty / contains NUL) — \
                     refusing to pass it as a positional argument"
                ),
            ),
        ));
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
/// The timeout markers are kept *specific* (`connection timed out` /
/// `operation timed out`) rather than a bare `timed out`, which would also match
/// unrelated, non-network "timed out" messages (a lock wait, a hook) and trigger a
/// spurious fetch retry.
const TRANSIENT_FETCH_MARKERS: &[&str] = &[
    "could not resolve host",
    "couldn't resolve host",
    "temporary failure in name resolution",
    "connection timed out",
    "connection refused",
    "operation timed out",
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

/// Whether a failed `fetch`/`fetch_branch`/`remote_branch_exists` looks
/// transient (DNS, a dropped connection, a fast network blip) and is worth
/// retrying.
///
/// A processkit-level **timeout** is deliberately **not** classified transient
/// (R6). A `.timeout()`-bounded run that expired has already consumed the caller's
/// full deadline — retrying it would multiply the wall-clock by [`FETCH_ATTEMPTS`]
/// (e.g. a black-holed remote under a 120 s deadline would block ≈ 6 min, three
/// times the advertised ceiling). The deadline *is* the patience budget; a caller
/// who wants longer should raise the timeout, not have it silently tripled. Fast
/// transient failures (the io-level and marker cases below) still retry, because
/// they fail quickly and a retry is cheap.
pub fn is_transient_fetch_error(err: &Error) -> bool {
    // An io-level transient from the spawn itself (interrupted / would-block / busy),
    // which processkit classifies via `Error::is_transient()` (it covers `Spawn`/`Io`,
    // not `Exit`/`Timeout`, so it composes cleanly with the marker scan below).
    err.is_transient() || exit_output_matches(err, TRANSIENT_FETCH_MARKERS)
}

/// Lower-case substrings marking a **whole-repository / working-copy lock**
/// contention failure — another process held the *one* repo-wide lock, so the
/// command **never started** (clean, pre-execution) and touched nothing.
///
/// These are deliberately limited to the locks that guard the *entire* operation
/// up front, so retrying is safe even on a **mutating** command: the repo was not
/// modified at all. We intentionally do **not** include per-ref lock messages
/// (`cannot lock ref`, `<ref>.lock`/`packed-refs.lock: File exists`): a multi-ref
/// `push`/`fetch` updates refs sequentially, so a ref-lock failure can arrive
/// *after* earlier refs already moved — replaying that is not idempotent. Network
/// markers
/// ([`TRANSIENT_FETCH_MARKERS`]) and conflict/exit failures are likewise absent.
const LOCK_CONTENTION_MARKERS: &[&str] = &[
    // git: the whole-repo index lock (pre-write). Match the **locale-stable path
    // fragment** `index.lock`, not the translated `': File exists'` suffix — git
    // localizes its messages, so a `LANG=de_DE` runner would never match the full
    // English phrase. `index.lock` names the index lock specifically; per-ref locks
    // (`<ref>.lock`, `packed-refs.lock`) are ruled out by the `refs/` guard in
    // `is_lock_contention`. (This matches any `index.lock` *create* failure — a
    // held lock, or e.g. `Permission denied` — all pre-write, so retrying is safe.)
    "index.lock",
    // jj: the working-copy lock and the operation-heads lock (both pre-mutation).
    // These are jj's exact wordings (lower-cased for the classifier). NOTE: modern
    // jj generally **blocks** on these locks until they're free rather than failing,
    // so contention usually surfaces as a wait, not a classifiable error — these
    // markers catch only the residual cases where jj does surface a lock error.
    "failed to lock working copy",
    "failed to lock operation heads store",
];

/// Whether `err` is a **whole-repository lock-contention** failure — another
/// process held git's `index.lock` or jj's working-copy / op-heads lock, so the
/// command couldn't even start. Such a failure is *pre-execution* and therefore
/// safe to retry even on a **mutating** operation (the repo was never modified).
/// Per-ref lock failures (`cannot lock ref`, `<ref>.lock`) are deliberately **not**
/// classified here — they can occur mid-way through a multi-ref `push`/`fetch`,
/// where a retry would not be idempotent. Conflict, "nothing to commit", a real
/// non-zero exit, a timeout, a signal, or a missing binary are also **not** lock
/// contention and must not be retried this way.
pub fn is_lock_contention(err: &Error) -> bool {
    // Rule out a **per-ref** lock first: it is *not* safely retryable (a multi-ref
    // push/fetch can fail one ref's lock after earlier refs already moved). git's
    // per-ref lock lives under `refs/` (`…/refs/heads/<name>.lock`) and its message
    // names `refs/…`, whereas the whole-repo `index.lock` (`<gitdir>/index.lock`)
    // never does — so a `refs/` mention excludes it, locale-independently. This also
    // stops a branch literally named `index`/`reindex` (whose `…/reindex.lock`
    // contains the substring `index.lock`) from matching the bare `index.lock`
    // marker. (A repo whose *path* contains `refs/` then misses the index-lock retry
    // — a benign false-negative, safer than a wrong retry.)
    if exit_output_matches(err, &["refs/"]) {
        return false;
    }
    exit_output_matches(err, LOCK_CONTENTION_MARKERS)
}

/// Whether `err` is an **input rejection** — a bad caller argument, encoded as an
/// [`Error::Spawn`] whose source is `io::ErrorKind::InvalidInput`. This is the
/// pattern the toolkit's own argument guards raise ([`reject_flag_like`] and the
/// validating newtypes `RefName`/`RevSpec`/`RevsetExpr`) for a value that would be
/// misparsed as a flag, is empty, or contains a NUL — and it also covers the
/// spawn-time `InvalidInput` the OS raises for an un-spawnable argument (an interior
/// NUL in a flag-value, or Windows' batch-arg-escaping refusal). All are genuine
/// bad input, distinct from a real spawn failure (missing binary → `NotFound`, no
/// perms → `PermissionDenied`) or a non-zero exit. A binding maps this to a
/// `ValueError`; the facades re-expose it as `Error::is_invalid_input()`.
pub fn is_invalid_input(err: &Error) -> bool {
    matches!(
        err,
        Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput
    )
}

/// A bounded retry strategy: how many attempts, the (exponential) backoff between
/// them, and whether to add full jitter. Used by [`ManagedClient`] to retry
/// [`is_lock_contention`] failures. The [`Default`] is [`none`](RetryPolicy::none)
/// (no retry) — retry is **opt-in**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total attempts including the first; `1` means no retry.
    pub attempts: u32,
    /// Delay before the first retry; doubles each subsequent retry (capped by
    /// [`max_backoff`](RetryPolicy::max_backoff)). `ZERO` means retry immediately.
    pub base_backoff: Duration,
    /// Upper bound on the (pre-jitter) backoff delay. `ZERO` means uncapped.
    pub max_backoff: Duration,
    /// Apply **full jitter** — the actual delay is uniform in `[0, computed]` — to
    /// avoid a thundering herd when many workers retry against one repository.
    pub jitter: bool,
}

impl RetryPolicy {
    /// No retry: a single attempt. The default.
    pub const fn none() -> Self {
        Self {
            attempts: 1,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            jitter: false,
        }
    }

    /// A sensible default for repository lock contention: a handful of attempts
    /// with short, jittered, exponential backoff (25 ms → 500 ms).
    pub const fn lock_contention() -> Self {
        Self {
            attempts: 5,
            base_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(500),
            jitter: true,
        }
    }

    /// Set the total number of attempts (clamped to at least 1).
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// Set the base backoff (the delay before the first retry).
    pub fn base_backoff(mut self, backoff: Duration) -> Self {
        self.base_backoff = backoff;
        self
    }

    /// Cap the (pre-jitter) backoff delay; `ZERO` leaves it uncapped.
    pub fn max_backoff(mut self, max: Duration) -> Self {
        self.max_backoff = max;
        self
    }

    /// Toggle full jitter on the backoff delay.
    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }
}

impl Default for RetryPolicy {
    /// No retry — retry is opt-in.
    fn default() -> Self {
        Self::none()
    }
}

/// The (possibly jittered) backoff before the `retry_index`-th retry (0 = first).
fn backoff_for(policy: &RetryPolicy, retry_index: u32) -> Duration {
    if policy.base_backoff.is_zero() {
        return Duration::ZERO;
    }
    let base = policy.base_backoff.as_nanos();
    let scaled = base.saturating_mul(1u128 << retry_index.min(20));
    let capped = if policy.max_backoff.is_zero() {
        scaled
    } else {
        scaled.min(policy.max_backoff.as_nanos())
    };
    let delay = Duration::from_nanos(capped.min(u64::MAX as u128) as u64);
    if policy.jitter {
        full_jitter(delay)
    } else {
        delay
    }
}

/// Full jitter: a uniform delay in `[0, max]`. Dependency-free randomness via the
/// OS-seeded [`RandomState`](std::collections::hash_map::RandomState) — good enough
/// to de-correlate retries, not cryptographic.
fn full_jitter(max: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let nanos = max.as_nanos();
    if nanos == 0 {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(nanos as u64);
    let r = hasher.finish() as u128;
    Duration::from_nanos((r % (nanos + 1)).min(u64::MAX as u128) as u64)
}

/// Run `op`, retrying its result while `should_retry` says so and `policy` has
/// attempts left, sleeping the (jittered, exponential) backoff between tries. The
/// op is re-invoked from scratch each attempt, so it must be idempotent for the
/// errors `should_retry` selects (lock-contention failures are — the command never
/// ran). Returns the first `Ok`, or the last `Err`.
pub async fn retry_async<T, Fut>(
    policy: &RetryPolicy,
    should_retry: impl Fn(&Error) -> bool,
    mut op: impl FnMut() -> Fut,
) -> Result<T>
where
    Fut: Future<Output = Result<T>>,
{
    let attempts = policy.attempts.max(1);
    for attempt in 1..=attempts {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if attempt == attempts || !should_retry(&err) {
                    return Err(err);
                }
                let delay = backoff_for(policy, attempt - 1);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    unreachable!("the loop returns on the final attempt")
}

/// A [`CliClient`] wrapper that adds two opt-in concerns the CLI wrappers
/// (`vcs-git`, `vcs-jj`, `vcs-github`, `vcs-gitlab`) all share, without touching a
/// single call site:
///
/// 1. **Lock-contention retry** ([`is_lock_contention`]) per a [`RetryPolicy`] —
///    off by default ([`RetryPolicy::none`]); enable with
///    [`with_retry`](ManagedClient::with_retry). Safe even for mutating commands,
///    since lock contention is a clean pre-execution failure.
/// 2. **Credential injection** from an opt-in [`CredentialProvider`] — off by
///    default (no provider); attach one with
///    [`with_credentials`](ManagedClient::with_credentials). When a forge
///    *token-env* binding is configured
///    ([`with_token_env`](ManagedClient::with_token_env)), every command run
///    through this client gets the resolved token in that environment variable
///    (e.g. `GH_TOKEN`). Backends that inject the secret differently (git's
///    `credential.helper`) instead call
///    [`resolve_credential`](ManagedClient::resolve_credential) at the command
///    site. Resolution happens once per call, before the retry loop.
///
/// Both default to inert, so a client with neither configured behaves exactly
/// like a bare `CliClient`.
pub struct ManagedClient<R: ProcessRunner = JobRunner> {
    inner: CliClient<R>,
    retry: RetryPolicy,
    credentials: Option<Arc<dyn CredentialProvider>>,
    /// When set, the token is auto-injected into this env var on every command,
    /// resolved for this service. Used by the forge clients (`GH_TOKEN`, …).
    token_env: Option<(CredentialService, &'static str)>,
}

impl<R: ProcessRunner> fmt::Debug for ManagedClient<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedClient")
            .field("inner", &self.inner)
            .field("retry", &self.retry)
            // Never render the provider itself (it may close over a secret); just
            // whether one is configured, plus the token-env binding.
            .field("credentials", &self.credentials.is_some())
            .field("token_env", &self.token_env)
            .finish()
    }
}

impl ManagedClient<JobRunner> {
    /// A retrying client driving `program` on the real job-backed runner (no retry
    /// until [`with_retry`](ManagedClient::with_retry)).
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            inner: CliClient::new(program),
            retry: RetryPolicy::none(),
            credentials: None,
            token_env: None,
        }
    }
}

impl<R: ProcessRunner> ManagedClient<R> {
    /// A retrying client driving `program` on `runner` — inject a fake in tests.
    pub fn with_runner(program: impl AsRef<OsStr>, runner: R) -> Self {
        Self {
            inner: CliClient::with_runner(program, runner),
            retry: RetryPolicy::none(),
            credentials: None,
            token_env: None,
        }
    }

    /// Set the lock-contention retry policy (opt-in; default is no retry).
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// The active retry policy.
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry
    }

    /// Attach a [`CredentialProvider`] (opt-in; default is none → ambient auth).
    /// The provider is consulted per operation: automatically when a
    /// [`with_token_env`](ManagedClient::with_token_env) binding is set, or
    /// on demand via [`resolve_credential`](ManagedClient::resolve_credential).
    ///
    /// **Precedence:** a resolved token is injected *after* any
    /// [`default_env`](ManagedClient::default_env), so the provider wins over a
    /// static default and over the ambient CLI login. **Cancellation:** a
    /// [`default_cancel_on`](ManagedClient::default_cancel_on) token bounds the
    /// spawned *process*, not provider resolution — if your provider does slow I/O
    /// (a vault lookup), bound it yourself.
    #[must_use]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.credentials = Some(provider);
        self
    }

    /// Bind the resolved token to an environment variable injected on **every**
    /// command this client runs (the forge case: `GH_TOKEN`, `GITLAB_TOKEN`). The
    /// `service` tags the [`CredentialRequest`]. No effect without a provider.
    #[must_use]
    pub fn with_token_env(mut self, service: CredentialService, var: &'static str) -> Self {
        self.token_env = Some((service, var));
        self
    }

    /// Whether a credential provider is configured.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    /// Resolve a credential for `service`/`host` from the configured provider, or
    /// `Ok(None)` if no provider is set or it defers to ambient auth. Backends
    /// that inject the secret at the command site (git's `credential.helper`) call
    /// this directly; the forge token-env path uses it internally.
    pub async fn resolve_credential(
        &self,
        service: CredentialService,
        host: Option<&str>,
    ) -> Result<Option<Credential>> {
        let Some(provider) = &self.credentials else {
            return Ok(None);
        };
        let request = CredentialRequest { service, host };
        // An empty (or whitespace-only) secret is not a usable credential —
        // injecting an empty `GH_TOKEN`/`GITLAB_TOKEN` (or a `password=` line)
        // would *override* the ambient login with nothing rather than defer to it.
        // Treat it as `None` (ambient), keeping the "no usable credential ⇒
        // ambient auth" contract consistent regardless of which adapter produced
        // it (matching `EnvToken`'s own whitespace-only ⇒ unset rule).
        Ok(provider
            .credential(&request)
            .await?
            .filter(|cred| !cred.secret().expose().trim().is_empty()))
    }

    /// Materialize `call` into a [`Command`], injecting the forge token env if a
    /// [`with_token_env`](ManagedClient::with_token_env) binding and a provider
    /// are both configured. The single place the auto-injection happens, shared by
    /// every retrying verb.
    async fn prepare(&self, call: impl IntoCommand<R>) -> Result<Command> {
        let cmd = call.into_command(&self.inner);
        let Some((service, var)) = self.token_env else {
            return Ok(cmd);
        };
        match self.resolve_credential(service, None).await? {
            Some(cred) => Ok(cmd.env(var, cred.secret().expose())),
            None => Ok(cmd),
        }
    }

    /// Apply a default timeout to every command this client builds.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.default_timeout(timeout);
        self
    }

    /// Set an environment variable on every command this client builds.
    pub fn default_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.inner = self.inner.default_env(key, value);
        self
    }

    /// Remove an inherited environment variable on every command this client builds.
    pub fn default_env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.inner = self.inner.default_env_remove(key);
        self
    }

    /// Cancel every command this client builds when `token` fires.
    pub fn default_cancel_on(mut self, token: processkit::CancellationToken) -> Self {
        self.inner = self.inner.default_cancel_on(token);
        self
    }

    /// Build a [`Command`] for this client's program (passthrough).
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.command(args)
    }

    /// Build a [`Command`] bound to `dir` (passthrough).
    pub fn command_in<I, S>(&self, dir: &Path, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.command_in(dir, args)
    }

    /// The underlying process runner (passthrough — e.g. for `output_all`).
    pub fn runner(&self) -> &R {
        self.inner.runner()
    }

    /// Like [`CliClient::run`], with credential injection and lock-retry.
    pub async fn run(&self, call: impl IntoCommand<R>) -> Result<String> {
        let cmd = self.prepare(call).await?;
        retry_async(&self.retry, is_lock_contention, || {
            self.inner.run(cmd.clone())
        })
        .await
    }

    /// Like [`CliClient::run_unit`], with credential injection and lock-retry.
    pub async fn run_unit(&self, call: impl IntoCommand<R>) -> Result<()> {
        let cmd = self.prepare(call).await?;
        retry_async(&self.retry, is_lock_contention, || {
            self.inner.run_unit(cmd.clone())
        })
        .await
    }

    /// Like [`CliClient::output_string`], with credential injection. **No lock-retry:**
    /// `output_string` returns `Ok` on a non-zero exit (it captures the result), so a
    /// lock failure surfaces as an `Ok` here, not an `Err` the retry predicate could
    /// match — route mutations that need lock-retry through
    /// [`run`](Self::run)/[`run_unit`](Self::run_unit) instead.
    pub async fn output_string(&self, call: impl IntoCommand<R>) -> Result<ProcessResult<String>> {
        let cmd = self.prepare(call).await?;
        self.inner.output_string(cmd).await
    }

    /// Like [`CliClient::output_bytes`], with credential injection. Captures stdout
    /// as **raw bytes**, byte-exact — unlike [`output_string`](Self::output_string),
    /// which reassembles stdout from decoded lines and so drops a trailing newline.
    /// This is the byte-faithful path [`run_untrimmed`](Self::run_untrimmed) needs.
    /// **No lock-retry**, for the same reason as `output_string`: it returns `Ok`
    /// on a non-zero exit (it captures the result), so a lock failure surfaces as an
    /// `Ok` here rather than an `Err` the retry predicate could match.
    pub async fn output_bytes(&self, call: impl IntoCommand<R>) -> Result<ProcessResult<Vec<u8>>> {
        let cmd = self.prepare(call).await?;
        self.inner.output_bytes(cmd).await
    }

    /// Like [`run`](Self::run), but returns stdout **verbatim** — no `trim_end`.
    /// For **content**-returning verbs (a file's bytes at a rev, a diff, a raw
    /// template render) where the trailing newline(s) are part of the value, not
    /// noise: trimming them corrupts a read-modify-write round-trip and desyncs a
    /// diff's last hunk from its `@@` line count. Exit-checked like `run`; no
    /// lock-retry (a content read is not a mutation).
    ///
    /// Routed through [`output_bytes`](Self::output_bytes) (raw stdout), not
    /// `output_string`, so the exact bytes — trailing newline included — survive:
    /// `output_string` rebuilds stdout from decoded lines and would drop that final
    /// `\n`. The raw bytes are then decoded losslessly with
    /// [`String::from_utf8_lossy`], the same raw-stdout-to-`String` convention used
    /// elsewhere in this workspace (e.g. `vcs-jj`).
    pub async fn run_untrimmed(&self, call: impl IntoCommand<R>) -> Result<String> {
        let bytes = self
            .output_bytes(call)
            .await?
            .ensure_success()?
            .into_stdout();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Like [`CliClient::probe`] (zero-or-nonzero exit → `bool`), with credential
    /// injection and lock-retry.
    pub async fn probe(&self, call: impl IntoCommand<R>) -> Result<bool> {
        let cmd = self.prepare(call).await?;
        retry_async(&self.retry, is_lock_contention, || {
            self.inner.probe(cmd.clone())
        })
        .await
    }

    /// Like [`CliClient::exit_code`] (the raw exit code; a spawn failure or timeout
    /// still errors), with credential injection and lock-retry.
    pub async fn exit_code(&self, call: impl IntoCommand<R>) -> Result<i32> {
        let cmd = self.prepare(call).await?;
        retry_async(&self.retry, is_lock_contention, || {
            self.inner.exit_code(cmd.clone())
        })
        .await
    }

    /// Like [`CliClient::parse`] (credential injection applied; the `FnOnce` parser
    /// can't be re-run, so lock-retry does not — parsing is a read, where lock
    /// contention is not a concern anyway).
    pub async fn parse<T>(
        &self,
        call: impl IntoCommand<R>,
        parser: impl FnOnce(&str) -> T + Send,
    ) -> Result<T>
    where
        T: Send,
    {
        let cmd = self.prepare(call).await?;
        self.inner.parse(cmd, parser).await
    }

    /// Like [`CliClient::try_parse`] (credential injection applied; `FnOnce` parser,
    /// and a read, so no lock-retry).
    pub async fn try_parse<T>(
        &self,
        call: impl IntoCommand<R>,
        parser: impl FnOnce(&str) -> Result<T> + Send,
    ) -> Result<T>
    where
        T: Send,
    {
        let cmd = self.prepare(call).await?;
        self.inner.try_parse(cmd, parser).await
    }
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
        // Leading whitespace before a dash is still refused (the flag-check trims).
        assert!(reject_flag_like("git", "remote", " --upload-pack=evil").is_err());
        assert!(reject_flag_like("git", "remote", "\t-x").is_err());
        // An interior NUL is refused (can't go in argv; opaque OS error otherwise).
        assert!(reject_flag_like("git", "path", "a\0b").is_err());
        // A leading-whitespace non-flag value is still accepted (not flag-like).
        assert!(reject_flag_like("git", "branch name", "  feature").is_ok());
        // The error names the program and surfaces as a spawn-side refusal.
        let err = reject_flag_like("jj", "revset", "--remote").unwrap_err();
        assert!(matches!(err, Error::Spawn { program, .. } if program == "jj"));
    }

    #[test]
    fn classifies_merge_conflict() {
        let on_stdout = Error::exit("git", 1, "CONFLICT (content): Merge conflict in a.rs", "");
        let on_stderr = Error::exit(
            "git",
            1,
            "",
            "Automatic merge failed; fix conflicts and then commit",
        );
        let unrelated = Error::exit("git", 128, "", "fatal: not a git repository");
        assert!(is_merge_conflict(&on_stdout));
        assert!(is_merge_conflict(&on_stderr));
        assert!(!is_merge_conflict(&unrelated));
        assert!(!is_nothing_to_commit(&on_stdout));
    }

    #[test]
    fn classifies_nothing_to_commit_and_transient_fetch() {
        let nothing = Error::exit("git", 1, "nothing to commit, working tree clean", "");
        assert!(is_nothing_to_commit(&nothing));

        let dns = Error::exit(
            "git",
            128,
            "",
            "fatal: unable to access 'https://x/': Could not resolve host: x",
        );
        assert!(is_transient_fetch_error(&dns));
        assert!(!is_transient_fetch_error(&nothing));

        // A processkit timeout is deliberately NOT retried (R6): it already consumed
        // the caller's full deadline, so retrying would multiply the wall-clock by
        // FETCH_ATTEMPTS. The deadline is the patience budget; raise it, don't triple it.
        let timeout = Error::timeout("git", Duration::from_secs(10), "", "");
        assert!(!is_transient_fetch_error(&timeout));
    }

    // R9: an io-level transient from the spawn (EINTR / EAGAIN / busy) is fetch-
    // retryable too, via processkit's `Error::is_transient()`.
    #[test]
    fn classifies_io_transient_as_fetch_retryable() {
        let interrupted =
            Error::spawn("git", std::io::Error::from(std::io::ErrorKind::Interrupted));
        assert!(
            interrupted.is_transient(),
            "processkit treats Interrupted as a transient io error"
        );
        assert!(is_transient_fetch_error(&interrupted));
        // A non-transient io error (e.g. NotFound — the binary is missing) is not retried.
        let missing = Error::spawn("git", std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(!is_transient_fetch_error(&missing));
    }

    // R2: regression for the processkit 0.9.1 untruncated-`Error::Exit` fix. A large
    // output (well past the old 4 KiB cap) with the decisive marker near the END must
    // still classify — proving the classifiers see the whole captured stream.
    #[test]
    fn classifies_on_large_output_past_the_old_4kib_cap() {
        let padding = "noise line that says nothing\n".repeat(500); // ~14 KiB
        let conflict = Error::exit(
            "git",
            1,
            format!("{padding}CONFLICT (content): Merge conflict in late.rs"),
            "",
        );
        assert!(
            is_merge_conflict(&conflict),
            "a conflict marker past 4 KiB must still classify"
        );

        let transient = Error::exit(
            "git",
            128,
            "",
            format!("{padding}fatal: unable to access: Could not resolve host: x"),
        );
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
        let signalled = Error::signalled(
            "git",
            Some(15),
            "",
            "fatal: unable to access: Could not resolve host: x",
        );
        assert!(!signalled.is_transient());
        assert!(!is_transient_fetch_error(&signalled));
        assert!(!is_merge_conflict(&signalled));
        assert!(!is_nothing_to_commit(&signalled));
    }

    fn exit(program: &str, code: i32, stderr: &str) -> Error {
        Error::exit(program, code, "", stderr)
    }

    // `is_lock_contention` recognises ONLY the *whole-repo* / working-copy lock
    // failures (git index.lock, jj working-copy/op-heads lock) — the ones where the
    // command did nothing, so a retry is idempotent even on a mutation. Per-ref lock
    // failures and conflicts/timeouts are deliberately NOT classified (a multi-ref
    // op can fail a ref lock mid-way, where a retry would not be idempotent).
    #[test]
    fn classifies_lock_contention() {
        let lock_failures = [
            // git always names `index.lock` (locale-stable) in the lock-contention
            // message, even on a non-English runner where the surrounding prose is
            // translated.
            exit(
                "git",
                128,
                "fatal: Unable to create '/r/.git/index.lock': File exists.",
            ),
            // A German runner: the path fragment `index.lock` still matches.
            exit(
                "git",
                128,
                "fatal: Konnte '/r/.git/index.lock' nicht erstellen: Datei existiert bereits",
            ),
            // jj's *actual* wordings (verified against jj source) — note no "the".
            exit("jj", 1, "Error: Failed to lock working copy"),
            exit("jj", 1, "Error: Failed to lock operation heads store"),
        ];
        for e in &lock_failures {
            assert!(is_lock_contention(e), "should be lock contention: {e:?}");
            // A lock failure is NOT a transient *fetch* error — different class.
            assert!(!is_transient_fetch_error(e), "not a fetch error: {e:?}");
        }
        let not_locks = [
            exit("git", 1, "CONFLICT (content): Merge conflict in a.rs"),
            exit("git", 1, "error: pathspec 'x' did not match any file(s)"),
            exit("git", 128, "fatal: not a git repository"),
            // Per-ref locks are NOT classified — a multi-ref push/fetch can fail a
            // ref lock after earlier refs already moved (non-idempotent to replay).
            exit(
                "git",
                1,
                "error: cannot lock ref 'refs/heads/x': reference already exists",
            ),
            exit(
                "git",
                128,
                "Unable to create '/r/.git/packed-refs.lock': File exists.",
            ),
            // A per-ref lock for a branch literally named `index`: its
            // `…/refs/heads/index.lock` path contains the substring `index.lock`,
            // but the `refs/` mention correctly rules it out (not a whole-repo lock).
            exit(
                "git",
                128,
                "error: cannot lock ref 'refs/heads/index': Unable to create \
                 '/r/.git/refs/heads/index.lock': File exists.",
            ),
            Error::timeout("git", Duration::from_secs(1), "", ""),
        ];
        for e in &not_locks {
            assert!(
                !is_lock_contention(e),
                "should NOT be lock contention: {e:?}"
            );
        }
    }

    #[test]
    fn classifies_invalid_input_from_the_guards() {
        // What `reject_flag_like` / the newtypes actually produce.
        let rejected = reject_flag_like("git", "reference", "-x").unwrap_err();
        assert!(
            is_invalid_input(&rejected),
            "guard rejection is invalid input"
        );
        assert!(is_invalid_input(
            &reject_flag_like("git", "x", "").unwrap_err()
        ));

        // A real spawn failure (missing binary), a non-zero exit, and a timeout are
        // NOT invalid input — they're environment/usage failures, not a bad argument.
        let not_input = [
            Error::spawn("git", std::io::Error::from(std::io::ErrorKind::NotFound)),
            exit("git", 1, "fatal: not a git repository"),
            Error::timeout("git", Duration::from_secs(1), "", ""),
        ];
        for e in &not_input {
            assert!(!is_invalid_input(e), "should NOT be invalid input: {e:?}");
        }
    }

    // Backoff is exponential off the base, capped at `max_backoff`, and zero when
    // there's no base (immediate retry).
    #[test]
    fn backoff_is_exponential_capped_and_zero_without_base() {
        let p = RetryPolicy::none()
            .attempts(6)
            .base_backoff(Duration::from_millis(10))
            .max_backoff(Duration::from_millis(80));
        assert_eq!(backoff_for(&p, 0), Duration::from_millis(10));
        assert_eq!(backoff_for(&p, 1), Duration::from_millis(20));
        assert_eq!(backoff_for(&p, 2), Duration::from_millis(40));
        assert_eq!(backoff_for(&p, 3), Duration::from_millis(80));
        assert_eq!(
            backoff_for(&p, 4),
            Duration::from_millis(80),
            "capped at max"
        );
        assert_eq!(
            backoff_for(&RetryPolicy::none(), 3),
            Duration::ZERO,
            "no base → no wait"
        );
    }

    // Full jitter (used by `RetryPolicy::lock_contention`): every sampled backoff
    // stays within `[0, exponential cap]`, and successive samples de-correlate
    // (more than one distinct value) so retries don't thunder together. Pins the
    // jitter path, which the exponential test above deliberately turns off.
    #[test]
    fn jitter_stays_within_cap_and_decorrelates() {
        let p = RetryPolicy::none()
            .attempts(8)
            .base_backoff(Duration::from_millis(10))
            .max_backoff(Duration::from_millis(80))
            .with_jitter(true);
        // The cap at retry_index 3 is the full 80ms exponential value.
        let cap = Duration::from_millis(80);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let d = backoff_for(&p, 3);
            assert!(
                d <= cap,
                "jittered backoff {d:?} must stay within the cap {cap:?}"
            );
            seen.insert(d.as_nanos());
        }
        assert!(
            seen.len() > 1,
            "full jitter must produce a spread of delays, not a constant"
        );
        // A zero base still short-circuits to zero even with jitter on.
        assert_eq!(
            backoff_for(&RetryPolicy::none().with_jitter(true), 2),
            Duration::ZERO
        );
    }

    // The executor: retries while the predicate matches and attempts remain, returns
    // the first Ok, doesn't retry a non-matching error, and exhausts to the last Err.
    #[tokio::test]
    async fn retry_async_retries_then_succeeds_and_respects_the_predicate() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // Zero backoff → no sleep, deterministic & fast.
        let policy = RetryPolicy::none().attempts(4);
        let lock = || {
            exit(
                "git",
                128,
                "Unable to create '/r/.git/index.lock': File exists.",
            )
        };

        // Fails twice with a lock error, then succeeds — retried to success.
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, is_lock_contention, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let lock = lock();
            async move { if n < 2 { Err(lock) } else { Ok(n) } }
        })
        .await;
        assert_eq!(out.unwrap(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "1 try + 2 retries");

        // A non-lock error is returned immediately (not retried).
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(exit("git", 1, "real, deterministic failure")) }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "non-retryable → single attempt"
        );

        // Persistent lock contention exhausts the attempt budget.
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(exit("git", 128, "index.lock': File exists")) }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 4, "all attempts used");
    }

    // `resolve_credential` returns `None` until a provider is attached, then the
    // provider's credential. (No process is spawned, so the real runner is fine.)
    #[tokio::test]
    async fn retrying_client_resolves_credential_opt_in() {
        let client = ManagedClient::new("git");
        assert!(!client.has_credentials());
        assert!(
            client
                .resolve_credential(CredentialService::Git, None)
                .await
                .unwrap()
                .is_none(),
            "no provider → ambient (None)"
        );

        let client = client.with_credentials(Arc::new(StaticCredential::token("t0k")));
        assert!(client.has_credentials());
        let got = client
            .resolve_credential(CredentialService::Git, None)
            .await
            .unwrap()
            .expect("provider yields a credential");
        assert_eq!(got.secret().expose(), "t0k");
    }

    // An empty (or whitespace-only) secret is treated as `None` (ambient):
    // injecting an empty token would override the ambient login with nothing
    // instead of deferring to it. Mirrors `EnvToken`'s whitespace-only ⇒ unset rule.
    #[tokio::test]
    async fn resolve_credential_treats_empty_secret_as_ambient() {
        // Service-agnostic: both the forge (token-env) and git (helper) paths route
        // through this chokepoint, so a blank secret is ambient for either.
        for blank in ["", "   ", "\t\n"] {
            let client = ManagedClient::new("git")
                .with_credentials(Arc::new(StaticCredential::token(blank)));
            for service in [CredentialService::GitHub, CredentialService::Git] {
                assert!(
                    client
                        .resolve_credential(service, None)
                        .await
                        .unwrap()
                        .is_none(),
                    "blank secret {blank:?} → ambient (None) for {service:?}"
                );
            }
        }
    }
}
