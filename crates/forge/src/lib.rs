#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-forge` — one PR/MR lifecycle across GitHub, GitLab, and Gitea.
//!
//! You hold one handle, [`Forge`], and run the operations all three forges share —
//! it sends each to whichever CLI (`gh` / `glab` / `tea`) backs the handle and
//! returns plain result types ([`ForgePr`], [`ForgeIssue`], [`ForgeRelease`],
//! [`ForgeRepo`], …) that don't mention which forge produced them. It's the
//! `gh`/`glab`/`tea` analogue of how [`vcs-core`](https://docs.rs/vcs-core)'s `Repo`
//! sits over git and jj.
//!
//! # What you can do
//!
//! From one [`Forge`] handle: check auth · view the repo/project · the PR/MR
//! lifecycle (list / view / create / merge / mark-ready / close, CI checks) · issues
//! (list / view / create) · releases (list / view). One tiny call:
//!
//! ```no_run
//! use vcs_forge::{Forge, ForgeApi};
//! # async fn demo() -> Result<(), vcs_forge::Error> {
//! let forge = Forge::github("."); // or ::gitlab(".") / ::gitea(".")
//! for pr in forge.pr_list().await? {
//!     println!("#{} {}", pr.number, pr.title);
//! }
//! # Ok(()) }
//! ```
//!
//! Unlike a repository, a forge has **no filesystem marker** (`.git`/`.jj`) to
//! detect — it's identified by the remote *host* — so a [`Forge`] is
//! **constructed explicitly** ([`Forge::github`] / [`Forge::gitlab`] /
//! [`Forge::gitea`]), optionally guided by [`ForgeKind::from_remote_url`] applied to
//! a remote URL the caller already holds. Forges differ, so a few operations are
//! `Unsupported` on some backends (see below).
//!
//! # The surface (engineering reference)
//!
//! - **[`Forge`]** — the cwd-bound, forge-agnostic handle. Operations run against
//!   the bound directory ([`cwd`](Forge::cwd)); the CLI infers the repository from
//!   that directory's git remote. [`Forge::github`] / [`gitlab`](Forge::gitlab) /
//!   [`gitea`](Forge::gitea) build over the real job-backed runner;
//!   [`at`](Forge::at) re-binds the cwd, sharing the client; [`kind`](Forge::kind)
//!   reports which forge drives it.
//! - **[`ForgeApi`]** — the object-safe trait the common surface lives on. Hold a
//!   `Box<dyn ForgeApi>` / `&dyn ForgeApi` to code against the operations without
//!   naming the [`ProcessRunner`] generic. Every method mirrors the like-named
//!   inherent method on [`Forge`]; the trait adds nothing but the `&dyn` boundary.
//! - **[`ForgeKind`]** — `GitHub` / `GitLab` / `Gitea`. Its pure, best-effort
//!   [`from_remote_url`](ForgeKind::from_remote_url) classifies the *public SaaS*
//!   hosts (github.com, gitlab.com, gitea.com, codeberg.org, and proper subdomains)
//!   with an anchored match — a lookalike like `gitlab.com.attacker.net` and a
//!   self-hosted instance on an arbitrary domain both return `None` (pick the kind
//!   yourself).
//! - **Unified DTOs** — [`ForgePr`] (+ [`ForgePrState`]), [`ForgeIssue`]
//!   (+ [`ForgeIssueState`]), [`ForgeRelease`], [`ForgeRepo`], [`CiStatus`]; the
//!   inputs [`PrCreate`] (open-a-PR spec: `new(title, body)` then
//!   `.source(branch)` / `.target(branch)`, defaulting to the current branch and
//!   repo default) and [`MergeStrategy`] (`Merge` / `Squash` / `Rebase`). Each
//!   normalises the three CLIs' shapes — e.g. GitLab's `iid` becomes `number`, and
//!   `OPEN` / `opened` / `open` all read as one state. A few fields are
//!   best-effort: a PR's `draft`, and a release's `body`/`url` absent from lean
//!   `release_list` output (see each DTO's field docs).
//! - **Operation groups** — auth ([`auth_status`](Forge::auth_status)); the repo
//!   ([`repo_view`](Forge::repo_view)); the PR/MR lifecycle
//!   ([`pr_list`](Forge::pr_list) / [`pr_view`](Forge::pr_view) /
//!   [`pr_create`](Forge::pr_create) / [`pr_merge`](Forge::pr_merge) /
//!   [`pr_mark_ready`](Forge::pr_mark_ready) / [`pr_close`](Forge::pr_close) /
//!   [`pr_checks`](Forge::pr_checks)); issues ([`issue_list`](Forge::issue_list) /
//!   [`issue_view`](Forge::issue_view) / [`issue_create`](Forge::issue_create));
//!   releases ([`release_list`](Forge::release_list) /
//!   [`release_view`](Forge::release_view)). List ops cap at 100 — drop to the
//!   wrapped client for more.
//! - **Capability gaps** — `tea` has no current-repo view, draft toggle, checks
//!   command, or single-release view, so on a Gitea handle
//!   [`repo_view`](Forge::repo_view), [`pr_mark_ready`](Forge::pr_mark_ready),
//!   [`pr_checks`](Forge::pr_checks), and [`release_view`](Forge::release_view)
//!   return [`Error::Unsupported`] **without spawning**. Classify it with
//!   [`Error::is_unsupported`].
//! - **Capability introspection** — to branch *before* calling rather than
//!   handling the error, [`Forge::supports`]`(`[`ForgeOp`]`)` answers whether a
//!   varying operation is available, and [`Forge::capabilities`] returns the whole
//!   matrix as a [`ForgeCapabilities`] — so an agent or TUI can hide an
//!   unavailable action up front. ([`ForgeOp::ALL`] enumerates the varying ops.)
//!
//! The wrappers are re-exported (`vcs_forge::vcs_github` / `vcs_gitlab` /
//! `vcs_gitea`) so anything beyond the portable intersection — a forge-specific op,
//! or one the facade marks `Unsupported` — is one constructor away without a new
//! dependency.
//!
//! # Recipes
//!
//! Open a PR/MR with [`PrCreate`] — the facade maps `source`/`target` to each
//! CLI's own flags, and returns the CLI's success output (a URL on GitHub/GitLab):
//!
//! ```no_run
//! use vcs_forge::{Forge, ForgeApi, PrCreate};
//! # async fn demo(forge: &Forge) -> Result<(), vcs_forge::Error> {
//! let spec = PrCreate::new("Add widget", "Closes #12").source("feature");
//! let out = forge.pr_create(spec).await?;
//! # let _ = out;
//! # Ok(()) }
//! ```
//!
//! # Testing
//!
//! The facade trait has **no mock feature** — `mockall` can't process the
//! macro-generated [`ForgeApi`] signatures. Test the *real* dispatch instead:
//! build a [`Forge`] over an explicit client wrapping a fake runner — e.g.
//! `Forge::for_github(cwd, GitHub::with_runner(ScriptedRunner::new()))` (likewise
//! [`for_gitlab`](Forge::for_gitlab) / [`for_gitea`](Forge::for_gitea)) — and
//! script the canned CLI output, exercising the argv-building and DTO parsing
//! end to end. The cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use processkit::{JobRunner, ProcessRunner};
use vcs_gitea::Gitea;
use vcs_github::GitHub;
use vcs_gitlab::GitLab;

