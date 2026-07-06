#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-gitea` — automate Gitea (and Forgejo) from Rust by driving the `tea` CLI.
//!
//! You call typed `async` methods; `vcs-gitea` runs the real `tea`, asks each
//! command for `--output json`, and deserializes that into typed values — so you
//! get *tea's own* auth, config, and instance handling, not a reimplementation of
//! the Gitea API. Async, structured errors, mockable. Every command runs inside an
//! OS **job** (an OS-level container that kills the whole process tree if your
//! program exits, via [`processkit`]) so a `tea` subprocess is never orphaned, with
//! an optional per-client [timeout](Gitea::default_timeout).
//!
//! # What you can do
//!
//! Check auth · the lean pull-request lifecycle (list / view / create / merge /
//! close) · issues (list / view / create) · release listing. This is deliberately
//! narrower than `gh`/`glab` — `tea` itself lacks some operations (see the surface
//! notes below). One tiny call to start:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitea::{Gitea, GiteaApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let tea = Gitea::new();
//! let prs = tea.pr_list(Path::new(".")).await?; // open PRs (≈50/page server cap)
//! # let _ = prs; Ok(()) }
//! ```
//!
//! # The surface (engineering reference)
//!
//! - **[`GiteaApi`]** — the object-safe trait every operation lives on. Depend on
//!   `&dyn GiteaApi` (or generically on `impl GiteaApi`) so a test can swap the
//!   real client for a double. The repo-scoped methods take the working directory
//!   as the first argument and return typed results ([`PullRequest`], [`Issue`],
//!   [`Release`]) or a structured [`Error`]; unmodelled `tea` commands go through
//!   [`run`](GiteaApi::run).
//! - **[`Gitea`]** — the real client. [`Gitea::new`] uses the job-backed runner;
//!   [`Gitea::with_runner`] injects a fake one for tests. It is generic over the
//!   [`ProcessRunner`] seam, defaulting to the production runner.
//! - **[`GiteaAt`]** — a cwd-bound view ([`Gitea::at`]) whose repo-scoped methods
//!   drop the leading `dir`, so `tea.at(dir).pr_list()` reads as
//!   `tea.pr_list(dir)` — handy when one client drives one checkout.
//! - **Specs & enums** — [`PrCreate`] (`#[non_exhaustive]`, a constructor plus
//!   chained `.head` / `.base` setters named after the flags they emit),
//!   [`PrEdit`] (optional `title` and/or `body` for `pr edit`), and
//!   [`MergeStrategy`] (`Merge` / `Squash` / `Rebase` → `tea pr merge --style`).
//!
//! The exposed operations are the **lean lifecycle** `tea` actually supports:
//! auth ([`auth_status`](GiteaApi::auth_status)), the PR lifecycle
//! ([list](GiteaApi::pr_list) / [view](GiteaApi::pr_view) /
//! [create](GiteaApi::pr_create) / [merge](GiteaApi::pr_merge) /
//! [close](GiteaApi::pr_close) / [comment](GiteaApi::pr_comment) /
//! [edit](GiteaApi::pr_edit)), issues
//! ([list](GiteaApi::issue_list) / [view](GiteaApi::issue_view) /
//! [create](GiteaApi::issue_create)), and [release listing](GiteaApi::release_list).
//! It is deliberately narrower than
//! [`vcs-github`](https://crates.io/crates/vcs-github) /
//! [`vcs-gitlab`](https://crates.io/crates/vcs-gitlab): `tea` has **no** single-PR
//! `view`, **no** current-repo view, **no** draft toggle, **no** PR-checks
//! command, and **no** single-release view (`tea releases` ignores any positional
//! and always lists), so those operations are simply absent here (the
//! [`vcs-forge`](https://crates.io/crates/vcs-forge) facade reports them
//! `Unsupported` for the Gitea backend). [`pr_view`](GiteaApi::pr_view) is
//! synthesized by **paging** `--state all` and filtering by number (so it finds a PR
//! past the server's ~50-row page cap); [`issue_view`](GiteaApi::issue_view), by
//! contrast, is a first-class `tea issues <index>`.
//!
//! One shape caveat: `tea`'s `--output json` is **not** the Gitea REST shape. Its
//! *list* commands emit tea's print-*table* — a JSON array of string-maps whose
//! keys are snake-cased column headers and whose values are **all strings** (no
//! `html_url`, no nested branch objects, no typed bools); we pick columns with
//! `--fields`. Its *detail* view (`issues <n>`) is a separate *typed* object. The
//! parsers model both (the `#[ignore]` real-`tea` tests in `tests/cli.rs` are the
//! contract check).
//!
//! # Recipes
//!
//! Read state — depend on the trait so the same code takes a real client or a mock:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitea::{Gitea, GiteaApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let tea = Gitea::new();
//! let repo = Path::new(".");
//! let authed = tea.auth_status().await?;             // any login configured?
//! for pr in tea.pr_list(repo).await? {               // open PRs (≈50/page cap)
//!     println!("#{} [{}] {}", pr.number, pr.state, pr.title);
//! }
//! # let _ = authed; Ok(()) }
//! ```
//!
//! Drive the PR lifecycle — `pr_create` takes the [`PrCreate`] spec; merge picks a
//! [`MergeStrategy`]:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_gitea::{Gitea, GiteaApi, MergeStrategy, PrCreate};
//! # async fn demo(tea: &Gitea, repo: &Path) -> Result<(), processkit::Error> {
//! tea.pr_create(repo, PrCreate::new("Add streaming", "Implements …")
//!         .head("feat/streaming").base("main")).await?;
//! tea.pr_merge(repo, 7, MergeStrategy::Squash).await?;
//! # Ok(()) }
//! ```
//!
//! # Testing
//!
//! Two seams: enable the **`mock`** feature for a `mockall`-generated
//! `MockGiteaApi` (stub whole methods), or inject a
//! [`ScriptedRunner`](processkit::testing::ScriptedRunner) with [`Gitea::with_runner`] to
//! exercise the *real* argv-building and JSON parsing against canned output. The
//! cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::fmt::{self, Debug, Formatter};
use std::path::Path;

