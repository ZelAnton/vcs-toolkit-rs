#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-gitlab` — automate GitLab from Rust by driving the `glab` CLI.
//!
//! You call typed `async` methods; `vcs-gitlab` runs the real `glab`, asks for
//! `--output json`, and deserializes the result into typed values — so you get
//! *glab's own* behaviour, host config, and credentials, not a reimplementation of
//! the GitLab API client. Async, structured errors, mockable. Every command runs
//! inside an OS **job** (an OS-level container that kills the whole process tree if
//! your program exits, via [`processkit`]) so a `glab` subprocess is never orphaned,
//! with an optional per-client [timeout](GitLab::default_timeout).
//!
//! # What you can do
//!
//! Check auth · view the project · the lean merge-request lifecycle (list / view /
//! create / merge / mark-ready / close / checkout) · CI/pipeline status · issues ·
//! releases.
//! One tiny call to start:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitlab::{GitLab, GitLabApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let glab = GitLab::new();
//! let mrs = glab.mr_list(Path::new(".")).await?; // up to 100 open MRs
//! # let _ = mrs; Ok(()) }
//! ```
//!
//! # The surface (engineering reference)
//!
//! The modelled surface is the **lean merge-request lifecycle** — auth, project
//! view, the MR lifecycle, plus issues and releases. It deserializes `glab …
//! --output json` (GitLab's REST JSON, which `glab` passes through) into typed
//! structs; it never scrapes human-readable output. The sibling
//! [`vcs-github`](https://crates.io/crates/vcs-github) and
//! [`vcs-gitea`](https://crates.io/crates/vcs-gitea) wrappers mirror this shape,
//! and the [`vcs-forge`](https://crates.io/crates/vcs-forge) facade unifies all
//! three.
//!
//! - **[`GitLabApi`]** — the object-safe trait every operation lives on. Depend on
//!   `&dyn GitLabApi` (or generically on `impl GitLabApi`) so a test can swap the
//!   real client for a double. Project-scoped methods take the working directory
//!   as the first argument and return typed results ([`RepoView`],
//!   [`MergeRequest`], [`Issue`], [`Release`], [`CiStatus`]) or a structured
//!   [`Error`]. Unmodelled `glab` commands go through [`run`](GitLabApi::run); any
//!   REST/GraphQL endpoint through [`api`](GitLabApi::api) (`glab api <endpoint>`).
//! - **[`GitLab`]** — the real client. [`GitLab::new`] uses the job-backed runner;
//!   [`GitLab::with_runner`] injects a fake one for tests. It is generic over the
//!   [`ProcessRunner`] seam, defaulting to the production runner.
//!   [`with_credentials`](GitLab::with_credentials) attaches a
//!   [`CredentialProvider`] to supply a token per operation (injected as
//!   `GITLAB_TOKEN`, never in `argv`) — opt-in, off by default (ambient `glab` auth).
//! - **[`GitLabAt`]** — a cwd-bound view ([`GitLab::at`]) whose project-scoped
//!   methods drop the leading `dir`, so `glab.at(dir).mr_list()` reads as
//!   `glab.mr_list(dir)` — handy when one client drives one checkout.
//! - **Builder specs** for the multi-option commands — [`MrCreate`] (title, body,
//!   optional source/target branch), [`MrEdit`] (optional `title` and/or `body` for
//!   `mr update`), and [`MrMerge`] (the [`MergeStrategy`] `Merge`/`Squash`/`Rebase`
//!   plus the gh-style `auto`/`delete_branch` options, which `glab` reports
//!   `Unsupported` rather than silently drop) — `#[non_exhaustive]`, built with a
//!   constructor + chained setters, named after the flags they emit.
//! - **[`auth_status`](GitLabApi::auth_status)** — a best-effort signal, *not* a
//!   guarantee: a long-standing glab bug can make `glab auth status` exit `0` even
//!   when unauthenticated, so a `true` means "probably"; a subsequent API call is
//!   the real test. A `false`, spawn failure, or timeout are faithful.
//!
//! # Recipes
//!
//! Read state — depend on the trait so the same code takes a real client or a mock:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitlab::{GitLab, GitLabApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let glab = GitLab::new();
//! let dir = Path::new(".");
//! for mr in glab.mr_list(dir).await? {                 // up to 100 open MRs
//!     println!("!{} [{}] {}", mr.iid, mr.state, mr.title);
//! }
//! # Ok(()) }
//! ```
//!
//! Mutate through the builder specs — `mr_merge` merges *immediately*
//! (`--auto-merge=false`) rather than enabling merge-when-pipeline-succeeds:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitlab::{GitLab, GitLabApi, MrCreate, MrMerge};
//! # async fn demo(glab: &GitLab) -> Result<(), processkit::Error> {
//! let dir = Path::new(".");
//! let url = glab
//!     .mr_create(dir, MrCreate::new("Add streaming", "Implements …").target("main"))
//!     .await?;                                          // the new MR's URL
//! glab.mr_merge(dir, 12, MrMerge::squash()).await?;
//! # let _ = url; Ok(()) }
//! ```
//!
//! # Testing
//!
//! Two seams: enable the **`mock`** feature for a `mockall`-generated
//! `MockGitLabApi` (stub whole methods), or inject a
//! [`ScriptedRunner`](processkit::testing::ScriptedRunner) with [`GitLab::with_runner`] to
//! exercise the *real* argv-building and JSON parsing against canned output. The
//! cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::path::Path;
use std::sync::Arc;

// The credential seam (the shared managed client behind `GitLab` is generated by
// `vcs_cli_support::managed_client!`) — re-exported so a consumer can supply a
// token provider.
pub use vcs_cli_support::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, FnProvider,
    Secret, StaticCredential, provider_fn,
};
// Re-export the processkit types in this crate's public API, so consumers needn't
// depend on processkit directly — incl. `ProcessRunner` (the `with_runner`/
// `GitLab<R>` seam) and the `JobRunner` default. (Also brings
// `Error`/`Result`/`ProcessResult`/`ProcessRunner` into scope here.)
pub use processkit::{Error, JobRunner, ProcessResult, ProcessRunner, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency. (Cancellation is core in processkit
// 0.10 — always available, no feature.)
pub use processkit::CancellationToken;

mod parse;
pub use parse::{CiStatus, Issue, MergeRequest, Release, RepoView};
// Re-exported so `vcs_gitlab::FileDiff` (and the types nested in it) resolve
// without a direct `vcs-diff` dependency — `mr_diff` returns `vcs-diff`'s
// model verbatim (`glab mr diff` emits the same git-format diff `git diff`/
// `jj diff --git` do; `crates/diff/src/diff.rs`'s parser is shared, not
// duplicated).
pub use vcs_diff::{ChangeKind, DiffLine, FileDiff, Hunk};

/// Options for [`GitLabApi::mr_create`] (`glab mr create`).
///
/// `#[non_exhaustive]`, so build it through [`MrCreate::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MrCreate {
    /// The MR title (`--title`).
    pub title: String,
    /// The MR description (`--description`).
    pub body: String,
    /// The source branch (`--source-branch`); `None` = the current branch.
    pub source: Option<String>,
    /// The target branch (`--target-branch`); `None` = the project default.
    pub target: Option<String>,
}

impl MrCreate {
    /// An MR with `title` and `body`, source/target left to glab's defaults
    /// (current branch → project default).
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            source: None,
            target: None,
        }
    }

    /// Set the source branch (`--source-branch`) instead of the current branch.
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the target branch (`--target-branch`) instead of the project default.
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }
}

