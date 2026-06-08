#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-gitea` — automate Gitea (and Forgejo) from Rust by driving the `tea` CLI.
//!
//! It shells out to the installed `tea` binary, asks each command for
//! `--output json`, and deserializes that into typed values — so you get *tea's
//! own* auth, config, and instance handling, not a reimplementation of the Gitea
//! API. Async throughout, structured errors, and mockable. Every command runs
//! inside an OS **job** (via [`processkit`]) so a `tea` subprocess tree can never
//! be orphaned, and honours an optional per-client [timeout](Gitea::default_timeout).
//!
//! # The surface
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
//!   chained `.head` / `.base` setters named after the flags they emit) and
//!   [`MergeStrategy`] (`Merge` / `Squash` / `Rebase` → `tea pr merge --style`).
//!
//! The exposed operations are the **lean lifecycle** `tea` actually supports:
//! auth ([`auth_status`](GiteaApi::auth_status)), the PR lifecycle
//! ([list](GiteaApi::pr_list) / [view](GiteaApi::pr_view) /
//! [create](GiteaApi::pr_create) / [merge](GiteaApi::pr_merge) /
//! [close](GiteaApi::pr_close)), issues
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
//! synthesized by listing with `--state all` and filtering by number;
//! [`issue_view`](GiteaApi::issue_view), by contrast, is a first-class
//! `tea issues <index>`.
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
//! for pr in tea.pr_list(repo).await? {               // up to 100 open PRs
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
//! [`ScriptedRunner`](processkit::ScriptedRunner) with [`Gitea::with_runner`] to
//! exercise the *real* argv-building and JSON parsing against canned output. The
//! cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
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

/// Name of the underlying CLI binary this crate drives.
///
/// Note on injection safety: like `vcs-gitlab`, the lean surface has **no bare
/// positional string slot** for a caller value — PR numbers are `u64`, the
/// title/body/branch arguments ride in flag-VALUE positions, and `run` is the
/// caller-owns-the-argv escape hatch. So there is nothing here to guard with
/// `vcs_cli_support::reject_flag_like`.
pub const BINARY: &str = "tea";

// tea's `list` commands serialize a print-table whose columns are chosen with
// `--fields`. We request exactly the columns the parsers read; every value comes
// back as a JSON string (see `parse.rs`). These names are validated by tea
// against its `PullFields`/`IssueFields` lists — keep them in that set.
const PR_FIELDS: &str = "index,title,state,head,base,url";
const ISSUE_FIELDS: &str = "index,title,state,body,url";

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
    /// Open pull requests for `dir` (`tea pr list --limit 100 --output json`).
    /// Returns up to 100 open PRs; use [`run`](GiteaApi::run) for more.
    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>>;
    /// A single pull request by number. `tea` has no single-PR view, so this
    /// **lists** (`tea pr list --state all --limit 999 --output json`) and filters
    /// by number; a missing number is an [`Error::Parse`]. The high `--limit`
    /// guards against a false "not found", but PRs beyond the first 999 are still
    /// not found.
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
    /// Open issues for `dir` (`tea issues list --limit 100 --output json`).
    /// Returns up to 100 open issues; use [`run`](GiteaApi::run) for more.
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
    /// Releases for `dir` (`tea releases list --limit 100 --output json`).
    /// Returns up to 100 releases; use [`run`](GiteaApi::run) for more.
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
    pub struct Gitea => BINARY
);

#[async_trait::async_trait]
impl<R: ProcessRunner> GiteaApi for Gitea<R> {
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
        // `tea login list --output json` is a global (non-repo) command that
        // yields the configured logins as a JSON array; non-empty ⇒ logged in.
        // `output` (not `run`) so a NON-ZERO exit — e.g. tea erroring because no
        // config file exists yet — reads as "not logged in" rather than surfacing
        // as an error; a spawn failure or timeout still errors via `ensure_success`.
        let res = self
            .core
            .output(self.core.command(["login", "list", "--output", "json"]))
            .await?;
        if res.code() != Some(0) {
            // A timeout / signal-kill (no exit code) is a genuine failure;
            // `ensure_success` surfaces it as `Error::Timeout`/IO. A plain
            // non-zero exit, however, just means "no logins" → false.
            if res.code().is_none() {
                res.ensure_success()?;
            }
            return Ok(false);
        }
        let json = res.stdout().trim();
        // Treat empty output as "no logins" rather than a parse error — some tea
        // builds print nothing (not `[]`) when none are configured.
        if json.is_empty() {
            return Ok(false);
        }
        let logins: Vec<serde_json::Value> = parse::from_json(json)?;
        Ok(!logins.is_empty())
    }

    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>> {
        // `--limit 100` overrides tea's default page size (30), which would
        // otherwise silently truncate the list. `--fields` selects exactly the
        // table columns we parse — tea's default field set omits `head`/`base`/
        // `url`, so without this the branches and URL would always be empty.
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
        // `tea` has no single-PR view; list all states and filter by number. A
        // high `--limit` is essential here: without it, tea's default page size
        // (30) would make any PR past the first page a false "not found".
        // `--fields` selects the columns we parse (see `pr_list`).
        let prs = self
            .core
            .try_parse(
                self.core.command_in(
                    dir,
                    [
                        "pr", "list", "--state", "all", "--limit", "999", "--fields", PR_FIELDS,
                        "--output", "json",
                    ],
                ),
                parse::parse_pr_list,
            )
            .await?;
        prs.into_iter()
            .find(|pr| pr.number == number)
            .ok_or_else(|| Error::Parse {
                program: BINARY.to_string(),
                message: format!("no pull request #{number} in `tea pr list`"),
            })
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

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        // `--limit 100` overrides tea's default page size (30), mirroring
        // `pr_list`, so the list is not silently truncated. `--fields` selects
        // the columns we parse — tea's default issue fields omit `body`/`url`,
        // so without this both would always come back empty.
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
        // `--limit 100` overrides tea's default page size (30); `tea releases`
        // has no `--state`, so this returns the most recent 100 releases.
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
        self.core.run(self.core.command(args)).await
    }

    /// Like [`run_args`](Gitea::run_args) but never errors on a non-zero exit
    /// (mirrors [`GiteaApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output(self.core.command(args)).await
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