// Re-export the processkit types in this crate's public API, so consumers needn't
// depend on processkit directly — incl. `ProcessRunner` (the `with_runner`/`Gitea<R>`
// seam) and the `JobRunner` default. (Also brings `Error`/`Result`/`ProcessResult`/
// `ProcessRunner` into scope here.)
pub use processkit::{Error, JobRunner, ProcessResult, ProcessRunner, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency.
pub use processkit::CancellationToken;

mod parse;
pub use parse::{Issue, PullRequest, Release};

/// Options for [`GiteaApi::pr_create`] (`tea pr create`).
///
/// `#[non_exhaustive]`, so build it through [`PrCreate::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PrCreate {
    /// The PR title (`--title`).
    pub title: String,
    /// The PR description (`--description`).
    pub body: String,
    /// The source branch (`--head`); `None` = the current branch.
    pub head: Option<String>,
    /// The target branch (`--base`); `None` = the repo default.
    pub base: Option<String>,
}

impl PrCreate {
    /// A PR with `title` and `body`, head/base left to tea's defaults
    /// (current branch → repo default).
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            head: None,
            base: None,
        }
    }

    /// Set the source branch (`--head`) instead of the current branch.
    pub fn head(mut self, head: impl Into<String>) -> Self {
        self.head = Some(head.into());
        self
    }

    /// Set the target branch (`--base`) instead of the repo default.
    pub fn base(mut self, base: impl Into<String>) -> Self {
        self.base = Some(base.into());
        self
    }
}

/// Options for [`GiteaApi::pr_edit`] (`tea pr edit`).
///
/// `#[non_exhaustive]`, so build it through [`PrEdit::new`] and the chained
/// [`title`](PrEdit::title) / [`body`](PrEdit::body) setters rather than a
/// struct literal. At least one of `title` or `body` must be `Some`; both
/// `None` is rejected by the facade before spawning (an explicit error, not a
/// silent no-op). An empty string is a real value — tea clears the field on
/// `--title ""` / `--description ""` — not a `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrEdit {
    /// The new title (`--title`); `None` leaves the title alone.
    pub title: Option<String>,
    /// The new description (`--description`); `None` leaves the description alone.
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

    /// Set the new description (`--description`).
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

/// Name of the underlying CLI binary this crate drives.
///
/// Note on injection safety: most of the lean surface keeps caller values out of
/// bare positional slots — PR numbers are `u64`, and the title/body/branch
/// arguments ride in flag-VALUE positions. The one exception is `pr_comment`'s
/// body: `tea comment <n> <body>` takes it as a bare positional, so it is guarded
/// with `vcs_cli_support::reject_flag_like` (mirroring `vcs-gitlab`'s `release_view`
/// `<tag>`). `run` is the caller-owns-the-argv escape hatch; guard any future bare
/// positional the same way.
pub const BINARY: &str = "tea";

// tea's `list` commands serialize a print-table whose columns are chosen with
// `--fields`. We request exactly the columns the parsers read; every value comes
// back as a JSON string (see `parse.rs`). These names are validated by tea
// against its `PullFields`/`IssueFields` lists — keep them in that set.
const PR_FIELDS: &str = "index,title,state,head,base,url";
const ISSUE_FIELDS: &str = "index,title,state,body,url";

// `pr_view` has no single-PR endpoint in `tea`, so it lists all states and pages
// through, filtering by number. `PR_VIEW_PAGE_SIZE` is the requested per-page size
// (the Gitea server may clamp it lower, which the page-until-empty loop tolerates);
// `PR_VIEW_MAX_PAGES` bounds the walk so a pathological repo can't loop unboundedly.
const PR_VIEW_PAGE_SIZE: usize = 50;
const PR_VIEW_MAX_PAGES: usize = 200;

/// How [`GiteaApi::pr_merge`] merges the PR — maps to `tea pr merge --style`
/// (Gitea's default is a merge commit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MergeStrategy {
    /// A merge commit (`--style merge`).
    Merge,
    /// Squash the commits into one (`--style squash`).
    Squash,
    /// Rebase the source onto the target (`--style rebase`).
    Rebase,
}