/// Options for [`GitLabApi::mr_edit`] (`glab mr update`).
///
/// `#[non_exhaustive]`, so build it through [`MrEdit::new`] and the chained
/// [`title`](MrEdit::title) / [`body`](MrEdit::body) setters rather than a
/// struct literal. At least one of `title` or `body` must be `Some`; both
/// `None` is rejected by the facade before spawning (an explicit error, not a
/// silent no-op). An empty string is a real value — glab clears the field on
/// `--title ""` / `--description ""` — not a `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MrEdit {
    /// The new title (`--title`); `None` leaves the title alone.
    pub title: Option<String>,
    /// The new description (`--description`); `None` leaves the description alone.
    pub body: Option<String>,
}

impl MrEdit {
    /// An edit that leaves both fields alone (the facade rejects both-`None`
    /// before reaching the wrapper). Start with this and add what you want to
    /// change via [`title`](MrEdit::title) / [`body`](MrEdit::body).
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

    /// Set the new description (`--description`).
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl Default for MrEdit {
    fn default() -> Self {
        Self::new()
    }
}

/// Name of the underlying CLI binary this crate drives.
///
/// Note on injection safety: most of the surface has **no bare positional string
/// slot** for a caller value — MR/issue ids are `u64` (never flag-like), the
/// title/body/branch arguments ride in flag-VALUE positions (`--title <t>`,
/// `--source-branch <b>`) where glab consumes the next token verbatim, and
/// `run`/`run_args` are the caller-owns-the-argv escape hatch. The one exception
/// is [`release_view`](GitLabApi::release_view)'s bare `<tag>` positional, which
/// is guarded with `reject_flag_like` (mirroring `vcs-github`'s
/// `api`/`release_view`); guard any future bare positional the same way.
/// Separately, the description/body/comment flag-VALUE fields (`mr_create`,
/// `mr_edit`, `issue_create`, `mr_comment`) are guarded with
/// `reject_dash_sentinel` against glab's *own* `"-"` stdin/editor sentinel —
/// unrelated to argv injection, but the same "refuse before spawning" shape.
pub const BINARY: &str = "glab";

/// Injection guard for bare positional argv slots: a caller-supplied value with
/// a leading `-` would be parsed by glab's CLI as a *flag*, and an empty value
/// changes a command's meaning. Refuse both before anything spawns. Flag-VALUE
/// positions (`--title <t>`, `--source-branch <b>`) need no guard — glab consumes
/// the next token verbatim there.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

/// Guard against glab's dash-sentinel quirk: a description/comment body that is
/// *exactly* `"-"` makes glab treat the flag as "read from stdin"/"open
/// `$EDITOR`" instead of the literal string — a headless caller would hang
/// waiting on input that never comes (no `glab` timeout of its own). This is
/// **not** the shared `reject_flag_like` injection guard (these fields ride in
/// flag-VALUE positions, so a leading `-` is not parsed as a flag) — it is a
/// glab-specific value with special meaning to the CLI itself, so it lives here
/// rather than in `vcs-cli-support`. Refuse the bare `"-"` before anything
/// spawns, surfacing an `Error::Spawn` whose source is
/// `io::ErrorKind::InvalidInput`, naming `what` in the message. A caller who
/// needs a literal single-dash body must pick a different, non-sentinel
/// representation (e.g. `"-\u{200B}"` or wrap it) — there is no way to make
/// glab itself accept a byte-exact `"-"` non-interactively.
fn reject_dash_sentinel(what: &str, value: &str) -> Result<()> {
    if value == "-" {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} is a literal \"-\", which glab treats as a request to open an \
                     editor or read from stdin — refusing to pass it through non-interactively"
                ),
            ),
        ));
    }
    Ok(())
}

/// How [`GitLabApi::mr_merge`] merges the MR. GitLab's default is a merge commit;
/// `Squash`/`Rebase` add the corresponding flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeStrategy {
    /// A merge commit (glab's default — no extra flag).
    Merge,
    /// Squash the commits into one (`--squash`).
    Squash,
    /// Rebase the source onto the target (`--rebase`).
    Rebase,
}

impl MergeStrategy {
    /// The glab flag for this strategy, or `None` for the default merge commit.
    fn flag(self) -> Option<&'static str> {
        match self {
            MergeStrategy::Merge => None,
            MergeStrategy::Squash => Some("--squash"),
            MergeStrategy::Rebase => Some("--rebase"),
        }
    }
}

/// Options for [`GitLabApi::mr_merge`] (`glab mr merge`).
///
/// `#[non_exhaustive]`, so build it through the strategy constructors —
/// [`merge`](MrMerge::merge) / [`squash`](MrMerge::squash) /
/// [`rebase`](MrMerge::rebase), then [`auto`](MrMerge::auto) /
/// [`delete_branch`](MrMerge::delete_branch) — rather than a struct literal. The
/// shape mirrors `vcs-github`'s `PrMerge` and `vcs-gitea`'s `PrMerge` so the
/// [`vcs-forge`](https://crates.io/crates/vcs-forge) facade drives one merge spec
/// across all three backends.
///
/// **Backend capability.** `glab mr merge` merges **immediately**
/// (`--auto-merge=false`), and this wrapper deliberately does **not** map the
/// gh-style [`auto`](MrMerge::auto) (merge once requirements are met) or
/// [`delete_branch`](MrMerge::delete_branch) options: glab's own `--auto-merge` is
/// *merge-when-pipeline-succeeds*, a different contract from gh's `--auto`. So when
/// either option is set, [`mr_merge`](GitLabApi::mr_merge) returns a structured
/// `Error::Unsupported` rather than *silently* ignoring the request — for an
/// irreversible merge, quietly dropping an option could produce the wrong side
/// effects. The default (neither set) is the plain immediate merge.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MrMerge {
    /// The merge strategy (`Merge` = glab's default merge commit; `Squash`;
    /// `Rebase`).
    pub strategy: MergeStrategy,
    /// Request gh-style auto-merge (merge once requirements are met). **Not
    /// expressible on `glab`** — when set, [`mr_merge`](GitLabApi::mr_merge)
    /// returns `Error::Unsupported` instead of merging immediately anyway (see the
    /// type docs).
    pub auto: bool,
    /// Delete the source branch after merging. **Not expressible here** — when
    /// set, [`mr_merge`](GitLabApi::mr_merge) returns `Error::Unsupported` instead
    /// of silently leaving the branch in place.
    pub delete_branch: bool,
}

impl MrMerge {
    /// Merge with a merge commit (glab's default — no strategy flag).
    pub fn merge() -> Self {
        Self::with(MergeStrategy::Merge)
    }

    /// Squash-merge (`--squash`).
    pub fn squash() -> Self {
        Self::with(MergeStrategy::Squash)
    }

    /// Rebase-merge (`--rebase`).
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

    /// Request auto-merge (merge once requirements are met). **Unsupported on
    /// `glab`**: setting this makes [`mr_merge`](GitLabApi::mr_merge) return
    /// `Error::Unsupported` (see the type docs).
    pub fn auto(mut self) -> Self {
        self.auto = true;
        self
    }

    /// Request deleting the source branch after merging. **Unsupported on
    /// `glab`**: setting this makes [`mr_merge`](GitLabApi::mr_merge) return
    /// `Error::Unsupported`.
    pub fn delete_branch(mut self) -> Self {
        self.delete_branch = true;
        self
    }
}

