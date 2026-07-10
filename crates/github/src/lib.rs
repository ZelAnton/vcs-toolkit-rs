#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-github` — automate GitHub from Rust by driving the `gh` CLI.
//!
//! You call typed `async` methods; `vcs-github` runs the real `gh`, parses its
//! output, and hands you structured values — so you get *gh's own* behaviour, auth,
//! and host resolution, not a reimplementation of the GitHub REST/GraphQL API.
//! Async, structured errors, mockable. Every command runs inside an OS **job** (an
//! OS-level container that kills the whole process tree if your program exits, via
//! [`processkit`]) so a `gh` subprocess is never orphaned, with an optional
//! per-client [timeout](GitHub::default_timeout). Read-style methods ask `gh` for
//! `--json` and deserialize it; nothing scrapes human-readable output.
//!
//! # What you can do
//!
//! Check auth · view the repo · the full pull-request lifecycle (list / view /
//! create / merge / mark-ready / close, review / comment, CI checks, feedback) ·
//! issues · releases · GitHub Actions runs (list / view / watch). One tiny call to
//! start:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_github::{GitHub, GitHubApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let gh = GitHub::new();
//! let prs = gh.pr_list(Path::new(".")).await?; // up to 100 open PRs
//! # let _ = prs; Ok(()) }
//! ```
//!
//! # The surface (engineering reference)
//!
//! - **[`GitHubApi`]** — the object-safe trait every operation lives on. Depend
//!   on `&dyn GitHubApi` (or generically on `impl GitHubApi`) so a test can swap
//!   the real client for a double. Repo-scoped methods take the working
//!   directory as the first argument and return typed results ([`PullRequest`],
//!   [`Issue`], [`RepoView`], [`CheckRun`], [`WorkflowRun`], [`Release`],
//!   [`PrFeedback`], …) or a structured [`Error`].
//! - **[`GitHub`]** — the real client. [`GitHub::new`] uses the job-backed
//!   runner; [`GitHub::with_runner`] injects a fake one for tests. It is generic
//!   over the [`ProcessRunner`] seam, defaulting to the production runner.
//!   [`with_credentials`](GitHub::with_credentials) attaches a
//!   [`CredentialProvider`] to supply a token per operation (injected as
//!   `GH_TOKEN`, never in `argv`) — opt-in, off by default (ambient `gh` auth).
//!   [`with_host`](GitHub::with_host) targets a specific host (a [`GitHubHost`] —
//!   github.com or a GitHub Enterprise Server host), so the credential lands in
//!   the env var `gh` reads for *that* host (`GH_TOKEN` vs `GH_ENTERPRISE_TOKEN`)
//!   and [`auth_status_for`](GitHubApi::auth_status_for) probes just that host.
//! - **[`GitHubAt`]** — a cwd-bound view ([`GitHub::at`]) whose methods drop the
//!   leading `dir`, so `gh.at(dir).pr_list()` reads as `gh.pr_list(dir)` — handy
//!   when one client drives one checkout.
//! - **Method groups** on the trait: PRs ([`pr_list`](GitHubApi::pr_list),
//!   [`pr_view`](GitHubApi::pr_view), [`pr_create`](GitHubApi::pr_create),
//!   [`pr_merge`](GitHubApi::pr_merge), [`pr_mark_ready`](GitHubApi::pr_mark_ready),
//!   [`pr_close`](GitHubApi::pr_close), [`pr_checkout`](GitHubApi::pr_checkout),
//!   [`pr_review`](GitHubApi::pr_review),
//!   [`pr_comment`](GitHubApi::pr_comment), [`pr_edit`](GitHubApi::pr_edit), [`pr_checks`](GitHubApi::pr_checks),
//!   [`pr_feedback`](GitHubApi::pr_feedback), [`pr_diff`](GitHubApi::pr_diff), …); Actions runs
//!   ([`run_list`](GitHubApi::run_list), [`run_view`](GitHubApi::run_view),
//!   [`run_watch`](GitHubApi::run_watch) — *blocking*, bounded by the client
//!   timeout); issues & releases ([`issue_create`](GitHubApi::issue_create),
//!   [`release_view`](GitHubApi::release_view), …); plus the escape hatches
//!   [`run`](GitHubApi::run) / [`api`](GitHubApi::api) for anything unmodelled.
//! - **Builder specs** for the multi-option commands — [`PrCreate`] (title/body
//!   with optional `head`/`base`), [`PrEdit`] (optional `title` and/or `body`
//!   for `pr edit`), [`PrMerge`] (strategy [`MergeStrategy`],
//!   `--auto`, `--delete-branch`), [`PrClose`] (optional `--delete-branch`), and
//!   [`ReviewAction`] (whose private fields make
//!   an empty-body request-changes unrepresentable) — each `#[non_exhaustive]`,
//!   built with a constructor and chained setters, named after the flags they emit.
//!
//! # Recipes
//!
//! Read state — depend on the trait so the same code takes a real client or a mock:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_github::{GitHub, GitHubApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let gh = GitHub::new();
//! let dir = Path::new(".");
//! let authed = gh.auth_status().await?;          // is `gh` logged in?
//! let open = gh.pr_list(dir).await?;             // up to 100 open PRs
//! # let _ = (authed, open); Ok(()) }
//! ```
//!
//! Mutate through the builder specs — open a PR, approve it, then squash-merge:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_github::{GitHub, GitHubApi, PrCreate, PrMerge, ReviewAction};
//! # async fn demo(gh: &GitHub) -> Result<(), processkit::Error> {
//! let dir = Path::new(".");
//! let url = gh.pr_create(dir, PrCreate::new("Add X", "…").base("main")).await?;
//! gh.pr_review(dir, 7, ReviewAction::approve().with_body("LGTM")).await?;
//! gh.pr_merge(dir, 7, PrMerge::squash().delete_branch()).await?;
//! # let _ = url; Ok(()) }
//! ```
//!
//! # Testing
//!
//! Two seams: enable the **`mock`** feature for a `mockall`-generated
//! `MockGitHubApi` (stub whole methods), or inject a
//! [`ScriptedRunner`](processkit::testing::ScriptedRunner) with [`GitHub::with_runner`]
//! to exercise the *real* argv-building and parsing against canned output — no
//! `gh` binary or network needed, so it runs on CI. The cross-cutting testing
//! patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # Safety
//!
//! Caller values placed in a bare positional argv slot (an `api` endpoint, a
//! release `tag`) are refused before spawning if empty or starting with `-` —
//! `gh` would parse them as flags. Flag-value slots (`--body <b>`,
//! `--branch <b>`) are consumed verbatim and need no guard.
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::path::Path;
use std::sync::Arc;

// The credential seam (the shared managed client behind `GitHub` is generated by
// `vcs_cli_support::managed_client!`) — re-exported so a consumer can supply a
// token provider.
pub use vcs_cli_support::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, FnProvider,
    OutputBudget, Secret, StaticCredential, provider_fn,
};
// Re-export the processkit types in this crate's public API, so consumers needn't
// depend on processkit directly — incl. `ProcessRunner` (the `with_runner`/
// `GitHub<R>` seam) and the `JobRunner` default. (Also brings
// `Error`/`Result`/`ProcessResult`/`ProcessRunner` into scope here.)
pub use processkit::{Error, JobRunner, ProcessResult, ProcessRunner, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency. (Cancellation is core in processkit
// 0.10 — always available, no feature.)
pub use processkit::CancellationToken;

mod parse;
pub use parse::{
    CheckBucket, CheckRun, Comment, Issue, PrFeedback, PullRequest, Release, RepoView, Review,
    WorkflowRun,
};
// Re-exported so `vcs_github::FileDiff` (and the types nested in it) resolve
// without a direct `vcs-diff` dependency — `pr_diff` returns `vcs-diff`'s model
// verbatim (`gh pr diff` emits the same git-format diff `git diff`/`jj diff
// --git` do; `crates/diff/src/diff.rs`'s parser is shared, not duplicated).
pub use vcs_diff::{ChangeKind, DiffLine, FileDiff, Hunk};
// The parsed `gh --version`, re-exported as `GitHubVersion` — the shared
// `major.minor.patch` type `vcs-git`/`vcs-jj` also gate on (an alias of
// `vcs_diff::Version`), so a consumer needn't name `vcs-diff` to read
// [`GitHubCapabilities::version`].
pub use vcs_diff::Version as GitHubVersion;

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "gh";

const PR_FIELDS: &str = "number,title,state,isDraft,headRefName,baseRefName,url,labels,assignees";
const REPO_FIELDS: &str = "name,owner,description,url,isPrivate,defaultBranchRef";
const ISSUE_LIST_FIELDS: &str = "number,title,state,body,url,labels,assignees";
const ISSUE_VIEW_FIELDS: &str = "number,title,state,body,url,labels,assignees";
const RUN_FIELDS: &str =
    "databaseId,name,displayTitle,status,conclusion,workflowName,headBranch,event,url,createdAt";
const CHECK_FIELDS: &str = "name,state,bucket,workflow,link,startedAt,completedAt";
const RELEASE_LIST_FIELDS: &str = "tagName,name,isLatest,isDraft,isPrerelease,publishedAt";
const RELEASE_VIEW_FIELDS: &str = "tagName,name,body,url,publishedAt,isDraft,isPrerelease";

/// Injection guard for bare positional argv slots: a caller-supplied value
/// with a leading `-` is parsed by gh's CLI as a *flag* (verified: `gh api -evil` →
/// flag parsing), and an empty value changes a command's
/// meaning. Refuse both before anything spawns. Flag-VALUE positions
/// (`--body <b>`, `--branch <b>`) need no guard — gh consumes the next token
/// verbatim there (verified).
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

/// The GitHub host an operation targets: SaaS `github.com` or a **GitHub
/// Enterprise Server** (GHES) host. `gh` picks the credential environment variable
/// it reads *per host* — `GH_TOKEN` for github.com, `GH_ENTERPRISE_TOKEN` for a
/// GHES host — and its `auth status` can be scoped to a single host, so this type
/// carries that host so the client (1) injects a supplied credential into the
/// variable `gh` actually reads for it (see [`GitHub::with_host`]) and (2) can
/// probe auth for exactly that host (see [`GitHubApi::auth_status_for`]).
///
/// Build it for github.com ([`github_com`](GitHubHost::github_com)), from a bare
/// hostname ([`new`](GitHubHost::new)), or from a repository's remote URL
/// ([`from_remote_url`](GitHubHost::from_remote_url)). A hostname that cannot be
/// determined is an **error**, never a silent fall back to github.com — so an
/// ambiguous or unknown host is a diagnosable result at the call site rather than
/// a quiet authentication against the wrong host with the github.com token.
///
/// ```
/// # use vcs_github::GitHubHost;
/// let saas = GitHubHost::github_com();
/// assert!(saas.is_github_com() && !saas.is_enterprise());
///
/// let ghes = GitHubHost::new("ghe.example.com").unwrap();
/// assert!(ghes.is_enterprise());
/// assert_eq!(ghes.as_str(), "ghe.example.com");
///
/// // github.com (any case) classifies as SaaS; every other valid host is GHES.
/// assert!(GitHubHost::new("GitHub.com").unwrap().is_github_com());
/// // An unparseable / hostless remote is an error, not a github.com guess.
/// assert!(GitHubHost::from_remote_url("not-a-url").is_err());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubHost {
    /// The canonical (lower-cased) hostname, e.g. `github.com` / `ghe.example.com`.
    host: String,
    /// `true` for a GitHub Enterprise Server host; `false` for SaaS github.com.
    enterprise: bool,
}

impl GitHubHost {
    /// The SaaS GitHub hostname (`github.com`).
    pub const SAAS_HOST: &'static str = "github.com";

    /// The SaaS github.com host — a supplied credential is injected as `GH_TOKEN`.
    #[must_use]
    pub fn github_com() -> Self {
        Self {
            host: Self::SAAS_HOST.to_string(),
            enterprise: false,
        }
    }

    /// Classify a bare `host`: `github.com` (case-insensitive) is SaaS; any other
    /// valid hostname is treated as a GitHub Enterprise Server host (its credential
    /// goes to `GH_ENTERPRISE_TOKEN`). Returns an error for an empty, flag-like, or
    /// otherwise malformed hostname (a scheme, path, port, userinfo, or whitespace)
    /// rather than guessing — the value must be a bare DNS-style host.
    pub fn new(host: impl AsRef<str>) -> Result<Self> {
        let host = validate_host(host.as_ref())?;
        let enterprise = host != Self::SAAS_HOST;
        Ok(Self { host, enterprise })
    }

    /// Derive the host from a repository **remote URL** and classify it. Handles
    /// `scheme://[user@]host[:port]/…` (HTTPS/SSH/…) and the scp-like
    /// `[user@]host:path` SSH form; any userinfo and port are dropped. A remote
    /// whose host can't be determined (unparseable, hostless, or ambiguous — an
    /// IPv6 literal, a bare single-label scp authority, a local path) is an
    /// **error**, not a silent github.com fallback, so the caller can surface an
    /// ambiguous remote as a diagnosable result.
    pub fn from_remote_url(url: &str) -> Result<Self> {
        match host_from_remote_url(url) {
            Some(host) => Self::new(host),
            None => Err(invalid_host_error(
                url,
                "no GitHub host could be determined from the remote URL",
            )),
        }
    }

    /// The canonical hostname (`github.com`, `ghe.example.com`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.host
    }

    /// Whether this is a GitHub Enterprise Server host (anything but github.com).
    #[must_use]
    pub fn is_enterprise(&self) -> bool {
        self.enterprise
    }

    /// Whether this is SaaS github.com.
    #[must_use]
    pub fn is_github_com(&self) -> bool {
        !self.enterprise
    }

    /// The environment variable `gh` reads for a credential on this host —
    /// `GH_TOKEN` for github.com, `GH_ENTERPRISE_TOKEN` for a GHES host. `'static`
    /// so it can seed the client's token-env binding.
    fn token_env_var(&self) -> &'static str {
        if self.enterprise {
            "GH_ENTERPRISE_TOKEN"
        } else {
            "GH_TOKEN"
        }
    }
}

