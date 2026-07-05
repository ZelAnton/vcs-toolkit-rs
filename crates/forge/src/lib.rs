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
//! lifecycle (list / view / create / comment / edit / merge / mark-ready /
//! close, CI checks) · the flat capability map · issues (list / view / create)
//! · releases (list / view). One tiny call:
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
//!   [`gitea`](Forge::gitea) build over the real job-backed runner (the CLI's ambient
//!   login); [`github_with_token`](Forge::github_with_token) /
//!   [`gitlab_with_token`](Forge::gitlab_with_token) authenticate with an explicit
//!   token instead (Gitea is ambient-only — `tea` has no token override).
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
//!   [`pr_create`](Forge::pr_create) / [`pr_comment`](Forge::pr_comment) /
//!   [`pr_edit`](Forge::pr_edit) / [`pr_merge`](Forge::pr_merge) /
//!   [`pr_mark_ready`](Forge::pr_mark_ready) / [`pr_close`](Forge::pr_close) /
//!   [`pr_checks`](Forge::pr_checks)); the capability map
//!   ([`capabilities`](Forge::capabilities)); issues ([`issue_list`](Forge::issue_list) /
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
//!   varying operation is available, and [`ForgeOp::ALL`] enumerates those
//!   varying ops.
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
//! `Forge::from_github(cwd, GitHub::with_runner(ScriptedRunner::new()))` (likewise
//! [`from_gitlab`](Forge::from_gitlab) / [`from_gitea`](Forge::from_gitea)) — and
//! script the canned CLI output, exercising the argv-building and DTO parsing
//! end to end. The cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::fmt::{self, Debug, Formatter};
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
    ForgePrState, ForgeRelease, ForgeRepo, IssueCreate, MergeStrategy, PrClose, PrCreate, PrEdit,
};
pub use error::{Error, Result};

// Re-export the underlying wrappers so a consumer depending only on `vcs-forge`
// can construct the clients (`Forge::from_github(cwd, GitHub::new())`) and reach
// forge-specific operations off the common surface.
pub use vcs_gitea;
pub use vcs_github;
pub use vcs_gitlab;
// Re-export `Secret` so a consumer can name the token type the `*_with_token`
// constructors accept (a plain `&str`/`String` also coerces via `Into<Secret>`, so
// most callers never name it). It is `vcs_cli_support::Secret`, the very type the
// wrappers' `with_token` takes.
pub use vcs_cli_support::Secret;
// Re-export `processkit` itself so a `vcs-forge`-only consumer can match the
// wrapped error — `Error::Forge(vcs_forge::processkit::Error::Timeout { .. })` —
// and name the `CancellationToken` for a `default_cancel_on` client, without a
// direct `processkit` dependency. (Mirrors `vcs_core::processkit`.)
pub use processkit;
pub use processkit::CancellationToken;

/// The per-CLI client behind a [`Forge`]. Shared via `Arc` so [`Forge::at`] can
/// re-anchor the cwd cheaply without rebuilding the client. `Unknown` carries
/// no client — the remote URL didn't classify as a known forge, so no CLI can
/// be picked; the handle exists only to surface the all-`false` capability map.
enum Backend<R: ProcessRunner> {
    GitHub(Arc<GitHub<R>>),
    GitLab(Arc<GitLab<R>>),
    Gitea(Arc<Gitea<R>>),
    Unknown,
}

impl<R: ProcessRunner> Backend<R> {
    fn shared(&self) -> Self {
        match self {
            Backend::GitHub(c) => Backend::GitHub(Arc::clone(c)),
            Backend::GitLab(c) => Backend::GitLab(Arc::clone(c)),
            Backend::Gitea(c) => Backend::Gitea(Arc::clone(c)),
            Backend::Unknown => Backend::Unknown,
        }
    }
}

// Manual Debug, mirroring `vcs_core::Repo`'s `Backend`: no `R: Debug` bound, and
// the inner `GitHub`/`GitLab`/`Gitea` client is never formatted — only the
// discriminant is printed — so a credential token set via `with_token` can't leak
// through `{:?}`. `Unknown` carries no client, so it's printed as a plain unit
// (`finish()`, not `finish_non_exhaustive()` — there's no elided field behind it).
impl<R: ProcessRunner> Debug for Backend<R> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Backend::GitHub(_) => f.debug_tuple("GitHub").finish_non_exhaustive(),
            Backend::GitLab(_) => f.debug_tuple("GitLab").finish_non_exhaustive(),
            Backend::Gitea(_) => f.debug_tuple("Gitea").finish_non_exhaustive(),
            Backend::Unknown => f.debug_tuple("Unknown").finish(),
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