mod dto;
mod error;
mod gitea_forge;
mod github_forge;
mod gitlab_forge;

pub use dto::{
    CiStatus, ForgeCapabilities, ForgeIssue, ForgeIssueState, ForgeKind, ForgeOp, ForgePr,
    ForgePrState, ForgeRelease, ForgeRepo, MergeStrategy, PrCreate,
};
pub use error::{Error, Result};

// Re-export the underlying wrappers so a consumer depending only on `vcs-forge`
// can construct the clients (`Forge::for_github(cwd, GitHub::new())`) and reach
// forge-specific operations off the common surface.
pub use vcs_gitea;
pub use vcs_github;
pub use vcs_gitlab;
// Re-export `processkit` itself so a `vcs-forge`-only consumer can match the
// wrapped error — `Error::Forge(vcs_forge::processkit::Error::Timeout { .. })` —
// and name the `CancellationToken` for a `default_cancel_on` client, without a
// direct `processkit` dependency. (Mirrors `vcs_core::processkit`.)
pub use processkit;
pub use processkit::CancellationToken;

/// The per-CLI client behind a [`Forge`]. Shared via `Arc` so [`Forge::at`] can
/// re-anchor the cwd cheaply without rebuilding the client.
enum Backend<R: ProcessRunner> {
    GitHub(Arc<GitHub<R>>),
    GitLab(Arc<GitLab<R>>),
    Gitea(Arc<Gitea<R>>),
}