/// Validate a bare gh hostname, returning it **lower-cased** (its canonical form —
/// hostnames are case-insensitive and `gh` stores them lower-cased). A host must
/// be a non-empty DNS-style name (ASCII letters/digits/`.`/`-`), not start with
/// `-`/`.` nor end with `.`, and carry no scheme, path, port, userinfo, or
/// whitespace. Anything else is refused as invalid input — `gh` would misread it,
/// or it is not a host at all.
fn validate_host(host: &str) -> Result<String> {
    let trimmed = host.trim();
    let well_formed = !trimmed.is_empty()
        && !trimmed.starts_with('-')
        && !trimmed.starts_with('.')
        && !trimmed.ends_with('.')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !well_formed {
        return Err(invalid_host_error(host, "not a valid GitHub hostname"));
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// The `Error::Spawn` / `InvalidInput` the crate raises for a rejected caller
/// value (the same shape as [`reject_flag_like`], classified by
/// `vcs_cli_support::is_invalid_input`), naming the bad host and why.
fn invalid_host_error(value: &str, reason: &str) -> Error {
    Error::spawn(
        BINARY,
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("GitHub host {value:?}: {reason}"),
        ),
    )
}

/// Extract the hostname from a repository remote URL (HTTPS / SSH / scp-like),
/// dropping any userinfo and port. Returns `None` when no unambiguous host is
/// present, so [`GitHubHost::from_remote_url`] surfaces a diagnosable error rather
/// than defaulting to github.com. An IPv6-literal authority (`[::1]`) and a bare
/// single-label scp authority (indistinguishable from a Windows drive path) return
/// `None` too — a GitHub host is a dotted DNS name.
fn host_from_remote_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // scheme://[user@]host[:port]/…  (https, http, ssh, git, …). The authority
    // ends at the first `/`, `?`, or `#`; drop any `user:pass@` userinfo.
    if let Some((_scheme, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        return strip_port(host_port);
    }
    // scp-like SSH: `[user@]host:path` (no scheme). The host ends at the first `:`.
    if let Some((authority, _path)) = url.split_once(':') {
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        // Require a dotted host so a Windows drive path (`C:\…`) or a bare
        // single-label authority isn't misread as a remote host — those are
        // ambiguous, and the caller gets a diagnosable error instead of a guess.
        if host.contains('.') && !host.contains('/') && !host.contains('\\') {
            return Some(host.to_string());
        }
    }
    None
}

/// Drop a trailing `:port` from `host[:port]`, refusing an IPv6-literal authority
/// (`[::1]`) — a GitHub host is never a bracketed literal, and gh names hosts
/// without a port.
fn strip_port(host_port: &str) -> Option<String> {
    if host_port.is_empty() || host_port.starts_with('[') {
        return None;
    }
    Some(
        host_port
            .split_once(':')
            .map_or(host_port, |(h, _)| h)
            .to_string(),
    )
}

/// How [`GitHubApi::pr_merge`] merges the PR — exactly one of gh's mutually
/// exclusive strategy flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeStrategy {
    /// A merge commit (`--merge`).
    Merge,
    /// Squash into one commit (`--squash`).
    Squash,
    /// Rebase the commits onto the base (`--rebase`).
    Rebase,
}

impl MergeStrategy {
    fn flag(self) -> &'static str {
        match self {
            MergeStrategy::Merge => "--merge",
            MergeStrategy::Squash => "--squash",
            MergeStrategy::Rebase => "--rebase",
        }
    }
}

/// Options for [`GitHubApi::pr_merge`] (`gh pr merge`).
///
/// `#[non_exhaustive]`, so build it through the strategy constructors —
/// [`merge`](PrMerge::merge) / [`squash`](PrMerge::squash) /
/// [`rebase`](PrMerge::rebase), then [`auto`](PrMerge::auto) /
/// [`delete_branch`](PrMerge::delete_branch) — rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PrMerge {
    /// The merge strategy (exactly one of gh's `--merge`/`--squash`/`--rebase`).
    pub strategy: MergeStrategy,
    /// Enable auto-merge: merge once requirements are met (`--auto`).
    pub auto: bool,
    /// Delete the head branch after the merge (`--delete-branch`).
    pub delete_branch: bool,
}

impl PrMerge {
    /// Merge with a merge commit (`gh pr merge --merge`).
    pub fn merge() -> Self {
        Self::with(MergeStrategy::Merge)
    }

    /// Squash-merge (`gh pr merge --squash`).
    pub fn squash() -> Self {
        Self::with(MergeStrategy::Squash)
    }

    /// Rebase-merge (`gh pr merge --rebase`).
    pub fn rebase() -> Self {
        Self::with(MergeStrategy::Rebase)
    }

    fn with(strategy: MergeStrategy) -> Self {
        Self {
            strategy,
            auto: false,
            delete_branch: false,
        }
    }

    /// Merge automatically once requirements are met (`--auto`).
    pub fn auto(mut self) -> Self {
        self.auto = true;
        self
    }

    /// Delete the head branch after merging (`--delete-branch`).
    pub fn delete_branch(mut self) -> Self {
        self.delete_branch = true;
        self
    }
}

/// Options for [`GitHubApi::pr_close`] (`gh pr close`).
///
/// `#[non_exhaustive]`, so build it through [`PrClose::new`] and the chained
/// [`delete_branch`](PrClose::delete_branch) setter rather than a bare `bool`
/// (`pr_close(n, true)` doesn't say what `true` does).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrClose {
    /// Delete the head branch after closing the PR (`--delete-branch`).
    pub delete_branch: bool,
}

impl PrClose {
    /// Close the PR, leaving the head branch in place.
    pub fn new() -> Self {
        Self::default()
    }

    /// Delete the head branch after closing (`--delete-branch`).
    pub fn delete_branch(mut self) -> Self {
        self.delete_branch = true;
        self
    }
}

/// Options for [`GitHubApi::pr_create`] (`gh pr create`).
///
/// `#[non_exhaustive]`, so build it through [`PrCreate::new`] (title + body)
/// and the chained [`head`](PrCreate::head) / [`base`](PrCreate::base) setters
/// rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PrCreate {
    /// The PR title (`--title`).
    pub title: String,
    /// The PR body (`--body`).
    pub body: String,
    /// The source branch (`--head`); `None` = the current branch.
    pub head: Option<String>,
    /// The target branch (`--base`); `None` = the repo default.
    pub base: Option<String>,
}

impl PrCreate {
    /// A PR with the given title and body, opened from the current branch into
    /// the repo default (`gh pr create --title <title> --body <body>`).
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            head: None,
            base: None,
        }
    }

    /// Set the source branch (`--head`).
    pub fn head(mut self, head: impl Into<String>) -> Self {
        self.head = Some(head.into());
        self
    }

    /// Set the target branch (`--base`).
    pub fn base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }
}

/// Options for [`GitHubApi::pr_edit`] (`gh pr edit`).
///
/// `#[non_exhaustive]`, so build it through [`PrEdit::new`] and the chained
/// [`title`](PrEdit::title) / [`body`](PrEdit::body) setters rather than a
/// struct literal. At least one of `title` or `body` must be `Some`; both
/// `None` is rejected by the facade before spawning (an explicit error, not a
/// silent no-op). An empty string is a real value — gh clears the field on
/// `--title ""` / `--body ""` — not a `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrEdit {
    /// The new title (`--title`); `None` leaves the title alone.
    pub title: Option<String>,
    /// The new body (`--body`); `None` leaves the body alone.
    pub body: Option<String>,
}

impl PrEdit {
    /// An edit that leaves both fields alone (the facade rejects both-`None`
    /// before reaching the wrapper). Start with this and add what you want to
    /// change via [`title`](PrEdit::title) / [`body`](PrEdit::body).
    pub fn new() -> Self {
        Self {
            title: None,
            body: None,
        }
    }

    /// Set the new title (`--title`).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the new body (`--body`).
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl Default for PrEdit {
    fn default() -> Self {
        Self::new()
    }
}

/// Which kind of review [`GitHubApi::pr_review`] submits — match on
/// [`ReviewAction::kind`] to read it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReviewKind {
    /// Approve (`--approve`).
    Approve,
    /// Request changes (`--request-changes`).
    RequestChanges,
    /// A comment-only review (`--comment`).
    Comment,
}

/// What [`GitHubApi::pr_review`] submits (`gh pr review`).
///
/// The fields are **private** so the invariant holds by construction: gh
/// *requires* a body for request-changes/comment reviews, so those are only
/// reachable through [`request_changes`](ReviewAction::request_changes) /
/// [`comment`](ReviewAction::comment), which both take the body — an empty-body
/// request-changes is unrepresentable. Approve's body is optional
/// ([`approve`](ReviewAction::approve) starts with none; attach one with
/// [`with_body`](ReviewAction::with_body)). Read the parts back via
/// [`kind`](ReviewAction::kind) / [`body`](ReviewAction::body).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReviewAction {
    kind: ReviewKind,
    body: Option<String>,
}

impl ReviewAction {
    /// Approve, with no body (`--approve`). Attach one with
    /// [`with_body`](ReviewAction::with_body).
    pub fn approve() -> Self {
        Self {
            kind: ReviewKind::Approve,
            body: None,
        }
    }

    /// Request changes; gh requires the body
    /// (`--request-changes --body <body>`).
    pub fn request_changes(body: impl Into<String>) -> Self {
        Self {
            kind: ReviewKind::RequestChanges,
            body: Some(body.into()),
        }
    }

    /// A comment-only review; gh requires the body (`--comment --body <body>`).
    pub fn comment(body: impl Into<String>) -> Self {
        Self {
            kind: ReviewKind::Comment,
            body: Some(body.into()),
        }
    }

    /// Attach or replace the body — mainly to give an [`approve`](ReviewAction::approve)
    /// a message.
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Which kind of review this is.
    pub fn kind(&self) -> ReviewKind {
        self.kind
    }

    /// The review body, if any.
    pub fn body(&self) -> Option<&str> {
        self.body.as_deref()
    }
}

/// What the installed `gh` binary supports, probed via
/// [`GitHubApi::capabilities`]. A value type — the client holds no state, so
/// probe once and keep the result (callers cache it). Mirrors
/// [`vcs_git::GitCapabilities`](../vcs_git/struct.GitCapabilities.html) /
/// [`vcs_jj::JjCapabilities`](../vcs_jj/struct.JjCapabilities.html).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitHubCapabilities {
    /// The binary's parsed version.
    pub version: GitHubVersion,
}

/// The oldest `gh` this crate is written against — **2.0.0**, the first release of
/// the modern `gh` line. Every command this crate's argv drives lives in 2.x: the
/// `--json` read surface (`pr`/`issue`/`repo`/`release … --json`, incl.
/// `pr checks --json`), the `pr edit` / `pr checkout` / `pr ready` lifecycle verbs,
/// and `api`. A `gh` from the 1.x line is missing parts of that surface, so gating
/// here lets [`ensure_supported`](GitHubCapabilities::ensure_supported) reject a
/// too-old binary up front with a clear message instead of letting an operation
/// fail deep inside gh with a cryptic `unknown command`/`unknown flag`.
const MIN_SUPPORTED: GitHubVersion = GitHubVersion {
    major: 2,
    minor: 0,
    patch: 0,
};