/// The GitLab operations this crate exposes — the interface consumers code
/// against and mock in tests. The **lean MR lifecycle**; reach unmodelled `glab`
/// commands through [`run`](GitLabApi::run).
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitLabApi: Send + Sync {
    /// Run `glab <args>` **in the process's current directory**, returning trimmed
    /// stdout (throws on a non-zero exit). A raw escape hatch — you supply the whole
    /// argv, so pass `-R group/project` to target a specific repo. This method on the
    /// client is the **process-cwd** escape hatch; the `at(dir)` bound view's
    /// [`run`](GitLabAt::run) is instead **bound to `dir`** (it forwards to
    /// [`GitLab::run_in`], so `glab.at(dir).run(…)` runs in the bound project's cwd,
    /// like [`api`](GitLabApi::api)). Use `glab.at(dir).run(…)` (or [`GitLab::run_in`])
    /// for the bound project (T-035).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitLabApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Make an authenticated GitLab API request through glab (`glab api
    /// <endpoint>`), returning the raw response body — the escape hatch for any
    /// REST/GraphQL endpoint this crate doesn't model (mirrors
    /// [`GitHubApi::api`](../vcs_github/trait.GitHubApi.html#tymethod.api)). Run in
    /// `dir` so a relative endpoint's `:id`/project placeholder resolves against the
    /// bound repository, not whatever repo the process's current directory is in. The
    /// `endpoint` is guarded against being parsed as a flag (empty or leading `-`
    /// is refused before spawning); pass query/body flags via [`run`](GitLabApi::run).
    async fn api(&self, dir: &Path, endpoint: &str) -> Result<String>;
    /// Installed GitLab CLI version (`glab --version`).
    async fn version(&self) -> Result<String>;
    /// Whether the user is authenticated (`glab auth status` exits zero). Reflects
    /// the exit code as a bool — any non-zero exit reads as `false`, never an
    /// error; only a spawn failure or timeout errors.
    ///
    /// **Caveat:** this reflects glab's exit code, and a long-standing glab bug
    /// ([gitlab-org/cli#911]) can make `glab auth status` exit `0` even when *not*
    /// authenticated, so a `true` here is a best-effort signal, not a guarantee —
    /// a subsequent API call is the real test. A `false`, a spawn failure, or a
    /// timeout are still reported faithfully.
    ///
    /// [gitlab-org/cli#911]: https://gitlab.com/gitlab-org/cli/-/issues/911
    async fn auth_status(&self) -> Result<bool>;
    /// The project for `dir` (`glab repo view --output json`).
    async fn repo_view(&self, dir: &Path) -> Result<RepoView>;
    /// Open merge requests for `dir`
    /// (`glab mr list --per-page 100 --output json`). Returns up to 100 (100 is
    /// the GitLab API per-page max); use [`run`](GitLabApi::run) for more.
    async fn mr_list(&self, dir: &Path) -> Result<Vec<MergeRequest>>;
    /// A single merge request by its project-scoped number — GitLab's `iid`
    /// (`glab mr view <number> --output json`). Named `number` for consistency
    /// with the issue methods and the other forge wrappers (`vcs-github`/
    /// `vcs-gitea`); the underlying value is GitLab's `iid`.
    async fn mr_view(&self, dir: &Path, number: u64) -> Result<MergeRequest>;
    /// Open a merge request, returning the command's output (the MR URL on
    /// success) (`glab mr create`). The [`MrCreate`] spec carries the title,
    /// body, and the optional source (`None` = the current branch) and target
    /// (`None` = the project default) branches. A body that is *exactly* `"-"`
    /// is glab's own stdin/editor sentinel (not the literal string) — refused
    /// with an `Error::Spawn` whose source is `io::ErrorKind::InvalidInput`
    /// before anything spawns, so a headless caller never hangs waiting on an
    /// editor/stdin that never comes.
    async fn mr_create(&self, dir: &Path, spec: MrCreate) -> Result<String>;
    /// Merge a merge request **immediately** (`glab mr merge <id> --yes
    /// --auto-merge=false [--squash|--rebase]`) — `--auto-merge=false` overrides
    /// glab's default of enabling merge-when-pipeline-succeeds. Takes a [`MrMerge`]
    /// spec (the [`MergeStrategy`] plus the gh-style `auto`/`delete_branch`
    /// options). `glab` can express **neither** `auto` nor `delete_branch` through
    /// this wrapper, so requesting either returns a structured `Error::Unsupported`
    /// rather than silently dropping it (see [`MrMerge`]).
    async fn mr_merge(&self, dir: &Path, number: u64, merge: MrMerge) -> Result<()>;
    /// Mark a draft merge request as ready (`glab mr update <id> --ready`).
    async fn mr_mark_ready(&self, dir: &Path, number: u64) -> Result<()>;
    /// Close a merge request without merging (`glab mr close <id>`).
    async fn mr_close(&self, dir: &Path, number: u64) -> Result<()>;
    /// Check out a merge request's source branch into the working copy at `dir`
    /// (`glab mr checkout <id>`) — the branch is fetched and switched to, so a
    /// subsequent build/test/edit runs against the MR locally. Mutates the working
    /// copy. **Defaulted** to `Error::Unsupported` so external implementers keep
    /// compiling when the crate bumps.
    #[allow(unused_variables)]
    async fn mr_checkout(&self, dir: &Path, number: u64) -> Result<()> {
        Err(Error::Unsupported {
            operation: "mr_checkout".into(),
        })
    }
    /// Add a comment to a merge request, returning the command's output
    /// (`glab mr note <id> -m <message>`). The note body rides in a
    /// flag-VALUE position, so no argv-injection guard is needed — but a body
    /// that is *exactly* `"-"` is glab's stdin/editor sentinel, refused with
    /// an `Error::Spawn` whose source is `io::ErrorKind::InvalidInput` before
    /// anything spawns (same rule as [`mr_create`](GitLabApi::mr_create)).
    /// **Defaulted** to
    /// `Error::Unsupported` so external implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn mr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String> {
        Err(Error::Unsupported {
            operation: "mr_comment".into(),
        })
    }
    /// Edit a merge request's title and/or description
    /// (`glab mr update <id> [--title <title>] [--description <body>] --yes`).
    /// At least one of `title` or `body` must be `Some` — the facade rejects
    /// both-`None` before reaching the wrapper. `--yes` skips glab's
    /// confirmation prompt. A `Some` body that is *exactly* `"-"` is glab's
    /// stdin/editor sentinel, refused with an `Error::Spawn` whose source is
    /// `io::ErrorKind::InvalidInput` before anything spawns (same rule as
    /// [`mr_create`](GitLabApi::mr_create)). **Defaulted** to
    /// `Error::Unsupported`.
    #[allow(unused_variables)]
    async fn mr_edit(&self, dir: &Path, number: u64, edit: MrEdit) -> Result<()> {
        Err(Error::Unsupported {
            operation: "mr_edit".into(),
        })
    }
    /// The MR's pipeline status, bucketed (`glab mr view <id> --output json`,
    /// reading `head_pipeline.status`). [`CiStatus::None`] when no pipeline ran.
    async fn mr_checks(&self, dir: &Path, number: u64) -> Result<CiStatus>;
    /// The MR's diff, one [`FileDiff`] per changed file (`glab mr diff <id>
    /// --color never`), through the same unified-diff parser
    /// [`vcs-git`](https://docs.rs/vcs-git)/[`vcs-jj`](https://docs.rs/vcs-jj)
    /// use — `glab mr diff` emits the same git-format diff `git diff` does.
    async fn mr_diff(&self, dir: &Path, number: u64) -> Result<Vec<FileDiff>>;
    /// Open issues for `dir`
    /// (`glab issue list --per-page 100 --output json`). Returns up to 100 (100
    /// is the GitLab API per-page max); use [`run`](GitLabApi::run) for more.
    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>>;
    /// A single issue by its project-scoped id (`iid`)
    /// (`glab issue view <number> --output json`).
    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue>;
    /// Open an issue, returning the command's output (the issue URL on success)
    /// (`glab issue create --title <t> --description <d> --yes`). `--yes` skips
    /// glab's interactive submission prompt — mirrors
    /// [`mr_create`](GitLabApi::mr_create), including the same `"-"`
    /// dash-sentinel guard on `body` (refused with an `Error::Spawn` whose
    /// source is `io::ErrorKind::InvalidInput` before anything spawns).
    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String>;
    /// Releases for `dir` (`glab release list --per-page 100 --output json`).
    /// Returns up to 100 (100 is the GitLab API per-page max); use
    /// [`run`](GitLabApi::run) for more.
    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>>;
    /// A single release by its tag (`glab release view <tag> --output json`).
    /// The `tag` is a bare positional, so it is guarded with
    /// `reject_flag_like` (a leading `-` or empty value is rejected before any
    /// process spawns).
    async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release>;
}