impl<R: ProcessRunner> Backend<R> {
    fn shared(&self) -> Self {
        match self {
            Backend::GitHub(c) => Backend::GitHub(Arc::clone(c)),
            Backend::GitLab(c) => Backend::GitLab(Arc::clone(c)),
            Backend::Gitea(c) => Backend::Gitea(Arc::clone(c)),
        }
    }
}

/// A cwd-bound, forge-agnostic handle. Operations run against the bound directory
/// ([`cwd`](Forge::cwd)); the CLI infers the repository from that directory's git
/// remote. Use [`at`](Forge::at) for a sibling handle bound elsewhere.
pub struct Forge<R: ProcessRunner = JobRunner> {
    cwd: PathBuf,
    backend: Backend<R>,
}

impl Forge<JobRunner> {
    /// A GitHub-backed handle bound to `cwd`, using the real job-backed runner.
    pub fn github(cwd: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitHub(Arc::new(GitHub::new())),
        }
    }

    /// A GitLab-backed handle bound to `cwd`, using the real job-backed runner.
    pub fn gitlab(cwd: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitLab(Arc::new(GitLab::new())),
        }
    }

    /// A Gitea-backed handle bound to `cwd`, using the real job-backed runner.
    pub fn gitea(cwd: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::Gitea(Arc::new(Gitea::new())),
        }
    }
}