/// Generate [`GiteaAt`] forwarders: `bare` methods forward verbatim, `dir`
/// methods inject `self.dir` as the first argument.
macro_rules! gitea_at_forwarders {
    (
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
    ) => {
        impl<'a, R: ProcessRunner> GiteaAt<'a, R> {
            $(
                #[doc = concat!("Bound form of [`Gitea`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.tea.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`Gitea`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.tea.$dn(self.dir, $($da),*).await
                }
            )*
        }
    };
}

gitea_at_forwarders! {
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
        fn issue_list() -> Result<Vec<Issue>>;
        fn issue_view(number: u64) -> Result<Issue>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn release_list() -> Result<Vec<Release>>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{RecordingRunner, Reply, ScriptedRunner};

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
        assert_eq!(calls[1].cwd.as_deref(), Some(dir.as_os_str()));
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let tea = Gitea::with_runner(ScriptedRunner::new().on(["whoami"], Reply::ok("me\n")));
        assert_eq!(tea.run_args(&["whoami"]).await.unwrap(), "me");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `tea` binary or network needed, so this runs on CI. The fixture
    // is tea's *table* shape: all-string values, flat `head`/`base`, `url` column.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"index":"7","title":"Add X","state":"open","head":"feat/x","base":"main","url":"u"}]"#;
        let tea = Gitea::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
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
        let tea = Gitea::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
        let pr = tea.pr_view(Path::new("."), 9).await.expect("pr_view");
        assert_eq!(pr.title, "Nine");
        assert!(pr.merged);
    }

    // pr_view passes `--state all` + `--fields` so a closed/merged PR is found
    // with its branches/url, and a missing number is a parse error, not a panic.
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
                "999",
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
            ScriptedRunner::new().on(["login", "list"], Reply::ok(r#"[{"name":"gitea"}]"#)),
        );
        assert!(yes.auth_status().await.unwrap());
        let no = Gitea::with_runner(ScriptedRunner::new().on(["login", "list"], Reply::ok("[]")));
        assert!(!no.auth_status().await.unwrap());
        // Some tea builds print nothing (not `[]`) when no login is configured;
        // that must read as `false`, not a parse error.
        let empty = Gitea::with_runner(ScriptedRunner::new().on(["login", "list"], Reply::ok("")));
        assert!(!empty.auth_status().await.unwrap());
        // A non-zero exit (e.g. tea erroring because no config file exists) must
        // read as "not logged in" — never an error.
        let failed = Gitea::with_runner(
            ScriptedRunner::new().on(["login", "list"], Reply::fail(1, "no config")),
        );
        assert!(!failed.auth_status().await.unwrap());
        let weird =
            Gitea::with_runner(ScriptedRunner::new().on(["login", "list"], Reply::fail(2, "boom")));
        assert!(!weird.auth_status().await.unwrap());
    }

    // A timed-out login check must error, not silently report "not logged in".
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let tea = Gitea::with_runner(ScriptedRunner::new().on(["login", "list"], Reply::timeout()));
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

    // issue_list parses tea's table shape (all-string `index` column) and pins
    // `--limit 100 --fields … --output json`.
    #[tokio::test]
    async fn issue_list_parses_scripted_json() {
        let json = r#"[{"index":"12","title":"Bug","state":"open","body":"broken","url":"u"}]"#;
        let tea = Gitea::with_runner(ScriptedRunner::new().on(["issues", "list"], Reply::ok(json)));
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
        let tea =
            Gitea::with_runner(ScriptedRunner::new().on(["releases", "list"], Reply::ok(json)));
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