vcs_cli_support::managed_client! {
    /// The real GitLab client. Generic over the [`ProcessRunner`] so tests can inject
    /// a fake process executor; [`GitLab::new`] uses the real job-backed runner.
    ///
    /// Wraps a [`ManagedClient`](vcs_cli_support::ManagedClient). By default it authenticates through `glab`'s own
    /// ambient login; attach a [`CredentialProvider`] with
    /// [`with_credentials`](GitLab::with_credentials) to supply a token per operation
    /// — it is injected as `GITLAB_TOKEN` on every `glab` invocation.
    pub struct GitLab => BINARY, token_env = (CredentialService::GitLab, "GITLAB_TOKEN")
}

impl<R: ProcessRunner> GitLab<R> {
    /// Supply credentials per operation via a [`CredentialProvider`] — opt-in, off
    /// by default (ambient `glab` auth). The resolved token is injected as
    /// `GITLAB_TOKEN` on every `glab` invocation, overriding the ambient login.
    ///
    /// This client has **no host binding** yet, so each [`CredentialRequest`] carries
    /// no host. A simple provider ([`StaticCredential`] / [`EnvToken`]) is unaffected;
    /// a *host-keyed* provider sees `None` and should defer to ambient for a host it
    /// can't place (per [`ManagedClient::resolve_credential`](vcs_cli_support::ManagedClient::resolve_credential)),
    /// rather than substitute a default secret — so a self-hosted-vs-SaaS provider
    /// stays safe until GitLab grows an explicit host binding (as `vcs-github` has).
    #[must_use]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.core = self.core.with_credentials(provider);
        self
    }

    /// Convenience for the common case: authenticate with a single static `token`,
    /// injected as `GITLAB_TOKEN`. Shorthand for
    /// `with_credentials(Arc::new(StaticCredential::token(token)))`.
    #[must_use]
    pub fn with_token(self, token: impl Into<Secret>) -> Self {
        self.with_credentials(Arc::new(StaticCredential::token(token)))
    }

    /// Convenience: read the token from environment variable `var` at request time
    /// (injected as `GITLAB_TOKEN`); if `var` is unset/empty, fall back to ambient
    /// auth. Shorthand for `with_credentials(Arc::new(EnvToken::new(var)))`.
    #[must_use]
    pub fn with_env_token(self, var: impl Into<String>) -> Self {
        self.with_credentials(Arc::new(EnvToken::new(var)))
    }
}

#[async_trait::async_trait]
impl<R: ProcessRunner> GitLabApi for GitLab<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(args).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    async fn api(&self, dir: &Path, endpoint: &str) -> Result<String> {
        reject_flag_like("endpoint", endpoint)?;
        self.core
            .run(self.core.command_in(dir, ["api", endpoint]))
            .await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(["--version"]).await
    }

    async fn auth_status(&self) -> Result<bool> {
        // `glab auth status` exits 0 when authenticated, non-zero when not — an
        // exit-code answer. `exit_code` reads the exit code without erroring on a
        // non-zero one (a spawn failure or timeout still errors), so ANY non-zero
        // exit — not just the documented 1 — maps to "not authenticated" rather
        // than surfacing as an error (glab's exit codes are not contractual; see
        // the #911 caveat on the trait method). `probe` would reject an unusual
        // exit code.
        Ok(self.core.exit_code(["auth", "status"]).await? == 0)
    }

    async fn repo_view(&self, dir: &Path) -> Result<RepoView> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["repo", "view", "--output", "json"]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn mr_list(&self, dir: &Path) -> Result<Vec<MergeRequest>> {
        // `--per-page 100` (the GitLab API max) overrides glab's default page size
        // of 30, which would otherwise silently truncate the list.
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["mr", "list", "--per-page", "100", "--output", "json"]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn mr_view(&self, dir: &Path, number: u64) -> Result<MergeRequest> {
        let id = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["mr", "view", id.as_str(), "--output", "json"]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn mr_create(&self, dir: &Path, spec: MrCreate) -> Result<String> {
        // A literal `-` description is glab's stdin/editor sentinel, not the
        // string itself — refuse it before spawning (see `reject_dash_sentinel`).
        reject_dash_sentinel("description", spec.body.as_str())?;
        // `--yes` skips glab's interactive submission confirmation (a headless run
        // would otherwise hang waiting on the prompt).
        let mut args = vec![
            "mr",
            "create",
            "--title",
            spec.title.as_str(),
            "--description",
            spec.body.as_str(),
            "--yes",
        ];
        if let Some(source) = spec.source.as_deref() {
            args.push("--source-branch");
            args.push(source);
        }
        if let Some(target) = spec.target.as_deref() {
            args.push("--target-branch");
            args.push(target);
        }
        self.core.run(self.core.command_in(dir, args)).await
    }

    async fn mr_merge(&self, dir: &Path, number: u64, merge: MrMerge) -> Result<()> {
        // `glab mr merge` can express neither gh-style auto-merge nor
        // source-branch deletion through this wrapper: glab's own `--auto-merge`
        // is *merge-when-pipeline-succeeds* (a different contract from gh's
        // `--auto`), and we drive an *immediate* merge. Rather than silently
        // ignore a requested option — which, for an irreversible merge, could
        // produce the wrong side effects — report it as `Unsupported`. The default
        // (neither set) is the plain immediate merge.
        if merge.auto {
            return Err(Error::Unsupported {
                operation: "mr_merge(auto)".into(),
            });
        }
        if merge.delete_branch {
            return Err(Error::Unsupported {
                operation: "mr_merge(delete_branch)".into(),
            });
        }
        let id = number.to_string();
        // `--yes` skips the confirmation prompt. `--auto-merge=false` forces an
        // *immediate* merge: glab's `--auto-merge` defaults to `true`, which —
        // with a running pipeline — would enable merge-when-pipeline-succeeds
        // instead of merging now, so a method named `mr_merge` wouldn't actually
        // merge. The strategy flag is added only for squash/rebase (a plain merge
        // commit is glab's default).
        let mut args = vec!["mr", "merge", id.as_str(), "--yes", "--auto-merge=false"];
        if let Some(flag) = merge.strategy.flag() {
            args.push(flag);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn mr_mark_ready(&self, dir: &Path, number: u64) -> Result<()> {
        let id = number.to_string();
        self.core
            .run_unit(
                self.core
                    .command_in(dir, ["mr", "update", id.as_str(), "--ready"]),
            )
            .await
    }

    async fn mr_close(&self, dir: &Path, number: u64) -> Result<()> {
        let id = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["mr", "close", id.as_str()]))
            .await
    }

    async fn mr_checkout(&self, dir: &Path, number: u64) -> Result<()> {
        // `number` is a `u64`, so it can never look like a flag — nothing to
        // guard. `glab mr checkout` fetches the MR's source branch and switches
        // the working copy to it (no structured output).
        let id = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["mr", "checkout", id.as_str()]))
            .await
    }

    async fn mr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String> {
        // `-m` is a flag-VALUE position; glab consumes the next token verbatim.
        // No `--yes` here: `mr note` is non-destructive in spirit (adds a
        // comment, doesn't change the MR's state) and doesn't trigger the
        // submission prompt `mr create` does.
        // A literal `-` note body is glab's stdin/editor sentinel, not the
        // string itself — refuse it before spawning (see `reject_dash_sentinel`).
        reject_dash_sentinel("comment body", body)?;
        let id = number.to_string();
        self.core
            .run(
                self.core
                    .command_in(dir, ["mr", "note", id.as_str(), "-m", body]),
            )
            .await
    }

    async fn mr_edit(&self, dir: &Path, number: u64, edit: MrEdit) -> Result<()> {
        // `--title` and `--description` are flag-VALUE positions: no argv-injection
        // guard needed. `--yes` skips the confirmation prompt `mr update` would
        // otherwise show when neither --fill nor --ready is passed.
        let id = number.to_string();
        let mut args = vec!["mr", "update", id.as_str()];
        if let Some(title) = edit.title.as_deref() {
            args.push("--title");
            args.push(title);
        }
        if let Some(body) = edit.body.as_deref() {
            // A literal `-` description is glab's stdin/editor sentinel, not the
            // string itself — refuse it before spawning.
            reject_dash_sentinel("description", body)?;
            args.push("--description");
            args.push(body);
        }
        args.push("--yes");
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn mr_checks(&self, dir: &Path, number: u64) -> Result<CiStatus> {
        let id = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["mr", "view", id.as_str(), "--output", "json"]),
                parse::parse_ci_status,
            )
            .await
    }

    async fn mr_diff(&self, dir: &Path, number: u64) -> Result<Vec<FileDiff>> {
        // `run_untrimmed`: a diff's trailing content is meaningful (a hunk's
        // last line, a missing trailing newline) — trimming it before parsing
        // could desync the parser from `git`'s own byte-exact output. `--color
        // never` keeps the output free of ANSI even if stdout were ever a tty.
        let id = number.to_string();
        let text = self
            .core
            .run_untrimmed(
                self.core
                    .command_in(dir, ["mr", "diff", id.as_str(), "--color", "never"]),
            )
            .await?;
        Ok(vcs_diff::parse_diff(&text))
    }

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        // `--per-page 100` (the GitLab API max) overrides glab's default page
        // size of 30, which would otherwise silently truncate the list.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    ["issue", "list", "--per-page", "100", "--output", "json"],
                ),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue> {
        let number = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["issue", "view", number.as_str(), "--output", "json"]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }

    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String> {
        // A literal `-` description is glab's stdin/editor sentinel, not the
        // string itself — refuse it before spawning (see `reject_dash_sentinel`).
        reject_dash_sentinel("description", body)?;
        // `--yes` skips glab's interactive submission confirmation (a headless
        // run would otherwise hang on the prompt) — same as `mr_create`.
        self.core
            .run(self.core.command_in(
                dir,
                [
                    "issue",
                    "create",
                    "--title",
                    title,
                    "--description",
                    body,
                    "--yes",
                ],
            ))
            .await
    }

    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>> {
        // `--per-page 100` (the GitLab API max) overrides glab's default page
        // size of 30, which would otherwise silently truncate the list.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    ["release", "list", "--per-page", "100", "--output", "json"],
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
                    .command_in(dir, ["release", "view", tag, "--output", "json"]),
                |s| vcs_cli_support::json::from_json(BINARY, s),
            )
            .await
    }
}