impl GitHubCapabilities {
    /// Whether the binary meets the supported floor (gh ≥ 2.0). Every typed
    /// operation on [`GitHubApi`] is guaranteed against this minimum.
    pub fn is_supported(&self) -> bool {
        self.version >= MIN_SUPPORTED
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs gh ≥ 2.0,
    /// found 1.14.0" instead of a cryptic `unknown command`/`unknown flag` failure
    /// once an operation reaches a command the old binary lacks. The pre-flight
    /// check a caller runs before driving operations against an untrusted `gh`.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-github requires gh >= {MIN_SUPPORTED}, found {}",
                    self.version
                ),
            ),
        ))
    }
}

/// The GitHub operations this crate exposes — the interface consumers code
/// against and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitHubApi: Send + Sync {
    /// Run `gh <args>` **in the process's current directory**, returning trimmed
    /// stdout (throws on a non-zero exit). A raw escape hatch — you supply the whole
    /// argv, so pass `-R owner/repo` to target a specific repo. This method on the
    /// client is the **process-cwd** escape hatch; the `at(dir)` bound view's
    /// [`run`](GitHubAt::run) is instead **bound to `dir`** (it forwards to
    /// [`GitHub::run_in`], so `gh.at(dir).run(…)` runs in the bound repo's cwd, like
    /// [`api`](GitHubApi::api)). Use `gh.at(dir).run(…)` (or [`GitHub::run_in`]) for
    /// the bound repo (T-035).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitHubApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed GitHub CLI version (`gh --version`).
    async fn version(&self) -> Result<String>;
    /// The installed binary's parsed version, as [`GitHubCapabilities`]
    /// (`gh --version`). A value type — probe once and keep it; an unrecognisable
    /// version banner is an [`Error::Parse`]. Gate an operation on a minimum `gh`
    /// with [`GitHubCapabilities::ensure_supported`].
    async fn capabilities(&self) -> Result<GitHubCapabilities>;
    /// Whether the user is authenticated (`gh auth status` exits zero). Reflects
    /// the exit code as a bool — any non-zero exit reads as `false`, never an
    /// error; only a spawn failure or timeout errors. Unscoped: it inspects
    /// *every* configured host, so a broken session for one host can make it
    /// report `false` even when the host you care about is fine — reach for
    /// [`auth_status_for`](GitHubApi::auth_status_for) to scope it.
    async fn auth_status(&self) -> Result<bool>;
    /// Whether the user is authenticated **for `host`** (`gh auth status
    /// --hostname <host>` exits zero) — the host-scoped twin of
    /// [`auth_status`](GitHubApi::auth_status). Scoping to the repository's host
    /// (build a [`GitHubHost`] from its remote, e.g.
    /// [`GitHubHost::from_remote_url`]) means a broken or absent session for
    /// *another* host can't turn this into a false negative for the host you
    /// target. Like `auth_status`, it folds only the exit code into the bool (any
    /// non-zero exit → `false`); a spawn failure or timeout still errors.
    /// **Defaulted** to `Error::Unsupported` so external implementers of the trait
    /// keep compiling when the crate bumps (only the `GitHub` concrete impl and the
    /// regenerated `MockGitHubApi` override it).
    #[allow(unused_variables)]
    async fn auth_status_for(&self, host: &GitHubHost) -> Result<bool> {
        Err(Error::Unsupported {
            operation: "auth_status_for".into(),
        })
    }
    /// The repository for `dir` (`gh repo view --json …`).
    async fn repo_view(&self, dir: &Path) -> Result<RepoView>;
    /// Pull requests for `dir` (`gh pr list --limit 100 --json …`). Returns up to
    /// 100 open PRs; use [`run`](GitHubApi::run) for more.
    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>>;
    /// Pull requests that merge `head` into `base`, in any state — open, closed,
    /// or merged (`gh pr list --head <head> --base <base> --state all --limit 100
    /// --json …`). Each carries its title, URL, and `state`. Empty when none
    /// match; returns up to 100 (use [`run`](GitHubApi::run) for more).
    async fn pr_list_for_branch(
        &self,
        dir: &Path,
        head: &str,
        base: &str,
    ) -> Result<Vec<PullRequest>>;
    /// A single pull request by number (`gh pr view <n> --json …`).
    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest>;
    /// Issues for `dir` (`gh issue list --limit 100 --json …`). Returns up to 100
    /// open issues; use [`run`](GitHubApi::run) for more.
    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>>;
    /// Open a pull request, returning its URL (`gh pr create`) — see
    /// [`PrCreate`] for the title/body and the optional `head` (source branch;
    /// `None` = current branch) / `base` (target; `None` = repo default).
    async fn pr_create(&self, dir: &Path, spec: PrCreate) -> Result<String>;
    /// Raw GitHub REST/GraphQL response body (`gh api <endpoint>`), run in `dir` so
    /// a relative endpoint's `{owner}/{repo}` placeholder resolves against the bound
    /// repository — not whatever repo the process's current directory happens to be in.
    async fn api(&self, dir: &Path, endpoint: &str) -> Result<String>;

    // --- PR lifecycle ----------------------------------------------------

    /// Merge a pull request (`gh pr merge <n> --merge|--squash|--rebase
    /// [--auto] [--delete-branch]`) — see [`PrMerge`].
    async fn pr_merge(&self, dir: &Path, number: u64, merge: PrMerge) -> Result<()>;
    /// Mark a draft pull request as ready for review (`gh pr ready <n>`).
    async fn pr_mark_ready(&self, dir: &Path, number: u64) -> Result<()>;
    /// Close a pull request without merging (`gh pr close <n>
    /// [--delete-branch]`); see [`PrClose`].
    async fn pr_close(&self, dir: &Path, number: u64, spec: PrClose) -> Result<()>;
    /// Check out a pull request's branch into the working copy at `dir`
    /// (`gh pr checkout <n>`) — the head branch is fetched and switched to, so a
    /// subsequent build/test/edit runs against the PR locally. Mutates the working
    /// copy. **Defaulted** to `Error::Unsupported` so external implementers of the
    /// trait keep compiling when the crate bumps (only the `GitHub` concrete impl
    /// and the regenerated `MockGitHubApi` override it).
    #[allow(unused_variables)]
    async fn pr_checkout(&self, dir: &Path, number: u64) -> Result<()> {
        Err(Error::Unsupported {
            operation: "pr_checkout".into(),
        })
    }
    /// The PR's checks (`gh pr checks <n> --json …`). gh signals the overall
    /// outcome through its exit code — 0 all passed, 8 still pending, 1 some
    /// failed — and emits the same JSON either way, so all three return the
    /// parsed list; branch on each entry's [`bucket`](CheckRun::bucket). A PR
    /// with no checks at all yields an empty list (gh's "no checks reported"
    /// exit). Any other exit (no such PR, auth required, …) errors.
    async fn pr_checks(&self, dir: &Path, number: u64) -> Result<Vec<CheckRun>>;
    /// Submit a review (`gh pr review <n> --approve|--request-changes|--comment
    /// [--body <body>]`) — see [`ReviewAction`] (request-changes/comment carry a
    /// required body by construction).
    async fn pr_review(&self, dir: &Path, number: u64, action: ReviewAction) -> Result<()>;
    /// Add a conversation comment, returning its URL
    /// (`gh pr comment <n> --body <body>`).
    async fn pr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String>;
    /// Edit a pull request's title and/or body
    /// (`gh pr edit <n> [--title <title>] [--body <body>]`). At least one of
    /// `title` or `body` must be `Some` — the facade rejects both-`None`
    /// before reaching the wrapper, so the default implementation is
    /// unreachable in normal use. **Defaulted** to `Error::Unsupported` so
    /// external implementers of the trait keep compiling when the crate
    /// bumps.
    #[allow(unused_variables)]
    async fn pr_edit(&self, dir: &Path, number: u64, edit: PrEdit) -> Result<()> {
        Err(Error::Unsupported {
            operation: "pr_edit".into(),
        })
    }
    /// The PR's submitted reviews and conversation comments
    /// (`gh pr view <n> --json reviews,comments`).
    async fn pr_feedback(&self, dir: &Path, number: u64) -> Result<PrFeedback>;
    /// The PR's diff, one [`FileDiff`] per changed file (`gh pr diff <n>
    /// --color never`), through the same unified-diff parser
    /// [`vcs-git`](https://docs.rs/vcs-git)/[`vcs-jj`](https://docs.rs/vcs-jj)
    /// use — `gh pr diff` emits the same git-format diff `git diff` does.
    async fn pr_diff(&self, dir: &Path, number: u64) -> Result<Vec<FileDiff>>;

    // --- Actions runs ------------------------------------------------------

    /// Recent workflow runs, newest first (`gh run list --limit <n>
    /// [--branch <b>] --json …`). `branch` is an owned `Option<String>` to keep
    /// the trait `mockall`-friendly.
    async fn run_list(
        &self,
        dir: &Path,
        limit: u64,
        branch: Option<String>,
    ) -> Result<Vec<WorkflowRun>>;
    /// A single workflow run by id (`gh run view <id> --json …`); the id is
    /// [`WorkflowRun::database_id`].
    async fn run_view(&self, dir: &Path, id: u64) -> Result<WorkflowRun>;
    /// Block until the run finishes, then return its final state
    /// (`gh run watch <id>`, then a `run view`). Inspect
    /// [`conclusion`](WorkflowRun::conclusion) for the outcome — exit codes
    /// can't distinguish a failed run from a cancelled one.
    ///
    /// **Blocks for the whole run.** A client
    /// [`default_timeout`](GitHub::default_timeout) kills the watch when it
    /// elapses (`Error::Timeout`) — drive this from a client with no (or a
    /// generous) timeout.
    async fn run_watch(&self, dir: &Path, id: u64) -> Result<WorkflowRun>;

    // --- Issues / releases ---------------------------------------------------

    /// Open an issue, returning its URL
    /// (`gh issue create --title <title> --body <body>`).
    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String>;
    /// A single issue by number, with `body`/`url` filled
    /// (`gh issue view <n> --json …`).
    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue>;
    /// Releases, newest first (`gh release list --limit 100 --json …`); `body`/`url`
    /// are not fetched here — use [`release_view`](GitHubApi::release_view).
    /// Returns up to 100 releases; use [`run`](GitHubApi::run) for more.
    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>>;
    /// A single release by tag, with `body`/`url` filled
    /// (`gh release view <tag> --json …`). gh reports `is_latest` only from
    /// [`release_list`](GitHubApi::release_list); here it defaults to `false`.
    async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release>;
}

vcs_cli_support::managed_client! {
    /// The real GitHub client. Generic over the [`ProcessRunner`] so tests can inject
    /// a fake process executor; [`GitHub::new`] uses the real job-backed runner.
    ///
    /// Wraps a [`ManagedClient`](vcs_cli_support::ManagedClient). By default it authenticates through `gh`'s own
    /// ambient login; attach a [`CredentialProvider`] with
    /// [`with_credentials`](GitHub::with_credentials) to supply a token per operation
    /// — it is injected as `GH_TOKEN` on every `gh` invocation (or, after
    /// [`with_host`](GitHub::with_host) targets a GitHub Enterprise Server host,
    /// as `GH_ENTERPRISE_TOKEN` — the variable `gh` reads for that host).
    pub struct GitHub => BINARY, token_env = (CredentialService::GitHub, "GH_TOKEN")
}