impl<R: ProcessRunner> Forge<R> {
    /// Build a GitHub-backed handle from an explicit client — for a custom runner
    /// (e.g. a test seam) or a pre-configured [`GitHub`].
    pub fn for_github(cwd: impl Into<PathBuf>, client: GitHub<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitHub(Arc::new(client)),
        }
    }

    /// Build a GitLab-backed handle from an explicit [`GitLab`] client.
    pub fn for_gitlab(cwd: impl Into<PathBuf>, client: GitLab<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitLab(Arc::new(client)),
        }
    }

    /// Build a Gitea-backed handle from an explicit [`Gitea`] client.
    pub fn for_gitea(cwd: impl Into<PathBuf>, client: Gitea<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::Gitea(Arc::new(client)),
        }
    }

    /// Which forge drives this handle.
    pub fn kind(&self) -> ForgeKind {
        match &self.backend {
            Backend::GitHub(_) => ForgeKind::GitHub,
            Backend::GitLab(_) => ForgeKind::GitLab,
            Backend::Gitea(_) => ForgeKind::Gitea,
        }
    }

    /// Whether this handle's backend supports `op`. The capability-varying
    /// operations ([`ForgeOp`]) are all present on GitHub and GitLab; Gitea
    /// (`tea`) supports **none** of them — it has no current-repo view, draft
    /// toggle, PR-checks command, or single-release view. Every other facade
    /// operation works on all three. Branch on this to hide an unavailable
    /// operation up front instead of calling it and handling
    /// [`Unsupported`](Error::Unsupported).
    pub fn supports(&self, op: ForgeOp) -> bool {
        match (self.kind(), op) {
            // The four operations `tea` can't do; GitHub/GitLab do everything.
            (
                ForgeKind::Gitea,
                ForgeOp::RepoView | ForgeOp::PrMarkReady | ForgeOp::PrChecks | ForgeOp::ReleaseView,
            ) => false,
            _ => true,
        }
    }

    /// A snapshot of which capability-varying operations this backend supports —
    /// the struct form of [`supports`](Forge::supports) across every [`ForgeOp`].
    pub fn capabilities(&self) -> ForgeCapabilities {
        ForgeCapabilities {
            repo_view: self.supports(ForgeOp::RepoView),
            pr_mark_ready: self.supports(ForgeOp::PrMarkReady),
            pr_checks: self.supports(ForgeOp::PrChecks),
            release_view: self.supports(ForgeOp::ReleaseView),
        }
    }

    /// The directory operations run against.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// A sibling handle bound to `dir`, sharing this handle's client.
    pub fn at(&self, dir: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: dir.into(),
            backend: self.backend.shared(),
        }
    }

    /// Whether the user is authenticated (GitHub/GitLab: a zero-exit `auth
    /// status`; Gitea: at least one configured login).
    pub async fn auth_status(&self) -> Result<bool> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::auth_status(c).await,
            Backend::GitLab(c) => gitlab_forge::auth_status(c).await,
            Backend::Gitea(c) => gitea_forge::auth_status(c).await,
        }
    }

    /// The repository/project for the bound directory. **[`Unsupported`](Error::Unsupported)
    /// on Gitea** (`tea` has no current-repo view).
    pub async fn repo_view(&self) -> Result<ForgeRepo> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::repo_view(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::repo_view(c, &self.cwd).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "repo_view")),
        }
    }

    /// Open pull/merge requests for the bound directory.
    pub async fn pr_list(&self) -> Result<Vec<ForgePr>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::pr_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::pr_list(c, &self.cwd).await,
        }
    }

    /// A single PR/MR by number (GitLab `iid`). On Gitea this lists and filters
    /// (`tea` has no single-PR view).
    pub async fn pr_view(&self, number: u64) -> Result<ForgePr> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_view(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_view(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::pr_view(c, &self.cwd, number).await,
        }
    }

    /// Open a PR/MR (see [`PrCreate`]), returning the CLI's success output — a
    /// URL on GitHub/GitLab; `tea` prints a textual summary (no URL).
    pub async fn pr_create(&self, spec: PrCreate) -> Result<String> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_create(c, &self.cwd, spec).await,
            Backend::GitLab(c) => gitlab_forge::pr_create(c, &self.cwd, spec).await,
            Backend::Gitea(c) => gitea_forge::pr_create(c, &self.cwd, spec).await,
        }
    }

    /// Merge a PR/MR with the given [`MergeStrategy`].
    pub async fn pr_merge(&self, number: u64, strategy: MergeStrategy) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_merge(c, &self.cwd, number, strategy).await,
            Backend::GitLab(c) => gitlab_forge::pr_merge(c, &self.cwd, number, strategy).await,
            Backend::Gitea(c) => gitea_forge::pr_merge(c, &self.cwd, number, strategy).await,
        }
    }

    /// Mark a draft PR/MR as ready for review. **[`Unsupported`](Error::Unsupported)
    /// on Gitea** (`tea` has no draft toggle — a Gitea draft is a `WIP:` title
    /// prefix, edited via the raw client).
    pub async fn pr_mark_ready(&self, number: u64) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_mark_ready(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_mark_ready(c, &self.cwd, number).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "pr_mark_ready")),
        }
    }

    /// Close a PR/MR without merging. `delete_branch` applies to GitHub only
    /// (`gh pr close --delete-branch`); GitLab and Gitea ignore it.
    pub async fn pr_close(&self, number: u64, delete_branch: bool) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_close(c, &self.cwd, number, delete_branch).await,
            Backend::GitLab(c) => gitlab_forge::pr_close(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::pr_close(c, &self.cwd, number).await,
        }
    }

    /// The PR/MR's coarse CI status (see [`CiStatus`]). **[`Unsupported`](Error::Unsupported)
    /// on Gitea** (`tea` has no checks command).
    pub async fn pr_checks(&self, number: u64) -> Result<CiStatus> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_checks(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_checks(c, &self.cwd, number).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "pr_checks")),
        }
    }

    /// Open issues for the bound directory (up to 100; drop to the underlying
    /// client for more).
    pub async fn issue_list(&self) -> Result<Vec<ForgeIssue>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::issue_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::issue_list(c, &self.cwd).await,
        }
    }

    /// A single issue by number (GitLab `iid`), with `body`/`url` filled.
    pub async fn issue_view(&self, number: u64) -> Result<ForgeIssue> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_view(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::issue_view(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::issue_view(c, &self.cwd, number).await,
        }
    }

    /// Open an issue, returning the CLI's success output — a URL on
    /// GitHub/GitLab; `tea` prints a textual summary whose final line is the
    /// URL. (The same honest-output contract as [`pr_create`](Forge::pr_create).)
    pub async fn issue_create(&self, title: &str, body: &str) -> Result<String> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_create(c, &self.cwd, title, body).await,
            Backend::GitLab(c) => gitlab_forge::issue_create(c, &self.cwd, title, body).await,
            Backend::Gitea(c) => gitea_forge::issue_create(c, &self.cwd, title, body).await,
        }
    }

    /// Releases for the bound directory, newest first (up to 100; drop to the
    /// underlying client for more).
    pub async fn release_list(&self) -> Result<Vec<ForgeRelease>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::release_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::release_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::release_list(c, &self.cwd).await,
        }
    }

    /// A single release by tag. **[`Unsupported`](Error::Unsupported) on Gitea**
    /// (`tea releases` always lists — it has no single-release view; filter
    /// [`release_list`](Forge::release_list) instead).
    pub async fn release_view(&self, tag: &str) -> Result<ForgeRelease> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::release_view(c, &self.cwd, tag).await,
            Backend::GitLab(c) => gitlab_forge::release_view(c, &self.cwd, tag).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "release_view")),
        }
    }
}