// Manual Debug (no `R: Debug` bound — the reason for hand-writing it rather than
// deriving, matching `vcs_core::Repo`).
impl<R: ProcessRunner> Debug for Forge<R> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let Forge { cwd, backend } = self;
        f.debug_struct("Forge")
            .field("cwd", cwd)
            .field("backend", backend)
            .finish()
    }
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
    ///
    /// Gitea authenticates **only** through `tea`'s ambient login (`tea login add`);
    /// there is deliberately no `gitea_with_token` constructor, because `tea` reads
    /// its credentials from its own config file and offers no token-via-environment
    /// override the way `gh`/`glab` do. Authenticate once, out of band, with
    /// `tea login`.
    pub fn gitea(cwd: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::Gitea(Arc::new(Gitea::new())),
        }
    }

    /// A GitHub-backed handle bound to `cwd` that authenticates with an explicit
    /// personal-access `token` (injected as `GH_TOKEN` for the spawned `gh`) instead
    /// of `gh`'s ambient login. Convenience for the common
    /// `Forge::from_github(cwd, GitHub::new().with_token(token))`; a plain
    /// `&str`/`String` works (it coerces into a [`Secret`]). For an env-var
    /// indirection or a rotating provider, build the [`GitHub`] client yourself and
    /// pass it to [`from_github`](Forge::from_github).
    pub fn github_with_token(cwd: impl Into<PathBuf>, token: impl Into<Secret>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitHub(Arc::new(GitHub::new().with_token(token))),
        }
    }

    /// A GitLab-backed handle bound to `cwd` that authenticates with an explicit
    /// `token` (injected as `GITLAB_TOKEN` for the spawned `glab`) instead of
    /// `glab`'s ambient login — the GitLab analogue of
    /// [`github_with_token`](Forge::github_with_token).
    pub fn gitlab_with_token(cwd: impl Into<PathBuf>, token: impl Into<Secret>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitLab(Arc::new(GitLab::new().with_token(token))),
        }
    }
}

