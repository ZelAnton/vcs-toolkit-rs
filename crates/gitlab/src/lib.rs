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
//! create / merge / mark-ready / close) · CI/pipeline status · issues · releases.
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
//!   as the first argument and return typed results ([`Project`],
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
//!   `mr update`), and the [`MergeStrategy`] enum (`Merge`/`Squash`/`Rebase`) —
//!   `#[non_exhaustive]`, built with a constructor + chained setters, named after
//!   the flags they emit.
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
//! use vcs_gitlab::{GitLab, GitLabApi, MergeStrategy, MrCreate};
//! # async fn demo(glab: &GitLab) -> Result<(), processkit::Error> {
//! let dir = Path::new(".");
//! let url = glab
//!     .mr_create(dir, MrCreate::new("Add streaming", "Implements …").target("main"))
//!     .await?;                                          // the new MR's URL
//! glab.mr_merge(dir, 12, MergeStrategy::Squash).await?;
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

use processkit::ProcessRunner;
// The shared managed client (lock-retry + credential injection) and the
// credential seam — re-exported so a consumer can supply a token provider.
use vcs_cli_support::ManagedClient;
pub use vcs_cli_support::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, FnProvider,
    Secret, StaticCredential, provider_fn,
};
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency. (Cancellation is core in processkit
// 0.10 — always available, no feature.)
pub use processkit::CancellationToken;

mod parse;
pub use parse::{CiStatus, Issue, MergeRequest, Project, Release};

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
pub const BINARY: &str = "glab";

/// Injection guard for bare positional argv slots: a caller-supplied value with
/// a leading `-` would be parsed by glab's CLI as a *flag*, and an empty value
/// changes a command's meaning. Refuse both before anything spawns. Flag-VALUE
/// positions (`--title <t>`, `--source-branch <b>`) need no guard — glab consumes
/// the next token verbatim there.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
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

/// The GitLab operations this crate exposes — the interface consumers code
/// against and mock in tests. The **lean MR lifecycle**; reach unmodelled `glab`
/// commands through [`run`](GitLabApi::run).
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitLabApi: Send + Sync {
    /// Run `glab <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitLabApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Make an authenticated GitLab API request through glab (`glab api
    /// <endpoint>`), returning the raw response body — the escape hatch for any
    /// REST/GraphQL endpoint this crate doesn't model (mirrors
    /// [`GitHubApi::api`](../vcs_github/trait.GitHubApi.html#tymethod.api)). The
    /// `endpoint` is guarded against being parsed as a flag (empty or leading `-`
    /// is refused before spawning); pass query/body flags via [`run`](GitLabApi::run).
    async fn api(&self, endpoint: &str) -> Result<String>;
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
    async fn repo_view(&self, dir: &Path) -> Result<Project>;
    /// Open merge requests for `dir`
    /// (`glab mr list --per-page 100 --output json`). Returns up to 100 (100 is
    /// the GitLab API per-page max); use [`run`](GitLabApi::run) for more.
    async fn mr_list(&self, dir: &Path) -> Result<Vec<MergeRequest>>;
    /// A single merge request by its project-scoped id
    /// (`glab mr view <id> --output json`).
    async fn mr_view(&self, dir: &Path, id: u64) -> Result<MergeRequest>;
    /// Open a merge request, returning the command's output (the MR URL on
    /// success) (`glab mr create`). The [`MrCreate`] spec carries the title,
    /// body, and the optional source (`None` = the current branch) and target
    /// (`None` = the project default) branches.
    async fn mr_create(&self, dir: &Path, spec: MrCreate) -> Result<String>;
    /// Merge a merge request **immediately** (`glab mr merge <id> --yes
    /// --auto-merge=false [--squash|--rebase]`) — `--auto-merge=false` overrides
    /// glab's default of enabling merge-when-pipeline-succeeds. See
    /// [`MergeStrategy`].
    async fn mr_merge(&self, dir: &Path, id: u64, strategy: MergeStrategy) -> Result<()>;
    /// Mark a draft merge request as ready (`glab mr update <id> --ready`).
    async fn mr_ready(&self, dir: &Path, id: u64) -> Result<()>;
    /// Close a merge request without merging (`glab mr close <id>`).
    async fn mr_close(&self, dir: &Path, id: u64) -> Result<()>;
    /// Add a comment to a merge request, returning the command's output
    /// (`glab mr note <id> -m <message>`). The note body rides in a
    /// flag-VALUE position, so no argv-guard is needed. **Defaulted** to
    /// `Error::Unsupported` so external implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn mr_comment(&self, dir: &Path, id: u64, body: &str) -> Result<String> {
        Err(Error::Unsupported {
            operation: "mr_comment".into(),
        })
    }
    /// Edit a merge request's title and/or description
    /// (`glab mr update <id> [--title <title>] [--description <body>] --yes`).
    /// At least one of `title` or `body` must be `Some` — the facade rejects
    /// both-`None` before reaching the wrapper. `--yes` skips glab's
    /// confirmation prompt. **Defaulted** to `Error::Unsupported`.
    #[allow(unused_variables)]
    async fn mr_edit(&self, dir: &Path, id: u64, edit: MrEdit) -> Result<()> {
        Err(Error::Unsupported {
            operation: "mr_edit".into(),
        })
    }
    /// The MR's pipeline status, bucketed (`glab mr view <id> --output json`,
    /// reading `head_pipeline.status`). [`CiStatus::None`] when no pipeline ran.
    async fn mr_checks(&self, dir: &Path, id: u64) -> Result<CiStatus>;
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
    /// [`mr_create`](GitLabApi::mr_create).
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

/// The real GitLab client. Generic over the [`ProcessRunner`] so tests can inject
/// a fake process executor; [`GitLab::new`] uses the real job-backed runner.
///
/// Wraps a [`ManagedClient`]. By default it authenticates through `glab`'s own
/// ambient login; attach a [`CredentialProvider`] with
/// [`with_credentials`](GitLab::with_credentials) to supply a token per operation
/// — it is injected as `GITLAB_TOKEN` on every `glab` invocation.
pub struct GitLab<R: ProcessRunner = processkit::JobRunner> {
    core: ManagedClient<R>,
}

impl GitLab<processkit::JobRunner> {
    /// Create a client driving the real job-backed runner.
    pub fn new() -> Self {
        Self {
            core: ManagedClient::new(BINARY)
                .with_token_env(CredentialService::GitLab, "GITLAB_TOKEN"),
        }
    }
}

impl Default for GitLab<processkit::JobRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: ProcessRunner> GitLab<R> {
    /// Create a client driving `runner` — inject a fake in tests.
    pub fn with_runner(runner: R) -> Self {
        Self {
            core: ManagedClient::with_runner(BINARY, runner)
                .with_token_env(CredentialService::GitLab, "GITLAB_TOKEN"),
        }
    }

    /// Apply a default timeout to every command this client builds.
    pub fn default_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.core = self.core.default_timeout(timeout);
        self
    }

    /// Set an environment variable on every command this client builds.
    pub fn default_env(
        mut self,
        key: impl AsRef<std::ffi::OsStr>,
        value: impl AsRef<std::ffi::OsStr>,
    ) -> Self {
        self.core = self.core.default_env(key, value);
        self
    }

    /// Remove an inherited environment variable on every command this client builds.
    pub fn default_env_remove(mut self, key: impl AsRef<std::ffi::OsStr>) -> Self {
        self.core = self.core.default_env_remove(key);
        self
    }

    /// Cancel every command this client builds when `token` fires.
    pub fn default_cancel_on(mut self, token: CancellationToken) -> Self {
        self.core = self.core.default_cancel_on(token);
        self
    }

    /// Supply credentials per operation via a [`CredentialProvider`] — opt-in, off
    /// by default (ambient `glab` auth). The resolved token is injected as
    /// `GITLAB_TOKEN` on every `glab` invocation, overriding the ambient login.
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
        self.core.output(args).await
    }

    async fn api(&self, endpoint: &str) -> Result<String> {
        reject_flag_like("endpoint", endpoint)?;
        self.core.run(["api", endpoint]).await
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

    async fn repo_view(&self, dir: &Path) -> Result<Project> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["repo", "view", "--output", "json"]),
                parse::from_json,
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
                parse::from_json,
            )
            .await
    }

    async fn mr_view(&self, dir: &Path, id: u64) -> Result<MergeRequest> {
        let id = id.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["mr", "view", id.as_str(), "--output", "json"]),
                parse::from_json,
            )
            .await
    }

    async fn mr_create(&self, dir: &Path, spec: MrCreate) -> Result<String> {
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

    async fn mr_merge(&self, dir: &Path, id: u64, strategy: MergeStrategy) -> Result<()> {
        let id = id.to_string();
        // `--yes` skips the confirmation prompt. `--auto-merge=false` forces an
        // *immediate* merge: glab's `--auto-merge` defaults to `true`, which —
        // with a running pipeline — would enable merge-when-pipeline-succeeds
        // instead of merging now, so a method named `mr_merge` wouldn't actually
        // merge. The strategy flag is added only for squash/rebase (a plain merge
        // commit is glab's default).
        let mut args = vec!["mr", "merge", id.as_str(), "--yes", "--auto-merge=false"];
        if let Some(flag) = strategy.flag() {
            args.push(flag);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn mr_ready(&self, dir: &Path, id: u64) -> Result<()> {
        let id = id.to_string();
        self.core
            .run_unit(
                self.core
                    .command_in(dir, ["mr", "update", id.as_str(), "--ready"]),
            )
            .await
    }

    async fn mr_close(&self, dir: &Path, id: u64) -> Result<()> {
        let id = id.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["mr", "close", id.as_str()]))
            .await
    }

    async fn mr_comment(&self, dir: &Path, id: u64, body: &str) -> Result<String> {
        // `-m` is a flag-VALUE position; glab consumes the next token verbatim.
        // No `--yes` here: `mr note` is non-destructive in spirit (adds a
        // comment, doesn't change the MR's state) and doesn't trigger the
        // submission prompt `mr create` does.
        let id = id.to_string();
        self.core
            .run(
                self.core
                    .command_in(dir, ["mr", "note", id.as_str(), "-m", body]),
            )
            .await
    }

    async fn mr_edit(&self, dir: &Path, id: u64, edit: MrEdit) -> Result<()> {
        // `--title` and `--description` are flag-VALUE positions: no argv-guard
        // needed. `--yes` skips the confirmation prompt `mr update` would
        // otherwise show when neither --fill nor --ready is passed.
        let id = id.to_string();
        let mut args = vec!["mr", "update", id.as_str()];
        if let Some(title) = edit.title.as_deref() {
            args.push("--title");
            args.push(title);
        }
        if let Some(body) = edit.body.as_deref() {
            args.push("--description");
            args.push(body);
        }
        args.push("--yes");
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn mr_checks(&self, dir: &Path, id: u64) -> Result<CiStatus> {
        let id = id.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["mr", "view", id.as_str(), "--output", "json"]),
                parse::parse_ci_status,
            )
            .await
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
                parse::from_json,
            )
            .await
    }

    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue> {
        let number = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["issue", "view", number.as_str(), "--output", "json"]),
                parse::from_json,
            )
            .await
    }

    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String> {
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
                parse::from_json,
            )
            .await
    }

    async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release> {
        reject_flag_like("tag", tag)?;
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["release", "view", tag, "--output", "json"]),
                parse::from_json,
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
        self.core.output(args).await
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