fn unsupported(forge: ForgeKind, operation: &'static str) -> Error {
    Error::Unsupported { forge, operation }
}

/// Generate a facade trait from one signature table: the `#[async_trait]` trait
/// declaration *and* the delegating `impl … for $Ty<R>`, so the two can never drift
/// out of sync (a hazard when each is hand-maintained). Every generated body is a
/// trivial delegation to the like-named inherent method — which method resolution
/// prefers, so this never recurses; the real backend-`match` dispatch stays
/// hand-written on the inherent `impl`. `async` methods doc-link to their inherent
/// twin; `sync` methods carry an explicit doc string (their docs aren't uniform).
///
/// A near-identical copy lives in `vcs-core` (`facade_trait!`); the two are
/// deliberately not shared (separate crates, ~40-line macro — duplication beats a
/// new dependency). Signatures only — each entry is a bare `&self`/sync method (no
/// method-level generics, no `&mut self`, no default bodies; a method shaped that
/// way needs a grammar tweak, not just a table row).
/// No `mockall::automock`: a Wave-S spike proved it can't process a
/// trait whose signatures come from `macro_rules!` — captured `$_:ty` fragments
/// reach `automock` as opaque nonterminal token groups its `syn` parser rejects
/// ("unsupported type in this position"), whereas `#[async_trait]` tolerates them.
/// The facade stays test-seam-tested (build a [`Forge`] over a fake runner).
macro_rules! facade_trait {
    (
        $(#[doc = $tdoc:expr])*
        trait $Trait:ident for $Ty:ident;
        sync {
            $( #[doc = $sdoc:expr] fn $sn:ident( $($sa:ident: $sat:ty),* $(,)? ) -> $sr:ty; )*
        }
        async {
            $( fn $an:ident( $($aa:ident: $aat:ty),* $(,)? ) -> $ar:ty; )*
        }
    ) => {
        $(#[doc = $tdoc])*
        #[async_trait::async_trait]
        pub trait $Trait: Send + Sync {
            $(
                #[doc = $sdoc]
                fn $sn(&self, $($sa: $sat),*) -> $sr;
            )*
            $(
                #[doc = concat!("See [`", stringify!($Ty), "::", stringify!($an), "`].")]
                async fn $an(&self, $($aa: $aat),*) -> $ar;
            )*
        }

        // Delegates to the inherent methods, which method resolution prefers — so
        // these bodies dispatch through the concrete type's real implementations,
        // not back into the trait.
        #[async_trait::async_trait]
        impl<R: ProcessRunner> $Trait for $Ty<R> {
            $(
                fn $sn(&self, $($sa: $sat),*) -> $sr {
                    self.$sn($($sa),*)
                }
            )*
            $(
                async fn $an(&self, $($aa: $aat),*) -> $ar {
                    self.$an($($aa),*).await
                }
            )*
        }
    };
}

facade_trait! {
    /// The forge-agnostic common surface of [`Forge`], as a trait — so a consumer can
    /// hold a `Box<dyn ForgeApi>` / `&dyn ForgeApi` and code against the operations
    /// without naming the [`ProcessRunner`] generic.
    ///
    /// Every method mirrors the like-named inherent method on [`Forge`].
    trait ForgeApi for Forge;
    sync {
        #[doc = "Which forge drives this handle."]
        fn kind() -> ForgeKind;
        #[doc = "The directory operations run against."]
        fn cwd() -> &Path;
    }
    async {
        fn auth_status() -> Result<bool>;
        fn repo_view() -> Result<ForgeRepo>;
        fn pr_list() -> Result<Vec<ForgePr>>;
        fn pr_view(number: u64) -> Result<ForgePr>;
        fn pr_create(spec: PrCreate) -> Result<String>;
        fn pr_merge(number: u64, strategy: MergeStrategy) -> Result<()>;
        fn pr_mark_ready(number: u64) -> Result<()>;
        fn pr_close(number: u64, delete_branch: bool) -> Result<()>;
        fn pr_checks(number: u64) -> Result<CiStatus>;
        fn issue_list() -> Result<Vec<ForgeIssue>>;
        fn issue_view(number: u64) -> Result<ForgeIssue>;
        fn issue_create(title: &str, body: &str) -> Result<String>;
        fn release_list() -> Result<Vec<ForgeRelease>>;
        fn release_view(tag: &str) -> Result<ForgeRelease>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    fn github(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::for_github("/repo", GitHub::with_runner(runner))
    }
    fn gitlab(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::for_gitlab("/repo", GitLab::with_runner(runner))
    }
    fn gitea(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::for_gitea("/repo", Gitea::with_runner(runner))
    }

    #[tokio::test]
    async fn kind_reflects_backend() {
        assert_eq!(github(ScriptedRunner::new()).kind(), ForgeKind::GitHub);
        assert_eq!(gitlab(ScriptedRunner::new()).kind(), ForgeKind::GitLab);
        assert_eq!(gitea(ScriptedRunner::new()).kind(), ForgeKind::Gitea);
    }

    // GitHub's "OPEN"/"MERGED" states map onto the unified ForgePrState.
    #[tokio::test]
    async fn github_pr_list_maps_to_unified() {
        let json = r#"[{"number":7,"title":"X","state":"MERGED","headRefName":"feat","baseRefName":"main","url":"u"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].state, ForgePrState::Merged);
        assert_eq!(prs[0].source_branch, "feat");
    }

    // GitLab `repo_view` maps a known "public" visibility to private == false.
    #[tokio::test]
    async fn gitlab_repo_view_maps_public_visibility() {
        let json = r#"{"name":"cli","path_with_namespace":"gitlab-org/cli","default_branch":"main","web_url":"u","visibility":"public"}"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "repo", "view"], Reply::ok(json)));
        let repo = forge.repo_view().await.unwrap();
        assert_eq!(repo.owner, "gitlab-org");
        assert_eq!(repo.name, "cli");
        assert!(!repo.private);
    }

    // When glab omits `visibility`, the facade must NOT report the repo as private
    // — an unknown visibility is the conservative `false`, never a false privacy.
    #[tokio::test]
    async fn gitlab_repo_view_absent_visibility_is_not_private() {
        let json =
            r#"{"name":"cli","path_with_namespace":"o/cli","default_branch":"main","web_url":"u"}"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "repo", "view"], Reply::ok(json)));
        let repo = forge.repo_view().await.unwrap();
        assert!(!repo.private, "absent visibility must not be private");
    }

    // GitLab's `iid` becomes the number and "opened" maps to Open.
    #[tokio::test]
    async fn gitlab_pr_list_maps_iid_and_state() {
        let json = r#"[{"iid":12,"title":"X","state":"opened","source_branch":"feat","target_branch":"main","web_url":"u","draft":true}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].number, 12);
        assert_eq!(prs[0].state, ForgePrState::Open);
        assert!(prs[0].draft);
    }

    // Gitea's `merged` flag drives Merged even though `state` is "closed".
    #[tokio::test]
    async fn gitea_pr_view_filters_and_maps_merged() {
        // tea's table shape: all-string values, flat head/base, merge folded
        // into the `state` column.
        let json =
            r#"[{"index":"9","title":"Nine","state":"merged","head":"f","base":"main","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let pr = forge.pr_view(9).await.unwrap();
        assert_eq!(pr.state, ForgePrState::Merged);
        assert_eq!(pr.target_branch, "main");
    }

    // The Gitea backend reports the four unmodelled ops as Unsupported, naming
    // the operation — and without spawning anything.
    #[tokio::test]
    async fn gitea_unsupported_ops_error_without_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let forge = Forge::for_gitea("/repo", Gitea::with_runner(&rec));
        for err in [
            forge.repo_view().await.unwrap_err(),
            forge.pr_mark_ready(1).await.unwrap_err(),
            forge.pr_checks(1).await.unwrap_err(),
            forge.release_view("v1.0.0").await.unwrap_err(),
        ] {
            assert!(err.is_unsupported(), "{err:?}");
        }
        assert!(rec.calls().is_empty(), "unsupported ops must not spawn");
    }

    // `supports`/`capabilities` must agree exactly with the runtime `Unsupported`
    // behaviour above: Gitea reports `false` for the four varying ops, GitHub and
    // GitLab report `true` for all of them — a pure, no-spawn capability check.
    #[test]
    fn capability_matrix_matches_unsupported_ops() {
        let gitea = Forge::for_gitea("/repo", Gitea::with_runner(ScriptedRunner::new()));
        for &op in ForgeOp::ALL {
            assert!(!gitea.supports(op), "gitea should not support {op:?}");
        }
        let caps = gitea.capabilities();
        assert_eq!(
            caps,
            ForgeCapabilities {
                repo_view: false,
                pr_mark_ready: false,
                pr_checks: false,
                release_view: false,
            }
        );
        for forge in [
            Forge::for_github("/repo", GitHub::with_runner(ScriptedRunner::new())),
            Forge::for_gitlab("/repo", GitLab::with_runner(ScriptedRunner::new())),
        ] {
            for &op in ForgeOp::ALL {
                assert!(
                    forge.supports(op),
                    "{:?} should support {op:?}",
                    forge.kind()
                );
            }
        }
    }

    // Each backend's issue states map onto the unified ForgeIssueState — note
    // the three different spellings of "open": "OPEN" (gh), "opened" (glab),
    // "open" (tea) — all must read as Open, and "closed" (any case) as Closed.
    #[tokio::test]
    async fn issue_list_maps_states_per_backend() {
        // gh's `issue_list` now fetches body+url too (widened field list), so they
        // arrive on the listed issues, not just via `issue_view`.
        let json = r#"[{"number":3,"title":"A","state":"OPEN","body":"desc","url":"https://gh/i/3"},{"number":4,"title":"B","state":"CLOSED"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "issue", "list"], Reply::ok(json)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].state, ForgeIssueState::Open);
        assert_eq!(issues[0].body, "desc");
        assert_eq!(issues[0].url, "https://gh/i/3");
        assert_eq!(issues[1].state, ForgeIssueState::Closed);

        let json = r#"[{"iid":12,"title":"X","state":"opened","description":"d","web_url":"u"}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "issue", "list"], Reply::ok(json)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].number, 12);
        assert_eq!(issues[0].state, ForgeIssueState::Open);
        assert_eq!(issues[0].body, "d");

        // tea's table shape: all-string values, `index` column.
        let json = r#"[{"index":"9","title":"Y","state":"open","body":"b","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "issues", "list"], Reply::ok(json)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].number, 9);
        assert_eq!(issues[0].state, ForgeIssueState::Open);
    }

    // Releases map per backend; an empty/absent publish timestamp (a draft)
    // surfaces as None, a present one as Some.
    #[tokio::test]
    async fn release_list_maps_published_at_per_backend() {
        // gh `release list` fetches isDraft/isPrerelease but NOT body — body only
        // comes from `release_view` (RELEASE_LIST_FIELDS omits it), so it's None here.
        let json = r#"[{"tagName":"v1","name":"One","publishedAt":"2026-01-01T00:00:00Z","isPrerelease":true},{"tagName":"v2-draft","name":"","publishedAt":"","isDraft":true}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "release", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].tag, "v1");
        assert_eq!(
            rels[0].published_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(rels[0].body, None, "gh release_list does not fetch body");
        assert!(rels[0].prerelease && !rels[0].draft);
        assert_eq!(rels[1].published_at, None);
        assert!(rels[1].draft && !rels[1].prerelease);

        let json = r#"[{"tag_name":"v1","name":"One","released_at":"2026-01-01T00:00:00Z","description":"gl notes","_links":{"self":"u"}}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "release", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].url, "u");
        assert!(rels[0].published_at.is_some());
        assert_eq!(rels[0].body.as_deref(), Some("gl notes"));
        // GitLab has no draft/pre-release concept.
        assert!(!rels[0].draft && !rels[0].prerelease);

        // tea's release table: `toSnakeCase`d string keys (`tag-_name`,
        // `published _at`), no release-page URL column.
        let json = r#"[{"tag-_name":"v1","title":"One","status":"prerelease","published _at":"2026-01-01T00:00:00Z"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "releases", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].tag, "v1");
        assert_eq!(rels[0].title, "One");
        assert_eq!(rels[0].url, ""); // tea exposes no release-page URL
        assert!(rels[0].published_at.is_some());
        assert_eq!(rels[0].body, None, "tea has no release body");
        assert!(rels[0].prerelease, "tea status 'prerelease' → prerelease");
    }

    // The unified MergeStrategy maps to each CLI's own flag.
    #[tokio::test]
    async fn pr_merge_maps_strategy_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::for_github("/repo", GitHub::with_runner(&rec))
            .pr_merge(5, MergeStrategy::Squash)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "merge", "5", "--squash"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::for_gitlab("/repo", GitLab::with_runner(&rec))
            .pr_merge(5, MergeStrategy::Rebase)
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "mr",
                "merge",
                "5",
                "--yes",
                "--auto-merge=false",
                "--rebase"
            ]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::for_gitea("/repo", Gitea::with_runner(&rec))
            .pr_merge(5, MergeStrategy::Merge)
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "merge", "5", "--style", "merge"]
        );
    }

    // GitHub's per-check buckets aggregate into one coarse CiStatus.
    #[tokio::test]
    async fn github_pr_checks_aggregates_buckets() {
        let json = r#"[{"name":"a","bucket":"pass"},{"name":"b","bucket":"fail"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::Failing);

        let json = r#"[{"name":"a","bucket":"pass"},{"name":"b","bucket":"pending"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::Pending);

        // A cancelled check is a failure (short-circuits like `fail`).
        let json = r#"[{"name":"a","bucket":"pass"},{"name":"b","bucket":"cancel"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::Failing);

        // All-skipped (no pass/fail/pending) and an empty list both read as None.
        let json = r#"[{"name":"a","bucket":"skipping"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::None);
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok("[]")));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::None);
    }

    // `at` re-binds the cwd while sharing the backend.
    #[tokio::test]
    async fn at_rebinds_cwd_and_shares_backend() {
        let forge = github(ScriptedRunner::new());
        let moved = forge.at("/repo/sub");
        assert_eq!(moved.cwd(), Path::new("/repo/sub"));
        assert_eq!(moved.kind(), ForgeKind::GitHub);
    }

    // `&dyn ForgeApi` must dispatch through the real inherent methods (a delegating
    // body that recursed would stack-overflow here instead of returning).
    #[tokio::test]
    async fn forge_api_trait_object_dispatches() {
        let json = r#"[{"iid":1,"title":"X","state":"opened","source_branch":"f","target_branch":"main","web_url":"u"}]"#;
        let forge = gitlab(
            ScriptedRunner::new()
                .on(["glab", "mr", "list"], Reply::ok(json))
                .on(["glab", "issue", "create"], Reply::ok("https://gl/i/9\n")),
        );
        let dynamic: &dyn ForgeApi = &forge;
        assert_eq!(dynamic.kind(), ForgeKind::GitLab);
        assert_eq!(dynamic.pr_list().await.unwrap()[0].number, 1);
        // Exercise a reference-argument async method through `&dyn` — pins the
        // async_trait lifetime capture the macro relies on (no-arg calls don't).
        assert_eq!(
            dynamic.issue_create("T", "B").await.unwrap(),
            "https://gl/i/9"
        );
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/forge.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