impl<R: ProcessRunner> GitHub<R> {
    /// Supply credentials per operation via a [`CredentialProvider`] — opt-in, off
    /// by default (ambient `gh` auth). The resolved token is injected as `GH_TOKEN`
    /// on every `gh` invocation, overriding the ambient login for this client.
    #[must_use]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.core = self.core.with_credentials(provider);
        self
    }

    /// Convenience for the common case: authenticate with a single static `token`,
    /// injected as `GH_TOKEN`. Shorthand for
    /// `with_credentials(Arc::new(StaticCredential::token(token)))`.
    #[must_use]
    pub fn with_token(self, token: impl Into<Secret>) -> Self {
        self.with_credentials(Arc::new(StaticCredential::token(token)))
    }

    /// Convenience: read the token from environment variable `var` at request time
    /// (injected as `GH_TOKEN`); if `var` is unset/empty, fall back to ambient auth.
    /// Shorthand for `with_credentials(Arc::new(EnvToken::new(var)))`.
    #[must_use]
    pub fn with_env_token(self, var: impl Into<String>) -> Self {
        self.with_credentials(Arc::new(EnvToken::new(var)))
    }

    /// Bind this client to a GitHub `host`, so a supplied credential is injected
    /// into the environment variable `gh` reads for **that** host, and gh's default
    /// host is set accordingly:
    ///
    /// - **github.com** ([`GitHubHost::github_com`]) → the token goes to `GH_TOKEN`
    ///   (the SaaS default, unchanged) and `GH_HOST` is `github.com`.
    /// - a **GitHub Enterprise Server** host → the token goes to
    ///   `GH_ENTERPRISE_TOKEN` (the variable `gh` uses for a non-github.com host)
    ///   and `GH_HOST` is set to that host, so gh's non-repo commands resolve
    ///   against it. The github.com `GH_TOKEN` is **not** set, so an enterprise
    ///   secret never lands in the github.com token env (nor vice versa).
    ///
    /// Compose with [`with_credentials`](GitHub::with_credentials) /
    /// [`with_token`](GitHub::with_token) / [`with_env_token`](GitHub::with_env_token)
    /// in either order — the host selects the env var, the provider supplies the
    /// secret. The bound host also travels in each operation's [`CredentialRequest`],
    /// so a **host-keyed** provider returns the secret for *this* host and never a
    /// neighbouring instance's. For several hosts, build **one client per host**:
    /// each injects only its own host's token, so a broken or missing credential for
    /// one host can't leak into another. Without a host binding the client behaves
    /// exactly as before — github.com semantics, credential injected as `GH_TOKEN`,
    /// and the request carries no host (a host-keyed provider that can't place it
    /// defers to ambient auth).
    ///
    /// `GH_HOST` only steers gh's host inference for commands with **no repository
    /// context**; a repo-scoped command still resolves its host from the working
    /// directory's remote, so binding a host does not override a repo you point a
    /// method at — use a host-bound client with repositories on that host.
    #[must_use]
    pub fn with_host(mut self, host: GitHubHost) -> Self {
        self.core = self
            .core
            .with_token_env(CredentialService::GitHub, host.token_env_var())
            // Carry the (canonical, lower-cased) host into every operation's
            // `CredentialRequest`, so a host-keyed `CredentialProvider` resolves the
            // secret for *this* host and nothing else — one instance's token can't
            // land in another host's `gh` command.
            .with_expected_host(host.as_str())
            .default_env("GH_HOST", host.as_str());
        self
    }
}

#[async_trait::async_trait]
impl<R: ProcessRunner> GitHubApi for GitHub<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(args).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(["--version"]).await
    }

    async fn capabilities(&self) -> Result<GitHubCapabilities> {
        let raw = self.version().await?;
        let version = parse::parse_gh_version(&raw).ok_or_else(|| {
            Error::parse(
                BINARY,
                format!("unrecognisable `gh --version` output: {raw:?}"),
            )
        })?;
        Ok(GitHubCapabilities { version })
    }

    async fn auth_status(&self) -> Result<bool> {
        // `gh auth status` exits 0 when authenticated, non-zero when not — an
        // exit-code answer. `exit_code` reads the exit code without erroring on a
        // non-zero one (a spawn failure or timeout still errors), so ANY non-zero
        // exit — not just the documented 1 — maps to "not authenticated" rather
        // than surfacing as an error. `probe` would reject an unusual exit code.
        Ok(self.core.exit_code(["auth", "status"]).await? == 0)
    }

    async fn auth_status_for(&self, host: &GitHubHost) -> Result<bool> {
        // `--hostname <host>` scopes the probe to one host: `gh auth status` with
        // no hostname inspects *every* configured host, so a single broken session
        // (a different host, an expired enterprise login) can flip the exit code
        // non-zero — a false negative for the host we actually target. Same
        // exit-code-as-bool contract as `auth_status` (a spawn failure or timeout
        // still errors — see `exit_code`). `host` is a validated `GitHubHost`, so
        // the `--hostname` value can never be flag-like or empty.
        Ok(self
            .core
            .exit_code(["auth", "status", "--hostname", host.as_str()])
            .await?
            == 0)
    }

    async fn repo_view(&self, dir: &Path) -> Result<RepoView> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["repo", "view", "--json", REPO_FIELDS]),
                parse::parse_repo,
            )
            .await
    }

    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["pr", "list", "--limit", "100", "--json", PR_FIELDS]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn pr_list_for_branch(
        &self,
        dir: &Path,
        head: &str,
        base: &str,
    ) -> Result<Vec<PullRequest>> {
        // `--state all` so a closed/merged PR for this branch pair is reported
        // too, not just open ones (gh's default); the caller filters on `state`.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "pr", "list", "--head", head, "--base", base, "--state", "all", "--limit",
                        "100", "--json", PR_FIELDS,
                    ],
                ),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
        let n = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["pr", "view", n.as_str(), "--json", PR_FIELDS]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "issue",
                        "list",
                        "--limit",
                        "100",
                        "--json",
                        ISSUE_LIST_FIELDS,
                    ],
                ),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn pr_create(&self, dir: &Path, spec: PrCreate) -> Result<String> {
        let mut args = vec![
            "pr",
            "create",
            "--title",
            spec.title.as_str(),
            "--body",
            spec.body.as_str(),
        ];
        if let Some(head) = spec.head.as_deref() {
            args.push("--head");
            args.push(head);
        }
        if let Some(base) = spec.base.as_deref() {
            args.push("--base");
            args.push(base);
        }
        self.core.run(self.core.command_in(dir, args)).await
    }

    async fn api(&self, dir: &Path, endpoint: &str) -> Result<String> {
        reject_flag_like("endpoint", endpoint)?;
        self.core
            .run(self.core.command_in(dir, ["api", endpoint]))
            .await
    }

    async fn pr_merge(&self, dir: &Path, number: u64, merge: PrMerge) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "merge", n.as_str(), merge.strategy.flag()];
        if merge.auto {
            args.push("--auto");
        }
        if merge.delete_branch {
            args.push("--delete-branch");
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn pr_mark_ready(&self, dir: &Path, number: u64) -> Result<()> {
        let n = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["pr", "ready", n.as_str()]))
            .await
    }

    async fn pr_close(&self, dir: &Path, number: u64, spec: PrClose) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "close", n.as_str()];
        if spec.delete_branch {
            args.push("--delete-branch");
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn pr_checkout(&self, dir: &Path, number: u64) -> Result<()> {
        // `number` is a `u64`, so it can never look like a flag — nothing to
        // guard with `reject_flag_like`. `gh pr checkout` fetches the PR's head
        // branch and switches the working copy to it (no structured output).
        let n = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["pr", "checkout", n.as_str()]))
            .await
    }

    async fn pr_checks(&self, dir: &Path, number: u64) -> Result<Vec<CheckRun>> {
        let n = number.to_string();
        let res = self
            .core
            .output_string(
                self.core
                    .command_in(dir, ["pr", "checks", n.as_str(), "--json", CHECK_FIELDS]),
            )
            .await?;
        match res.code() {
            // gh's exit code carries the *overall* outcome (0 = all pass,
            // 8 = pending, 1 = some failed) but prints the same JSON for all
            // three — parse it and let the caller branch on each `bucket`.
            // A parse failure here is a real schema problem and must surface
            // as `Error::Parse`, not be masked by the exit code.
            Some(0) => vcs_cli_support::json::from_json(BINARY, res.stdout()),
            Some(1 | 8) if !res.stdout().trim().is_empty() => {
                vcs_cli_support::json::from_json(BINARY, res.stdout())
            }
            // gh exits 1 with NO JSON for a PR that simply has no checks — the
            // one bare non-zero we read as an empty list (cf. jj's
            // `resolve_list` and its "No conflicts" exit). Matched
            // case-insensitively so a capitalization tweak in gh's wording
            // ("no checks reported on the 'X' branch") doesn't turn the empty case
            // into a hard error.
            _ if res
                .stderr()
                .to_ascii_lowercase()
                .contains("no checks reported") =>
            {
                Ok(Vec::new())
            }
            // Anything else (no such PR, auth required, timeout, signal…) is a
            // genuine failure; `ensure_success` builds the faithful error.
            _ => {
                let _ = res.ensure_success()?;
                Ok(Vec::new()) // unreachable: a non-zero exit always errors above.
            }
        }
    }

    async fn pr_review(&self, dir: &Path, number: u64, action: ReviewAction) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "review", n.as_str()];
        args.push(match action.kind() {
            ReviewKind::Approve => "--approve",
            ReviewKind::RequestChanges => "--request-changes",
            ReviewKind::Comment => "--comment",
        });
        if let Some(body) = action.body() {
            args.push("--body");
            args.push(body);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn pr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String> {
        // `--body` is mandatory here: without it gh falls back to an
        // interactive prompt, which would hang a headless run.
        let n = number.to_string();
        self.core
            .run(
                self.core
                    .command_in(dir, ["pr", "comment", n.as_str(), "--body", body]),
            )
            .await
    }

    async fn pr_edit(&self, dir: &Path, number: u64, edit: PrEdit) -> Result<()> {
        // `--title` and `--body` are flag-VALUE positions: gh consumes the
        // next token verbatim, so the leading-`-` check is not needed here.
        // The facade rejects both-`None` before reaching this; an empty string
        // is intentional (clears the field). We still skip absent fields so
        // the argv doesn't carry a stray `--title` with no value.
        let n = number.to_string();
        let mut args = vec!["pr", "edit", n.as_str()];
        if let Some(title) = edit.title.as_deref() {
            args.push("--title");
            args.push(title);
        }
        if let Some(body) = edit.body.as_deref() {
            args.push("--body");
            args.push(body);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn pr_feedback(&self, dir: &Path, number: u64) -> Result<PrFeedback> {
        let n = number.to_string();
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    ["pr", "view", n.as_str(), "--json", "reviews,comments"],
                ),
                parse::parse_feedback,
            )
            .await
    }

    async fn pr_diff(&self, dir: &Path, number: u64) -> Result<Vec<FileDiff>> {
        self.pr_diff_within(dir, number, self.core.output_budget())
            .await
    }

    async fn run_list(
        &self,
        dir: &Path,
        limit: u64,
        branch: Option<String>,
    ) -> Result<Vec<WorkflowRun>> {
        let limit = limit.to_string();
        let mut args = vec!["run", "list", "--limit", limit.as_str()];
        if let Some(branch) = branch.as_deref() {
            args.push("--branch");
            args.push(branch);
        }
        args.extend(["--json", RUN_FIELDS]);
        self.core
            .try_parse(self.core.command_in(dir, args), |s| {
                vcs_cli_support::json::from_json(BINARY, s)
            })
            .await
    }

    async fn run_view(&self, dir: &Path, id: u64) -> Result<WorkflowRun> {
        let id = id.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["run", "view", id.as_str(), "--json", RUN_FIELDS]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn run_watch(&self, dir: &Path, id: u64) -> Result<WorkflowRun> {
        // Block until the run completes. `--exit-status` is deliberately NOT
        // passed: it would map the run's outcome onto the exit code (1 failed,
        // 2 cancelled), which can't be reported faithfully — the follow-up
        // `run view`'s `conclusion` can. Without it, a non-zero watch exit is a
        // genuine error (no such run, auth, …). `output_string` does NOT error on a
        // timeout (it returns the result with a timeout flag), so
        // `ensure_success` is what surfaces a killed watch as `Error::Timeout`
        // instead of reading a half-finished run below.
        let id_str = id.to_string();
        // `gh run watch` re-prints the full job table every ~3 s until the run ends,
        // so over a multi-hour run its stdout grows to tens of MB — all of which we
        // discard (only the exit status matters; the result comes from `run_view`).
        // Bound the retained buffer (drop-oldest) so a long watch can't accumulate
        // unboundedly; the last 256 lines / 256 KiB are plenty for a failure message.
        // (`docs/audit-2026-07.md` R5.)
        //
        // Expressed through the shared [`OutputBudget`] so this fixed watch cap and
        // the configurable content-op budget are the *same* mechanism (T-049): this
        // is the drop-oldest *diagnostic* projection (`diagnostic_policy`) — a bounded
        // tail that never turns a real watch failure into `OutputTooLarge` — not the
        // fail-loud *content* projection the diff/show verbs use.
        let watch_budget = OutputBudget::bytes(256 * 1024).with_max_lines(256);
        let cmd = self
            .core
            .command_in(dir, ["run", "watch", id_str.as_str()])
            .output_buffer(
                watch_budget
                    .diagnostic_policy()
                    .expect("a byte/line budget yields a diagnostic policy"),
            );
        let _ = self.core.output_string(cmd).await?.ensure_success()?;
        self.run_view(dir, id).await
    }

    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String> {
        self.core
            .run(
                self.core
                    .command_in(dir, ["issue", "create", "--title", title, "--body", body]),
            )
            .await
    }

    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue> {
        let n = number.to_string();
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    ["issue", "view", n.as_str(), "--json", ISSUE_VIEW_FIELDS],
                ),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>> {
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "release",
                        "list",
                        "--limit",
                        "100",
                        "--json",
                        RELEASE_LIST_FIELDS,
                    ],
                ),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release> {
        reject_flag_like("tag", tag)?;
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["release", "view", tag, "--json", RELEASE_VIEW_FIELDS]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }
}