impl MergeStrategy {
    /// The `tea pr merge --style` value for this strategy.
    fn style(self) -> &'static str {
        match self {
            MergeStrategy::Merge => "merge",
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
        }
    }
}

/// Injection guard for bare positional argv slots: a caller-supplied value
/// with a leading `-` would be parsed by tea's CLI as a *flag* (verified:
/// `tea … -evil` → "unknown switch"), and an empty value changes a command's
/// meaning. Refuse both before anything spawns. Flag-VALUE positions
/// (`--title <t>`, `--description <b>`) need no guard — tea consumes the next
/// token verbatim there.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

/// The Gitea operations this crate exposes — the interface consumers code
/// against and mock in tests. The **lean PR lifecycle** `tea` supports; reach
/// unmodelled `tea` commands through [`run`](GiteaApi::run).
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GiteaApi: Send + Sync {
    /// Run `tea <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GiteaApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed Gitea CLI version (`tea --version`).
    async fn version(&self) -> Result<String>;
    /// Whether at least one login is configured (`tea login list --output json`
    /// is a non-empty array). `tea` has no per-instance `auth status`, so this is
    /// the closest "are we logged in" signal. Must not error on an unusual
    /// outcome: a non-zero exit (e.g. no config file yet) reads as `false`, the
    /// same as an empty array; only a spawn failure or timeout errors.
    async fn auth_status(&self) -> Result<bool>;
    /// Open pull requests for `dir` (`tea pr list --output json`). The Gitea
    /// **server** caps an API page at `MAX_RESPONSE_ITEMS` (default 50) and `tea`
    /// makes a single call, so this returns **at most ~50** open PRs regardless of
    /// the requested limit — a busier repo is silently truncated here. For the full
    /// set, page via [`run`](GiteaApi::run) (`tea pr list --page N`) or the API.
    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>>;
    /// A single pull request by number. `tea` has no single-PR view, so this
    /// **pages** through `tea pr list --state all` (`--page N`) and filters by
    /// number — correctly finding a PR past the server's ~50-row page cap, unlike a
    /// single capped listing. It stops at the first empty page (a genuine absence →
    /// [`Error::Parse`]) or a large safety bound; a miss is not a false negative for
    /// any normally-sized repo.
    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest>;
    /// Open a pull request, returning the command's output (`tea pr create`).
    /// Unlike `gh`/`glab`, `tea` prints a textual summary on success, **not** the
    /// new PR's URL (it has no `--output`/`--fields` flag to shape create output),
    /// so do not parse this as a URL. The [`PrCreate`] spec carries the title,
    /// body, and the optional head (`None` = the current branch) and base
    /// (`None` = the repo default) branches.
    async fn pr_create(&self, dir: &Path, spec: PrCreate) -> Result<String>;
    /// Merge a pull request (`tea pr merge <number> --style merge|rebase|squash`)
    /// — see [`MergeStrategy`].
    async fn pr_merge(&self, dir: &Path, number: u64, strategy: MergeStrategy) -> Result<()>;
    /// Close a pull request without merging (`tea pr close <number>`).
    async fn pr_close(&self, dir: &Path, number: u64) -> Result<()>;
    /// Add a comment to a pull request, returning the command's output
    /// (`tea comment <index> <body>`). Gitea PRs and issues share the `index`
    /// space and the same `tea comment` subcommand hits both. The `body` is a
    /// bare positional, so the trait method guards it with
    /// `reject_flag_like` (a leading `-` or empty value is rejected before
    /// any process spawns). **Defaulted** to `Error::Unsupported` so external
    /// implementers keep compiling when the crate bumps.
    #[allow(unused_variables)]
    async fn pr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String> {
        Err(Error::Unsupported {
            operation: "pr_comment".into(),
        })
    }
    /// Edit a pull request's title and/or description
    /// (`tea pr edit <index> [--title <title>] [--description <body>]`). At
    /// least one of `title` or `body` must be `Some` — the facade rejects
    /// both-`None` before reaching the wrapper. **Defaulted** to
    /// `Error::Unsupported`.
    #[allow(unused_variables)]
    async fn pr_edit(&self, dir: &Path, number: u64, edit: PrEdit) -> Result<()> {
        Err(Error::Unsupported {
            operation: "pr_edit".into(),
        })
    }
    /// Open issues for `dir` (`tea issues list --output json`). As with
    /// [`pr_list`](GiteaApi::pr_list), the Gitea server caps a page at
    /// `MAX_RESPONSE_ITEMS` (default 50), so this returns **at most ~50** open issues
    /// in one call — page via [`run`](GiteaApi::run) (`--page N`) for the rest.
    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>>;
    /// A single issue by number. Unlike PRs, `tea` *does* have a single-issue
    /// view — `tea issues <number>` (the bare index form), here run as
    /// `tea issues <number> --output json`, deserializing one object rather than
    /// listing and filtering.
    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue>;
    /// Open an issue, returning the command's output (`tea issues create
    /// --title <t> --description <d>`). Like [`pr_create`](GiteaApi::pr_create),
    /// `tea` prints a textual summary of the new issue (and, on the final line,
    /// its URL) — there is no `--output`/`--fields` flag to shape create output —
    /// so this returns the trimmed stdout verbatim rather than a parsed URL.
    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String>;
    /// Releases for `dir` (`tea releases list --output json`). As with
    /// [`pr_list`](GiteaApi::pr_list), the Gitea server caps a page at
    /// `MAX_RESPONSE_ITEMS` (default 50), so this returns **at most ~50** releases in
    /// one call — page via [`run`](GiteaApi::run) (`--page N`) for the rest.
    ///
    /// There is intentionally no `release_view`: `tea releases` takes no
    /// positional and always lists, so a single-release-by-tag view does not
    /// exist in `tea` (the [`vcs-forge`](https://crates.io/crates/vcs-forge)
    /// facade reports it `Unsupported`).
    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>>;
}