impl<R: ProcessRunner> GitLab<R> {
    /// Run `glab <args>` over string slices — `glab.run_args(&["mr", "list"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GitLabApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(args).await
    }

    /// Like [`run_args`](GitLab::run_args) but never errors on a non-zero exit
    /// (mirrors [`GitLabApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    /// Run `glab <args>` **in `dir`** (the process is spawned with `dir` as its
    /// working directory, so `glab` infers the project from `dir`'s remote),
    /// returning trimmed stdout — the dir-bound twin of the process-cwd
    /// [`run`](GitLabApi::run). This is what [`GitLabAt::run`] forwards to; call
    /// [`run`](GitLabApi::run) on the client for the process-cwd escape hatch. Argv
    /// is forwarded verbatim (only the working directory is bound, no `-R`/extra
    /// flag is injected).
    pub async fn run_in(&self, dir: &Path, args: &[String]) -> Result<String> {
        self.core.run(self.core.command_in(dir, args)).await
    }

    /// Like [`run_in`](GitLab::run_in) but never errors on a non-zero exit — the
    /// dir-bound twin of [`run_raw`](GitLabApi::run_raw). What [`GitLabAt::run_raw`]
    /// forwards to.
    pub async fn run_raw_in(&self, dir: &Path, args: &[String]) -> Result<ProcessResult<String>> {
        self.core
            .output_string(self.core.command_in(dir, args))
            .await
    }

    /// Like [`run_args`](GitLab::run_args) but **bound to `dir`** — the `&[&str]`
    /// twin of [`run_in`](GitLab::run_in). What [`GitLabAt::run_args`] forwards to.
    pub async fn run_args_in(&self, dir: &Path, args: &[&str]) -> Result<String> {
        self.core.run(self.core.command_in(dir, args)).await
    }

    /// Like [`run_raw_args`](GitLab::run_raw_args) but **bound to `dir`** — the
    /// `&[&str]` twin of [`run_raw_in`](GitLab::run_raw_in). What
    /// [`GitLabAt::run_raw_args`] forwards to.
    pub async fn run_raw_args_in(
        &self,
        dir: &Path,
        args: &[&str],
    ) -> Result<ProcessResult<String>> {
        self.core
            .output_string(self.core.command_in(dir, args))
            .await
    }

    /// Bind a working directory, so the project-scoped methods omit that argument:
    /// `glab.at(dir).mr_list()` runs [`mr_list`](GitLabApi::mr_list) against `dir`.
    pub fn at<'a>(&'a self, dir: &'a Path) -> GitLabAt<'a, R> {
        GitLabAt { glab: self, dir }
    }
}

/// A [`GitLab`] client with a working directory bound, so its project-scoped
/// methods drop the leading `dir` argument (`glab.at(dir).mr_list()`). Construct
/// one with [`GitLab::at`].
pub struct GitLabAt<'a, R: ProcessRunner = processkit::JobRunner> {
    glab: &'a GitLab<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: holding only references, the view is `Copy`
// for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy` bound the
// default `JobRunner` doesn't satisfy, silently dropping `Copy` on the handle.
impl<R: ProcessRunner> Clone for GitLabAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for GitLabAt<'_, R> {}