impl<R: ProcessRunner> GitHub<R> {
    /// [`pr_diff`](GitHubApi::pr_diff) with an explicit per-call [`OutputBudget`],
    /// instead of this client's [`default_output_budget`](GitHub::default_output_budget).
    /// Past the ceiling the read errors with
    /// [`Error::OutputTooLarge`] (actual and
    /// allowed sizes) rather than buffering an unbounded diff — the override for a
    /// legitimately huge PR.
    pub async fn pr_diff_within(
        &self,
        dir: &Path,
        number: u64,
        budget: OutputBudget,
    ) -> Result<Vec<FileDiff>> {
        // `run_untrimmed_within`: a diff's trailing content is meaningful (a hunk's
        // last line, a missing trailing newline) — trimming it before parsing could
        // desync the parser from `git`'s own byte-exact output. `--color never` keeps
        // the output free of ANSI even if stdout were ever a tty. The budget bounds it.
        let n = number.to_string();
        let text = self
            .core
            .run_untrimmed_within(
                self.core
                    .command_in(dir, ["pr", "diff", n.as_str(), "--color", "never"]),
                budget,
            )
            .await?;
        Ok(vcs_diff::parse_diff(&text))
    }

    /// Run `gh <args>` over string slices — `gh.run_args(&["pr", "list"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GitHubApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(args).await
    }

    /// Like [`run_args`](GitHub::run_args) but never errors on a non-zero exit
    /// (mirrors [`GitHubApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    /// Run `gh <args>` **in `dir`** (the process is spawned with `dir` as its
    /// working directory, so `gh` infers the repo from `dir`'s remote), returning
    /// trimmed stdout — the dir-bound twin of the process-cwd [`run`](GitHubApi::run).
    /// This is what [`GitHubAt::run`] forwards to; call [`run`](GitHubApi::run) on the
    /// client for the process-cwd escape hatch. Argv is forwarded verbatim (only the
    /// working directory is bound, no `-R`/extra flag is injected).
    pub async fn run_in(&self, dir: &Path, args: &[String]) -> Result<String> {
        self.core.run(self.core.command_in(dir, args)).await
    }

    /// Like [`run_in`](GitHub::run_in) but never errors on a non-zero exit — the
    /// dir-bound twin of [`run_raw`](GitHubApi::run_raw). What [`GitHubAt::run_raw`]
    /// forwards to.
    pub async fn run_raw_in(&self, dir: &Path, args: &[String]) -> Result<ProcessResult<String>> {
        self.core
            .output_string(self.core.command_in(dir, args))
            .await
    }

    /// Like [`run_args`](GitHub::run_args) but **bound to `dir`** — the `&[&str]`
    /// twin of [`run_in`](GitHub::run_in). What [`GitHubAt::run_args`] forwards to.
    pub async fn run_args_in(&self, dir: &Path, args: &[&str]) -> Result<String> {
        self.core.run(self.core.command_in(dir, args)).await
    }

    /// Like [`run_raw_args`](GitHub::run_raw_args) but **bound to `dir`** — the
    /// `&[&str]` twin of [`run_raw_in`](GitHub::run_raw_in). What
    /// [`GitHubAt::run_raw_args`] forwards to.
    pub async fn run_raw_args_in(
        &self,
        dir: &Path,
        args: &[&str],
    ) -> Result<ProcessResult<String>> {
        self.core
            .output_string(self.core.command_in(dir, args))
            .await
    }

    /// Bind this client to `dir`, returning a [`GitHubAt`] handle whose `dir`-taking
    /// methods omit that argument: `gh.at(dir).pr_list()` runs
    /// [`pr_list`](GitHubApi::pr_list) against `dir`.
    pub fn at<'a>(&'a self, dir: &'a Path) -> GitHubAt<'a, R> {
        GitHubAt { gh: self, dir }
    }
}

/// A [`GitHub`] client with a working directory bound, so its repo-scoped methods
/// drop the leading `dir` argument (`gh.at(dir).pr_list()`). Construct one with
/// [`GitHub::at`].
pub struct GitHubAt<'a, R: ProcessRunner = processkit::JobRunner> {
    gh: &'a GitHub<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: holding only references, the view is `Copy`
// for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy` bound the
// default `JobRunner` doesn't satisfy, silently dropping `Copy` on the handle.
impl<R: ProcessRunner> Clone for GitHubAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for GitHubAt<'_, R> {}