impl<R: ProcessRunner> Forge<R> {
    /// Build a GitHub-backed handle from an explicit client — for a custom runner
    /// (e.g. a test seam) or a pre-configured [`GitHub`].
    pub fn from_github(cwd: impl Into<PathBuf>, client: GitHub<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitHub(Arc::new(client)),
        }
    }

    /// Build a GitLab-backed handle from an explicit [`GitLab`] client.
    pub fn from_gitlab(cwd: impl Into<PathBuf>, client: GitLab<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::GitLab(Arc::new(client)),
        }
    }

    /// Build a Gitea-backed handle from an explicit [`Gitea`] client.
    pub fn from_gitea(cwd: impl Into<PathBuf>, client: Gitea<R>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::Gitea(Arc::new(client)),
        }
    }

    /// Build a handle for a remote URL that didn't classify as a known forge
    /// (a self-hosted instance, a lookalike, or a host [`ForgeKind::from_remote_url`]
    /// can't pin to `github.com`/`gitlab.com`/`gitea.com`/`codeberg.org`).
    /// The handle has no CLI client — every operation returns
    /// [`Error::Unsupported`], and [`capabilities`](Forge::capabilities) returns
    /// the all-`false` shape without spawning anything. Useful for a forge
    /// auto-detector that wants to surface a typed "I tried, no luck" rather
    /// than a guessed-but-wrong kind.
    pub fn from_unknown(cwd: impl Into<PathBuf>) -> Self {
        Forge {
            cwd: cwd.into(),
            backend: Backend::Unknown,
        }
    }

    /// Which forge drives this handle.
    pub fn kind(&self) -> ForgeKind {
        match &self.backend {
            Backend::GitHub(_) => ForgeKind::GitHub,
            Backend::GitLab(_) => ForgeKind::GitLab,
            Backend::Gitea(_) => ForgeKind::Gitea,
            Backend::Unknown => ForgeKind::Unknown,
        }
    }

    /// Whether this handle's backend supports `op`. The capability-varying
    /// operations ([`ForgeOp`]) are all present on GitHub and GitLab; Gitea
    /// (`tea`) supports **none** of them — it has no current-repo view, draft
    /// toggle, PR-checks command, or single-release view; and an
    /// [`Unknown`](ForgeKind::Unknown) backend (no classified CLI) supports
    /// nothing at all (every operation returns `Unsupported`). Every other facade
    /// operation works on all three real backends. Branch on this to hide an
    /// unavailable operation up front instead of calling it and handling
    /// [`Unsupported`](Error::Unsupported).
    pub fn supports(&self, op: ForgeOp) -> bool {
        match (self.kind(), op) {
            // An `Unknown` backend (no classified CLI) supports **nothing** — every
            // operation returns `Unsupported` — so `supports` must report `false`
            // for all ops, matching `capabilities()`'s all-`false` map. (Returning
            // `true` here made a UI render every op as available, each click then
            // failing with `Unsupported`.)
            (ForgeKind::Unknown, _) => false,
            // The four operations `tea` can't do; GitHub/GitLab do everything.
            (
                ForgeKind::Gitea,
                ForgeOp::RepoView | ForgeOp::PrMarkReady | ForgeOp::PrChecks | ForgeOp::ReleaseView,
            ) => false,
            _ => true,
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
    /// status`; Gitea: at least one configured login). An
    /// [`Unknown`](ForgeKind::Unknown) handle (no classified CLI) returns
    /// `Ok(false)` without spawning — there is no CLI to probe.
    pub async fn auth_status(&self) -> Result<bool> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::auth_status(c).await,
            Backend::GitLab(c) => gitlab_forge::auth_status(c).await,
            Backend::Gitea(c) => gitea_forge::auth_status(c).await,
            Backend::Unknown => Ok(false),
        }
    }

    /// The repository/project for the bound directory. **[`Unsupported`](Error::Unsupported)
    /// on Gitea** (`tea` has no current-repo view).
    pub async fn repo_view(&self) -> Result<ForgeRepo> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::repo_view(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::repo_view(c, &self.cwd).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "repo_view")),
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "repo_view")),
        }
    }

    /// Open pull/merge requests for the bound directory (up to 100 on GitHub/GitLab;
    /// **Gitea returns at most ~50** per its server page cap — drop to the underlying
    /// client and page for more).
    pub async fn pr_list(&self) -> Result<Vec<ForgePr>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::pr_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::pr_list(c, &self.cwd).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_list")),
        }
    }

    /// A single PR/MR by number (GitLab `iid`). On Gitea this **pages** a listing and
    /// filters (`tea` has no single-PR view), so it finds a PR past the ~50-row server
    /// page cap — a very large Gitea repo may issue several `tea` calls.
    pub async fn pr_view(&self, number: u64) -> Result<ForgePr> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_view(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_view(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::pr_view(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_view")),
        }
    }

    /// Open a PR/MR (see [`PrCreate`]), returning the CLI's success output — a
    /// URL on GitHub/GitLab; `tea` prints a textual summary (no URL).
    pub async fn pr_create(&self, spec: PrCreate) -> Result<String> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_create(c, &self.cwd, spec).await,
            Backend::GitLab(c) => gitlab_forge::pr_create(c, &self.cwd, spec).await,
            Backend::Gitea(c) => gitea_forge::pr_create(c, &self.cwd, spec).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_create")),
        }
    }

    /// Post a comment to an existing PR/MR. An empty (or whitespace-only) body is
    /// rejected with [`Error::InvalidInput`] before any CLI spawn — a blank comment
    /// is a caller bug the CLIs either post empty or reject opaquely, so fail fast
    /// and uniformly. (This guard is facade-level; the per-crate clients reached
    /// directly apply their own argument handling.)
    ///
    /// Body handling differs by backend: GitHub (`gh ... --body`) and GitLab
    /// (`glab ... -m`) put the body in a flag-value slot, so a body that begins
    /// with `-` is fine. Gitea's `tea comment <n> <body>` takes the body as a
    /// **positional**, so a body whose first non-space character is `-` (e.g. a
    /// Markdown bullet list, or `---`) is rejected with an error — it would
    /// otherwise be parsed as a `tea` flag. (Leading whitespace doesn't help: the
    /// guard trims first.) When targeting Gitea, start such a body with a non-`-`
    /// character — e.g. a heading or a sentence above the list.
    pub async fn pr_comment(&self, number: u64, body: &str) -> Result<String> {
        if body.trim().is_empty() {
            return Err(Error::InvalidInput(
                "pr_comment: comment body must not be empty".into(),
            ));
        }
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_comment(c, &self.cwd, number, body).await,
            Backend::GitLab(c) => gitlab_forge::mr_comment(c, &self.cwd, number, body).await,
            Backend::Gitea(c) => gitea_forge::pr_comment(c, &self.cwd, number, body).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_comment")),
        }
    }

    /// Edit a PR/MR's title and/or body (see [`PrEdit`]). At least one of
    /// `title` or `body` must be `Some` — both-`None` is rejected by the
    /// facade before any CLI is spawned.
    pub async fn pr_edit(&self, number: u64, edit: PrEdit) -> Result<()> {
        if edit.title.is_none() && edit.body.is_none() {
            return Err(Error::InvalidInput(
                "pr_edit: at least one of title or body must be set".into(),
            ));
        }
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_edit(c, &self.cwd, number, edit).await,
            Backend::GitLab(c) => gitlab_forge::mr_edit(c, &self.cwd, number, edit).await,
            Backend::Gitea(c) => gitea_forge::pr_edit(c, &self.cwd, number, edit).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_edit")),
        }
    }

    /// The forge's flat capability map — the intersection of "the CLI ships
    /// this command" and "the CLI is authenticated". Spawns `auth status` /
    /// `login list` exactly once; the per-forge static "ships the command" map
    /// is a constant. The Unknown handle's map is the all-`false` shape.
    pub async fn capabilities(&self) -> Result<ForgeCapabilities> {
        match &self.backend {
            Backend::GitHub(c) => {
                let mut caps = static_github_caps();
                caps.authed = github_forge::auth_status(c).await?;
                if !caps.authed {
                    zero_unauthed(&mut caps);
                }
                Ok(caps)
            }
            Backend::GitLab(c) => {
                let mut caps = static_gitlab_caps();
                caps.authed = gitlab_forge::auth_status(c).await?;
                if !caps.authed {
                    zero_unauthed(&mut caps);
                }
                Ok(caps)
            }
            Backend::Gitea(c) => {
                let mut caps = static_gitea_caps();
                caps.authed = gitea_forge::auth_status(c).await?;
                if !caps.authed {
                    zero_unauthed(&mut caps);
                }
                Ok(caps)
            }
            Backend::Unknown => Ok(ForgeCapabilities::all_false()),
        }
    }

    /// Merge a PR/MR with the given [`MergeStrategy`].
    pub async fn pr_merge(&self, number: u64, strategy: MergeStrategy) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_merge(c, &self.cwd, number, strategy).await,
            Backend::GitLab(c) => gitlab_forge::pr_merge(c, &self.cwd, number, strategy).await,
            Backend::Gitea(c) => gitea_forge::pr_merge(c, &self.cwd, number, strategy).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_merge")),
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
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_mark_ready")),
        }
    }

    /// Close a PR/MR without merging. The [`PrClose`] spec's
    /// [`delete_branch`](PrClose::delete_branch) applies to GitHub only
    /// (`gh pr close --delete-branch`); GitLab and Gitea have no such flag and ignore it.
    pub async fn pr_close(&self, spec: PrClose) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => {
                github_forge::pr_close(c, &self.cwd, spec.number, spec.delete_branch).await
            }
            Backend::GitLab(c) => gitlab_forge::pr_close(c, &self.cwd, spec.number).await,
            Backend::Gitea(c) => gitea_forge::pr_close(c, &self.cwd, spec.number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_close")),
        }
    }

    /// The PR/MR's coarse CI status (see [`CiStatus`]). **[`Unsupported`](Error::Unsupported)
    /// on Gitea** (`tea` has no checks command).
    pub async fn pr_checks(&self, number: u64) -> Result<CiStatus> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_checks(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_checks(c, &self.cwd, number).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "pr_checks")),
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_checks")),
        }
    }

    /// Open issues for the bound directory (up to 100 on GitHub/GitLab; **Gitea
    /// returns at most ~50** per its server page cap — drop to the underlying client
    /// and page for more).
    pub async fn issue_list(&self) -> Result<Vec<ForgeIssue>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::issue_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::issue_list(c, &self.cwd).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_list")),
        }
    }

    /// A single issue by number (GitLab `iid`), with `body`/`url` filled.
    pub async fn issue_view(&self, number: u64) -> Result<ForgeIssue> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_view(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::issue_view(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::issue_view(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_view")),
        }
    }

    /// Open an issue (see [`IssueCreate`]), returning the CLI's success output — a URL
    /// on GitHub/GitLab; `tea` prints a textual summary whose final line is the URL.
    /// (The same honest-output contract as [`pr_create`](Forge::pr_create).)
    pub async fn issue_create(&self, spec: IssueCreate) -> Result<String> {
        let IssueCreate { title, body } = &spec;
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_create(c, &self.cwd, title, body).await,
            Backend::GitLab(c) => gitlab_forge::issue_create(c, &self.cwd, title, body).await,
            Backend::Gitea(c) => gitea_forge::issue_create(c, &self.cwd, title, body).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_create")),
        }
    }

    /// Releases for the bound directory, newest first (up to 100 on GitHub/GitLab;
    /// **Gitea returns at most ~50** per its server page cap — drop to the underlying
    /// client and page for more).
    pub async fn release_list(&self) -> Result<Vec<ForgeRelease>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::release_list(c, &self.cwd).await,
            Backend::GitLab(c) => gitlab_forge::release_list(c, &self.cwd).await,
            Backend::Gitea(c) => gitea_forge::release_list(c, &self.cwd).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "release_list")),
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
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "release_view")),
        }
    }
}

