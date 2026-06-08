#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-github` — automate GitHub from Rust by driving the `gh` CLI.
//!
//! It shells out to the installed `gh` binary and parses its output into typed
//! values — so you get *gh's own* behaviour, auth, and host resolution, not a
//! reimplementation of the GitHub REST/GraphQL API. The PR / issue / Actions /
//! release lifecycle, async throughout, with structured errors, and mockable.
//! Every command runs inside an OS **job** (via [`processkit`]) so a `gh`
//! subprocess tree can never be orphaned, and honours an optional per-client
//! [timeout](GitHub::default_timeout). Read-style methods ask `gh` for `--json`
//! and deserialize it; nothing scrapes human-readable output.
//!
//! # The surface
//!
//! - **[`GitHubApi`]** — the object-safe trait every operation lives on. Depend
//!   on `&dyn GitHubApi` (or generically on `impl GitHubApi`) so a test can swap
//!   the real client for a double. Repo-scoped methods take the working
//!   directory as the first argument and return typed results ([`PullRequest`],
//!   [`Issue`], [`Repo`], [`CheckRun`], [`WorkflowRun`], [`Release`],
//!   [`PrFeedback`], …) or a structured [`Error`].
//! - **[`GitHub`]** — the real client. [`GitHub::new`] uses the job-backed
//!   runner; [`GitHub::with_runner`] injects a fake one for tests. It is generic
//!   over the [`ProcessRunner`] seam, defaulting to the production runner.
//! - **[`GitHubAt`]** — a cwd-bound view ([`GitHub::at`]) whose methods drop the
//!   leading `dir`, so `gh.at(dir).pr_list()` reads as `gh.pr_list(dir)` — handy
//!   when one client drives one checkout.
//! - **Method groups** on the trait: PRs ([`pr_list`](GitHubApi::pr_list),
//!   [`pr_view`](GitHubApi::pr_view), [`pr_create`](GitHubApi::pr_create),
//!   [`pr_merge`](GitHubApi::pr_merge), [`pr_review`](GitHubApi::pr_review),
//!   [`pr_checks`](GitHubApi::pr_checks),
//!   [`pr_feedback`](GitHubApi::pr_feedback), …); Actions runs
//!   ([`run_list`](GitHubApi::run_list), [`run_view`](GitHubApi::run_view),
//!   [`run_watch`](GitHubApi::run_watch) — *blocking*, bounded by the client
//!   timeout); issues & releases ([`issue_create`](GitHubApi::issue_create),
//!   [`release_view`](GitHubApi::release_view), …); plus the escape hatches
//!   [`run`](GitHubApi::run) / [`api`](GitHubApi::api) for anything unmodelled.
//! - **Builder specs** for the multi-option commands — [`PrCreate`] (title/body
//!   with optional `head`/`base`), [`PrMerge`] (strategy [`MergeStrategy`],
//!   `--auto`, `--delete-branch`), and [`ReviewAction`] (whose private fields make
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
//! [`ScriptedRunner`](processkit::ScriptedRunner) with [`GitHub::with_runner`]
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

use processkit::ProcessRunner;
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};
// Re-exported under the `cancellation` feature so a consumer can name the token
// for `default_cancel_on` without taking a direct `processkit` dependency.
#[cfg(feature = "cancellation")]
#[cfg_attr(docsrs, doc(cfg(feature = "cancellation")))]
pub use processkit::CancellationToken;

mod parse;
pub use parse::{
    CheckRun, Comment, Issue, PrFeedback, PullRequest, Release, Repo, Review, WorkflowRun,
};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "gh";

const PR_FIELDS: &str = "number,title,state,headRefName,baseRefName,url";
const REPO_FIELDS: &str = "name,owner,description,url,isPrivate,defaultBranchRef";
const ISSUE_LIST_FIELDS: &str = "number,title,state";
const ISSUE_VIEW_FIELDS: &str = "number,title,state,body,url";
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