processkit::cli_client!(
    /// The real Gitea client. Generic over the [`ProcessRunner`] so tests can
    /// inject a fake process executor; `Gitea::new()` uses the real job-backed
    /// runner.
    ///
    /// **Authentication is ambient.** Unlike `vcs-github`/`vcs-gitlab` (which
    /// accept a per-operation token provider via `with_credentials`), `tea` has no
    /// non-interactive per-invocation token mechanism — it authenticates only from
    /// the logins stored by `tea login add`. So this client offers no credential
    /// injection; configure `tea`'s logins out of band. (The shared
    /// `CredentialService::Gitea` is reserved for if/when `tea` gains env-token
    /// support.)
    pub struct Gitea => BINARY
);

// Manual Debug: `processkit::cli_client!` (an external macro from the `processkit`
// dependency, not ours to change) doesn't generate one. No `R: Debug` bound;
// delegates straight to the wrapped `core` (a `processkit::CliClient<R>`, which
// already has its own manual `Debug` redacting env values and never printing `R`).
impl<R: ProcessRunner> Debug for Gitea<R> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gitea").field("core", &self.core).finish()
    }
}

#[async_trait::async_trait]
impl<R: ProcessRunner> GiteaApi for Gitea<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(args).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(["--version"]).await
    }

    async fn auth_status(&self) -> Result<bool> {
        // `tea login list --output json` is a global (non-repo) command that
        // yields the configured logins as a JSON array; non-empty ⇒ logged in.
        // `output_string` (not `run`) so a NON-ZERO exit — e.g. tea erroring because no
        // config file exists yet — reads as "not logged in" rather than surfacing
        // as an error; a spawn failure or timeout still errors via `ensure_success`.
        let res = self
            .core
            .output_string(["login", "list", "--output", "json"])
            .await?;
        if res.code() != Some(0) {
            // A timeout / signal-kill (no exit code) is a genuine failure;
            // `ensure_success` surfaces it as `Error::Timeout`/IO. A plain
            // non-zero exit, however, just means "no logins" → false.
            if res.code().is_none() {
                let _ = res.ensure_success()?;
            }
            return Ok(false);
        }
        let json = res.stdout().trim();
        // Treat empty output as "no logins" rather than a parse error — some tea
        // builds print nothing (not `[]`) when none are configured.
        if json.is_empty() {
            return Ok(false);
        }
        let logins: Vec<serde_json::Value> = vcs_cli_support::json::from_json(BINARY, json)?;
        Ok(!logins.is_empty())
    }

    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>> {
        // `--limit 100` raises tea's default page size (30), but the Gitea *server*
        // caps a page at `MAX_RESPONSE_ITEMS` (default 50), so this returns at most
        // ~50 open PRs in one call — a repo with more is silently truncated here; page
        // via `run`/the API for the rest (see the trait doc). `--fields` selects the
        // table columns we parse — tea's default set omits `head`/`base`/`url`, so
        // without this the branches and URL would always be empty.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "pr", "list", "--limit", "100", "--fields", PR_FIELDS, "--output", "json",
                    ],
                ),
                parse::parse_pr_list,
            )
            .await
    }

    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
        // `tea` has no single-PR view (verified: `tea pulls`/`pr` has no `view`/index
        // subcommand — only list/checkout/create/…), so we list all states and filter
        // by number. The Gitea server caps each API page at `MAX_RESPONSE_ITEMS`
        // (default 50) and `tea` makes one call per page, so a single large `--limit`
        // is silently clamped — a PR past the first page would be a false "not found".
        // Instead page through (`--page`, a documented `tea pr list` flag) until
        // #number is found or a page returns empty (past the end — an empty page ends
        // the walk regardless of the server's actual clamp, so an instance whose cap
        // is below our request still tiles correctly). `--fields` selects the columns
        // we parse (see `pr_list`).
        let limit = PR_VIEW_PAGE_SIZE.to_string();
        for page in 1..=PR_VIEW_MAX_PAGES {
            let page_str = page.to_string();
            let prs = self
                .core
                .try_parse(
                    self.core.command_in(
                        dir,
                        [
                            "pr",
                            "list",
                            "--state",
                            "all",
                            "--limit",
                            limit.as_str(),
                            "--page",
                            page_str.as_str(),
                            "--fields",
                            PR_FIELDS,
                            "--output",
                            "json",
                        ],
                    ),
                    parse::parse_pr_list,
                )
                .await?;
            let exhausted = prs.is_empty();
            if let Some(pr) = prs.into_iter().find(|pr| pr.number == number) {
                return Ok(pr);
            }
            if exhausted {
                // An empty page means we walked past the last PR — a genuine absence.
                return Err(Error::parse(
                    BINARY,
                    format!("no pull request #{number} in `tea pr list`"),
                ));
            }
        }
        // Ran out of the page safety bound without finding it — an extremely large
        // repo. Report honestly rather than a confident false "not found".
        Err(Error::parse(
            BINARY,
            format!(
                "pull request #{number} not found in the first {} of `tea pr list` (stopped at \
                 the {PR_VIEW_MAX_PAGES}-page safety bound; query `tea`/the Gitea API directly for \
                 a repository this large)",
                PR_VIEW_MAX_PAGES * PR_VIEW_PAGE_SIZE
            ),
        ))
    }

    async fn pr_create(&self, dir: &Path, spec: PrCreate) -> Result<String> {
        let mut args = vec![
            "pr",
            "create",
            "--title",
            spec.title.as_str(),
            "--description",
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

    async fn pr_merge(&self, dir: &Path, number: u64, strategy: MergeStrategy) -> Result<()> {
        let n = number.to_string();
        self.core
            .run_unit(self.core.command_in(
                dir,
                ["pr", "merge", n.as_str(), "--style", strategy.style()],
            ))
            .await
    }

    async fn pr_close(&self, dir: &Path, number: u64) -> Result<()> {
        let n = number.to_string();
        self.core
            .run_unit(self.core.command_in(dir, ["pr", "close", n.as_str()]))
            .await
    }

    async fn pr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String> {
        // `body` is a bare positional, so guard it the way `release_view` does
        // in `vcs-gitlab`. Without this, `tea comment 7 --evil` would let a
        // caller-supplied string be parsed as a flag.
        reject_flag_like("body", body)?;
        let n = number.to_string();
        self.core
            .run(self.core.command_in(dir, ["comment", n.as_str(), body]))
            .await
    }

    async fn pr_edit(&self, dir: &Path, number: u64, edit: PrEdit) -> Result<()> {
        // `--title` and `--description` are flag-VALUE positions: no argv-guard
        // needed. The facade rejects both-`None` before reaching here; an empty
        // string is intentional (clears the field).
        let n = number.to_string();
        let mut args = vec!["pr", "edit", n.as_str()];
        if let Some(title) = edit.title.as_deref() {
            args.push("--title");
            args.push(title);
        }
        if let Some(body) = edit.body.as_deref() {
            args.push("--description");
            args.push(body);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        // `--limit 100` raises tea's default page size (30), but the Gitea server
        // caps a page at `MAX_RESPONSE_ITEMS` (default 50), so this returns at most
        // ~50 issues in one call (page via `run` for more), mirroring `pr_list`.
        // `--fields` selects the columns we parse — tea's default issue fields omit
        // `body`/`url`, so without this both would always come back empty.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "issues",
                        "list",
                        "--limit",
                        "100",
                        "--fields",
                        ISSUE_FIELDS,
                        "--output",
                        "json",
                    ],
                ),
                parse::parse_issue_list,
            )
            .await
    }

    async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue> {
        // `tea issues <index>` is the documented bare-index single-issue view;
        // `--output json` yields one object. `number` is a `u64`, so it can
        // never look like a flag — nothing to guard with `reject_flag_like`.
        let n = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["issues", n.as_str(), "--output", "json"]),
                parse::parse_issue,
            )
            .await
    }

    async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String> {
        self.core
            .run(self.core.command_in(
                dir,
                ["issues", "create", "--title", title, "--description", body],
            ))
            .await
    }

    async fn release_list(&self, dir: &Path) -> Result<Vec<Release>> {
        // `--limit 100` raises tea's default page size (30), but the Gitea server
        // caps a page at `MAX_RESPONSE_ITEMS` (default 50), so this returns at most
        // ~50 (most-recent) releases in one call — page via `run` for more.
        self.core
            .try_parse(
                self.core.command_in(
                    dir,
                    ["releases", "list", "--limit", "100", "--output", "json"],
                ),
                parse::parse_release_list,
            )
            .await
    }
}