// Generate [`GitHubAt`] forwarders: `bare` methods forward verbatim, `dir`
// methods inject `self.dir` as the first argument. The shared macro lives in
// `vcs-cli-support` (see `vcs_cli_support::at_forwarders!`).
vcs_cli_support::at_forwarders! {
    GitHubAt, gh, "GitHub",
    bare {
        fn version() -> Result<String>;
        fn capabilities() -> Result<GitHubCapabilities>;
        fn auth_status() -> Result<bool>;
        fn auth_status_for(host: &GitHubHost) -> Result<bool>;
    }
    dir {
        fn api(endpoint: &str) -> Result<String>;
        fn repo_view() -> Result<RepoView>;
        fn pr_list() -> Result<Vec<PullRequest>>;
        fn pr_list_for_branch(head: &str, base: &str) -> Result<Vec<PullRequest>>;
        fn pr_view(number: u64) -> Result<PullRequest>;
        fn issue_list() -> Result<Vec<Issue>>;
        fn pr_create(spec: PrCreate) -> Result<String>;
        fn pr_merge(number: u64, merge: PrMerge) -> Result<()>;
        fn pr_mark_ready(number: u64) -> Result<()>;
        fn pr_close(number: u64, spec: PrClose) -> Result<()>;
        fn pr_checkout(number: u64) -> Result<()>;
        fn pr_checks(number: u64) -> Result<Vec<CheckRun>>;
        fn pr_review(number: u64, action: ReviewAction) -> Result<()>;
        fn pr_comment(number: u64, body: &str) -> Result<String>;
        fn pr_edit(number: u64, edit: PrEdit) -> Result<()>;
        fn pr_feedback(number: u64) -> Result<PrFeedback>;
        fn pr_diff(number: u64) -> Result<Vec<FileDiff>>;
        fn run_list(limit: u64, branch: Option<String>) -> Result<Vec<WorkflowRun>>;
        fn run_view(id: u64) -> Result<WorkflowRun>;
        fn run_watch(id: u64) -> Result<WorkflowRun>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn release_list() -> Result<Vec<Release>>;
        fn release_view(tag: &str) -> Result<Release>;
    }
    // Raw escape hatches: bound to `self.dir` (forward to the client's `*_in`
    // twins) so `gh.at(dir).run(…)` targets the bound repo's cwd, not the process
    // cwd. For the process-cwd hatch call `run`/`run_raw`/… on `GitHub` directly.
    raw {
        fn run(args: &[String]) -> Result<String> => run_in;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>> => run_raw_in;
        fn run_args(args: &[&str]) -> Result<String> => run_args_in;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>> => run_raw_args_in;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_gh() {
        assert_eq!(BINARY, "gh");
    }

    // `capabilities()` parses the real `gh --version` banner and gates on the 2.0
    // floor — covering the minimum, a modern release, and an unrecognisable banner
    // (the three cases the scheduled-drift lane also exercises against a real gh).
    #[tokio::test]
    async fn capability_version_gate_parses_and_gates() {
        // Modern gh (the `(date)` trailer and release-URL line are ignored).
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            ["gh", "--version"],
            Reply::ok(
                "gh version 2.40.1 (2024-01-05)\nhttps://github.com/cli/cli/releases/tag/v2.40.1\n",
            ),
        ));
        let caps = gh.capabilities().await.expect("capabilities");
        assert_eq!(caps.version.to_string(), "2.40.1");
        assert!(caps.is_supported());
        caps.ensure_supported().expect("supported");

        // Exactly at the floor (2.0.0) is supported.
        let at_floor = GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "--version"], Reply::ok("gh version 2.0.0\n")),
        );
        assert!(
            at_floor.capabilities().await.unwrap().is_supported(),
            "2.0.0 is exactly the floor"
        );

        // An old 1.x gh is rejected with a clear message naming the floor + found.
        let old = GitHub::with_runner(ScriptedRunner::new().on(
            ["gh", "--version"],
            Reply::ok("gh version 1.14.0 (2021-11-02)\n"),
        ));
        let caps = old.capabilities().await.expect("capabilities");
        assert_eq!(
            caps.version,
            GitHubVersion {
                major: 1,
                minor: 14,
                patch: 0
            }
        );
        assert!(!caps.is_supported(), "1.14 is below the 2.0 floor");
        let err = caps.ensure_supported().expect_err("unsupported");
        let Error::Spawn { source, .. } = &err else {
            panic!("expected Spawn, got {err:?}");
        };
        let message = source.to_string();
        assert!(message.contains(">= 2.0.0"), "names the floor: {message}");
        assert!(
            message.contains("1.14.0"),
            "names the found version: {message}"
        );

        // A banner with no version token is a parse error, not a silent zero.
        let garbage = GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "--version"], Reply::ok("gh version unknowable\n")),
        );
        let err = garbage.capabilities().await.expect_err("unrecognisable");
        assert!(matches!(err, Error::Parse { .. }), "got {err:?}");
    }

    // Compile-time guard: the bound view stays `Copy` for the default `JobRunner`.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GitHubAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`gh.at(dir)`) must produce byte-identical argv to the
    // dir-taking call.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec);

        gh.pr_list_for_branch(dir, "feat", "main").await.unwrap();
        gh.at(dir).pr_list_for_branch("feat", "main").await.unwrap();
        // One of the new lifecycle methods.
        gh.run_list(dir, 3, None).await.unwrap();
        gh.at(dir).run_list(3, None).await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
    }

    // T-035: the raw escape hatches reached *through* the bound view
    // (`gh.at(dir).run…`) now run in the bound `dir`, while the same-named methods
    // on the client stay in the process cwd.
    #[tokio::test]
    async fn bound_view_raw_hatch_runs_in_bound_dir() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);

        // Through the bound view: every raw form carries the bound dir as its cwd.
        gh.at(dir)
            .run(&["pr".to_string(), "list".to_string()])
            .await
            .unwrap();
        let _ = gh
            .at(dir)
            .run_raw(&["pr".to_string(), "list".to_string()])
            .await
            .unwrap();
        gh.at(dir).run_args(&["pr", "list"]).await.unwrap();
        let _ = gh.at(dir).run_raw_args(&["pr", "list"]).await.unwrap();
        // On the client directly: the process-cwd escape hatch (no bound dir).
        gh.run(&["pr".to_string(), "list".to_string()])
            .await
            .unwrap();
        let _ = gh
            .run_raw(&["pr".to_string(), "list".to_string()])
            .await
            .unwrap();
        gh.run_args(&["pr", "list"]).await.unwrap();
        let _ = gh.run_raw_args(&["pr", "list"]).await.unwrap();

        let calls = rec.calls();
        for c in &calls[0..4] {
            assert_eq!(
                c.cwd.as_deref(),
                Some(dir),
                "raw call through the bound view runs in the bound dir"
            );
            assert_eq!(c.args_str(), ["pr", "list"]);
        }
        for c in &calls[4..8] {
            assert_eq!(
                c.cwd.as_deref(),
                None,
                "raw call on the client stays in the process cwd"
            );
            assert_eq!(c.args_str(), ["pr", "list"]);
        }
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "api", "user"], Reply::ok("ok\n")));
        assert_eq!(gh.run_args(&["api", "user"]).await.unwrap(), "ok");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `gh` binary or network needed, so this runs on CI.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"number":7,"title":"Add X","state":"OPEN","headRefName":"feat/x","baseRefName":"main","url":"u"}]"#;
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = gh.pr_list(Path::new(".")).await.expect("pr_list");
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].base_ref_name, "main");
    }

    // Hermetic: auth_status reflects the exit code without erroring. ANY non-zero
    // exit — not just the documented 1 — must read as `false`, never an error
    // (an unusual exit code must not be mistaken for a hard failure).
    #[tokio::test]
    async fn auth_status_reads_exit_code() {
        let yes = GitHub::with_runner(ScriptedRunner::new().on(["gh", "auth"], Reply::ok("")));
        assert!(yes.auth_status().await.unwrap());
        let no = GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "auth"], Reply::fail(1, "not logged in")),
        );
        assert!(!no.auth_status().await.unwrap());
        // An unexpected exit code (e.g. 2) is still just "not authenticated".
        let weird =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "auth"], Reply::fail(2, "boom")));
        assert!(!weird.auth_status().await.unwrap());
    }

    // Regression guard for the timeout fix: a timed-out auth check must error,
    // not silently report "not authenticated" (the old hand-rolled mapping bug).
    // Relies on processkit surfacing a timed-out run as `Error::Timeout`.
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["gh", "auth"], Reply::timeout()));
        assert!(matches!(
            gh.auth_status().await.unwrap_err(),
            Error::Timeout { .. }
        ));
    }

    // pr_create appends `--base <branch>` when given one, and returns the trimmed
    // PR URL. The exact command (incl. --base) is the only scripted rule.
    #[tokio::test]
    async fn pr_create_appends_base_and_returns_url() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            [
                "gh", "pr", "create", "--title", "T", "--body", "B", "--base", "main",
            ],
            Reply::ok("https://gh/pr/1\n"),
        ));
        let url = gh
            .pr_create(Path::new("."), PrCreate::new("T", "B").base("main"))
            .await
            .expect("should build `pr create … --base main`");
        assert_eq!(url, "https://gh/pr/1");
    }

    // With an explicit head, `pr_create` inserts `--head <branch>` before
    // `--base` — so a PR can target an arbitrary source→target pair.
    #[tokio::test]
    async fn pr_create_appends_head_and_base() {
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::replying(Reply::ok("https://gh/pr/9\n"));
        let gh = GitHub::with_runner(&rec);
        gh.pr_create(
            Path::new("/repo"),
            PrCreate::new("T", "B").head("feat/x").base("main"),
        )
        .await
        .expect("pr_create");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr", "create", "--title", "T", "--body", "B", "--head", "feat/x", "--base", "main"
            ]
        );
    }

    // pr_list_for_branch filters by head + base and parses the PR list (title +
    // url available on each result).
    #[tokio::test]
    async fn pr_list_for_branch_filters_and_parses() {
        use processkit::testing::RecordingRunner;
        let json = r#"[{"number":9,"title":"Merge feat","state":"OPEN","headRefName":"feat/x","baseRefName":"main","url":"https://gh/pr/9"}]"#;
        let rec = RecordingRunner::replying(Reply::ok(json));
        let gh = GitHub::with_runner(&rec);
        let prs = gh
            .pr_list_for_branch(Path::new("/repo"), "feat/x", "main")
            .await
            .expect("pr_list_for_branch");
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].title, "Merge feat");
        assert_eq!(prs[0].url, "https://gh/pr/9");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr", "list", "--head", "feat/x", "--base", "main", "--state", "all", "--limit",
                "100", "--json", PR_FIELDS
            ]
        );
    }

    // The list methods pin an explicit `--limit 100` so the CLI's default page
    // size (30) does not silently truncate the result.
    #[tokio::test]
    async fn list_methods_pin_limit_100() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec);
        gh.pr_list(Path::new("/r")).await.expect("pr_list");
        gh.issue_list(Path::new("/r")).await.expect("issue_list");
        gh.release_list(Path::new("/r"))
            .await
            .expect("release_list");
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["pr", "list", "--limit", "100", "--json", PR_FIELDS]
        );
        assert_eq!(
            calls[1].args_str(),
            [
                "issue",
                "list",
                "--limit",
                "100",
                "--json",
                ISSUE_LIST_FIELDS
            ]
        );
        assert_eq!(
            calls[2].args_str(),
            [
                "release",
                "list",
                "--limit",
                "100",
                "--json",
                RELEASE_LIST_FIELDS
            ]
        );
    }

    // Without a base, `pr_create` must omit `--base` entirely. RecordingRunner
    // captures the exact invocation (and `&rec` plumbs through CliClient), so we
    // can assert flag *absence* and the cwd — which prefix matching can't.
    #[tokio::test]
    async fn pr_create_omits_base_when_none() {
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::replying(Reply::ok("https://gh/pr/2\n"));
        let gh = GitHub::with_runner(&rec);
        let url = gh
            .pr_create(Path::new("/repo"), PrCreate::new("T", "B"))
            .await
            .expect("pr_create");
        assert_eq!(url, "https://gh/pr/2");

        let call = rec.only_call();
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/repo")));
        assert_eq!(
            call.args_str(),
            ["pr", "create", "--title", "T", "--body", "B"]
        );
        assert!(!call.has_flag("--base"), "no base was given");
        assert!(!call.has_flag("--head"), "no head was given");
    }

    // The injection guard on gh's exposed positionals.
    #[tokio::test]
    async fn flag_like_positionals_are_rejected_before_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        assert!(gh.api(Path::new("."), "-evil").await.is_err());
        assert!(gh.release_view(Path::new("."), "-evil").await.is_err());
        assert!(
            gh.api(Path::new("."), "").await.is_err(),
            "empty refused too"
        );
        assert!(rec.calls().is_empty(), "nothing may spawn");
    }

    #[tokio::test]
    async fn api_runs_in_the_bound_repo_dir() {
        let rec = RecordingRunner::replying(Reply::ok("{}\n"));
        let gh = GitHub::with_runner(&rec);
        gh.api(Path::new("/repo"), "repos/o/r/pulls")
            .await
            .expect("api");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["api", "repos/o/r/pulls"]);
        // H9: the request runs in the bound repo dir, so gh resolves a relative
        // endpoint's `{owner}/{repo}` from *that* repo — not the process cwd.
        assert_eq!(call.cwd, Some(std::path::PathBuf::from("/repo")));
    }

    // pr_merge builds the strategy flag plus the optional --auto/--delete-branch.
    #[tokio::test]
    async fn pr_merge_builds_strategy_and_flags() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_merge(Path::new("/r"), 7, PrMerge::squash().auto().delete_branch())
            .await
            .expect("pr_merge");
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "merge", "7", "--squash", "--auto", "--delete-branch"]
        );

        let bare = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&bare);
        gh.pr_merge(Path::new("/r"), 7, PrMerge::merge())
            .await
            .expect("pr_merge");
        let call = bare.only_call();
        assert_eq!(call.args_str(), ["pr", "merge", "7", "--merge"]);
        assert!(!call.has_flag("--auto"));
        assert!(!call.has_flag("--delete-branch"));
    }

    #[tokio::test]
    async fn pr_mark_ready_and_close_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_mark_ready(Path::new("/r"), 3)
            .await
            .expect("pr_mark_ready");
        gh.pr_close(Path::new("/r"), 3, PrClose::new().delete_branch())
            .await
            .expect("close");
        gh.pr_close(Path::new("/r"), 4, PrClose::new())
            .await
            .expect("close");
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["pr", "ready", "3"]);
        assert_eq!(calls[1].args_str(), ["pr", "close", "3", "--delete-branch"]);
        assert_eq!(calls[2].args_str(), ["pr", "close", "4"]);
    }

    // pr_checkout maps to `pr checkout <n>` and runs in the bound repo dir.
    #[tokio::test]
    async fn pr_checkout_builds_args_in_repo_dir() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_checkout(Path::new("/repo"), 7)
            .await
            .expect("pr_checkout");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["pr", "checkout", "7"]);
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/repo")));
        // The bound view produces byte-identical argv.
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.at(Path::new("/repo"))
            .pr_checkout(7)
            .await
            .expect("pr_checkout");
        assert_eq!(rec.only_call().args_str(), ["pr", "checkout", "7"]);
    }

    // gh signals the checks outcome via exit code (0 pass / 8 pending / 1 some
    // failed) but emits the same JSON for all three — all must parse. Other
    // exits (and timeouts) are genuine errors.
    #[tokio::test]
    async fn pr_checks_parses_all_outcome_exit_codes() {
        let json = r#"[{"name":"build","state":"SUCCESS","bucket":"pass",
            "workflow":"CI","link":"l","startedAt":"s","completedAt":"c"}]"#;
        for reply in [
            Reply::ok(json),
            Reply::fail(8, "checks pending").with_stdout(json),
            Reply::fail(1, "some checks failed").with_stdout(json),
        ] {
            let gh = GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "checks"], reply));
            let checks = gh.pr_checks(Path::new("."), 7).await.expect("pr_checks");
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].bucket, CheckBucket::Pass);
        }

        // A PR with no checks at all: gh exits 1 with NO JSON and a
        // "no checks reported" message — an empty list, not an error. Matched
        // case-insensitively, so a capitalized variant is still the empty case.
        for stderr in [
            "no checks reported on the 'feat/x' branch",
            "No Checks Reported on the 'feat/x' branch",
        ] {
            let gh = GitHub::with_runner(
                ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::fail(1, stderr)),
            );
            assert!(
                gh.pr_checks(Path::new("."), 7)
                    .await
                    .expect("no checks → empty")
                    .is_empty(),
                "no-checks must read as empty for stderr {stderr:?}"
            );
        }
        // …while a bare exit 1 for a different reason stays an error.
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            ["gh", "pr", "checks"],
            Reply::fail(1, "no pull requests found for branch 'feat/x'"),
        ));
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Exit { .. }
        ));

        // Exit 4 (auth required) is a real failure, not an outcome.
        let gh = GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::fail(4, "auth required")),
        );
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Exit { .. }
        ));

        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::timeout()));
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Timeout { .. }
        ));
    }

    // Hermetic: real pr_diff() arg-building (incl. `--color never`) + the
    // shared unified-diff parser against canned `gh pr diff` output.
    #[tokio::test]
    async fn pr_diff_builds_args_and_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let rec = RecordingRunner::replying(Reply::ok(out));
        let gh = GitHub::with_runner(&rec);
        let files = gh.pr_diff(Path::new("/r"), 7).await.expect("pr_diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "diff", "7", "--color", "never"]
        );
    }

    // T-049: `pr_diff` over the client's default OutputBudget is refused with
    // `OutputTooLarge` (actual + allowed sizes), never a silently truncated diff.
    #[tokio::test]
    async fn pr_diff_over_budget_errors_output_too_large() {
        let big = "diff --git a/m b/m\n".to_string() + &"+line\n".repeat(20_000);
        assert!(big.len() > 64 * 1024, "fixture must exceed the budget");
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(&big)))
                .default_output_budget(OutputBudget::bytes(64 * 1024));
        match gh.pr_diff(Path::new("/r"), 7).await {
            Err(Error::OutputTooLarge {
                program,
                max_bytes,
                total_bytes,
                ..
            }) => {
                assert_eq!(program, "gh");
                assert_eq!(max_bytes, Some(64 * 1024));
                assert!(total_bytes > 64 * 1024, "actual exceeds allowed");
            }
            other => panic!("expected OutputTooLarge, got {other:?}"),
        }
    }

    // The per-call override reads a legitimately large PR diff past the tight
    // client default that would otherwise refuse it.
    #[tokio::test]
    async fn pr_diff_within_override_reads_past_the_default() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(out)))
                .default_output_budget(OutputBudget::bytes(4)); // absurdly tight default
        assert!(matches!(
            gh.pr_diff(Path::new("/r"), 7).await,
            Err(Error::OutputTooLarge { .. })
        ));
        let files = gh
            .pr_diff_within(Path::new("/r"), 7, OutputBudget::unlimited())
            .await
            .expect("override reads the diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
    }

    // T-049: `gh run watch`'s fixed cap is reconciled onto the shared OutputBudget
    // as its DROP-OLDEST *diagnostic* projection — a bounded tail that NEVER turns a
    // long, chatty watch into `OutputTooLarge`. A watch that reprints far past the
    // 256 KiB / 256-line cap still succeeds and reads the final run state.
    #[tokio::test]
    async fn run_watch_bounds_output_without_failing_loud() {
        // ~5 MiB of repeated job-table frames — well past the watch cap.
        let flood = "watching run… job A: running\n".repeat(180_000);
        let run_json = r#"{"databaseId":42,"name":"CI","displayTitle":"t",
            "status":"completed","conclusion":"success","workflowName":"CI",
            "headBranch":"main","event":"push","url":"u","createdAt":"c"}"#;
        let gh = GitHub::with_runner(
            ScriptedRunner::new()
                .on(["gh", "run", "watch"], Reply::ok(&flood))
                .on(["gh", "run", "view"], Reply::ok(run_json)),
        );
        // Must NOT error out with OutputTooLarge — the diagnostic projection drops
        // the oldest frames and keeps going, then `run view` yields the state.
        let run = gh
            .run_watch(Path::new("/r"), 42)
            .await
            .expect("a chatty watch is bounded, not failed loud");
        assert_eq!(run.database_id, 42);
    }

    // Each review action maps to its flag; the body is carried on the action
    // (approve's is optional and omitted when absent).
    #[tokio::test]
    async fn pr_review_builds_action_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_review(Path::new("/r"), 7, ReviewAction::approve())
            .await
            .expect("approve");
        gh.pr_review(
            Path::new("/r"),
            7,
            ReviewAction::request_changes("fix the parser"),
        )
        .await
        .expect("request changes");
        gh.pr_review(Path::new("/r"), 7, ReviewAction::comment("nice"))
            .await
            .expect("comment");
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["pr", "review", "7", "--approve"]);
        assert!(!calls[0].has_flag("--body"));
        assert_eq!(
            calls[1].args_str(),
            [
                "pr",
                "review",
                "7",
                "--request-changes",
                "--body",
                "fix the parser"
            ]
        );
        assert_eq!(
            calls[2].args_str(),
            ["pr", "review", "7", "--comment", "--body", "nice"]
        );
    }

    // `approve().with_body(..)` attaches the optional approve message, emitting
    // `--approve --body <body>`; the accessors read the parts back.
    #[tokio::test]
    async fn pr_review_approve_with_body() {
        let action = ReviewAction::approve().with_body("LGTM");
        assert_eq!(action.kind(), ReviewKind::Approve);
        assert_eq!(action.body(), Some("LGTM"));

        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_review(Path::new("/r"), 7, action)
            .await
            .expect("approve with body");
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "review", "7", "--approve", "--body", "LGTM"]
        );
    }

    #[tokio::test]
    async fn pr_comment_and_issue_create_return_urls() {
        let rec = RecordingRunner::replying(Reply::ok("https://gh/x\n"));
        let gh = GitHub::with_runner(&rec);
        assert_eq!(
            gh.pr_comment(Path::new("/r"), 7, "hello").await.unwrap(),
            "https://gh/x"
        );
        assert_eq!(
            gh.issue_create(Path::new("/r"), "T", "B").await.unwrap(),
            "https://gh/x"
        );
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["pr", "comment", "7", "--body", "hello"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["issue", "create", "--title", "T", "--body", "B"]
        );
    }

    // pr_edit emits only the flags the caller set. The flag-VALUE slots
    // (`--title <t>`, `--body <b>`) are passed verbatim — no argv-guard needed
    // since gh consumes the next token as a value, not as a flag.
    #[tokio::test]
    async fn pr_edit_emits_only_provided_fields() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);

        gh.pr_edit(Path::new("/r"), 7, PrEdit::new().title("New title"))
            .await
            .expect("title-only edit");
        gh.pr_edit(Path::new("/r"), 7, PrEdit::new().body("New body"))
            .await
            .expect("body-only edit");
        gh.pr_edit(Path::new("/r"), 7, PrEdit::new().title("T").body("B"))
            .await
            .expect("both-fields edit");

        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["pr", "edit", "7", "--title", "New title"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["pr", "edit", "7", "--body", "New body"]
        );
        assert_eq!(
            calls[2].args_str(),
            ["pr", "edit", "7", "--title", "T", "--body", "B"]
        );
    }

    // An empty string is a real value (clears the field) — it must reach the
    // CLI as `--title ""`, not be silently dropped. The argv is asserted
    // byte-for-byte so a future "treat empty as None" regression would
    // surface here.
    #[tokio::test]
    async fn pr_edit_some_empty_string_clears_field() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_edit(Path::new("/r"), 7, PrEdit::new().title(""))
            .await
            .expect("empty title");
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "edit", "7", "--title", ""]
        );
    }

    #[tokio::test]
    async fn with_credentials_injects_gh_token_and_default_does_not() {
        // With a provider: the token is set as GH_TOKEN on the command — and never
        // appears in argv (so it can't leak through `ps`).
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec)
            .with_credentials(Arc::new(StaticCredential::token("tok-123")));
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        let token = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("GH_TOKEN"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(
            token,
            Some("tok-123"),
            "provider token injected as GH_TOKEN"
        );
        assert!(
            !call.args_str().iter().any(|a| a.contains("tok-123")),
            "secret must never appear in argv"
        );

        // Without a provider: no GH_TOKEN injected — ambient `gh` auth is unchanged.
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec);
        gh.pr_list(Path::new("/r")).await.unwrap();
        assert!(
            !rec.only_call()
                .envs
                .iter()
                .any(|(k, _)| k.to_str() == Some("GH_TOKEN")),
            "no provider → no token env (ambient gh auth)"
        );
    }

    // The `with_token` convenience is the common path: a static token, no `Arc`/
    // `StaticCredential` ceremony, injected as GH_TOKEN.
    #[tokio::test]
    async fn with_token_convenience_injects_gh_token() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec).with_token("tok-conv");
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        let token = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("GH_TOKEN"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(token, Some("tok-conv"));
    }

    // A provider that yields `Ok(None)` defers to ambient auth: no GH_TOKEN is
    // injected, exactly as if no provider were attached. Pins the None=ambient
    // contract end-to-end (not just at the provider level).
    #[tokio::test]
    async fn provider_returning_none_falls_back_to_ambient() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec).with_credentials(Arc::new(provider_fn(|_| Ok(None))));
        gh.pr_list(Path::new("/r")).await.unwrap();
        assert!(
            !rec.only_call()
                .envs
                .iter()
                .any(|(k, _)| k.to_str() == Some("GH_TOKEN")),
            "Ok(None) provider injects no token (ambient)"
        );
    }

    #[tokio::test]
    async fn injected_token_overrides_ambient_default_env() {
        // A provider token is applied after any `default_env("GH_TOKEN", …)`, so it
        // wins — "I supplied a provider, use it" beats an ambient env default.
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec)
            .default_env("GH_TOKEN", "ambient-token")
            .with_credentials(Arc::new(StaticCredential::token("provider-token")));
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        let winner = call
            .envs
            .iter()
            .rev()
            .find(|(k, _)| k.to_str() == Some("GH_TOKEN"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(winner, Some("provider-token"), "provider token wins");
    }

    // --- Enterprise host + host-scoped auth (T-046) ------------------------

    // GitHubHost classifies github.com (any case) as SaaS and every other valid
    // host as GHES, canonicalizing to a lower-cased hostname.
    #[test]
    fn github_host_classifies_saas_and_enterprise() {
        let saas = GitHubHost::github_com();
        assert!(saas.is_github_com() && !saas.is_enterprise());
        assert_eq!(saas.as_str(), "github.com");

        for h in ["github.com", "GitHub.com", "GITHUB.COM"] {
            let host = GitHubHost::new(h).unwrap();
            assert!(host.is_github_com(), "{h} should classify as SaaS");
            assert_eq!(host.as_str(), "github.com", "canonicalized to lower-case");
        }

        let ghes = GitHubHost::new("GHE.Example.COM").unwrap();
        assert!(ghes.is_enterprise());
        assert_eq!(ghes.as_str(), "ghe.example.com");
    }

    // A malformed hostname is a diagnosable invalid-input error, not a silent
    // github.com guess — so a bad host can't quietly become the SaaS default.
    #[test]
    fn github_host_new_rejects_malformed_hosts() {
        for bad in [
            "",
            "  ",
            "-evil",
            "has space",
            "https://github.com",
            "github.com/owner",
            "ghe.example.com:8443",
            "user@github.com",
            ".leading",
            "trailing.",
        ] {
            let err = GitHubHost::new(bad).unwrap_err();
            assert!(
                vcs_cli_support::is_invalid_input(&err),
                "{bad:?} should be rejected as invalid input, got {err:?}"
            );
        }
    }

    // from_remote_url derives + classifies the host across HTTPS / SSH / scp-like
    // remotes, dropping userinfo and port.
    #[test]
    fn github_host_from_remote_url_parses_and_classifies() {
        let cases = [
            ("https://github.com/o/r.git", "github.com", false),
            (
                "https://x-access-token:tok@ghe.example.com:8443/o/r",
                "ghe.example.com",
                true,
            ),
            ("http://ghe.internal.corp/o/r", "ghe.internal.corp", true),
            ("ssh://git@github.com/o/r", "github.com", false),
            ("ssh://git@ghe.example.com:22/o/r", "ghe.example.com", true),
            ("git@github.com:o/r.git", "github.com", false),
            ("git@ghe.example.com:o/r.git", "ghe.example.com", true),
        ];
        for (url, host, enterprise) in cases {
            let parsed =
                GitHubHost::from_remote_url(url).unwrap_or_else(|e| panic!("parse {url}: {e:?}"));
            assert_eq!(parsed.as_str(), host, "host for {url}");
            assert_eq!(parsed.is_enterprise(), enterprise, "class for {url}");
        }
    }

    // An unparseable / hostless / ambiguous remote is a diagnosable error, never a
    // silent github.com fallback (which would authenticate the wrong host).
    #[test]
    fn github_host_from_remote_url_rejects_ambiguous() {
        for url in [
            "",
            "   ",
            "not-a-url",
            "https://",
            "ssh://",
            "git@internalhost:repo.git",
            "C:\\repo\\path",
            "https://[::1]:8443/x",
        ] {
            let err = GitHubHost::from_remote_url(url).unwrap_err();
            assert!(
                vcs_cli_support::is_invalid_input(&err),
                "{url:?} should be a diagnosable error, got {err:?}"
            );
        }
    }

    // Binding a github.com host injects the credential as GH_TOKEN (the SaaS
    // default) and pins GH_HOST — never the enterprise env.
    #[tokio::test]
    async fn with_host_github_com_injects_gh_token() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec)
            .with_host(GitHubHost::github_com())
            .with_token("saas-tok");
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert!(call.env_is("GH_TOKEN", "saas-tok"));
        assert!(
            !call.has_env("GH_ENTERPRISE_TOKEN"),
            "github.com must not touch the enterprise token env"
        );
        assert!(call.env_is("GH_HOST", "github.com"));
        assert!(!call.args_str().iter().any(|a| a.contains("saas-tok")));
    }

    // Binding a GHES host injects the credential as GH_ENTERPRISE_TOKEN — the env
    // gh reads for a non-github.com host — plus GH_HOST, and NEVER as GH_TOKEN, so
    // an enterprise secret can't leak into the github.com token env. The secret
    // stays out of argv.
    #[tokio::test]
    async fn with_host_enterprise_injects_enterprise_token_and_host() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec)
            .with_host(GitHubHost::new("ghe.example.com").unwrap())
            .with_token("ent-tok");
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert!(call.env_is("GH_ENTERPRISE_TOKEN", "ent-tok"));
        assert!(
            !call.has_env("GH_TOKEN"),
            "enterprise token must not land in the github.com env"
        );
        assert!(call.env_is("GH_HOST", "ghe.example.com"));
        assert!(
            !call.args_str().iter().any(|a| a.contains("ent-tok")),
            "secret must never appear in argv"
        );
    }

    // A host-bound client with NO provider injects no token at all (ambient gh
    // login for that host) but still pins GH_HOST, so gh targets the right server.
    #[tokio::test]
    async fn with_host_enterprise_without_credentials_is_ambient() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec).with_host(GitHubHost::new("ghe.corp.example").unwrap());
        gh.pr_list(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert!(!call.has_env("GH_ENTERPRISE_TOKEN"));
        assert!(!call.has_env("GH_TOKEN"));
        assert!(call.env_is("GH_HOST", "ghe.corp.example"));
    }

    // Several hosts, one client each: every client injects only its own host's
    // token/env — a credential for one host never leaks into another.
    #[tokio::test]
    async fn multiple_hosts_inject_independently() {
        let rec_a = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_a)
            .with_host(GitHubHost::new("ghe.a.example").unwrap())
            .with_token("tok-a")
            .pr_list(Path::new("/r"))
            .await
            .unwrap();

        let rec_b = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_b)
            .with_host(GitHubHost::new("ghe.b.example").unwrap())
            .with_token("tok-b")
            .pr_list(Path::new("/r"))
            .await
            .unwrap();

        let rec_saas = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_saas)
            .with_host(GitHubHost::github_com())
            .with_token("tok-saas")
            .pr_list(Path::new("/r"))
            .await
            .unwrap();

        let ca = rec_a.only_call();
        assert!(ca.env_is("GH_ENTERPRISE_TOKEN", "tok-a") && ca.env_is("GH_HOST", "ghe.a.example"));
        assert!(
            !ca.args_str()
                .iter()
                .any(|s| s.contains("tok-b") || s.contains("tok-saas")),
            "host A must not carry another host's secret"
        );

        let cb = rec_b.only_call();
        assert!(cb.env_is("GH_ENTERPRISE_TOKEN", "tok-b") && cb.env_is("GH_HOST", "ghe.b.example"));

        let cs = rec_saas.only_call();
        assert!(cs.env_is("GH_TOKEN", "tok-saas") && cs.env_is("GH_HOST", "github.com"));
        assert!(!cs.has_env("GH_ENTERPRISE_TOKEN"));
    }

    // A HOST-KEYED provider on a host-bound client injects ONLY that host's secret,
    // into the env gh reads for it — and a client bound to a *different* host draws a
    // different secret from the SAME provider, so one instance's token never lands in
    // another's command. (T-045: the bound host now reaches the CredentialRequest, so
    // the provider can tell SaaS from a self-hosted GHES instance.)
    #[tokio::test]
    async fn host_keyed_provider_injects_only_the_bound_hosts_token() {
        // Typed as the trait object so `Arc::clone` yields `Arc<dyn …>` directly
        // (the unsized coercion doesn't flow back through `Arc::clone`'s inference).
        let provider: Arc<dyn CredentialProvider> =
            Arc::new(provider_fn(|r: &CredentialRequest<'_>| {
                Ok(match r.host {
                    Some("github.com") => Some(Credential::token("saas-secret")),
                    Some("ghe.example.com") => Some(Credential::token("ent-secret")),
                    _ => None,
                })
            }));

        // SaaS client → GH_TOKEN carries the github.com secret, never the ent one.
        let rec_saas = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_saas)
            .with_host(GitHubHost::github_com())
            .with_credentials(Arc::clone(&provider))
            .pr_list(Path::new("/r"))
            .await
            .unwrap();
        let cs = rec_saas.only_call();
        assert!(cs.env_is("GH_TOKEN", "saas-secret"));
        assert!(!cs.has_env("GH_ENTERPRISE_TOKEN"));
        assert!(!cs.args_str().iter().any(|a| a.contains("saas-secret")));

        // Enterprise client → the ENT secret in GH_ENTERPRISE_TOKEN only, from the
        // very same provider; the github.com token env is untouched.
        let rec_ent = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_ent)
            .with_host(GitHubHost::new("ghe.example.com").unwrap())
            .with_credentials(Arc::clone(&provider))
            .pr_list(Path::new("/r"))
            .await
            .unwrap();
        let ce = rec_ent.only_call();
        assert!(ce.env_is("GH_ENTERPRISE_TOKEN", "ent-secret"));
        assert!(
            !ce.has_env("GH_TOKEN"),
            "the enterprise command must not carry the github.com token env"
        );
        assert!(!ce.args_str().iter().any(|a| a.contains("ent-secret")));
    }

    // Fallback policy, read vs write — `Ok(None)` (a host-keyed provider with nothing
    // for this host) leaves the command on ambient gh auth (no token env injected)
    // for BOTH a read (`pr_list`) and a write (`pr_merge`). (T-045)
    #[tokio::test]
    async fn provider_none_defers_to_ambient_for_read_and_write() {
        let rec_read = RecordingRunner::replying(Reply::ok("[]"));
        GitHub::with_runner(&rec_read)
            .with_host(GitHubHost::github_com())
            .with_credentials(Arc::new(provider_fn(|_r: &CredentialRequest<'_>| Ok(None))))
            .pr_list(Path::new("/r"))
            .await
            .unwrap();
        let cr = rec_read.only_call();
        assert!(
            !cr.has_env("GH_TOKEN") && !cr.has_env("GH_ENTERPRISE_TOKEN"),
            "read defers to ambient on Ok(None)"
        );

        let rec_write = RecordingRunner::replying(Reply::ok(""));
        GitHub::with_runner(&rec_write)
            .with_host(GitHubHost::github_com())
            .with_credentials(Arc::new(provider_fn(|_r: &CredentialRequest<'_>| Ok(None))))
            .pr_merge(Path::new("/r"), 7, PrMerge::squash())
            .await
            .unwrap();
        let cw = rec_write.only_call();
        assert!(
            !cw.has_env("GH_TOKEN") && !cw.has_env("GH_ENTERPRISE_TOKEN"),
            "write defers to ambient on Ok(None)"
        );
    }

    // Fallback policy, read vs write — a provider `Err` is FAIL-CLOSED: it aborts the
    // operation rather than silently running on ambient auth, proven separately for a
    // read (`pr_list`) and a write (`pr_merge`). gh is never spawned: the error
    // surfaces in `prepare`, before the process. (T-045)
    #[tokio::test]
    async fn provider_error_aborts_read_and_write_fail_closed() {
        fn boom() -> Arc<dyn CredentialProvider> {
            Arc::new(provider_fn(|_r: &CredentialRequest<'_>| {
                Err(Error::spawn(
                    BINARY,
                    std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "vault down"),
                ))
            }))
        }

        let rec_read = RecordingRunner::replying(Reply::ok("[]"));
        let read = GitHub::with_runner(&rec_read)
            .with_host(GitHubHost::github_com())
            .with_credentials(boom())
            .pr_list(Path::new("/r"))
            .await;
        assert!(read.is_err(), "a provider error must abort the read");
        assert!(
            rec_read.calls().is_empty(),
            "gh must not spawn when the provider errored (read)"
        );

        let rec_write = RecordingRunner::replying(Reply::ok(""));
        let write = GitHub::with_runner(&rec_write)
            .with_host(GitHubHost::github_com())
            .with_credentials(boom())
            .pr_merge(Path::new("/r"), 7, PrMerge::squash())
            .await;
        assert!(write.is_err(), "a provider error must abort the write");
        assert!(
            rec_write.calls().is_empty(),
            "gh must not spawn when the provider errored (write)"
        );
    }

    // auth_status_for pins `--hostname <host>` and reflects the exit code as a bool.
    #[tokio::test]
    async fn auth_status_for_scopes_to_hostname() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        let host = GitHubHost::new("ghe.example.com").unwrap();
        assert!(gh.auth_status_for(&host).await.unwrap());
        assert_eq!(
            rec.only_call().args_str(),
            ["auth", "status", "--hostname", "ghe.example.com"]
        );
    }

    // The scoped probe reports the TARGET host truthfully even when a DIFFERENT
    // host's session is broken — no false negative from the aggregate `gh auth
    // status` that the unscoped `auth_status` would fold together.
    #[tokio::test]
    async fn auth_status_for_is_independent_of_other_host_sessions() {
        let runner = ScriptedRunner::new()
            .on(
                ["gh", "auth", "status", "--hostname", "broken.example.com"],
                Reply::fail(1, "not logged in to broken.example.com"),
            )
            .on(
                ["gh", "auth", "status", "--hostname", "good.example.com"],
                Reply::ok(""),
            );
        let gh = GitHub::with_runner(runner);
        assert!(
            gh.auth_status_for(&GitHubHost::new("good.example.com").unwrap())
                .await
                .unwrap(),
            "the healthy target host reads as authenticated"
        );
        assert!(
            !gh.auth_status_for(&GitHubHost::new("broken.example.com").unwrap())
                .await
                .unwrap(),
            "a broken host reads as not authenticated, independently"
        );
    }

    // The bound view forwards auth_status_for verbatim (a bare, dir-independent
    // method): byte-identical argv, no cwd bound.
    #[tokio::test]
    async fn bound_view_auth_status_for_matches_client() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.at(Path::new("/repo"))
            .auth_status_for(&GitHubHost::github_com())
            .await
            .unwrap();
        let call = rec.only_call();
        assert_eq!(
            call.args_str(),
            ["auth", "status", "--hostname", "github.com"]
        );
        assert_eq!(call.cwd.as_deref(), None, "bare method binds no cwd");
    }

    #[tokio::test]
    async fn pr_feedback_requests_reviews_and_comments() {
        let json = r#"{"reviews":[{"author":{"login":"a"},"state":"APPROVED",
            "body":"","submittedAt":""}],"comments":[]}"#;
        let rec =
            RecordingRunner::new(ScriptedRunner::new().on(["gh", "pr", "view"], Reply::ok(json)));
        let gh = GitHub::with_runner(&rec);
        let feedback = gh.pr_feedback(Path::new("."), 7).await.expect("feedback");
        assert_eq!(feedback.reviews[0].author, "a");
        assert!(feedback.comments.is_empty());
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "view", "7", "--json", "reviews,comments"]
        );
    }

    // run_list appends --branch only when given one.
    #[tokio::test]
    async fn run_list_appends_branch_only_when_some() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let gh = GitHub::with_runner(&rec);
        gh.run_list(Path::new("/r"), 5, None).await.expect("list");
        gh.run_list(Path::new("/r"), 5, Some("main".into()))
            .await
            .expect("list");
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["run", "list", "--limit", "5", "--json", RUN_FIELDS]
        );
        assert_eq!(
            calls[1].args_str(),
            [
                "run", "list", "--limit", "5", "--branch", "main", "--json", RUN_FIELDS
            ]
        );
    }

    // run_watch blocks on `run watch` (no `--exit-status`, so a failed run still
    // exits 0 — the outcome is read via the follow-up view, the only channel
    // that can distinguish failed from cancelled).
    #[tokio::test]
    async fn run_watch_then_views_final_state() {
        let json = r#"{"databaseId":42,"name":"CI","displayTitle":"t",
            "status":"completed","conclusion":"failure","workflowName":"CI",
            "headBranch":"main","event":"push","url":"u","createdAt":"c"}"#;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["gh", "run", "watch"], Reply::ok("✓ run completed"))
                .on(["gh", "run", "view"], Reply::ok(json)),
        );
        let gh = GitHub::with_runner(&rec);
        let run = gh.run_watch(Path::new("."), 42).await.expect("run_watch");
        assert_eq!(run.conclusion, "failure");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args_str(), ["run", "watch", "42"]);
        assert_eq!(
            calls[1].args_str(),
            ["run", "view", "42", "--json", RUN_FIELDS]
        );
    }

    // A timed-out or failing watch must error — NOT report a half-finished run
    // via the follow-up view. (`output_string` does not error on a timeout; the
    // `ensure_success` in run_watch is what surfaces it.)
    #[tokio::test]
    async fn run_watch_surfaces_timeout_and_watch_errors() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "run", "watch"], Reply::timeout()),
        );
        let gh = GitHub::with_runner(&rec);
        assert!(matches!(
            gh.run_watch(Path::new("."), 42).await.unwrap_err(),
            Error::Timeout { .. }
        ));
        assert_eq!(rec.calls().len(), 1, "no view after a timed-out watch");

        let gh = GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "run", "watch"], Reply::fail(1, "no such run")),
        );
        assert!(matches!(
            gh.run_watch(Path::new("."), 42).await.unwrap_err(),
            Error::Exit { .. }
        ));
    }

    // Client-level cancellation (processkit 0.8 `cancellation` feature): a client
    // built with `default_cancel_on(token)` threads the token into every command
    // it builds, so a long `run_watch` parks until the token fires, then surfaces
    // `Error::Cancelled` — a controller cancels without touching the call site
    // (zero new vcs-* API). Hermetic via `Reply::pending()` (parks until the
    // command's token fires) on a paused clock: the 1 h `timeout` elapses
    // instantly while the call is parked, proving it does not resolve early.
    #[tokio::test(start_paused = true)]
    async fn run_watch_cancels_via_client_default_token() {
        use processkit::CancellationToken;
        let token = CancellationToken::new();
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "run", "watch"], Reply::pending()))
                .default_cancel_on(token.clone());
        let call = gh.run_watch(Path::new("."), 42);
        tokio::pin!(call);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3600), &mut call)
                .await
                .is_err(),
            "run_watch must park until the token fires"
        );
        token.cancel();
        match call.await {
            Err(Error::Cancelled { program }) => assert_eq!(program, "gh"),
            other => panic!("expected Error::Cancelled, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn release_view_requests_view_fields() {
        let json = r#"{"tagName":"v1","name":"","body":"notes","url":"u",
            "publishedAt":"p","isDraft":false,"isPrerelease":false}"#;
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "release", "view"], Reply::ok(json)),
        );
        let gh = GitHub::with_runner(&rec);
        let release = gh
            .release_view(Path::new("."), "v1")
            .await
            .expect("release_view");
        assert_eq!(release.tag_name, "v1");
        assert_eq!(release.body.as_deref(), Some("notes"));
        assert_eq!(release.url.as_deref(), Some("u"));
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "view", "v1", "--json", RELEASE_VIEW_FIELDS]
        );
    }

    // repo_view builds the --json request and flattens gh's nested owner/branch
    // objects into the public RepoView.
    #[tokio::test]
    async fn repo_view_parses_scripted_json() {
        let json = r#"{"name":"r","owner":{"login":"o"},"description":"d","url":"u","isPrivate":false,"defaultBranchRef":{"name":"main"}}"#;
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "repo", "view"], Reply::ok(json)));
        let repo = gh.repo_view(Path::new(".")).await.expect("repo_view");
        assert_eq!(repo.owner, "o");
        assert_eq!(repo.default_branch, "main");
        assert!(!repo.is_private);
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockGitHubApi::new();
        mock.expect_auth_status().returning(|| Ok(true));
        assert!(mock.auth_status().await.unwrap());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/github.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