/// The GitHub operations this crate exposes — the interface consumers code
/// against and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitHubApi: Send + Sync {
    /// Run `gh <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitHubApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed GitHub CLI version (`gh --version`).
    async fn version(&self) -> Result<String>;
    /// Whether the user is authenticated (`gh auth status` exits zero). Reflects
    /// the exit code as a bool — any non-zero exit reads as `false`, never an
    /// error; only a spawn failure or timeout errors.
    async fn auth_status(&self) -> Result<bool>;
    /// The repository for `dir` (`gh repo view --json …`).
    async fn repo_view(&self, dir: &Path) -> Result<Repo>;
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
    /// Raw GitHub REST/GraphQL response body (`gh api <endpoint>`).
    async fn api(&self, endpoint: &str) -> Result<String>;

    // --- PR lifecycle ----------------------------------------------------

    /// Merge a pull request (`gh pr merge <n> --merge|--squash|--rebase
    /// [--auto] [--delete-branch]`) — see [`PrMerge`].
    async fn pr_merge(&self, dir: &Path, number: u64, merge: PrMerge) -> Result<()>;
    /// Mark a draft pull request as ready for review (`gh pr ready <n>`).
    async fn pr_ready(&self, dir: &Path, number: u64) -> Result<()>;
    /// Close a pull request without merging (`gh pr close <n>
    /// [--delete-branch]`).
    async fn pr_close(&self, dir: &Path, number: u64, delete_branch: bool) -> Result<()>;
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
    /// The PR's submitted reviews and conversation comments
    /// (`gh pr view <n> --json reviews,comments`).
    async fn pr_feedback(&self, dir: &Path, number: u64) -> Result<PrFeedback>;

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

processkit::cli_client!(
    /// The real GitHub client. Generic over the [`ProcessRunner`] so tests can
    /// inject a fake process executor; `GitHub::new()` uses the real job-backed
    /// runner.
    pub struct GitHub => BINARY
);

#[async_trait::async_trait]
impl<R: ProcessRunner> GitHubApi for GitHub<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(self.core.command(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output(self.core.command(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(self.core.command(["--version"])).await
    }

    async fn auth_status(&self) -> Result<bool> {
        // `gh auth status` exits 0 when authenticated, non-zero when not — an
        // exit-code answer. `exit_code` reads the exit code without erroring on a
        // non-zero one (a spawn failure or timeout still errors), so ANY non-zero
        // exit — not just the documented 1 — maps to "not authenticated" rather
        // than surfacing as an error. `probe` would reject an unusual exit code.
        Ok(self
            .core
            .exit_code(self.core.command(["auth", "status"]))
            .await?
            == 0)
    }

    async fn repo_view(&self, dir: &Path) -> Result<Repo> {
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
                parse::from_json,
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
                parse::from_json,
            )
            .await
    }

    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
        let n = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["pr", "view", n.as_str(), "--json", PR_FIELDS]),
                parse::from_json,
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
                parse::from_json,
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

    async fn api(&self, endpoint: &str) -> Result<String> {
        reject_flag_like("endpoint", endpoint)?;
        self.core.run(self.core.command(["api", endpoint])).await
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

    async fn pr_ready(&self, dir: &Path, number: u64) -> Result<()> {
        let n = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["pr", "ready", n.as_str()]))
            .await
    }

    async fn pr_close(&self, dir: &Path, number: u64, delete_branch: bool) -> Result<()> {
        let n = number.to_string();
        let mut args = vec!["pr", "close", n.as_str()];
        if delete_branch {
            args.push("--delete-branch");
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn pr_checks(&self, dir: &Path, number: u64) -> Result<Vec<CheckRun>> {
        let n = number.to_string();
        let res = self
            .core
            .output(
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
            Some(0) => parse::from_json(res.stdout()),
            Some(1 | 8) if !res.stdout().trim().is_empty() => parse::from_json(res.stdout()),
            // gh exits 1 with NO JSON for a PR that simply has no checks — the
            // one bare non-zero we read as an empty list (cf. jj's
            // `resolve_list` and its "No conflicts" exit).
            _ if res.stderr().contains("no checks reported") => Ok(Vec::new()),
            // Anything else (no such PR, auth required, timeout, signal…) is a
            // genuine failure; `ensure_success` builds the faithful error.
            _ => {
                res.ensure_success()?;
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
            .try_parse(self.core.command_in(dir, args), parse::from_json)
            .await
    }

    async fn run_view(&self, dir: &Path, id: u64) -> Result<WorkflowRun> {
        let id = id.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["run", "view", id.as_str(), "--json", RUN_FIELDS]),
                parse::from_json,
            )
            .await
    }

    async fn run_watch(&self, dir: &Path, id: u64) -> Result<WorkflowRun> {
        // Block until the run completes. `--exit-status` is deliberately NOT
        // passed: it would map the run's outcome onto the exit code (1 failed,
        // 2 cancelled), which can't be reported faithfully — the follow-up
        // `run view`'s `conclusion` can. Without it, a non-zero watch exit is a
        // genuine error (no such run, auth, …). `output` does NOT error on a
        // timeout (it returns the result with a timeout flag), so
        // `ensure_success` is what surfaces a killed watch as `Error::Timeout`
        // instead of reading a half-finished run below.
        let id_str = id.to_string();
        self.core
            .output(self.core.command_in(dir, ["run", "watch", id_str.as_str()]))
            .await?
            .ensure_success()?;
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
                parse::from_json,
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
                parse::from_json,
            )
            .await
    }

    async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release> {
        reject_flag_like("tag", tag)?;
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["release", "view", tag, "--json", RELEASE_VIEW_FIELDS]),
                parse::from_json,
            )
            .await
    }
}