fn unsupported(forge: ForgeKind, operation: &'static str) -> Error {
    Error::unsupported(forge, operation)
}

/// The "what the CLI ships" map for GitHub. `authed` is left `false`; the
/// caller (`Forge::capabilities`) overwrites it from a single `auth status`
/// probe and zeroes the rest if unauthed.
fn static_github_caps() -> ForgeCapabilities {
    ForgeCapabilities {
        pr_create: true,
        pr_comment: true,
        pr_edit: true,
        pr_checks: true,
        pr_merge: true,
        issue_create: true,
        authed: false,
    }
}

/// The "what the CLI ships" map for GitLab. Same shape as GitHub post-fork:
/// `glab mr comment` / `glab mr update` are first-class in the current
/// `glab` (see the `gitlab-org/cli` repo).
fn static_gitlab_caps() -> ForgeCapabilities {
    ForgeCapabilities {
        pr_create: true,
        pr_comment: true,
        pr_edit: true,
        pr_checks: true,
        pr_merge: true,
        issue_create: true,
        authed: false,
    }
}

/// The "what the CLI ships" map for Gitea. `pr_checks` is `false` (no `tea`
/// checks command), and `pr_comment` depends on Q3-R: `tea comment <index>`
/// is documented to hit both issues and PRs (the `index` space is shared).
/// The capability table reports `true`; the wrapper layer is the source of
/// truth, and a future `tea` that drops PR-comment support would return
/// `Error::Unsupported` from the impl — at which point the capability table
/// flips `pr_comment: false`. Kept honest: the table does NOT speculate.
fn static_gitea_caps() -> ForgeCapabilities {
    ForgeCapabilities {
        pr_create: true,
        pr_comment: true,
        pr_edit: true,
        pr_checks: false,
        pr_merge: true,
        issue_create: true,
        authed: false,
    }
}