impl<R: ProcessRunner> Gitea<R> {
    /// Run `tea <args>` over string slices — `tea.run_args(&["pr", "list"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GiteaApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(args).await
    }

    /// Like [`run_args`](Gitea::run_args) but never errors on a non-zero exit
    /// (mirrors [`GiteaApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    /// Bind a working directory, so the repo-scoped methods omit that argument:
    /// `tea.at(dir).pr_list()` runs [`pr_list`](GiteaApi::pr_list) against `dir`.
    pub fn at<'a>(&'a self, dir: &'a Path) -> GiteaAt<'a, R> {
        GiteaAt { tea: self, dir }
    }
}

/// A [`Gitea`] client with a working directory bound, so its repo-scoped methods
/// drop the leading `dir` argument (`tea.at(dir).pr_list()`). Construct one with
/// [`Gitea::at`].
pub struct GiteaAt<'a, R: ProcessRunner = processkit::JobRunner> {
    tea: &'a Gitea<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: holding only references, the view is `Copy`
// for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy` bound the
// default `JobRunner` doesn't satisfy, silently dropping `Copy` on the handle.
impl<R: ProcessRunner> Clone for GiteaAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for GiteaAt<'_, R> {}

// Generate [`GiteaAt`] forwarders: `bare` methods forward verbatim, `dir`
// methods inject `self.dir` as the first argument. The shared macro lives in
// `vcs-cli-support` (see `vcs_cli_support::at_forwarders!`).
vcs_cli_support::at_forwarders! {
    GiteaAt, tea, "Gitea",
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
        fn auth_status() -> Result<bool>;
    }
    dir {
        fn pr_list() -> Result<Vec<PullRequest>>;
        fn pr_view(number: u64) -> Result<PullRequest>;
        fn pr_create(spec: PrCreate) -> Result<String>;
        fn pr_merge(number: u64, strategy: MergeStrategy) -> Result<()>;
        fn pr_close(number: u64) -> Result<()>;
        fn pr_comment(number: u64, body: &str) -> Result<String>;
        fn pr_edit(number: u64, edit: PrEdit) -> Result<()>;
        fn issue_list() -> Result<Vec<Issue>>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn release_list() -> Result<Vec<Release>>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_tea() {
        assert_eq!(BINARY, "tea");
    }

    // Compile-time guard: the bound view stays `Copy` for the default `JobRunner`.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GiteaAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`tea.at(dir)`) must produce byte-identical argv to the
    // dir-taking call.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let tea = Gitea::with_runner(&rec);

        tea.pr_list(dir).await.unwrap();
        tea.at(dir).pr_list().await.unwrap();
        tea.pr_close(dir, 7).await.unwrap();
        tea.at(dir).pr_close(7).await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let tea =
            Gitea::with_runner(ScriptedRunner::new().on(["tea", "whoami"], Reply::ok("me\n")));
        assert_eq!(tea.run_args(&["whoami"]).await.unwrap(), "me");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `tea` binary or network needed, so this runs on CI. The fixture
    // is tea's *table* shape: all-string values, flat `head`/`base`, `url` column.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"index":"7","title":"Add X","state":"open","head":"feat/x","base":"main","url":"u"}]"#;
        let tea =
            Gitea::with_runner(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let prs = tea.pr_list(Path::new(".")).await.expect("pr_list");
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].head_branch, "feat/x");
    }

    // pr_view lists all states and filters by number; tea folds merge into the
    // `state` column (`"merged"`), from which the `merged` flag is derived.
    #[tokio::test]
    async fn pr_view_filters_listing_by_number() {
        let json = r#"[
            {"index":"7","title":"Seven","state":"open","head":"a","base":"main","url":"u"},
            {"index":"9","title":"Nine","state":"merged","head":"b","base":"main","url":"u"}
        ]"#;
        let tea =
            Gitea::with_runner(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let pr = tea.pr_view(Path::new("."), 9).await.expect("pr_view");
        assert_eq!(pr.title, "Nine");
        assert!(pr.merged);
    }

    // pr_view pages past the server's per-page cap: a PR that only appears on a
    // *later* page is still found (H8) — a single capped listing would false-negative
    // it. `on_sequence` feeds successive `tea pr list` calls their page's rows; the
    // `RecordingRunner` wrapper lets us assert the `--page` counter increments.
    #[tokio::test]
    async fn pr_view_pages_past_the_server_cap() {
        // Page 1: a full 50 rows, none is #77. Page 2: #77 is present.
        let page1_rows: Vec<String> = (1..=50)
            .map(|i| {
                format!(
                    r#"{{"index":"{i}","title":"t","state":"open","head":"h","base":"main","url":"u"}}"#
                )
            })
            .collect();
        let page1 = format!("[{}]", page1_rows.join(","));
        let page2 = r#"[{"index":"77","title":"Target","state":"open","head":"h","base":"main","url":"u"}]"#;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on_sequence(["tea", "pr", "list"], [Reply::ok(&page1), Reply::ok(page2)]),
        );
        let tea = Gitea::with_runner(&rec);
        let pr = tea
            .pr_view(Path::new("."), 77)
            .await
            .expect("pr_view finds a PR on a later page");
        assert_eq!(pr.title, "Target");
        // Exactly two pages fetched, with an incrementing `--page`.
        let calls = rec.calls();
        assert_eq!(calls.len(), 2, "should fetch page 1 then page 2");
        assert!(calls[0].args_str().windows(2).any(|w| w == ["--page", "1"]));
        assert!(calls[1].args_str().windows(2).any(|w| w == ["--page", "2"]));
    }

    // The walk must stop on an *empty* page, not a *short* one: a page shorter than
    // the requested limit is still a real page (the server may clamp below our ask),
    // so pr_view must keep going. Here page 1 has only 3 rows (no #9) and #9 lives on
    // page 2 — a `len < limit` stop would false-negative it.
    #[tokio::test]
    async fn pr_view_continues_past_a_short_nonempty_page() {
        let page1 = r#"[
            {"index":"1","title":"a","state":"open","head":"h","base":"main","url":"u"},
            {"index":"2","title":"b","state":"open","head":"h","base":"main","url":"u"},
            {"index":"3","title":"c","state":"open","head":"h","base":"main","url":"u"}
        ]"#;
        let page2 = r#"[{"index":"9","title":"Found","state":"merged","head":"h","base":"main","url":"u"}]"#;
        let tea = Gitea::with_runner(
            ScriptedRunner::new()
                .on_sequence(["tea", "pr", "list"], [Reply::ok(page1), Reply::ok(page2)]),
        );
        let pr = tea.pr_view(Path::new("."), 9).await.expect("pr_view");
        assert_eq!(pr.title, "Found");
        assert!(pr.merged);
    }

    // pr_view passes `--state all` + `--fields` and pages from `--page 1`; an empty
    // first page is a genuine absence → parse error (not a panic), in one call.
    #[tokio::test]
    async fn pr_view_requests_all_states_and_errors_when_missing() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let tea = Gitea::with_runner(&rec);
        let err = tea.pr_view(Path::new("/repo"), 5).await.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                "50",
                "--page",
                "1",
                "--fields",
                "index,title,state,head,base,url",
                "--output",
                "json"
            ]
        );
    }

    // pr_list pins an explicit `--limit 100` (so tea's default page size of 30
    // does not silently truncate) and `--fields` (so head/base/url are present).
    #[tokio::test]
    async fn pr_list_pins_limit_and_fields() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let tea = Gitea::with_runner(&rec);
        tea.pr_list(Path::new("/repo")).await.expect("pr_list");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr",
                "list",
                "--limit",
                "100",
                "--fields",
                "index,title,state,head,base,url",
                "--output",
                "json"
            ]
        );
    }

    // auth_status reads the logins array: non-empty ⇒ true, empty ⇒ false.
    #[tokio::test]
    async fn auth_status_counts_logins() {
        let yes = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "login", "list"], Reply::ok(r#"[{"name":"gitea"}]"#)),
        );
        assert!(yes.auth_status().await.unwrap());
        let no =
            Gitea::with_runner(ScriptedRunner::new().on(["tea", "login", "list"], Reply::ok("[]")));
        assert!(!no.auth_status().await.unwrap());
        // Some tea builds print nothing (not `[]`) when no login is configured;
        // that must read as `false`, not a parse error.
        let empty =
            Gitea::with_runner(ScriptedRunner::new().on(["tea", "login", "list"], Reply::ok("")));
        assert!(!empty.auth_status().await.unwrap());
        // A non-zero exit (e.g. tea erroring because no config file exists) must
        // read as "not logged in" — never an error.
        let failed = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "login", "list"], Reply::fail(1, "no config")),
        );
        assert!(!failed.auth_status().await.unwrap());
        let weird = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "login", "list"], Reply::fail(2, "boom")),
        );
        assert!(!weird.auth_status().await.unwrap());
    }

    // A timed-out login check must error, not silently report "not logged in".
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let tea = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "login", "list"], Reply::timeout()),
        );
        assert!(matches!(
            tea.auth_status().await.unwrap_err(),
            Error::Timeout { .. }
        ));
    }

    // pr_create assembles title/description then optional head/base.
    #[tokio::test]
    async fn pr_create_appends_head_and_base() {
        let rec = RecordingRunner::replying(Reply::ok("#9\n"));
        let tea = Gitea::with_runner(&rec);
        tea.pr_create(
            Path::new("/repo"),
            PrCreate::new("T", "B").head("feat/x").base("main"),
        )
        .await
        .expect("pr_create");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr",
                "create",
                "--title",
                "T",
                "--description",
                "B",
                "--head",
                "feat/x",
                "--base",
                "main"
            ]
        );
    }

    // pr_merge maps the strategy to `--style`; pr_close to `pr close <n>`.
    #[tokio::test]
    async fn pr_merge_and_close_build_expected_argv() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let tea = Gitea::with_runner(&rec);
        tea.pr_merge(Path::new("/repo"), 5, MergeStrategy::Squash)
            .await
            .expect("merge");
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "merge", "5", "--style", "squash"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let tea = Gitea::with_runner(&rec);
        tea.pr_close(Path::new("/repo"), 5).await.expect("close");
        assert_eq!(rec.only_call().args_str(), ["pr", "close", "5"]);
    }

    // pr_comment builds `comment <n> <body>` — the body is a bare positional,
    // so it's argv-guarded the way `release_view` is in `vcs-gitlab`. A
    // flag-like or empty body is rejected BEFORE any process spawns.
    #[tokio::test]
    async fn pr_comment_builds_argv_and_returns_output() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let tea = Gitea::with_runner(&rec);
        let out = tea
            .pr_comment(Path::new("/r"), 7, "LGTM")
            .await
            .expect("pr_comment");
        assert_eq!(out, "");
        assert_eq!(rec.only_call().args_str(), ["comment", "7", "LGTM"]);
    }

    #[tokio::test]
    async fn pr_comment_rejects_flag_like_body() {
        let tea = Gitea::with_runner(ScriptedRunner::new());
        assert!(tea.pr_comment(Path::new("."), 7, "-evil").await.is_err());
        assert!(tea.pr_comment(Path::new("."), 7, "").await.is_err());
    }

    // pr_edit emits only the flags the caller set. Flag-VALUE positions pass
    // through verbatim — the facade rejects both-`None` before reaching here.
    #[tokio::test]
    async fn pr_edit_emits_only_provided_fields() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let tea = Gitea::with_runner(&rec);

        tea.pr_edit(Path::new("/r"), 7, PrEdit::new().title("New title"))
            .await
            .expect("title-only edit");
        tea.pr_edit(Path::new("/r"), 7, PrEdit::new().body("New body"))
            .await
            .expect("body-only edit");
        tea.pr_edit(Path::new("/r"), 7, PrEdit::new().title("T").body("B"))
            .await
            .expect("both-fields edit");

        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["pr", "edit", "7", "--title", "New title"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["pr", "edit", "7", "--description", "New body"]
        );
        assert_eq!(
            calls[2].args_str(),
            ["pr", "edit", "7", "--title", "T", "--description", "B"]
        );
    }

    // An empty string is a real value (clears the field) — the argv must
    // carry `--title ""` literally, not silently drop it.
    #[tokio::test]
    async fn pr_edit_some_empty_string_clears_field() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let tea = Gitea::with_runner(&rec);
        tea.pr_edit(Path::new("/r"), 7, PrEdit::new().title(""))
            .await
            .expect("empty title");
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "edit", "7", "--title", ""]
        );
    }

    // issue_list parses tea's table shape (all-string `index` column) and pins
    // `--limit 100 --fields … --output json`.
    #[tokio::test]
    async fn issue_list_parses_scripted_json() {
        let json = r#"[{"index":"12","title":"Bug","state":"open","body":"broken","url":"u"}]"#;
        let tea = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "issues", "list"], Reply::ok(json)),
        );
        let issues = tea.issue_list(Path::new(".")).await.expect("issue_list");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].number, 12);
        assert_eq!(issues[0].title, "Bug");
    }

    #[tokio::test]
    async fn issue_list_pins_limit_and_fields() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let tea = Gitea::with_runner(&rec);
        tea.issue_list(Path::new("/repo"))
            .await
            .expect("issue_list");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "issues",
                "list",
                "--limit",
                "100",
                "--fields",
                "index,title,state,body,url",
                "--output",
                "json"
            ]
        );
    }

    // issue_view is a first-class `tea issues <index> --output json` returning a
    // single **typed** object (numeric `index`) — not a list+filter like pr_view.
    #[tokio::test]
    async fn issue_view_uses_bare_index_and_parses_object() {
        let rec = RecordingRunner::replying(Reply::ok(
            r#"{"index":7,"title":"One","state":"closed","body":"b","url":"u"}"#,
        ));
        let tea = Gitea::with_runner(&rec);
        let issue = tea
            .issue_view(Path::new("/repo"), 7)
            .await
            .expect("issue_view");
        assert_eq!(issue.number, 7);
        assert_eq!(issue.state, "closed");
        assert_eq!(
            rec.only_call().args_str(),
            ["issues", "7", "--output", "json"]
        );
    }

    // issue_create assembles title/description; returns the trimmed stdout.
    #[tokio::test]
    async fn issue_create_builds_argv_and_returns_output() {
        let rec = RecordingRunner::replying(Reply::ok("#12 Bug\nhttps://gitea/issues/12\n"));
        let tea = Gitea::with_runner(&rec);
        let out = tea
            .issue_create(Path::new("/repo"), "Bug", "broken")
            .await
            .expect("issue_create");
        assert_eq!(out, "#12 Bug\nhttps://gitea/issues/12");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "issues",
                "create",
                "--title",
                "Bug",
                "--description",
                "broken"
            ]
        );
    }

    // release_list parses tea's fixed release table (all-string values, tea's
    // `toSnakeCase`d `tag-_name`/`published _at`/`status` keys) and pins the argv.
    // tea exposes no release-page URL, so `url` is empty.
    #[tokio::test]
    async fn release_list_parses_scripted_json() {
        let json = r#"[{"tag-_name":"0.1","title":"First","status":"released","published _at":"2023-07-26T13:02:36Z","tar/_zip url":"https://gitea/0.1.tar.gz\nhttps://gitea/0.1.zip"}]"#;
        let tea = Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "releases", "list"], Reply::ok(json)),
        );
        let releases = tea
            .release_list(Path::new("."))
            .await
            .expect("release_list");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, "0.1");
        assert_eq!(releases[0].title, "First");
        assert_eq!(releases[0].url, "");
        assert!(!releases[0].draft);
    }

    #[tokio::test]
    async fn release_list_pins_limit_100() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let tea = Gitea::with_runner(&rec);
        tea.release_list(Path::new("/repo"))
            .await
            .expect("release_list");
        assert_eq!(
            rec.only_call().args_str(),
            ["releases", "list", "--limit", "100", "--output", "json"]
        );
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockGiteaApi::new();
        mock.expect_auth_status().returning(|| Ok(true));
        assert!(mock.auth_status().await.unwrap());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/gitea.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