impl<R: ProcessRunner> GitHub<R> {
    /// Run `gh <args>` over string slices — `gh.run_args(&["pr", "list"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GitHubApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(self.core.command(args)).await
    }

    /// Like [`run_args`](GitHub::run_args) but never errors on a non-zero exit
    /// (mirrors [`GitHubApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output(self.core.command(args)).await
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

/// Generate [`GitHubAt`] forwarders: `bare` methods forward verbatim, `dir`
/// methods inject `self.dir` as the first argument.
macro_rules! github_at_forwarders {
    (
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
    ) => {
        impl<'a, R: ProcessRunner> GitHubAt<'a, R> {
            $(
                #[doc = concat!("Bound form of [`GitHub`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.gh.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`GitHub`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.gh.$dn(self.dir, $($da),*).await
                }
            )*
        }
    };
}

github_at_forwarders! {
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
        fn auth_status() -> Result<bool>;
        fn api(endpoint: &str) -> Result<String>;
    }
    dir {
        fn repo_view() -> Result<Repo>;
        fn pr_list() -> Result<Vec<PullRequest>>;
        fn pr_list_for_branch(head: &str, base: &str) -> Result<Vec<PullRequest>>;
        fn pr_view(number: u64) -> Result<PullRequest>;
        fn issue_list() -> Result<Vec<Issue>>;
        fn pr_create(spec: PrCreate) -> Result<String>;
        fn pr_merge(number: u64, merge: PrMerge) -> Result<()>;
        fn pr_ready(number: u64) -> Result<()>;
        fn pr_close(number: u64, delete_branch: bool) -> Result<()>;
        fn pr_checks(number: u64) -> Result<Vec<CheckRun>>;
        fn pr_review(number: u64, action: ReviewAction) -> Result<()>;
        fn pr_comment(number: u64, body: &str) -> Result<String>;
        fn pr_feedback(number: u64) -> Result<PrFeedback>;
        fn run_list(limit: u64, branch: Option<String>) -> Result<Vec<WorkflowRun>>;
        fn run_view(id: u64) -> Result<WorkflowRun>;
        fn run_watch(id: u64) -> Result<WorkflowRun>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn release_list() -> Result<Vec<Release>>;
        fn release_view(tag: &str) -> Result<Release>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_gh() {
        assert_eq!(BINARY, "gh");
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
        assert_eq!(calls[1].cwd.as_deref(), Some(dir.as_os_str()));
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["api", "user"], Reply::ok("ok\n")));
        assert_eq!(gh.run_args(&["api", "user"]).await.unwrap(), "ok");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `gh` binary or network needed, so this runs on CI.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"number":7,"title":"Add X","state":"OPEN","headRefName":"feat/x","baseRefName":"main","url":"u"}]"#;
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
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
        let yes = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Reply::ok("")));
        assert!(yes.auth_status().await.unwrap());
        let no = GitHub::with_runner(
            ScriptedRunner::new().on(["auth"], Reply::fail(1, "not logged in")),
        );
        assert!(!no.auth_status().await.unwrap());
        // An unexpected exit code (e.g. 2) is still just "not authenticated".
        let weird = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Reply::fail(2, "boom")));
        assert!(!weird.auth_status().await.unwrap());
    }

    // Regression guard for the timeout fix: a timed-out auth check must error,
    // not silently report "not authenticated" (the old hand-rolled mapping bug).
    // Relies on processkit surfacing a timed-out run as `Error::Timeout`.
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Reply::timeout()));
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
                "pr", "create", "--title", "T", "--body", "B", "--base", "main",
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
        use processkit::RecordingRunner;
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
        use processkit::RecordingRunner;
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
        use processkit::RecordingRunner;
        use std::ffi::OsStr;
        let rec = RecordingRunner::replying(Reply::ok("https://gh/pr/2\n"));
        let gh = GitHub::with_runner(&rec);
        let url = gh
            .pr_create(Path::new("/repo"), PrCreate::new("T", "B"))
            .await
            .expect("pr_create");
        assert_eq!(url, "https://gh/pr/2");

        let call = rec.only_call();
        assert_eq!(call.cwd.as_deref(), Some(OsStr::new("/repo")));
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
        assert!(gh.api("-evil").await.is_err());
        assert!(gh.release_view(Path::new("."), "-evil").await.is_err());
        assert!(gh.api("").await.is_err(), "empty refused too");
        assert!(rec.calls().is_empty(), "nothing may spawn");
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
    async fn pr_ready_and_close_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let gh = GitHub::with_runner(&rec);
        gh.pr_ready(Path::new("/r"), 3).await.expect("pr_ready");
        gh.pr_close(Path::new("/r"), 3, true).await.expect("close");
        gh.pr_close(Path::new("/r"), 4, false).await.expect("close");
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["pr", "ready", "3"]);
        assert_eq!(calls[1].args_str(), ["pr", "close", "3", "--delete-branch"]);
        assert_eq!(calls[2].args_str(), ["pr", "close", "4"]);
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
            let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "checks"], reply));
            let checks = gh.pr_checks(Path::new("."), 7).await.expect("pr_checks");
            assert_eq!(checks.len(), 1);
            assert_eq!(checks[0].bucket, "pass");
        }

        // A PR with no checks at all: gh exits 1 with NO JSON and a
        // "no checks reported" message — an empty list, not an error.
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            ["pr", "checks"],
            Reply::fail(1, "no checks reported on the 'feat/x' branch"),
        ));
        assert!(
            gh.pr_checks(Path::new("."), 7)
                .await
                .expect("no checks → empty")
                .is_empty()
        );
        // …while a bare exit 1 for a different reason stays an error.
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            ["pr", "checks"],
            Reply::fail(1, "no pull requests found for branch 'feat/x'"),
        ));
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Exit { .. }
        ));

        // Exit 4 (auth required) is a real failure, not an outcome.
        let gh = GitHub::with_runner(
            ScriptedRunner::new().on(["pr", "checks"], Reply::fail(4, "auth required")),
        );
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Exit { .. }
        ));

        let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "checks"], Reply::timeout()));
        assert!(matches!(
            gh.pr_checks(Path::new("."), 7).await.unwrap_err(),
            Error::Timeout { .. }
        ));
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

    #[tokio::test]
    async fn pr_feedback_requests_reviews_and_comments() {
        let json = r#"{"reviews":[{"author":{"login":"a"},"state":"APPROVED",
            "body":"","submittedAt":""}],"comments":[]}"#;
        let rec = RecordingRunner::new(ScriptedRunner::new().on(["pr", "view"], Reply::ok(json)));
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
                .on(["run", "watch"], Reply::ok("✓ run completed"))
                .on(["run", "view"], Reply::ok(json)),
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
    // via the follow-up view. (`output` does not error on a timeout; the
    // `ensure_success` in run_watch is what surfaces it.)
    #[tokio::test]
    async fn run_watch_surfaces_timeout_and_watch_errors() {
        let rec =
            RecordingRunner::new(ScriptedRunner::new().on(["run", "watch"], Reply::timeout()));
        let gh = GitHub::with_runner(&rec);
        assert!(matches!(
            gh.run_watch(Path::new("."), 42).await.unwrap_err(),
            Error::Timeout { .. }
        ));
        assert_eq!(rec.calls().len(), 1, "no view after a timed-out watch");

        let gh = GitHub::with_runner(
            ScriptedRunner::new().on(["run", "watch"], Reply::fail(1, "no such run")),
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
    #[cfg(feature = "cancellation")]
    #[tokio::test(start_paused = true)]
    async fn run_watch_cancels_via_client_default_token() {
        use processkit::CancellationToken;
        let token = CancellationToken::new();
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["run", "watch"], Reply::pending()))
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
        let rec =
            RecordingRunner::new(ScriptedRunner::new().on(["release", "view"], Reply::ok(json)));
        let gh = GitHub::with_runner(&rec);
        let release = gh
            .release_view(Path::new("."), "v1")
            .await
            .expect("release_view");
        assert_eq!(release.tag_name, "v1");
        assert_eq!(release.body, "notes");
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "view", "v1", "--json", RELEASE_VIEW_FIELDS]
        );
    }

    // repo_view builds the --json request and flattens gh's nested owner/branch
    // objects into the public Repo.
    #[tokio::test]
    async fn repo_view_parses_scripted_json() {
        let json = r#"{"name":"r","owner":{"login":"o"},"description":"d","url":"u","isPrivate":false,"defaultBranchRef":{"name":"main"}}"#;
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["repo", "view"], Reply::ok(json)));
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