// Generate [`GitLabAt`] forwarders: `bare` methods forward verbatim, `dir`
// methods inject `self.dir` as the first argument. The shared macro lives in
// `vcs-cli-support` (see `vcs_cli_support::at_forwarders!`).
vcs_cli_support::at_forwarders! {
    GitLabAt, glab, "GitLab",
    bare {
        fn version() -> Result<String>;
        fn auth_status() -> Result<bool>;
    }
    dir {
        fn api(endpoint: &str) -> Result<String>;
        fn repo_view() -> Result<RepoView>;
        fn mr_list() -> Result<Vec<MergeRequest>>;
        fn mr_view(number: u64) -> Result<MergeRequest>;
        fn mr_create(spec: MrCreate) -> Result<String>;
        fn mr_merge(number: u64, merge: MrMerge) -> Result<()>;
        fn mr_mark_ready(number: u64) -> Result<()>;
        fn mr_close(number: u64) -> Result<()>;
        fn mr_checkout(number: u64) -> Result<()>;
        fn mr_comment(number: u64, body: &str) -> Result<String>;
        fn mr_edit(number: u64, edit: MrEdit) -> Result<()>;
        fn mr_checks(number: u64) -> Result<CiStatus>;
        fn mr_diff(number: u64) -> Result<Vec<FileDiff>>;
        fn issue_list() -> Result<Vec<Issue>>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn release_list() -> Result<Vec<Release>>;
        fn release_view(tag: &str) -> Result<Release>;
    }
    // Raw escape hatches: bound to `self.dir` (forward to the client's `*_in`
    // twins) so `glab.at(dir).run(…)` targets the bound project's cwd, not the
    // process cwd. For the process-cwd hatch call `run`/`run_raw`/… on `GitLab`
    // directly.
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
    fn binary_name_is_glab() {
        assert_eq!(BINARY, "glab");
    }

    // Compile-time guard: the bound view stays `Copy` for the default `JobRunner`.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GitLabAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`glab.at(dir)`) must produce byte-identical argv to the
    // dir-taking call.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let glab = GitLab::with_runner(&rec);

        glab.mr_list(dir).await.unwrap();
        glab.at(dir).mr_list().await.unwrap();
        glab.mr_mark_ready(dir, 7).await.unwrap();
        glab.at(dir).mr_mark_ready(7).await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
    }

    // T-035: the raw escape hatches reached *through* the bound view
    // (`glab.at(dir).run…`) now run in the bound `dir`, while the same-named methods
    // on the client stay in the process cwd.
    #[tokio::test]
    async fn bound_view_raw_hatch_runs_in_bound_dir() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);

        // Through the bound view: every raw form carries the bound dir as its cwd.
        glab.at(dir)
            .run(&["mr".to_string(), "list".to_string()])
            .await
            .unwrap();
        let _ = glab
            .at(dir)
            .run_raw(&["mr".to_string(), "list".to_string()])
            .await
            .unwrap();
        glab.at(dir).run_args(&["mr", "list"]).await.unwrap();
        let _ = glab.at(dir).run_raw_args(&["mr", "list"]).await.unwrap();
        // On the client directly: the process-cwd escape hatch (no bound dir).
        glab.run(&["mr".to_string(), "list".to_string()])
            .await
            .unwrap();
        let _ = glab
            .run_raw(&["mr".to_string(), "list".to_string()])
            .await
            .unwrap();
        glab.run_args(&["mr", "list"]).await.unwrap();
        let _ = glab.run_raw_args(&["mr", "list"]).await.unwrap();

        let calls = rec.calls();
        for c in &calls[0..4] {
            assert_eq!(
                c.cwd.as_deref(),
                Some(dir),
                "raw call through the bound view runs in the bound dir"
            );
            assert_eq!(c.args_str(), ["mr", "list"]);
        }
        for c in &calls[4..8] {
            assert_eq!(
                c.cwd.as_deref(),
                None,
                "raw call on the client stays in the process cwd"
            );
            assert_eq!(c.args_str(), ["mr", "list"]);
        }
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let glab = GitLab::with_runner(
            ScriptedRunner::new().on(["glab", "api", "/version"], Reply::ok("ok\n")),
        );
        assert_eq!(glab.run_args(&["api", "/version"]).await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn api_builds_endpoint_and_guards_flags() {
        let rec = RecordingRunner::replying(Reply::ok("{}\n"));
        let glab = GitLab::with_runner(&rec);
        glab.api(Path::new("/repo"), "/projects/1")
            .await
            .expect("api");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["api", "/projects/1"]);
        // H9: the request runs in the bound repo dir, so glab resolves the project
        // from *that* repo's remote — not the process's current directory.
        assert_eq!(call.cwd, Some(std::path::PathBuf::from("/repo")));
        // A flag-like endpoint is refused before spawning.
        let glab = GitLab::with_runner(ScriptedRunner::new());
        assert!(glab.api(Path::new("/repo"), "-X").await.is_err());
        assert!(glab.api(Path::new("/repo"), "").await.is_err());
    }

    // Hermetic: real mr_list() arg-building + JSON deserialization against canned
    // output — no `glab` binary or network needed, so this runs on CI.
    #[tokio::test]
    async fn mr_list_parses_scripted_json() {
        let json = r#"[{"iid":7,"title":"Add X","state":"opened","source_branch":"feat/x","target_branch":"main","web_url":"u","draft":false}]"#;
        let glab =
            GitLab::with_runner(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
        let mrs = glab.mr_list(Path::new(".")).await.expect("mr_list");
        assert_eq!(mrs.len(), 1);
        assert_eq!(mrs[0].iid, 7);
        assert_eq!(mrs[0].target_branch, "main");
    }

    // mr_list builds the `--per-page 100 --output json` argv — the explicit
    // per-page max overrides glab's default page size (30) so the list is not
    // silently truncated.
    #[tokio::test]
    async fn mr_list_builds_output_json_argv() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let glab = GitLab::with_runner(&rec);
        glab.mr_list(Path::new("/repo")).await.expect("mr_list");
        assert_eq!(
            rec.only_call().args_str(),
            ["mr", "list", "--per-page", "100", "--output", "json"]
        );
    }

    // A credential provider injects the token as GITLAB_TOKEN (glab's own
    // non-interactive auth env) — never in argv; no provider → no token env.
    #[tokio::test]
    async fn with_credentials_injects_gitlab_token_and_default_does_not() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let glab = GitLab::with_runner(&rec)
            .with_credentials(Arc::new(StaticCredential::token("glpat-xyz")));
        glab.mr_list(Path::new("/repo")).await.expect("mr_list");
        let call = rec.only_call();
        let token = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("GITLAB_TOKEN"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(token, Some("glpat-xyz"), "token injected as GITLAB_TOKEN");
        assert!(
            !call.args_str().iter().any(|a| a.contains("glpat-xyz")),
            "secret must never appear in argv"
        );

        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let glab = GitLab::with_runner(&rec);
        glab.mr_list(Path::new("/repo")).await.expect("mr_list");
        assert!(
            !rec.only_call()
                .envs
                .iter()
                .any(|(k, _)| k.to_str() == Some("GITLAB_TOKEN")),
            "no provider → no token env (ambient glab auth)"
        );
    }

    // The `with_token` convenience injects GITLAB_TOKEN (parity with `with_credentials`).
    #[tokio::test]
    async fn with_token_convenience_injects_gitlab_token() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let glab = GitLab::with_runner(&rec).with_token("glpat-conv");
        glab.mr_list(Path::new("/repo")).await.expect("mr_list");
        let call = rec.only_call();
        let token = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("GITLAB_TOKEN"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(token, Some("glpat-conv"));
    }

    // Hermetic: auth_status reflects the exit code without erroring. ANY non-zero
    // exit — not just the documented 1 — must read as `false`, never an error.
    #[tokio::test]
    async fn auth_status_reads_exit_code() {
        let yes = GitLab::with_runner(ScriptedRunner::new().on(["glab", "auth"], Reply::ok("")));
        assert!(yes.auth_status().await.unwrap());
        let no = GitLab::with_runner(
            ScriptedRunner::new().on(["glab", "auth"], Reply::fail(1, "not logged in")),
        );
        assert!(!no.auth_status().await.unwrap());
        // An unexpected exit code (e.g. 2) is still just "not authenticated".
        let weird =
            GitLab::with_runner(ScriptedRunner::new().on(["glab", "auth"], Reply::fail(2, "boom")));
        assert!(!weird.auth_status().await.unwrap());
    }

    // A timed-out auth check must error, not silently report "not authenticated".
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let glab =
            GitLab::with_runner(ScriptedRunner::new().on(["glab", "auth"], Reply::timeout()));
        assert!(matches!(
            glab.auth_status().await.unwrap_err(),
            Error::Timeout { .. }
        ));
    }

    // mr_create assembles title/description/--yes, then the optional source/target
    // branch flags, and returns the trimmed output (the MR URL).
    #[tokio::test]
    async fn mr_create_appends_source_and_target() {
        let rec = RecordingRunner::replying(Reply::ok("https://gl/mr/9\n"));
        let glab = GitLab::with_runner(&rec);
        let url = glab
            .mr_create(
                Path::new("/repo"),
                MrCreate::new("T", "B").source("feat/x").target("main"),
            )
            .await
            .expect("mr_create");
        assert_eq!(url, "https://gl/mr/9");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "mr",
                "create",
                "--title",
                "T",
                "--description",
                "B",
                "--yes",
                "--source-branch",
                "feat/x",
                "--target-branch",
                "main"
            ]
        );
    }

    // With no source/target, mr_create omits both branch flags.
    #[tokio::test]
    async fn mr_create_omits_branch_flags_when_none() {
        let rec = RecordingRunner::replying(Reply::ok("https://gl/mr/1\n"));
        let glab = GitLab::with_runner(&rec);
        glab.mr_create(Path::new("/repo"), MrCreate::new("T", "B"))
            .await
            .expect("mr_create");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "mr",
                "create",
                "--title",
                "T",
                "--description",
                "B",
                "--yes"
            ]
        );
    }

    // A literal `-` description/comment body is glab's stdin/editor sentinel —
    // every entry point that carries one must refuse it BEFORE any spawn (the
    // scripted runner has no rule, so a leak-through would fail differently),
    // and a non-sentinel value (even one that merely contains a dash) must go
    // through untouched.
    #[tokio::test]
    async fn dash_sentinel_body_rejected_before_spawn_everywhere() {
        let no_run = || GitLab::with_runner(ScriptedRunner::new());

        let err = no_run()
            .mr_create(Path::new("/repo"), MrCreate::new("T", "-"))
            .await
            .expect_err("mr_create must reject a bare dash body");
        assert!(
            matches!(&err, Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput),
            "expected Spawn(InvalidInput), got {err:?}"
        );

        let err = no_run()
            .mr_edit(Path::new("/repo"), 1, MrEdit::new().body("-"))
            .await
            .expect_err("mr_edit must reject a bare dash body");
        assert!(
            matches!(&err, Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput),
            "expected Spawn(InvalidInput), got {err:?}"
        );

        let err = no_run()
            .issue_create(Path::new("/repo"), "T", "-")
            .await
            .expect_err("issue_create must reject a bare dash body");
        assert!(
            matches!(&err, Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput),
            "expected Spawn(InvalidInput), got {err:?}"
        );

        let err = no_run()
            .mr_comment(Path::new("/repo"), 1, "-")
            .await
            .expect_err("mr_comment must reject a bare dash body");
        assert!(
            matches!(&err, Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput),
            "expected Spawn(InvalidInput), got {err:?}"
        );

        // A value that merely *contains* a dash (not exactly "-") is a real,
        // literal body — it must pass through untouched, byte-exact, for every
        // entry point that carries a guarded body (not just mr_create).
        let rec = RecordingRunner::replying(Reply::ok("https://gl/mr/9\n"));
        let glab = GitLab::with_runner(&rec);
        glab.mr_create(Path::new("/repo"), MrCreate::new("T", "- not a sentinel"))
            .await
            .expect("a body that isn't exactly \"-\" must be accepted");
        assert!(
            rec.only_call()
                .args_str()
                .iter()
                .any(|a| a == "- not a sentinel"),
            "the literal body must reach argv byte-exact"
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_edit(
            Path::new("/repo"),
            1,
            MrEdit::new().body("- not a sentinel"),
        )
        .await
        .expect("a body that isn't exactly \"-\" must be accepted");
        assert!(
            rec.only_call()
                .args_str()
                .iter()
                .any(|a| a == "- not a sentinel"),
            "the literal body must reach mr_edit's argv byte-exact"
        );

        let rec = RecordingRunner::replying(Reply::ok("https://gl/i/9\n"));
        let glab = GitLab::with_runner(&rec);
        glab.issue_create(Path::new("/repo"), "T", "- not a sentinel")
            .await
            .expect("a body that isn't exactly \"-\" must be accepted");
        assert!(
            rec.only_call()
                .args_str()
                .iter()
                .any(|a| a == "- not a sentinel"),
            "the literal body must reach issue_create's argv byte-exact"
        );

        let rec = RecordingRunner::replying(Reply::ok("https://gl/mr/1#note_1\n"));
        let glab = GitLab::with_runner(&rec);
        glab.mr_comment(Path::new("/repo"), 1, "- not a sentinel")
            .await
            .expect("a body that isn't exactly \"-\" must be accepted");
        assert!(
            rec.only_call()
                .args_str()
                .iter()
                .any(|a| a == "- not a sentinel"),
            "the literal body must reach mr_comment's argv byte-exact"
        );
    }

    // mr_merge adds `--yes --auto-merge=false`, and the strategy flag only for
    // squash/rebase. The default `MrMerge` (no auto/delete_branch) is the plain
    // immediate merge.
    #[tokio::test]
    async fn mr_merge_builds_strategy_argv() {
        for (merge, expected) in [
            (
                MrMerge::merge(),
                vec!["mr", "merge", "5", "--yes", "--auto-merge=false"],
            ),
            (
                MrMerge::squash(),
                vec![
                    "mr",
                    "merge",
                    "5",
                    "--yes",
                    "--auto-merge=false",
                    "--squash",
                ],
            ),
            (
                MrMerge::rebase(),
                vec![
                    "mr",
                    "merge",
                    "5",
                    "--yes",
                    "--auto-merge=false",
                    "--rebase",
                ],
            ),
        ] {
            let rec = RecordingRunner::replying(Reply::ok(""));
            let glab = GitLab::with_runner(&rec);
            glab.mr_merge(Path::new("/repo"), 5, merge)
                .await
                .expect("mr_merge");
            assert_eq!(rec.only_call().args_str(), expected);
        }
    }

    // `glab` cannot express gh-style auto-merge or source-branch deletion, so
    // requesting either is a structured `Unsupported` — never a silent drop that
    // would merge with the wrong side effects. The check happens BEFORE any spawn
    // (the runner has no rule; a leak-through would fail differently).
    #[tokio::test]
    async fn mr_merge_rejects_unexpressible_options() {
        for merge in [MrMerge::squash().auto(), MrMerge::merge().delete_branch()] {
            let glab = GitLab::with_runner(ScriptedRunner::new());
            let err = glab
                .mr_merge(Path::new("/repo"), 5, merge)
                .await
                .expect_err("auto/delete_branch are Unsupported on glab");
            assert!(
                matches!(err, Error::Unsupported { .. }),
                "expected Unsupported, got {err:?}"
            );
        }
    }

    // mr_mark_ready maps to `mr update <id> --ready`; mr_close to `mr close <id>`.
    #[tokio::test]
    async fn mr_mark_ready_and_close_build_expected_argv() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_mark_ready(Path::new("/repo"), 3)
            .await
            .expect("ready");
        assert_eq!(rec.only_call().args_str(), ["mr", "update", "3", "--ready"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_close(Path::new("/repo"), 3).await.expect("close");
        assert_eq!(rec.only_call().args_str(), ["mr", "close", "3"]);
    }

    // mr_checkout maps to `mr checkout <id>` and runs in the bound repo dir; the
    // bound view produces byte-identical argv.
    #[tokio::test]
    async fn mr_checkout_builds_expected_argv() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_checkout(Path::new("/repo"), 7)
            .await
            .expect("mr_checkout");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["mr", "checkout", "7"]);
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/repo")));

        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.at(Path::new("/repo"))
            .mr_checkout(7)
            .await
            .expect("mr_checkout");
        assert_eq!(rec.only_call().args_str(), ["mr", "checkout", "7"]);
    }

    // mr_checks reads the MR's head_pipeline status and buckets it.
    #[tokio::test]
    async fn mr_checks_buckets_pipeline_status() {
        let json = r#"{"iid":4,"head_pipeline":{"status":"failed"}}"#;
        let glab =
            GitLab::with_runner(ScriptedRunner::new().on(["glab", "mr", "view"], Reply::ok(json)));
        assert_eq!(
            glab.mr_checks(Path::new("."), 4).await.unwrap(),
            CiStatus::Failing
        );
    }

    // Hermetic: real mr_diff() arg-building (incl. `--color never`) + the
    // shared unified-diff parser against canned `glab mr diff` output.
    #[tokio::test]
    async fn mr_diff_builds_args_and_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let rec = RecordingRunner::replying(Reply::ok(out));
        let glab = GitLab::with_runner(&rec);
        let files = glab.mr_diff(Path::new("/r"), 4).await.expect("mr_diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
        assert_eq!(
            rec.only_call().args_str(),
            ["mr", "diff", "4", "--color", "never"]
        );
    }

    // issue_list builds the `--per-page 100 --output json` argv (per-page max
    // overrides glab's default page size of 30) and parses the JSON.
    #[tokio::test]
    async fn issue_list_builds_argv_and_parses() {
        let json = r#"[{"iid":3,"title":"Bug","state":"opened","description":"b","web_url":"u"}]"#;
        let rec = RecordingRunner::replying(Reply::ok(json));
        let glab = GitLab::with_runner(&rec);
        let issues = glab
            .issue_list(Path::new("/repo"))
            .await
            .expect("issue_list");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 3);
        assert_eq!(issues[0].state, "opened");
        assert_eq!(
            rec.only_call().args_str(),
            ["issue", "list", "--per-page", "100", "--output", "json"]
        );
    }

    // issue_view builds `issue view <number> --output json` and parses the JSON.
    #[tokio::test]
    async fn issue_view_builds_argv_and_parses() {
        let json = r#"{"iid":7,"title":"T","state":"closed","description":"body","web_url":"https://gl/i/7"}"#;
        let rec = RecordingRunner::replying(Reply::ok(json));
        let glab = GitLab::with_runner(&rec);
        let issue = glab
            .issue_view(Path::new("/repo"), 7)
            .await
            .expect("issue_view");
        assert_eq!(issue.number, 7);
        assert_eq!(issue.body, "body");
        assert_eq!(issue.url, "https://gl/i/7");
        assert_eq!(
            rec.only_call().args_str(),
            ["issue", "view", "7", "--output", "json"]
        );
    }

    // issue_create assembles title/description/--yes and returns the trimmed
    // output (the issue URL).
    #[tokio::test]
    async fn issue_create_builds_argv_and_returns_url() {
        let rec = RecordingRunner::replying(Reply::ok("https://gl/i/9\n"));
        let glab = GitLab::with_runner(&rec);
        let url = glab
            .issue_create(Path::new("/repo"), "T", "B")
            .await
            .expect("issue_create");
        assert_eq!(url, "https://gl/i/9");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "issue",
                "create",
                "--title",
                "T",
                "--description",
                "B",
                "--yes"
            ]
        );
    }

    // release_list builds the `--per-page 100 --output json` argv and parses the
    // JSON (URL comes off `_links.self`, date off `released_at`).
    #[tokio::test]
    async fn release_list_builds_argv_and_parses() {
        let json = r#"[{"tag_name":"v1.0","name":"Release 1.0","released_at":"2026-01-02T03:04:05.000Z","_links":{"self":"https://gl/-/releases/v1.0"}}]"#;
        let rec = RecordingRunner::replying(Reply::ok(json));
        let glab = GitLab::with_runner(&rec);
        let releases = glab
            .release_list(Path::new("/repo"))
            .await
            .expect("release_list");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag_name, "v1.0");
        assert_eq!(releases[0].url, "https://gl/-/releases/v1.0");
        assert_eq!(releases[0].published_at, "2026-01-02T03:04:05.000Z");
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "list", "--per-page", "100", "--output", "json"]
        );
    }

    // release_view builds `release view <tag> --output json` and parses the JSON.
    #[tokio::test]
    async fn release_view_builds_argv_and_parses() {
        let json =
            r#"{"tag_name":"v2.1","name":"R","_links":{"self":"https://gl/-/releases/v2.1"}}"#;
        let rec = RecordingRunner::replying(Reply::ok(json));
        let glab = GitLab::with_runner(&rec);
        let rel = glab
            .release_view(Path::new("/repo"), "v2.1")
            .await
            .expect("release_view");
        assert_eq!(rel.tag_name, "v2.1");
        assert_eq!(rel.url, "https://gl/-/releases/v2.1");
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "view", "v2.1", "--output", "json"]
        );
    }

    // release_view guards its bare `<tag>` positional: a flag-like or empty tag
    // is rejected before any process spawns.
    #[tokio::test]
    async fn release_view_rejects_flag_like_tag() {
        let glab = GitLab::with_runner(ScriptedRunner::new());
        assert!(glab.release_view(Path::new("."), "-evil").await.is_err());
        assert!(glab.release_view(Path::new("."), "").await.is_err());
    }

    // mr_comment builds `mr note <id> -m <body>` and returns the trimmed
    // output. `-m` is the alias of `--message`; either is accepted by glab.
    #[tokio::test]
    async fn mr_comment_builds_argv_and_returns_output() {
        let rec = RecordingRunner::replying(Reply::ok("https://gl/mr/7#note_99\n"));
        let glab = GitLab::with_runner(&rec);
        let out = glab
            .mr_comment(Path::new("/r"), 7, "LGTM")
            .await
            .expect("mr_comment");
        assert_eq!(out, "https://gl/mr/7#note_99");
        assert_eq!(
            rec.only_call().args_str(),
            ["mr", "note", "7", "-m", "LGTM"]
        );
    }

    // mr_edit emits only the flags the caller set and appends --yes. Flag-VALUE
    // positions pass through verbatim — the facade rejects both-`None` before
    // reaching here.
    #[tokio::test]
    async fn mr_edit_emits_only_provided_fields() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);

        glab.mr_edit(Path::new("/r"), 7, MrEdit::new().title("New title"))
            .await
            .expect("title-only edit");
        glab.mr_edit(Path::new("/r"), 7, MrEdit::new().body("New body"))
            .await
            .expect("body-only edit");
        glab.mr_edit(Path::new("/r"), 7, MrEdit::new().title("T").body("B"))
            .await
            .expect("both-fields edit");

        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["mr", "update", "7", "--title", "New title", "--yes"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["mr", "update", "7", "--description", "New body", "--yes"]
        );
        assert_eq!(
            calls[2].args_str(),
            [
                "mr",
                "update",
                "7",
                "--title",
                "T",
                "--description",
                "B",
                "--yes"
            ]
        );
    }

    // An empty string is a real value (clears the field) — the argv must carry
    // `--title ""` literally, not silently drop it.
    #[tokio::test]
    async fn mr_edit_some_empty_string_clears_field() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_edit(Path::new("/r"), 7, MrEdit::new().title(""))
            .await
            .expect("empty title");
        assert_eq!(
            rec.only_call().args_str(),
            ["mr", "update", "7", "--title", "", "--yes"]
        );
    }

    // repo_view parses the GitLab Project JSON.
    #[tokio::test]
    async fn repo_view_parses_project() {
        let json = r#"{"name":"cli","path_with_namespace":"gitlab-org/cli","default_branch":"main","web_url":"u","visibility":"public"}"#;
        let glab = GitLab::with_runner(
            ScriptedRunner::new().on(["glab", "repo", "view"], Reply::ok(json)),
        );
        let p = glab.repo_view(Path::new(".")).await.expect("repo_view");
        assert_eq!(p.path_with_namespace, "gitlab-org/cli");
        assert_eq!(p.default_branch, "main");
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockGitLabApi::new();
        mock.expect_auth_status().returning(|| Ok(true));
        assert!(mock.auth_status().await.unwrap());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/gitlab.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