/// Zero every per-op flag in `caps` — the spec's intersection with
/// `authed: false` (every op is reported as unavailable when the CLI isn't
/// authenticated). Leaves `authed` alone; the caller sets that from the
/// auth probe. Used by [`Forge::capabilities`] for the three known
/// backends.
fn zero_unauthed(caps: &mut ForgeCapabilities) {
    caps.pr_create = false;
    caps.pr_comment = false;
    caps.pr_edit = false;
    caps.pr_checks = false;
    caps.pr_merge = false;
    caps.issue_create = false;
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
///
// Macro `facade_trait!` removed in v0.1.1 — the v0.1.0 macro generated a
// trait + delegating impl from a signature table. Adding default bodies
// for the three post-v0.1.0 methods (`pr_comment`, `pr_edit`,
// `capabilities`) required extending the macro to learn explicit bodies,
// which clashed with the trait-vs-inherent method-resolution dance the
// `#[async_trait]` macro plays. The trait + concrete-impl are now
// hand-maintained just above this comment block — the duplication risk
// (a method added to the trait but not the impl, or vice versa) is small
// (~20 methods) and the compiler catches mismatches at the trait-method
// set (an unimpl'd method is a hard error). The vcs-core copy of the
// macro is unchanged — it's a v0.x crate that doesn't need the new
// methods, so the original signature-table form is still the right
// shape there.

// The trait below is hand-maintained (the v0.1.0 `facade_trait!` macro
// was removed — see the note above). The three additive methods
// (`pr_comment`, `pr_edit`, `capabilities`) have default bodies in the
// trait; the concrete `Forge<R>` impl below overrides them with the
// real dispatch. Rust's method resolution prefers the inherent method
// on the concrete type, so a `&dyn ForgeApi` that's actually a `Forge`
// lands on the real dispatch; an external implementer inherits the
// default body.
#[async_trait::async_trait]
pub trait ForgeApi: Send + Sync {
    /// Which forge drives this handle.
    fn kind(&self) -> ForgeKind;
    /// The directory operations run against.
    fn cwd(&self) -> &Path;
    /// See [`Forge::auth_status`](crate::Forge::auth_status).
    async fn auth_status(&self) -> Result<bool>;
    /// See [`Forge::repo_view`](crate::Forge::repo_view).
    async fn repo_view(&self) -> Result<ForgeRepo>;
    /// See [`Forge::pr_list`](crate::Forge::pr_list).
    async fn pr_list(&self) -> Result<Vec<ForgePr>>;
    /// See [`Forge::pr_view`](crate::Forge::pr_view).
    async fn pr_view(&self, number: u64) -> Result<ForgePr>;
    /// See [`Forge::pr_create`](crate::Forge::pr_create).
    async fn pr_create(&self, spec: PrCreate) -> Result<String>;
    /// See [`Forge::pr_comment`](crate::Forge::pr_comment). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling
    /// when the crate bumps.
    #[allow(unused_variables)]
    async fn pr_comment(&self, number: u64, body: &str) -> Result<String> {
        Err(Error::unsupported(self.kind(), "pr_comment"))
    }
    /// See [`Forge::pr_edit`](crate::Forge::pr_edit). **Defaulted** to
    /// `Error::Unsupported` (the real impl rejects both-`None` with
    /// `Error::InvalidInput` before any spawn).
    #[allow(unused_variables)]
    async fn pr_edit(&self, number: u64, edit: PrEdit) -> Result<()> {
        Err(Error::unsupported(self.kind(), "pr_edit"))
    }
    /// See [`Forge::capabilities`](crate::Forge::capabilities).
    /// **Defaulted** to the all-`false` shape.
    async fn capabilities(&self) -> Result<ForgeCapabilities> {
        Ok(ForgeCapabilities::all_false())
    }
    /// See [`Forge::pr_merge`](crate::Forge::pr_merge).
    async fn pr_merge(&self, number: u64, strategy: MergeStrategy) -> Result<()>;
    /// See [`Forge::pr_mark_ready`](crate::Forge::pr_mark_ready).
    async fn pr_mark_ready(&self, number: u64) -> Result<()>;
    /// See [`Forge::pr_close`](crate::Forge::pr_close).
    async fn pr_close(&self, spec: PrClose) -> Result<()>;
    /// See [`Forge::pr_checks`](crate::Forge::pr_checks).
    async fn pr_checks(&self, number: u64) -> Result<CiStatus>;
    /// See [`Forge::issue_list`](crate::Forge::issue_list).
    async fn issue_list(&self) -> Result<Vec<ForgeIssue>>;
    /// See [`Forge::issue_view`](crate::Forge::issue_view).
    async fn issue_view(&self, number: u64) -> Result<ForgeIssue>;
    /// See [`Forge::issue_create`](crate::Forge::issue_create).
    async fn issue_create(&self, spec: IssueCreate) -> Result<String>;
    /// See [`Forge::release_list`](crate::Forge::release_list).
    async fn release_list(&self) -> Result<Vec<ForgeRelease>>;
    /// See [`Forge::release_view`](crate::Forge::release_view).
    async fn release_view(&self, tag: &str) -> Result<ForgeRelease>;
}

// Concrete-type impl. The v0.1.0 macro generated this; the additive
// methods are added by hand to keep the trait in sync. Rust's method
// resolution prefers the inherent method on `&Forge<R>`, so calls to
// `pr_comment` / `pr_edit` / `capabilities` on a `&dyn ForgeApi` that
// happens to point at a `Forge` land on the real dispatch; an external
// `ForgeApi` implementer inherits the default body.
#[async_trait::async_trait]
impl<R: ProcessRunner> ForgeApi for Forge<R> {
    fn kind(&self) -> ForgeKind {
        self.kind()
    }
    fn cwd(&self) -> &Path {
        self.cwd()
    }
    async fn auth_status(&self) -> Result<bool> {
        self.auth_status().await
    }
    async fn repo_view(&self) -> Result<ForgeRepo> {
        self.repo_view().await
    }
    async fn pr_list(&self) -> Result<Vec<ForgePr>> {
        self.pr_list().await
    }
    async fn pr_view(&self, number: u64) -> Result<ForgePr> {
        self.pr_view(number).await
    }
    async fn pr_create(&self, spec: PrCreate) -> Result<String> {
        self.pr_create(spec).await
    }
    async fn pr_comment(&self, number: u64, body: &str) -> Result<String> {
        self.pr_comment(number, body).await
    }
    async fn pr_edit(&self, number: u64, edit: PrEdit) -> Result<()> {
        self.pr_edit(number, edit).await
    }
    async fn capabilities(&self) -> Result<ForgeCapabilities> {
        self.capabilities().await
    }
    async fn pr_merge(&self, number: u64, strategy: MergeStrategy) -> Result<()> {
        self.pr_merge(number, strategy).await
    }
    async fn pr_mark_ready(&self, number: u64) -> Result<()> {
        self.pr_mark_ready(number).await
    }
    async fn pr_close(&self, spec: PrClose) -> Result<()> {
        self.pr_close(spec).await
    }
    async fn pr_checks(&self, number: u64) -> Result<CiStatus> {
        self.pr_checks(number).await
    }
    async fn issue_list(&self) -> Result<Vec<ForgeIssue>> {
        self.issue_list().await
    }
    async fn issue_view(&self, number: u64) -> Result<ForgeIssue> {
        self.issue_view(number).await
    }
    async fn issue_create(&self, spec: IssueCreate) -> Result<String> {
        self.issue_create(spec).await
    }
    async fn release_list(&self) -> Result<Vec<ForgeRelease>> {
        self.release_list().await
    }
    async fn release_view(&self, tag: &str) -> Result<ForgeRelease> {
        self.release_view(tag).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    fn github(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::from_github("/repo", GitHub::with_runner(runner))
    }
    fn gitlab(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::from_gitlab("/repo", GitLab::with_runner(runner))
    }
    fn gitea(runner: ScriptedRunner) -> Forge<ScriptedRunner> {
        Forge::from_gitea("/repo", Gitea::with_runner(runner))
    }

    #[tokio::test]
    async fn kind_reflects_backend() {
        assert_eq!(github(ScriptedRunner::new()).kind(), ForgeKind::GitHub);
        assert_eq!(gitlab(ScriptedRunner::new()).kind(), ForgeKind::GitLab);
        assert_eq!(gitea(ScriptedRunner::new()).kind(), ForgeKind::Gitea);
    }

    // Regression test for the `Forge`/`Backend` `Debug` impl (T-002/T-003): `{:?}`
    // must not panic, must show the elided shape (`Forge { .. }` with a
    // `GitHub(..)` backend), and — the security-relevant part — must never print
    // a token configured via `with_token`, nor any inner-client internal.
    #[test]
    fn debug_output_shows_elided_github_backend_and_never_leaks_the_token() {
        let forge = Forge::from_github(
            "/repo",
            GitHub::with_runner(ScriptedRunner::new()).with_token("ghp_super_secret_token"),
        );
        let out = format!("{forge:?}");
        assert!(out.contains("Forge {"), "{out}");
        assert!(out.contains("cwd"), "{out}");
        assert!(out.contains("backend"), "{out}");
        assert!(out.contains("GitHub(.."), "{out}");
        assert!(
            !out.contains("ghp_super_secret_token"),
            "token must not leak through Debug: {out}"
        );
        assert!(!out.contains("ManagedClient"), "{out}");
        assert!(!out.contains("CliClient"), "{out}");
    }

    // The GitLab backend gets the same treatment as GitHub above — a second
    // credentialed backend, so the regression isn't pinned to GitHub alone.
    #[test]
    fn debug_output_shows_elided_gitlab_backend_and_never_leaks_the_token() {
        let forge = Forge::from_gitlab(
            "/repo",
            GitLab::with_runner(ScriptedRunner::new()).with_token("glpat-super-secret-token"),
        );
        let out = format!("{forge:?}");
        assert!(out.contains("Forge {"), "{out}");
        assert!(out.contains("GitLab(.."), "{out}");
        assert!(
            !out.contains("glpat-super-secret-token"),
            "token must not leak through Debug: {out}"
        );
    }

    // The `Unknown` backend carries no client at all, so its `Debug` must render
    // as a plain unit variant (not `Unknown(..)`, which would misleadingly imply
    // an elided field) and must not panic.
    #[test]
    fn debug_output_renders_unknown_backend_plainly() {
        let forge: Forge = Forge::from_unknown("/repo");
        let out = format!("{forge:?}");
        assert!(out.contains("Forge {"), "{out}");
        assert!(out.contains("Unknown"), "{out}");
        assert!(!out.contains("Unknown("), "{out}");
    }

    // The token convenience constructors build a real-runner handle of the right
    // backend; the `GH_TOKEN`/`GITLAB_TOKEN` injection itself is covered by the
    // wrapper tests. (Gitea has no such constructor — `tea` authenticates only
    // ambiently.) Constructing the handle spawns nothing.
    #[test]
    fn token_constructors_build_the_right_backend() {
        assert_eq!(
            Forge::github_with_token("/repo", "ghp_x").kind(),
            ForgeKind::GitHub
        );
        assert_eq!(
            Forge::gitlab_with_token("/repo", "glpat-x").kind(),
            ForgeKind::GitLab
        );
    }

    // GitHub's "OPEN"/"MERGED" states map onto the unified ForgePrState.
    #[tokio::test]
    async fn github_pr_list_maps_to_unified() {
        let json = r#"[{"number":7,"title":"X","state":"MERGED","isDraft":true,"headRefName":"feat","baseRefName":"main","url":"u"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].state, ForgePrState::Merged);
        assert_eq!(prs[0].source_branch, "feat");
        // `isDraft` flows through the GitHub mapper (regression guard: a revert to
        // a hardcoded `draft: false` would fail here, as the GitLab path does).
        assert!(prs[0].draft);
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
        let forge = Forge::from_gitea("/repo", Gitea::with_runner(&rec));
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

    // An Unknown handle (the remote didn't classify) reports Unsupported for
    // *every* operation and a `kind` of `Unknown` — and its capability map is
    // the all-`false` shape WITHOUT spawning.
    #[tokio::test]
    async fn unknown_forge_reports_all_unsupported() {
        let forge: Forge = Forge::from_unknown("/repo");
        assert_eq!(forge.kind(), ForgeKind::Unknown);
        assert!(!forge.auth_status().await.unwrap(), "unknown = not authed");
        let caps = forge.capabilities().await.unwrap();
        assert_eq!(caps, ForgeCapabilities::all_false());
        for err in [
            forge.repo_view().await.unwrap_err(),
            forge.pr_list().await.unwrap_err(),
            forge.pr_view(1).await.unwrap_err(),
            forge.pr_create(PrCreate::new("T", "B")).await.unwrap_err(),
            forge.pr_merge(1, MergeStrategy::Merge).await.unwrap_err(),
            forge.pr_mark_ready(1).await.unwrap_err(),
            forge.pr_close(PrClose::new(1)).await.unwrap_err(),
            forge.pr_checks(1).await.unwrap_err(),
            forge.issue_list().await.unwrap_err(),
            forge.issue_view(1).await.unwrap_err(),
            forge
                .issue_create(IssueCreate::new("T", "B"))
                .await
                .unwrap_err(),
            forge.release_list().await.unwrap_err(),
            forge.release_view("v1").await.unwrap_err(),
            forge.pr_comment(1, "x").await.unwrap_err(),
            forge
                .pr_edit(1, PrEdit::new().title("T"))
                .await
                .unwrap_err(),
        ] {
            assert!(err.is_unsupported(), "{err:?}");
        }
    }

    // pr_edit rejects both-None with InvalidInput BEFORE any spawn — the
    // explicit-error path per spec §2.
    #[tokio::test]
    async fn pr_edit_both_none_is_invalid_input_not_unsupported() {
        let forge = github(ScriptedRunner::new()); // no scripted rules: a spawn would error
        let err = forge.pr_edit(7, PrEdit::new()).await.unwrap_err();
        assert!(
            matches!(err, crate::Error::InvalidInput(_)),
            "both-None must surface as InvalidInput, got {err:?}"
        );
    }

    // pr_comment rejects an empty / whitespace-only body with InvalidInput BEFORE
    // any spawn — fail fast and uniformly instead of posting a blank comment.
    #[tokio::test]
    async fn pr_comment_empty_body_is_invalid_input() {
        let forge = github(ScriptedRunner::new()); // no scripted rules: a spawn would error
        for body in ["", "   ", "\t\n"] {
            let err = forge.pr_comment(7, body).await.unwrap_err();
            assert!(
                matches!(err, crate::Error::InvalidInput(_)),
                "empty body {body:?} must surface as InvalidInput, got {err:?}"
            );
        }
    }

    // pr_edit with a partial spec routes through to the wrapper and succeeds.
    // The GitHub wrapper's argv is pinned (the existing test covers both
    // fields too); the facade just needs to forward.
    #[tokio::test]
    async fn pr_edit_forwards_to_wrapper() {
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "edit"], Reply::ok("")));
        forge
            .pr_edit(7, PrEdit::new().title("New"))
            .await
            .expect("pr_edit title-only");
    }

    // The capability map for an authed GitHub is everything-true (post-fork).
    #[tokio::test]
    async fn github_capabilities_authed_lights_everything() {
        let forge = github(ScriptedRunner::new().on(["gh", "auth"], Reply::ok("")));
        let caps = forge.capabilities().await.unwrap();
        assert!(caps.pr_create);
        assert!(caps.pr_comment);
        assert!(caps.pr_edit);
        assert!(caps.pr_checks);
        assert!(caps.pr_merge);
        assert!(caps.issue_create);
        assert!(caps.authed);
    }

    // An unauthed GitHub keeps the static map's "ships the op" shape but flips
    // every op-specific flag to false (the intersection with `authed: false`
    // from spec §3). The `auth status` call exits non-zero ⇒ `auth_status()`
    // returns `false` (per the wrapper's documented exit-code reflection) and
    // the capability table zeros the ops.
    #[tokio::test]
    async fn github_capabilities_unauthed_zeros_ops_but_keeps_authed_false() {
        let forge = github(ScriptedRunner::new().on(["gh", "auth"], Reply::fail(1, "no")));
        let caps = forge.capabilities().await.unwrap();
        assert!(!caps.authed, "unauthed");
        assert!(!caps.pr_create);
        assert!(!caps.pr_comment);
        assert!(!caps.pr_edit);
        assert!(!caps.pr_checks);
        assert!(!caps.pr_merge);
        assert!(!caps.issue_create);
    }

    // Gitea's static map is the intersection of its CLI: `pr_checks` is the
    // only false when authed (no `tea` checks command). Everything else is
    // `true` post-fork.
    #[tokio::test]
    async fn gitea_capabilities_authed_has_only_pr_checks_false() {
        // Gitea's auth probe parses `tea login list --output json` on a zero
        // exit and reports authed = (the array is non-empty). Script a non-empty
        // array so the probe reports authed; `[]` would read as not-authed.
        let forge = gitea(
            ScriptedRunner::new().on(["tea", "login", "list"], Reply::ok(r#"[{"name":"a"}]"#)),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(caps.authed, "gitea authed");
        assert!(!caps.pr_checks, "gitea has no checks command");
        assert!(caps.pr_create);
        assert!(caps.pr_comment);
        assert!(caps.pr_edit);
        assert!(caps.pr_merge);
        assert!(caps.issue_create);
    }

    // `supports` must agree exactly with the runtime `Unsupported` behaviour
    // above: Gitea reports `false` for the four varying ops, GitHub and GitLab
    // report `true` for all of them — a pure, no-spawn capability check.
    #[test]
    fn supports_matches_unsupported_ops() {
        let gitea = Forge::from_gitea("/repo", Gitea::with_runner(ScriptedRunner::new()));
        for &op in ForgeOp::ALL {
            assert!(!gitea.supports(op), "gitea should not support {op:?}");
        }
        // An Unknown backend supports nothing — every op returns Unsupported, so
        // `supports` must be `false` for all of them (matches `capabilities()`).
        let unknown: Forge = Forge::from_unknown("/repo");
        for &op in ForgeOp::ALL {
            assert!(!unknown.supports(op), "unknown should not support {op:?}");
        }
        for forge in [
            Forge::from_github("/repo", GitHub::with_runner(ScriptedRunner::new())),
            Forge::from_gitlab("/repo", GitLab::with_runner(ScriptedRunner::new())),
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
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_merge(5, MergeStrategy::Squash)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "merge", "5", "--squash"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
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
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
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

        // An unmodeled bucket (a future `gh` value → `CheckBucket::Unknown`) read
        // ALONE is "not known to be done" → Pending (not the misleading None),
        // matching the GitLab mapper's unknown→Pending behavior.
        let json = r#"[{"name":"a","bucket":"frobnicate"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::Pending);

        // …but a modeled pass alongside an unmodeled bucket still reports Passing
        // (the checks we understand passed; an Unknown is not a recognized failure).
        let json = r#"[{"name":"a","bucket":"pass"},{"name":"b","bucket":"frobnicate"}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "checks"], Reply::ok(json)));
        assert_eq!(forge.pr_checks(1).await.unwrap(), CiStatus::Passing);
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
            dynamic
                .issue_create(IssueCreate::new("T", "B"))
                .await
                .unwrap(),
            "https://gl/i/9"
        );
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/forge.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