/// Generate [`GitLabAt`] forwarders: `bare` methods forward verbatim, `dir`
/// methods inject `self.dir` as the first argument.
macro_rules! gitlab_at_forwarders {
    (
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
    ) => {
        impl<'a, R: ProcessRunner> GitLabAt<'a, R> {
            $(
                #[doc = concat!("Bound form of [`GitLab`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.glab.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`GitLab`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.glab.$dn(self.dir, $($da),*).await
                }
            )*
        }
    };
}

gitlab_at_forwarders! {
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn api(endpoint: &str) -> Result<String>;
        fn version() -> Result<String>;
        fn auth_status() -> Result<bool>;
    }
    dir {
        fn repo_view() -> Result<Project>;
        fn mr_list() -> Result<Vec<MergeRequest>>;
        fn mr_view(id: u64) -> Result<MergeRequest>;
        fn mr_create(spec: MrCreate) -> Result<String>;
        fn mr_merge(id: u64, strategy: MergeStrategy) -> Result<()>;
        fn mr_ready(id: u64) -> Result<()>;
        fn mr_close(id: u64) -> Result<()>;
        fn mr_comment(id: u64, body: &str) -> Result<String>;
        fn mr_edit(id: u64, edit: MrEdit) -> Result<()>;
        fn mr_checks(id: u64) -> Result<CiStatus>;
        fn issue_list() -> Result<Vec<Issue>>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn release_list() -> Result<Vec<Release>>;
        fn release_view(tag: &str) -> Result<Release>;
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
        glab.mr_ready(dir, 7).await.unwrap();
        glab.at(dir).mr_ready(7).await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
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
        glab.api("/projects/1").await.expect("api");
        assert_eq!(rec.only_call().args_str(), ["api", "/projects/1"]);
        // A flag-like endpoint is refused before spawning.
        let glab = GitLab::with_runner(ScriptedRunner::new());
        assert!(glab.api("-X").await.is_err());
        assert!(glab.api("").await.is_err());
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

    // mr_merge adds `--yes`, and the strategy flag only for squash/rebase.
    #[tokio::test]
    async fn mr_merge_builds_strategy_argv() {
        for (strategy, expected) in [
            (
                MergeStrategy::Merge,
                vec!["mr", "merge", "5", "--yes", "--auto-merge=false"],
            ),
            (
                MergeStrategy::Squash,
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
                MergeStrategy::Rebase,
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
            glab.mr_merge(Path::new("/repo"), 5, strategy)
                .await
                .expect("mr_merge");
            assert_eq!(rec.only_call().args_str(), expected);
        }
    }

    // mr_ready maps to `mr update <id> --ready`; mr_close to `mr close <id>`.
    #[tokio::test]
    async fn mr_ready_and_close_build_expected_argv() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_ready(Path::new("/repo"), 3).await.expect("ready");
        assert_eq!(rec.only_call().args_str(), ["mr", "update", "3", "--ready"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        let glab = GitLab::with_runner(&rec);
        glab.mr_close(Path::new("/repo"), 3).await.expect("close");
        assert_eq!(rec.only_call().args_str(), ["mr", "close", "3"]);
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
