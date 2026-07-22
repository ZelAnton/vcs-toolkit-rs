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
//! close / checkout / approve / request-changes, CI checks) · the flat capability
//! map · issues (list / view / create) · releases (list / view / create / delete).
//! One tiny call:
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
//!   `OPEN` / `opened` / `open` all read as one state. Fields a backend can't
//!   report follow a **per-field support contract** — they are `Option` (a PR's
//!   `draft`/`labels`/`assignees`/`author`/`created_at`/`updated_at`/`milestone`,
//!   an issue's identical set, a repo's `private`, a release's
//!   `url`/`draft`/`prerelease`/`author`), so `None` ("backend can't/didn't report
//!   it") is distinct from a *confirmed* `Some(false)`/empty list, never a false
//!   sentinel (see each DTO's field docs).
//! - **Operation groups** — auth ([`auth_status`](Forge::auth_status)); the repo
//!   ([`repo_view`](Forge::repo_view)); the PR/MR lifecycle
//!   ([`pr_list`](Forge::pr_list) / [`pr_view`](Forge::pr_view) /
//!   [`pr_create`](Forge::pr_create) / [`pr_comment`](Forge::pr_comment) /
//!   [`pr_edit`](Forge::pr_edit) / [`pr_merge`](Forge::pr_merge) /
//!   [`pr_approve`](Forge::pr_approve) /
//!   [`pr_request_changes`](Forge::pr_request_changes) /
//!   [`pr_mark_ready`](Forge::pr_mark_ready) / [`pr_close`](Forge::pr_close) /
//!   [`pr_checkout`](Forge::pr_checkout) / [`pr_checks`](Forge::pr_checks) /
//!   [`pr_diff`](Forge::pr_diff)); the capability
//!   map ([`capabilities`](Forge::capabilities)); issues ([`issue_list`](Forge::issue_list) /
//!   [`issue_view`](Forge::issue_view) / [`issue_create`](Forge::issue_create) /
//!   [`issue_close`](Forge::issue_close) / [`issue_reopen`](Forge::issue_reopen) /
//!   [`issue_comment`](Forge::issue_comment));
//!   releases ([`release_list`](Forge::release_list) /
//!   [`release_view`](Forge::release_view) / [`release_create`](Forge::release_create) /
//!   [`release_delete`](Forge::release_delete)). List ops cap at 100 — drop to the
//!   wrapped client for more.
//! - **Capability gaps** — `tea` has no current-repo view, draft toggle, checks
//!   command, single-release view, or diff view, so on a Gitea handle
//!   [`repo_view`](Forge::repo_view), [`pr_mark_ready`](Forge::pr_mark_ready),
//!   [`pr_checks`](Forge::pr_checks), [`release_view`](Forge::release_view), and
//!   [`pr_diff`](Forge::pr_diff) return [`Error::Unsupported`] **without
//!   spawning**. GitLab's review model is approve/revoke, so
//!   [`pr_request_changes`](Forge::pr_request_changes) is
//!   [`Unsupported`](Error::Unsupported) on a GitLab handle (approve and
//!   request-changes are otherwise available on the other backends). Classify any of
//!   these with [`Error::is_unsupported`].
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
//! hand-maintained [`ForgeApi`] signatures. Test the *real* dispatch instead:
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
    PrMerge, ReleaseCreate,
};
pub use error::{Error, Result};

// Re-export the underlying wrappers so a consumer depending only on `vcs-forge`
// can construct the clients (`Forge::from_github(cwd, GitHub::new())`) and reach
// forge-specific operations off the common surface.
pub use vcs_gitea;
pub use vcs_github;
pub use vcs_gitlab;
// Re-export `vcs-diff`'s unified-diff model, since `pr_diff` returns it
// directly — `gh pr diff`/`glab mr diff` already emit the same git-format diff
// the parser expects, so no facade-specific DTO wraps it.
pub use vcs_diff;
pub use vcs_diff::{ChangeKind, DiffLine, FileDiff, Hunk};
// The parsed CLI version type behind [`ForgeCapabilities::version`] — re-exported so
// a `vcs-forge`-only consumer can name it without a direct `vcs-diff` dependency.
pub use vcs_diff::Version;
// Re-export `Secret` so a consumer can name the token type the `*_with_token`
// constructors accept (a plain `&str`/`String` also coerces via `Into<Secret>`, so
// most callers never name it). It is `vcs_cli_support::Secret`, the very type the
// wrappers' `with_token` takes.
pub use vcs_cli_support::{OutputBudget, Secret};
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

    /// Whether this handle's backend supports `op`. GitHub supports every operation
    /// in [`ForgeOp`]; GitLab supports all but [`PrRequestChanges`](ForgeOp::PrRequestChanges)
    /// (its review model is approve/revoke, with no request-changes action); Gitea
    /// (`tea`) supports [`PrCheckout`](ForgeOp::PrCheckout),
    /// [`PrApprove`](ForgeOp::PrApprove), [`PrRequestChanges`](ForgeOp::PrRequestChanges),
    /// [`ReleaseCreate`](ForgeOp::ReleaseCreate), [`ReleaseDelete`](ForgeOp::ReleaseDelete),
    /// and the three issue-lifecycle ops [`IssueClose`](ForgeOp::IssueClose) /
    /// [`IssueReopen`](ForgeOp::IssueReopen) / [`IssueComment`](ForgeOp::IssueComment)
    /// but has no current-repo view, draft toggle, PR-checks command, single-release
    /// view, or diff view; and an [`Unknown`](ForgeKind::Unknown) backend (no
    /// classified CLI) supports nothing at all (every operation returns
    /// `Unsupported`). Every other facade operation works on all three real backends.
    /// (`release_create` is supported on all three even though its `draft`/`prerelease`
    /// options are a GitLab gap — that per-option gap surfaces at call time, not here.)
    /// Branch on this to hide an unavailable operation up front instead of calling it
    /// and handling [`Unsupported`](Error::Unsupported).
    pub fn supports(&self, op: ForgeOp) -> bool {
        match (self.kind(), op) {
            // An `Unknown` backend (no classified CLI) supports **nothing** — every
            // operation returns `Unsupported` — so `supports` must report `false`
            // for all ops, matching `capabilities()`'s all-`false` map. (Returning
            // `true` here made a UI render every op as available, each click then
            // failing with `Unsupported`.)
            (ForgeKind::Unknown, _) => false,
            // The five operations `tea` can't do (it *does* ship approve/reject and
            // checkout); GitHub does everything.
            (
                ForgeKind::Gitea,
                ForgeOp::RepoView
                | ForgeOp::PrMarkReady
                | ForgeOp::PrChecks
                | ForgeOp::ReleaseView
                | ForgeOp::PrDiff,
            ) => false,
            // GitLab's review model is approve/revoke — there is no request-changes
            // action, so the facade reports it Unsupported for GitLab.
            (ForgeKind::GitLab, ForgeOp::PrRequestChanges) => false,
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

    /// The forge's flat capability map — the intersection of "the CLI ships this
    /// command", "the installed CLI meets the wrapper's version floor", and "the CLI
    /// is authenticated". Probes the CLI **version** (`gh`/`glab`/`tea --version`)
    /// and **auth** (`auth status` / `login list`) once each; the per-forge static
    /// "ships the command" map is a constant. A CLI below the version floor zeroes
    /// the per-op flags exactly like an unauthed one — the honest answer for an old
    /// binary that lacks the modern command surface. An unrecognisable `--version`
    /// banner degrades to `supported: false` / `version: None` (conservatively
    /// unavailable) rather than failing the whole probe; a genuine spawn/timeout
    /// failure still propagates. The Unknown handle's map is the all-`false` shape
    /// (no spawn).
    pub async fn capabilities(&self) -> Result<ForgeCapabilities> {
        match &self.backend {
            Backend::GitHub(c) => {
                let mut caps = static_github_caps();
                let (version, supported) = github_forge::version_support(c).await?;
                caps.version = version;
                caps.supported = supported;
                caps.authed = github_forge::auth_status(c).await?;
                if !caps.authed || !caps.supported {
                    zero_ops(&mut caps);
                }
                Ok(caps)
            }
            Backend::GitLab(c) => {
                let mut caps = static_gitlab_caps();
                let (version, supported) = gitlab_forge::version_support(c).await?;
                caps.version = version;
                caps.supported = supported;
                caps.authed = gitlab_forge::auth_status(c).await?;
                if !caps.authed || !caps.supported {
                    zero_ops(&mut caps);
                }
                Ok(caps)
            }
            Backend::Gitea(c) => {
                let mut caps = static_gitea_caps();
                let (version, supported) = gitea_forge::version_support(c).await?;
                caps.version = version;
                caps.supported = supported;
                caps.authed = gitea_forge::auth_status(c).await?;
                if !caps.authed || !caps.supported {
                    zero_ops(&mut caps);
                }
                Ok(caps)
            }
            Backend::Unknown => Ok(ForgeCapabilities::all_false()),
        }
    }

    /// Merge a PR/MR with the given [`PrMerge`] spec (strategy plus the optional
    /// `auto`/`delete_branch` flags). Those two options are **GitHub-only**: on
    /// GitLab/Gitea, requesting either returns [`Unsupported`](Error::Unsupported)
    /// rather than silently merging without it (see [`PrMerge`]).
    pub async fn pr_merge(&self, number: u64, merge: PrMerge) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_merge(c, &self.cwd, number, merge).await,
            Backend::GitLab(c) => gitlab_forge::pr_merge(c, &self.cwd, number, merge).await,
            Backend::Gitea(c) => gitea_forge::pr_merge(c, &self.cwd, number, merge).await,
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

    /// Submit an **approving** review on a PR/MR — `gh pr review --approve` /
    /// `glab mr approve` / `tea pr approve`. Supported on all three real backends; an
    /// [`Unknown`](ForgeKind::Unknown) handle returns
    /// [`Unsupported`](Error::Unsupported). The negative side of review differs by
    /// forge: [`pr_request_changes`](Forge::pr_request_changes) on GitHub/Gitea, and
    /// GitLab's approve/revoke model (withdraw via the wrapper's `mr_revoke`).
    pub async fn pr_approve(&self, number: u64) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_approve(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_approve(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::pr_approve(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_approve")),
        }
    }

    /// Submit a **request-changes** review carrying a required `body`/reason —
    /// `gh pr review --request-changes --body <body>` (GitHub) / `tea pr reject <n>
    /// <reason>` (Gitea). **[`Unsupported`](Error::Unsupported) on GitLab**, whose
    /// review model is approve/revoke with no request-changes action (withdraw an
    /// approval with the wrapper's `mr_revoke` instead). An empty or whitespace-only
    /// `body` is rejected with [`InvalidInput`](Error::InvalidInput) before any CLI
    /// spawn — a request-changes review needs a reason on every backend that supports
    /// it (and Gitea would reject a blank positional anyway), so fail fast and
    /// uniformly.
    pub async fn pr_request_changes(&self, number: u64, body: &str) -> Result<()> {
        if body.trim().is_empty() {
            return Err(Error::InvalidInput(
                "pr_request_changes: a request-changes review requires a non-empty body".into(),
            ));
        }
        match &self.backend {
            Backend::GitHub(c) => {
                github_forge::pr_request_changes(c, &self.cwd, number, body).await
            }
            Backend::GitLab(_) => Err(unsupported(ForgeKind::GitLab, "pr_request_changes")),
            Backend::Gitea(c) => gitea_forge::pr_request_changes(c, &self.cwd, number, body).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_request_changes")),
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

    /// Check out a PR/MR's branch into the bound working copy — `gh pr checkout
    /// <n>` / `glab mr checkout <n>` / `tea pr checkout <n>`. The head/source
    /// branch is fetched and switched to, so a subsequent build/test/edit runs
    /// against the PR locally. **Mutates the working copy.** Supported on all three
    /// real backends; an [`Unknown`](ForgeKind::Unknown) handle returns
    /// [`Unsupported`](Error::Unsupported).
    pub async fn pr_checkout(&self, number: u64) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_checkout(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_checkout(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::pr_checkout(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_checkout")),
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

    /// The PR/MR's diff, one [`FileDiff`] per changed file — `gh pr diff <n>` /
    /// `glab mr diff <n>`, through the same unified-diff parser `vcs-git`/
    /// `vcs-jj` use (both CLIs emit the same git-format diff `git diff` does).
    /// **[`Unsupported`](Error::Unsupported) on Gitea** (`tea` has no diff
    /// command).
    pub async fn pr_diff(&self, number: u64) -> Result<Vec<FileDiff>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_diff(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::pr_diff(c, &self.cwd, number).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "pr_diff")),
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_diff")),
        }
    }

    /// [`pr_diff`](Forge::pr_diff) with an explicit per-call [`OutputBudget`],
    /// instead of the underlying client's
    /// [`default_output_budget`](vcs_github::GitHub::default_output_budget). Past
    /// the ceiling the read errors with an `OutputTooLarge`-carrying
    /// [`Error::Forge`] (actual and allowed sizes) rather than buffering an
    /// unbounded diff — the override for a legitimately huge PR/MR.
    /// **[`Unsupported`](Error::Unsupported) on Gitea** (`tea` has no diff command).
    pub async fn pr_diff_within(&self, number: u64, budget: OutputBudget) -> Result<Vec<FileDiff>> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::pr_diff_within(c, &self.cwd, number, budget).await,
            Backend::GitLab(c) => gitlab_forge::pr_diff_within(c, &self.cwd, number, budget).await,
            Backend::Gitea(_) => Err(unsupported(ForgeKind::Gitea, "pr_diff")),
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "pr_diff")),
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

    /// Close an issue (`gh issue close` / `glab issue close` / `tea issues close`).
    /// Supported on all three real backends; an [`Unknown`](ForgeKind::Unknown)
    /// handle returns [`Unsupported`](Error::Unsupported).
    pub async fn issue_close(&self, number: u64) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_close(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::issue_close(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::issue_close(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_close")),
        }
    }

    /// Reopen a closed issue (`gh issue reopen` / `glab issue reopen` / `tea issues
    /// reopen`). Supported on all three real backends; an
    /// [`Unknown`](ForgeKind::Unknown) handle returns
    /// [`Unsupported`](Error::Unsupported).
    pub async fn issue_reopen(&self, number: u64) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_reopen(c, &self.cwd, number).await,
            Backend::GitLab(c) => gitlab_forge::issue_reopen(c, &self.cwd, number).await,
            Backend::Gitea(c) => gitea_forge::issue_reopen(c, &self.cwd, number).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_reopen")),
        }
    }

    /// Post a comment to an existing issue, returning the CLI's output (the comment
    /// URL on GitHub/GitLab; `tea` prints a textual summary). An empty (or
    /// whitespace-only) body is rejected with [`Error::InvalidInput`] before any CLI
    /// spawn — a blank comment is a caller bug the CLIs either post empty or reject
    /// opaquely, so fail fast and uniformly (the same facade-level guard as
    /// [`pr_comment`](Forge::pr_comment)).
    ///
    /// Body handling differs by backend, exactly as for [`pr_comment`](Forge::pr_comment):
    /// GitHub (`gh issue comment --body`) and GitLab (`glab issue note -m`) put the
    /// body in a flag-value slot, so a body that begins with `-` is fine. Gitea's
    /// `tea comment <n> <body>` takes the body as a **positional**, so a body whose
    /// first non-space character is `-` (e.g. a Markdown bullet list, or `---`) is
    /// rejected with an error. When targeting Gitea, start such a body with a
    /// non-`-` character.
    pub async fn issue_comment(&self, number: u64, body: &str) -> Result<String> {
        if body.trim().is_empty() {
            return Err(Error::InvalidInput(
                "issue_comment: comment body must not be empty".into(),
            ));
        }
        match &self.backend {
            Backend::GitHub(c) => github_forge::issue_comment(c, &self.cwd, number, body).await,
            Backend::GitLab(c) => gitlab_forge::issue_comment(c, &self.cwd, number, body).await,
            Backend::Gitea(c) => gitea_forge::issue_comment(c, &self.cwd, number, body).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "issue_comment")),
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

    /// Create a release (see [`ReleaseCreate`]), returning the CLI's success output
    /// — a URL on GitHub/GitLab; `tea` prints a textual summary. Supported on all
    /// three real backends. The spec's `draft`/`prerelease` options are
    /// **GitHub/Gitea only**: on GitLab (which has no such concept) requesting either
    /// returns [`Unsupported`](Error::Unsupported) rather than silently ignoring it
    /// (see [`ReleaseCreate`]). Asset uploads are out of scope — drop to the wrapped
    /// client for those.
    pub async fn release_create(&self, spec: ReleaseCreate) -> Result<String> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::release_create(c, &self.cwd, spec).await,
            Backend::GitLab(c) => gitlab_forge::release_create(c, &self.cwd, spec).await,
            Backend::Gitea(c) => gitea_forge::release_create(c, &self.cwd, spec).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "release_create")),
        }
    }

    /// Delete a release by its tag (`gh release delete` / `glab release delete` /
    /// `tea releases delete`). Deletes the release only, not the underlying git tag.
    /// Supported on all three real backends; an [`Unknown`](ForgeKind::Unknown)
    /// handle returns [`Unsupported`](Error::Unsupported).
    pub async fn release_delete(&self, tag: &str) -> Result<()> {
        match &self.backend {
            Backend::GitHub(c) => github_forge::release_delete(c, &self.cwd, tag).await,
            Backend::GitLab(c) => gitlab_forge::release_delete(c, &self.cwd, tag).await,
            Backend::Gitea(c) => gitea_forge::release_delete(c, &self.cwd, tag).await,
            Backend::Unknown => Err(unsupported(ForgeKind::Unknown, "release_delete")),
        }
    }
}

fn unsupported(forge: ForgeKind, operation: &'static str) -> Error {
    Error::unsupported(forge, operation)
}

/// The "what the CLI ships" map for GitHub. `version`/`supported`/`authed` are
/// left unset; the caller (`Forge::capabilities`) overwrites them from the
/// version + auth probes and zeroes the op flags if unsupported or unauthed.
fn static_github_caps() -> ForgeCapabilities {
    ForgeCapabilities {
        pr_create: true,
        pr_comment: true,
        pr_edit: true,
        pr_checks: true,
        pr_merge: true,
        pr_approve: true,
        pr_request_changes: true,
        issue_create: true,
        issue_close: true,
        issue_reopen: true,
        issue_comment: true,
        release_create: true,
        release_delete: true,
        version: None,
        supported: false,
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
        pr_approve: true,
        // GitLab's review model is approve/revoke — no request-changes action, so
        // this stays `false` even for an authed, modern `glab`.
        pr_request_changes: false,
        issue_create: true,
        // `glab` ships `issue close`/`issue reopen`/`issue note`.
        issue_close: true,
        issue_reopen: true,
        issue_comment: true,
        // `glab` ships `release create`/`release delete`. The `draft`/`prerelease`
        // create *options* are unsupported on GitLab, but the create command itself
        // is available — that per-option gap is enforced at call time, not here.
        release_create: true,
        release_delete: true,
        version: None,
        supported: false,
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
        // `tea` ships both `pr approve` and `pr reject` (request-changes).
        pr_approve: true,
        pr_request_changes: true,
        issue_create: true,
        // `tea` ships `issues close`/`issues reopen`, and issue comments ride the
        // shared `tea comment <index>` subcommand (the issue/PR index space is shared).
        issue_close: true,
        issue_reopen: true,
        issue_comment: true,
        // `tea` ships `releases create` (with draft/prerelease) and `releases delete`.
        release_create: true,
        release_delete: true,
        version: None,
        supported: false,
        authed: false,
    }
}

/// Zero every per-op flag in `caps` — the spec's intersection when the CLI is
/// either **not authenticated** or **below the version floor** (an op can't be
/// guaranteed in either case). Leaves `version`/`supported`/`authed` alone; the
/// caller sets those from the version + auth probes. Used by
/// [`Forge::capabilities`] for the three known backends.
fn zero_ops(caps: &mut ForgeCapabilities) {
    caps.pr_create = false;
    caps.pr_comment = false;
    caps.pr_edit = false;
    caps.pr_checks = false;
    caps.pr_merge = false;
    caps.pr_approve = false;
    caps.pr_request_changes = false;
    caps.issue_create = false;
    caps.issue_close = false;
    caps.issue_reopen = false;
    caps.issue_comment = false;
    caps.release_create = false;
    caps.release_delete = false;
}

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
    async fn pr_merge(&self, number: u64, merge: PrMerge) -> Result<()>;
    /// See [`Forge::pr_approve`](crate::Forge::pr_approve). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn pr_approve(&self, number: u64) -> Result<()> {
        Err(Error::unsupported(self.kind(), "pr_approve"))
    }
    /// See [`Forge::pr_request_changes`](crate::Forge::pr_request_changes).
    /// **Defaulted** to `Error::Unsupported` (the real impl rejects an empty body
    /// with `Error::InvalidInput` and reports GitLab `Unsupported`).
    #[allow(unused_variables)]
    async fn pr_request_changes(&self, number: u64, body: &str) -> Result<()> {
        Err(Error::unsupported(self.kind(), "pr_request_changes"))
    }
    /// See [`Forge::pr_mark_ready`](crate::Forge::pr_mark_ready).
    async fn pr_mark_ready(&self, number: u64) -> Result<()>;
    /// See [`Forge::pr_close`](crate::Forge::pr_close).
    async fn pr_close(&self, spec: PrClose) -> Result<()>;
    /// See [`Forge::pr_checkout`](crate::Forge::pr_checkout). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn pr_checkout(&self, number: u64) -> Result<()> {
        Err(Error::unsupported(self.kind(), "pr_checkout"))
    }
    /// See [`Forge::pr_checks`](crate::Forge::pr_checks).
    async fn pr_checks(&self, number: u64) -> Result<CiStatus>;
    /// See [`Forge::pr_diff`](crate::Forge::pr_diff).
    async fn pr_diff(&self, number: u64) -> Result<Vec<FileDiff>>;
    /// See [`Forge::issue_list`](crate::Forge::issue_list).
    async fn issue_list(&self) -> Result<Vec<ForgeIssue>>;
    /// See [`Forge::issue_view`](crate::Forge::issue_view).
    async fn issue_view(&self, number: u64) -> Result<ForgeIssue>;
    /// See [`Forge::issue_create`](crate::Forge::issue_create).
    async fn issue_create(&self, spec: IssueCreate) -> Result<String>;
    /// See [`Forge::issue_close`](crate::Forge::issue_close). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn issue_close(&self, number: u64) -> Result<()> {
        Err(Error::unsupported(self.kind(), "issue_close"))
    }
    /// See [`Forge::issue_reopen`](crate::Forge::issue_reopen). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn issue_reopen(&self, number: u64) -> Result<()> {
        Err(Error::unsupported(self.kind(), "issue_reopen"))
    }
    /// See [`Forge::issue_comment`](crate::Forge::issue_comment). **Defaulted** to
    /// `Error::Unsupported` (the real impl rejects an empty body with
    /// `Error::InvalidInput` before any spawn).
    #[allow(unused_variables)]
    async fn issue_comment(&self, number: u64, body: &str) -> Result<String> {
        Err(Error::unsupported(self.kind(), "issue_comment"))
    }
    /// See [`Forge::release_list`](crate::Forge::release_list).
    async fn release_list(&self) -> Result<Vec<ForgeRelease>>;
    /// See [`Forge::release_view`](crate::Forge::release_view).
    async fn release_view(&self, tag: &str) -> Result<ForgeRelease>;
    /// See [`Forge::release_create`](crate::Forge::release_create). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn release_create(&self, spec: ReleaseCreate) -> Result<String> {
        Err(Error::unsupported(self.kind(), "release_create"))
    }
    /// See [`Forge::release_delete`](crate::Forge::release_delete). **Defaulted** to
    /// `Error::Unsupported` so external trait implementers keep compiling when the
    /// crate bumps.
    #[allow(unused_variables)]
    async fn release_delete(&self, tag: &str) -> Result<()> {
        Err(Error::unsupported(self.kind(), "release_delete"))
    }
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
    async fn pr_merge(&self, number: u64, merge: PrMerge) -> Result<()> {
        self.pr_merge(number, merge).await
    }
    async fn pr_approve(&self, number: u64) -> Result<()> {
        self.pr_approve(number).await
    }
    async fn pr_request_changes(&self, number: u64, body: &str) -> Result<()> {
        self.pr_request_changes(number, body).await
    }
    async fn pr_mark_ready(&self, number: u64) -> Result<()> {
        self.pr_mark_ready(number).await
    }
    async fn pr_close(&self, spec: PrClose) -> Result<()> {
        self.pr_close(spec).await
    }
    async fn pr_checkout(&self, number: u64) -> Result<()> {
        self.pr_checkout(number).await
    }
    async fn pr_checks(&self, number: u64) -> Result<CiStatus> {
        self.pr_checks(number).await
    }
    async fn pr_diff(&self, number: u64) -> Result<Vec<FileDiff>> {
        self.pr_diff(number).await
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
    async fn issue_close(&self, number: u64) -> Result<()> {
        self.issue_close(number).await
    }
    async fn issue_reopen(&self, number: u64) -> Result<()> {
        self.issue_reopen(number).await
    }
    async fn issue_comment(&self, number: u64, body: &str) -> Result<String> {
        self.issue_comment(number, body).await
    }
    async fn release_list(&self) -> Result<Vec<ForgeRelease>> {
        self.release_list().await
    }
    async fn release_view(&self, tag: &str) -> Result<ForgeRelease> {
        self.release_view(tag).await
    }
    async fn release_create(&self, spec: ReleaseCreate) -> Result<String> {
        self.release_create(spec).await
    }
    async fn release_delete(&self, tag: &str) -> Result<()> {
        self.release_delete(tag).await
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
        // `isDraft` flows through the GitHub mapper as a *confirmed* `Some(true)`
        // (regression guard: a revert to a hardcoded `false`/`None` would fail
        // here). GitHub also confirms labels/assignees (`Some`, here empty).
        assert_eq!(prs[0].draft, Some(true));
        assert_eq!(prs[0].labels, Some(Vec::new()));
        assert_eq!(prs[0].assignees, Some(Vec::new()));
    }

    // GitLab `repo_view` maps a known "public" visibility to private == false.
    #[tokio::test]
    async fn gitlab_repo_view_maps_public_visibility() {
        let json = r#"{"name":"cli","path_with_namespace":"gitlab-org/cli","default_branch":"main","web_url":"u","visibility":"public"}"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "repo", "view"], Reply::ok(json)));
        let repo = forge.repo_view().await.unwrap();
        assert_eq!(repo.owner, "gitlab-org");
        assert_eq!(repo.name, "cli");
        // A *known* "public" visibility is a confirmed `Some(false)`.
        assert_eq!(repo.private, Some(false));
    }

    // When glab omits `visibility`, the facade must NOT report the repo as private
    // — an unknown visibility is the conservative `false`, never a false privacy.
    #[tokio::test]
    async fn gitlab_repo_view_absent_visibility_is_not_private() {
        let json =
            r#"{"name":"cli","path_with_namespace":"o/cli","default_branch":"main","web_url":"u"}"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "repo", "view"], Reply::ok(json)));
        let repo = forge.repo_view().await.unwrap();
        // Absent visibility is *unknown* (`None`), not a false `Some(false)` — a
        // consumer must be able to tell "unknown" from a proven-public repo.
        assert_eq!(repo.private, None, "absent visibility must be unknown");
    }

    // GitLab's `iid` becomes the number and "opened" maps to Open.
    #[tokio::test]
    async fn gitlab_pr_list_maps_iid_and_state() {
        let json = r#"[{"iid":12,"title":"X","state":"opened","source_branch":"feat","target_branch":"main","web_url":"u","draft":true}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].number, 12);
        assert_eq!(prs[0].state, ForgePrState::Open);
        assert_eq!(prs[0].draft, Some(true));
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

    // The Gitea backend reports the five unmodelled ops as Unsupported, naming
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
            forge.pr_diff(1).await.unwrap_err(),
        ] {
            assert!(err.is_unsupported(), "{err:?}");
        }
        assert!(rec.calls().is_empty(), "unsupported ops must not spawn");
    }

    // T-049: the output budget set on the underlying client is INHERITED by the
    // `Forge` facade — a `pr_diff` whose output exceeds it is refused with a
    // `OutputTooLarge`-carrying error (actual + allowed sizes), never a truncated
    // diff; the facade's `pr_diff_within` overrides the ceiling per-call. Verified
    // on both GitHub (`gh pr diff`) and GitLab (`glab mr diff`).
    #[tokio::test]
    async fn pr_diff_inherits_client_budget_and_overrides_per_call() {
        let big = "diff --git a/m b/m\n".to_string() + &"+line\n".repeat(20_000);
        assert!(big.len() > 64 * 1024, "fixture must exceed the budget");

        // GitHub, budget inherited from the injected client.
        let gh =
            GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(&big)))
                .default_output_budget(OutputBudget::bytes(64 * 1024));
        let forge = Forge::from_github("/repo", gh);
        match forge.pr_diff(7).await {
            Err(Error::Forge(processkit::Error::OutputTooLarge {
                program,
                max_bytes,
                total_bytes,
                ..
            })) => {
                assert_eq!(program, "gh");
                assert_eq!(max_bytes, Some(64 * 1024));
                assert!(total_bytes > 64 * 1024, "actual exceeds allowed");
            }
            other => panic!("expected wrapped OutputTooLarge, got {other:?}"),
        }
        // The per-call override reads the same large PR in full.
        let files = forge
            .pr_diff_within(7, OutputBudget::unlimited())
            .await
            .expect("facade override reads the large diff");
        assert_eq!(files[0].path, std::path::Path::new("m"));

        // GitLab, same inheritance through `glab mr diff`.
        let glab =
            GitLab::with_runner(ScriptedRunner::new().on(["glab", "mr", "diff"], Reply::ok(&big)))
                .default_output_budget(OutputBudget::bytes(64 * 1024));
        let forge = Forge::from_gitlab("/repo", glab);
        assert!(matches!(
            forge.pr_diff(4).await,
            Err(Error::Forge(processkit::Error::OutputTooLarge { .. }))
        ));
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
            forge.pr_merge(1, PrMerge::merge()).await.unwrap_err(),
            forge.pr_mark_ready(1).await.unwrap_err(),
            forge.pr_close(PrClose::new(1)).await.unwrap_err(),
            forge.pr_checkout(1).await.unwrap_err(),
            forge.pr_checks(1).await.unwrap_err(),
            forge.pr_diff(1).await.unwrap_err(),
            forge.issue_list().await.unwrap_err(),
            forge.issue_view(1).await.unwrap_err(),
            forge
                .issue_create(IssueCreate::new("T", "B"))
                .await
                .unwrap_err(),
            forge.release_list().await.unwrap_err(),
            forge.release_view("v1").await.unwrap_err(),
            forge
                .release_create(ReleaseCreate::new("v1"))
                .await
                .unwrap_err(),
            forge.release_delete("v1").await.unwrap_err(),
            forge.pr_comment(1, "x").await.unwrap_err(),
            forge
                .pr_edit(1, PrEdit::new().title("T"))
                .await
                .unwrap_err(),
            forge.pr_approve(1).await.unwrap_err(),
            forge.pr_request_changes(1, "please fix").await.unwrap_err(),
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

    // The capability map for an authed GitHub on a modern `gh` is everything-true
    // (post-fork). `capabilities()` now probes `gh --version` too, so script a
    // modern banner (above the 2.0 floor) alongside the auth probe.
    #[tokio::test]
    async fn github_capabilities_authed_lights_everything() {
        let forge = github(
            ScriptedRunner::new()
                .on(
                    ["gh", "--version"],
                    Reply::ok("gh version 2.40.1 (2024-01-05)\n"),
                )
                .on(["gh", "auth"], Reply::ok("")),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(caps.pr_create);
        assert!(caps.pr_comment);
        assert!(caps.pr_edit);
        assert!(caps.pr_checks);
        assert!(caps.pr_merge);
        assert!(caps.pr_approve);
        assert!(caps.pr_request_changes, "gh has a request-changes review");
        assert!(caps.issue_create);
        assert!(caps.issue_close, "gh ships `issue close`");
        assert!(caps.issue_reopen, "gh ships `issue reopen`");
        assert!(caps.issue_comment, "gh ships `issue comment`");
        assert!(caps.release_create, "gh ships `release create`");
        assert!(caps.release_delete, "gh ships `release delete`");
        assert!(caps.authed);
        // The version probe fills a confirmed version and clears the floor.
        assert!(caps.supported, "modern gh meets the 2.0 floor");
        assert_eq!(
            caps.version,
            Some(Version {
                major: 2,
                minor: 40,
                patch: 1
            })
        );
    }

    // An unauthed GitHub keeps the static map's "ships the op" shape but flips
    // every op-specific flag to false (the intersection with `authed: false`
    // from spec §3). The `auth status` call exits non-zero ⇒ `auth_status()`
    // returns `false` (per the wrapper's documented exit-code reflection) and
    // the capability table zeros the ops. The version is modern, so `supported`
    // stays true — auth, not version, is what zeroed the ops here.
    #[tokio::test]
    async fn github_capabilities_unauthed_zeros_ops_but_keeps_authed_false() {
        let forge = github(
            ScriptedRunner::new()
                .on(["gh", "--version"], Reply::ok("gh version 2.40.1\n"))
                .on(["gh", "auth"], Reply::fail(1, "no")),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(!caps.authed, "unauthed");
        assert!(caps.supported, "modern gh is still supported");
        assert!(!caps.pr_create);
        assert!(!caps.pr_comment);
        assert!(!caps.pr_edit);
        assert!(!caps.pr_checks);
        assert!(!caps.pr_merge);
        assert!(!caps.issue_create);
        assert!(!caps.issue_close);
        assert!(!caps.issue_reopen);
        assert!(!caps.issue_comment);
    }

    // A `gh` **below the version floor** zeroes the op flags exactly like an
    // unauthed CLI — even when authenticated — so the map never advertises a
    // command an old binary can't run. `authed`/`version` still report the truth.
    #[tokio::test]
    async fn github_capabilities_old_version_zeros_ops_even_when_authed() {
        let forge = github(
            ScriptedRunner::new()
                .on(
                    ["gh", "--version"],
                    Reply::ok("gh version 1.14.0 (2021-11-02)\n"),
                )
                .on(["gh", "auth"], Reply::ok("")),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(
            caps.authed,
            "authed, but the old gh still can't run the ops"
        );
        assert!(!caps.supported, "gh 1.14 is below the 2.0 floor");
        assert_eq!(
            caps.version,
            Some(Version {
                major: 1,
                minor: 14,
                patch: 0
            })
        );
        assert!(!caps.pr_create);
        assert!(!caps.pr_comment);
        assert!(!caps.pr_edit);
        assert!(!caps.pr_checks);
        assert!(!caps.pr_merge);
        assert!(!caps.issue_create);
    }

    // An unrecognisable `gh --version` banner degrades to `version: None` /
    // `supported: false` (conservatively unavailable) rather than failing the whole
    // probe — the ops are zeroed, but `authed` is still reported.
    #[tokio::test]
    async fn github_capabilities_unrecognizable_version_degrades_to_unsupported() {
        let forge = github(
            ScriptedRunner::new()
                .on(["gh", "--version"], Reply::ok("gh version unknowable\n"))
                .on(["gh", "auth"], Reply::ok("")),
        );
        let caps = forge.capabilities().await.unwrap();
        assert_eq!(
            caps.version, None,
            "unrecognisable banner → no known version"
        );
        assert!(!caps.supported, "can't confirm the floor → unsupported");
        assert!(caps.authed, "auth is still probed and reported");
        assert!(!caps.pr_create && !caps.issue_create, "ops zeroed");
    }

    // Gitea's static map is the intersection of its CLI: `pr_checks` is the
    // only false when authed on a modern `tea` (no `tea` checks command).
    // Everything else is `true` post-fork. `capabilities()` probes `tea --version`
    // too, so script a modern banner above the 0.9 floor.
    #[tokio::test]
    async fn gitea_capabilities_authed_has_only_pr_checks_false() {
        // Gitea's auth probe parses `tea login list --output json` on a zero
        // exit and reports authed = (the array is non-empty). Script a non-empty
        // array so the probe reports authed; `[]` would read as not-authed.
        let forge = gitea(
            ScriptedRunner::new()
                .on(["tea", "--version"], Reply::ok("tea version 0.9.2\n"))
                .on(["tea", "login", "list"], Reply::ok(r#"[{"name":"a"}]"#)),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(caps.authed, "gitea authed");
        assert!(caps.supported, "modern tea meets the 0.9 floor");
        assert_eq!(
            caps.version,
            Some(Version {
                major: 0,
                minor: 9,
                patch: 2
            })
        );
        assert!(!caps.pr_checks, "gitea has no checks command");
        assert!(caps.pr_create);
        assert!(caps.pr_comment);
        assert!(caps.pr_edit);
        assert!(caps.pr_merge);
        assert!(caps.pr_approve, "tea ships `pr approve`");
        assert!(caps.pr_request_changes, "tea ships `pr reject`");
        assert!(caps.issue_create);
        assert!(caps.release_create, "tea ships `releases create`");
        assert!(caps.release_delete, "tea ships `releases delete`");
    }

    // GitLab's static map lights every action EXCEPT `pr_request_changes` when
    // authed on a modern `glab` — GitLab's review model is approve/revoke, with no
    // request-changes action. `pr_approve` is available (`glab mr approve`).
    #[tokio::test]
    async fn gitlab_capabilities_authed_has_only_request_changes_false() {
        let forge = gitlab(
            ScriptedRunner::new()
                .on(["glab", "--version"], Reply::ok("glab 1.36.0\n"))
                .on(["glab", "auth"], Reply::ok("")),
        );
        let caps = forge.capabilities().await.unwrap();
        assert!(caps.authed, "gitlab authed");
        assert!(caps.supported, "modern glab meets the 1.25 floor");
        assert!(caps.pr_create);
        assert!(caps.pr_comment);
        assert!(caps.pr_edit);
        assert!(caps.pr_checks);
        assert!(caps.pr_merge);
        assert!(caps.pr_approve, "glab ships `mr approve`");
        assert!(
            !caps.pr_request_changes,
            "GitLab has no request-changes review action"
        );
        assert!(caps.issue_create);
        assert!(
            caps.release_create,
            "glab ships `release create` (the draft/prerelease options are a separate gap)"
        );
        assert!(caps.release_delete, "glab ships `release delete`");
    }

    // `supports` must agree exactly with the runtime `Unsupported` behaviour
    // above: Gitea reports `false` for its unsupported ops but `true` for
    // `pr_checkout` (which `tea` does ship); GitHub and GitLab report `true` for
    // all of them — a pure, no-spawn capability check.
    #[test]
    fn supports_matches_unsupported_ops() {
        let gitea = Forge::from_gitea("/repo", Gitea::with_runner(ScriptedRunner::new()));
        for &op in ForgeOp::ALL {
            // Gitea ships `tea pr checkout`, `pr approve`, `pr reject`
            // (request-changes), both `releases create`/`releases delete`, and the
            // three issue-lifecycle ops (`issues close`/`issues reopen`/`comment`);
            // the other varying ops are Unsupported.
            let expected = matches!(
                op,
                ForgeOp::PrCheckout
                    | ForgeOp::PrApprove
                    | ForgeOp::PrRequestChanges
                    | ForgeOp::ReleaseCreate
                    | ForgeOp::ReleaseDelete
                    | ForgeOp::IssueClose
                    | ForgeOp::IssueReopen
                    | ForgeOp::IssueComment
            );
            assert_eq!(gitea.supports(op), expected, "gitea supports({op:?})");
        }
        // An Unknown backend supports nothing — every op returns Unsupported, so
        // `supports` must be `false` for all of them (matches `capabilities()`).
        let unknown: Forge = Forge::from_unknown("/repo");
        for &op in ForgeOp::ALL {
            assert!(!unknown.supports(op), "unknown should not support {op:?}");
        }
        // GitHub supports every op.
        let github = Forge::from_github("/repo", GitHub::with_runner(ScriptedRunner::new()));
        for &op in ForgeOp::ALL {
            assert!(github.supports(op), "github should support {op:?}");
        }
        // GitLab supports every op EXCEPT request-changes (its review model is
        // approve/revoke) — the one op that varies for GitLab.
        let gitlab = Forge::from_gitlab("/repo", GitLab::with_runner(ScriptedRunner::new()));
        for &op in ForgeOp::ALL {
            let expected = op != ForgeOp::PrRequestChanges;
            assert_eq!(gitlab.supports(op), expected, "gitlab supports({op:?})");
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
        assert_eq!(rels[0].url, None, "gh release_list does not fetch url");
        // GitHub confirms draft/prerelease (`Some`) on both list and view.
        assert_eq!(rels[0].prerelease, Some(true));
        assert_eq!(rels[0].draft, Some(false));
        assert_eq!(rels[1].published_at, None);
        assert_eq!(rels[1].draft, Some(true));
        assert_eq!(rels[1].prerelease, Some(false));

        let json = r#"[{"tag_name":"v1","name":"One","released_at":"2026-01-01T00:00:00Z","description":"gl notes","_links":{"self":"u"}}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "release", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].url.as_deref(), Some("u"));
        assert!(rels[0].published_at.is_some());
        assert_eq!(rels[0].body.as_deref(), Some("gl notes"));
        // GitLab has no draft/pre-release concept — *unknown* (`None`), not false.
        assert_eq!(rels[0].draft, None);
        assert_eq!(rels[0].prerelease, None);

        // tea's release table: `toSnakeCase`d string keys (`tag-_name`,
        // `published _at`), no release-page URL column.
        let json = r#"[{"tag-_name":"v1","title":"One","status":"prerelease","published _at":"2026-01-01T00:00:00Z"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "releases", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].tag, "v1");
        assert_eq!(rels[0].title, "One");
        assert_eq!(rels[0].url, None, "tea exposes no release-page URL");
        assert!(rels[0].published_at.is_some());
        assert_eq!(rels[0].body, None, "tea has no release body");
        // tea *does* report draft/prerelease (from its Status column) — confirmed.
        assert_eq!(rels[0].prerelease, Some(true), "tea status 'prerelease'");
        assert_eq!(rels[0].draft, Some(false));
    }

    // The per-field support contract, per backend, across list and view: a backend
    // that can't report a field yields `None` (unknown); one that can yields `Some`
    // (confirmed — including a confirmed empty `Some(vec![])`), never a false
    // `Some(false)`/empty list. This is the core of T-034.
    #[tokio::test]
    async fn support_contract_unknown_vs_confirmed_per_backend() {
        // Gitea PR: draft/labels/assignees are all unknown (tea has no such
        // columns) — on both the list and the paged view path (same mapper).
        let json =
            r#"[{"index":"3","title":"T","state":"open","head":"f","base":"main","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].draft, None, "tea PR draft is unknown");
        assert_eq!(prs[0].labels, None, "tea PR labels are unknown");
        assert_eq!(prs[0].assignees, None, "tea PR assignees are unknown");
        let forge = gitea(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let pr = forge.pr_view(3).await.unwrap();
        assert_eq!((pr.draft, pr.labels, pr.assignees), (None, None, None));

        // Gitea issue: labels/assignees unknown on the list path.
        let list = r#"[{"index":"5","title":"I","state":"open","body":"b","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "issues", "list"], Reply::ok(list)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].labels, None);
        assert_eq!(issues[0].assignees, None);

        // GitHub issue view: labels/assignees are confirmed `Some(..)`.
        let json = r#"{"number":3,"title":"Docs","state":"OPEN","body":"b","url":"u",
            "labels":[{"name":"docs"}],"assignees":[{"login":"octocat"}]}"#;
        let forge = github(ScriptedRunner::new().on(["gh", "issue", "view"], Reply::ok(json)));
        let issue = forge.issue_view(3).await.unwrap();
        assert_eq!(issue.labels, Some(vec!["docs".to_string()]));
        assert_eq!(issue.assignees, Some(vec!["octocat".to_string()]));

        // GitHub PR with no labels is a *confirmed* empty `Some(vec![])`, never
        // `None` — "we asked and there are none" differs from "we couldn't ask".
        let json = r#"[{"number":1,"title":"X","state":"OPEN","isDraft":false,
            "headRefName":"h","baseRefName":"main","url":"u","labels":[],"assignees":[]}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(
            prs[0].draft,
            Some(false),
            "confirmed non-draft, not unknown"
        );
        assert_eq!(prs[0].labels, Some(Vec::new()), "confirmed no labels");
        assert_eq!(prs[0].assignees, Some(Vec::new()));

        // GitLab issue: labels/assignees are confirmed `Some(..)`.
        let json = r#"[{"iid":2,"title":"Y","state":"opened","description":"d","web_url":"u",
            "labels":["bug"],"assignees":[{"username":"steiza"}]}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "issue", "list"], Reply::ok(json)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].labels, Some(vec!["bug".to_string()]));
        assert_eq!(issues[0].assignees, Some(vec!["steiza".to_string()]));
    }

    // T-094: the deferred `author`/`created_at`/`updated_at`/`milestone` fields,
    // per backend — GitHub/GitLab confirm them (flattening nested author/milestone
    // objects, including the `null` cases), Gitea always reports them unknown.
    #[tokio::test]
    async fn author_timestamps_and_milestone_mapping_per_backend() {
        // GitHub PR: author/milestone flatten from nested objects; timestamps
        // pass through directly.
        let json = r#"[{"number":7,"title":"X","state":"OPEN","isDraft":false,
            "headRefName":"feat","baseRefName":"main","url":"u",
            "author":{"login":"octocat"},
            "createdAt":"2026-07-01T00:00:00Z","updatedAt":"2026-07-02T00:00:00Z",
            "milestone":{"title":"v1.0"}}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].author.as_deref(), Some("octocat"));
        assert_eq!(prs[0].created_at.as_deref(), Some("2026-07-01T00:00:00Z"));
        assert_eq!(prs[0].updated_at.as_deref(), Some("2026-07-02T00:00:00Z"));
        assert_eq!(prs[0].milestone.as_deref(), Some("v1.0"));

        // GitHub PR: a `null` author (deleted account) is a *confirmed* empty
        // string, and a `null` milestone (none attached) is `None` — neither is
        // the "backend can't report it" `None` Gitea uses below.
        let json = r#"[{"number":8,"title":"Y","state":"OPEN","isDraft":false,
            "headRefName":"f","baseRefName":"main","url":"u",
            "author":null,"milestone":null}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(
            prs[0].author.as_deref(),
            Some(""),
            "confirmed deleted account"
        );
        assert_eq!(prs[0].milestone, None, "no milestone attached");

        // GitLab MR: author/milestone flatten from nested objects too.
        let json = r#"[{"iid":12,"title":"X","state":"opened","source_branch":"feat",
            "target_branch":"main","web_url":"u","draft":false,
            "author":{"username":"steiza"},
            "created_at":"2026-07-01T00:00:00Z","updated_at":"2026-07-02T00:00:00Z",
            "milestone":{"title":"v2.0"}}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(prs[0].author.as_deref(), Some("steiza"));
        assert_eq!(prs[0].created_at.as_deref(), Some("2026-07-01T00:00:00Z"));
        assert_eq!(prs[0].updated_at.as_deref(), Some("2026-07-02T00:00:00Z"));
        assert_eq!(prs[0].milestone.as_deref(), Some("v2.0"));

        // GitLab MR: a `null` author (anonymised account) is a *confirmed*
        // empty string, and a `null` milestone (none attached) is `None` —
        // mirrors the GitHub PR null case above.
        let json = r#"[{"iid":13,"title":"Y","state":"opened","source_branch":"f",
            "target_branch":"main","web_url":"u","draft":false,
            "author":null,"milestone":null}]"#;
        let forge = gitlab(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(
            prs[0].author.as_deref(),
            Some(""),
            "confirmed anonymised account"
        );
        assert_eq!(prs[0].milestone, None, "no milestone attached");

        // GitHub issue: same flatten/null contract as the PR mapper.
        let json = r#"{"number":3,"title":"Docs","state":"OPEN","body":"b","url":"u",
            "author":{"login":"andyfeller"},
            "createdAt":"2026-07-01T00:00:00Z","updatedAt":"2026-07-02T00:00:00Z",
            "milestone":{"title":"v1.0"}}"#;
        let forge = github(ScriptedRunner::new().on(["gh", "issue", "view"], Reply::ok(json)));
        let issue = forge.issue_view(3).await.unwrap();
        assert_eq!(issue.author.as_deref(), Some("andyfeller"));
        assert_eq!(issue.milestone.as_deref(), Some("v1.0"));

        // GitHub release: author flattens from the nested object.
        let json = r#"[{"tagName":"v1","name":"One","isLatest":true,"isDraft":false,
            "isPrerelease":false,"publishedAt":"2026-07-01T00:00:00Z",
            "author":{"login":"octocat"}}]"#;
        let forge = github(ScriptedRunner::new().on(["gh", "release", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].author.as_deref(), Some("octocat"));

        // Gitea PR/issue/release: author/timestamps/milestone are all unknown
        // (`None`) — `tea` has no such columns.
        let json =
            r#"[{"index":"3","title":"T","state":"open","head":"f","base":"main","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
        let prs = forge.pr_list().await.unwrap();
        assert_eq!(
            (
                prs[0].author.clone(),
                prs[0].created_at.clone(),
                prs[0].updated_at.clone(),
                prs[0].milestone.clone()
            ),
            (None, None, None, None)
        );

        let json = r#"[{"index":"5","title":"I","state":"open","body":"b","url":"u"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "issues", "list"], Reply::ok(json)));
        let issues = forge.issue_list().await.unwrap();
        assert_eq!(issues[0].author, None);
        assert_eq!(issues[0].milestone, None);

        let json = r#"[{"tag-_name":"v1","title":"One","status":"released",
            "published _at":"2026-07-01T00:00:00Z"}]"#;
        let forge = gitea(ScriptedRunner::new().on(["tea", "releases", "list"], Reply::ok(json)));
        let rels = forge.release_list().await.unwrap();
        assert_eq!(rels[0].author, None, "tea releases have no author column");
    }

    // A GitHub release *view* fills url/body as `Some`, while the lean list leaves
    // them `None` (asserted in `release_list_maps_published_at_per_backend`) — the
    // list-vs-view distinction the raw `Option` now encodes honestly.
    #[tokio::test]
    async fn github_release_view_fills_url_and_body_some() {
        let json = r#"{"tagName":"v1","name":"One","body":"notes","url":"https://gh/r/v1",
            "publishedAt":"2026-01-01T00:00:00Z","isDraft":false,"isPrerelease":true}"#;
        let forge = github(ScriptedRunner::new().on(["gh", "release", "view"], Reply::ok(json)));
        let rel = forge.release_view("v1").await.unwrap();
        assert_eq!(rel.url.as_deref(), Some("https://gh/r/v1"));
        assert_eq!(rel.body.as_deref(), Some("notes"));
        assert_eq!(rel.draft, Some(false));
        assert_eq!(rel.prerelease, Some(true));
    }

    // `release_create` maps the unified spec onto each CLI's own create verb: gh
    // `release create <tag> --title --notes --draft --prerelease`, glab `release
    // create <tag> --name --notes`, tea `releases create --tag --title --note
    // --draft --prerelease`. The title lands under gh/tea `--title` but glab `--name`,
    // and tea's notes flag is the singular `--note`.
    #[tokio::test]
    async fn release_create_dispatches_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok("https://gh/r/v1\n"));
        let out = Forge::from_github("/repo", GitHub::with_runner(&rec))
            .release_create(
                ReleaseCreate::new("v1")
                    .title("One")
                    .notes("N")
                    .draft()
                    .prerelease(),
            )
            .await
            .unwrap();
        assert_eq!(out, "https://gh/r/v1");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "release",
                "create",
                "v1",
                "--title",
                "One",
                "--notes",
                "N",
                "--draft",
                "--prerelease"
            ]
        );

        // GitLab: title → `--name`; no draft/prerelease requested here.
        let rec = RecordingRunner::replying(Reply::ok("https://gl/-/releases/v1\n"));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .release_create(ReleaseCreate::new("v1").title("One").notes("N"))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "create", "v1", "--name", "One", "--notes", "N"]
        );

        // Gitea: tag is a `--tag` flag; notes flag is the singular `--note`.
        let rec = RecordingRunner::replying(Reply::ok("created\n"));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .release_create(ReleaseCreate::new("v1").title("One").notes("N").draft())
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "releases", "create", "--tag", "v1", "--title", "One", "--note", "N", "--draft"
            ]
        );
    }

    // GitLab has no draft/pre-release concept, so requesting either through the
    // facade is Unsupported (and spawns nothing); GitHub/Gitea accept both.
    #[tokio::test]
    async fn release_create_draft_prerelease_unsupported_on_gitlab_only() {
        for spec in [
            ReleaseCreate::new("v1").draft(),
            ReleaseCreate::new("v1").prerelease(),
        ] {
            let rec = RecordingRunner::replying(Reply::ok(""));
            let err = Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
                .release_create(spec)
                .await
                .unwrap_err();
            assert!(
                err.is_unsupported(),
                "expected Unsupported on GitLab, got {err:?}"
            );
            assert!(
                rec.calls().is_empty(),
                "an unsupported option must not spawn"
            );
        }

        // The same draft/prerelease spec is accepted on GitHub and Gitea.
        let rec = RecordingRunner::replying(Reply::ok("ok"));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .release_create(ReleaseCreate::new("v1").draft().prerelease())
            .await
            .expect("github accepts draft/prerelease");
        let rec = RecordingRunner::replying(Reply::ok("ok"));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .release_create(ReleaseCreate::new("v1").draft().prerelease())
            .await
            .expect("gitea accepts draft/prerelease");
    }

    // `release_delete` maps to each CLI's own delete verb: gh/glab `release delete
    // <tag> --yes` (--yes skips the confirm prompt), tea `releases delete <tag>`
    // (no confirm flag, matching tea's other mutators).
    #[tokio::test]
    async fn release_delete_dispatches_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .release_delete("v1")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "delete", "v1", "--yes"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .release_delete("v1")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["release", "delete", "v1", "--yes"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .release_delete("v1")
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["releases", "delete", "v1"]);
    }

    // `supports` reports the two new release mutators available on every real
    // backend and absent on an Unknown handle (matching the ForgeOp::ALL contract).
    #[tokio::test]
    async fn supports_reports_release_mutators_per_backend() {
        for op in [ForgeOp::ReleaseCreate, ForgeOp::ReleaseDelete] {
            assert!(github(ScriptedRunner::new()).supports(op), "github {op:?}");
            assert!(gitlab(ScriptedRunner::new()).supports(op), "gitlab {op:?}");
            assert!(gitea(ScriptedRunner::new()).supports(op), "gitea {op:?}");
            assert!(
                !Forge::<ScriptedRunner>::from_unknown("/repo").supports(op),
                "unknown {op:?}"
            );
        }
    }

    // The unified PrMerge spec maps its strategy to each CLI's own flag.
    #[tokio::test]
    async fn pr_merge_maps_strategy_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_merge(5, PrMerge::squash())
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "merge", "5", "--squash"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .pr_merge(5, PrMerge::rebase())
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
            .pr_merge(5, PrMerge::merge())
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "merge", "5", "--style", "merge"]
        );
    }

    // `auto`/`delete_branch` are GitHub-only. On GitHub they map to gh's own
    // `--auto`/`--delete-branch`; on GitLab/Gitea the facade surfaces a structured
    // `Unsupported` (bubbled from the wrapper) rather than silently merging without
    // them — so `is_unsupported()` is true and no wrong merge argv is emitted.
    #[tokio::test]
    async fn pr_merge_options_map_on_github_and_are_unsupported_elsewhere() {
        // GitHub expresses both options as real flags.
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_merge(5, PrMerge::squash().auto().delete_branch())
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "merge", "5", "--squash", "--auto", "--delete-branch"]
        );

        // GitLab / Gitea: an `auto` or `delete_branch` request is Unsupported and
        // spawns nothing (the runner has no rule, so a leak-through would error
        // differently than the classified `Unsupported`).
        for (make, merge) in [
            (
                Forge::from_gitlab("/repo", GitLab::with_runner(ScriptedRunner::new())),
                PrMerge::merge().auto(),
            ),
            (
                Forge::from_gitea("/repo", Gitea::with_runner(ScriptedRunner::new())),
                PrMerge::squash().delete_branch(),
            ),
        ] {
            let err = make.pr_merge(5, merge).await.unwrap_err();
            assert!(err.is_unsupported(), "expected Unsupported, got {err:?}");
        }
    }

    // `pr_checkout` dispatches to each CLI's own checkout verb (gh/tea `pr
    // checkout`, glab `mr checkout`) — supported on all three real backends.
    #[tokio::test]
    async fn pr_checkout_dispatches_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_checkout(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "checkout", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .pr_checkout(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["mr", "checkout", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .pr_checkout(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "checkout", "7"]);
    }

    // `pr_approve` dispatches to each CLI's own approving-review verb: gh `pr review
    // --approve`, glab `mr approve`, tea `pr approve` — supported on all three real
    // backends.
    #[tokio::test]
    async fn pr_approve_dispatches_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_approve(7)
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "review", "7", "--approve"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .pr_approve(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["mr", "approve", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .pr_approve(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["pr", "approve", "7"]);
    }

    // `pr_request_changes` maps to gh `pr review --request-changes --body <body>`
    // (the body rides in a flag-VALUE slot) and tea `pr reject <n> <reason>` (the
    // reason a bare positional). On GitLab it is Unsupported — no request-changes
    // review action — and nothing spawns (the runner has no rule).
    #[tokio::test]
    async fn pr_request_changes_dispatches_and_is_unsupported_on_gitlab() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .pr_request_changes(7, "please fix")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "pr",
                "review",
                "7",
                "--request-changes",
                "--body",
                "please fix"
            ]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .pr_request_changes(7, "please fix")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["pr", "reject", "7", "please fix"]
        );

        // GitLab: Unsupported, without spawning (the runner has no rule, so a
        // leak-through would error differently than the classified Unsupported).
        let err = Forge::from_gitlab("/repo", GitLab::with_runner(ScriptedRunner::new()))
            .pr_request_changes(7, "please fix")
            .await
            .unwrap_err();
        assert!(err.is_unsupported(), "expected Unsupported, got {err:?}");
    }

    // `pr_request_changes` rejects an empty / whitespace-only body with InvalidInput
    // BEFORE any spawn — a request-changes review needs a reason on every backend.
    #[tokio::test]
    async fn pr_request_changes_empty_body_is_invalid_input() {
        let forge = github(ScriptedRunner::new()); // no scripted rules: a spawn would error
        for body in ["", "   ", "\t\n"] {
            let err = forge.pr_request_changes(7, body).await.unwrap_err();
            assert!(
                matches!(err, crate::Error::InvalidInput(_)),
                "empty body {body:?} must surface as InvalidInput, got {err:?}"
            );
        }
    }

    // `issue_close` / `issue_reopen` dispatch to each backend's state-change verb:
    // gh `issue close`/`issue reopen`, glab `issue close`/`issue reopen`, tea
    // `issues close`/`issues reopen`. These are live mutations, so the hermetic argv
    // pin is the contract.
    #[tokio::test]
    async fn issue_close_and_reopen_dispatch_per_backend() {
        // close
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .issue_close(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issue", "close", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .issue_close(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issue", "close", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .issue_close(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issues", "close", "7"]);

        // reopen
        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_github("/repo", GitHub::with_runner(&rec))
            .issue_reopen(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issue", "reopen", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .issue_reopen(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issue", "reopen", "7"]);

        let rec = RecordingRunner::replying(Reply::ok(""));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .issue_reopen(7)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["issues", "reopen", "7"]);
    }

    // `issue_comment` maps to gh `issue comment --body` / glab `issue note -m` /
    // tea `comment <index> <body>`, and returns the CLI's trimmed output.
    #[tokio::test]
    async fn issue_comment_dispatches_per_backend() {
        let rec = RecordingRunner::replying(Reply::ok("https://gh/i/7#c1\n"));
        let out = Forge::from_github("/repo", GitHub::with_runner(&rec))
            .issue_comment(7, "ping")
            .await
            .unwrap();
        assert_eq!(out, "https://gh/i/7#c1");
        assert_eq!(
            rec.only_call().args_str(),
            ["issue", "comment", "7", "--body", "ping"]
        );

        let rec = RecordingRunner::replying(Reply::ok("https://gl/i/7#note_5\n"));
        Forge::from_gitlab("/repo", GitLab::with_runner(&rec))
            .issue_comment(7, "ping")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["issue", "note", "7", "-m", "ping"]
        );

        let rec = RecordingRunner::replying(Reply::ok("Comment created\n"));
        Forge::from_gitea("/repo", Gitea::with_runner(&rec))
            .issue_comment(7, "ping")
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["comment", "7", "ping"]);
    }

    // `issue_comment` rejects an empty / whitespace-only body with InvalidInput
    // BEFORE any spawn — uniform with `pr_comment` (a blank comment is a caller bug).
    #[tokio::test]
    async fn issue_comment_empty_body_is_invalid_input() {
        let forge = github(ScriptedRunner::new()); // no scripted rules: a spawn would error
        for body in ["", "   ", "\t\n"] {
            let err = forge.issue_comment(7, body).await.unwrap_err();
            assert!(
                matches!(err, crate::Error::InvalidInput(_)),
                "empty body {body:?} must surface as InvalidInput, got {err:?}"
            );
        }
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

    // `pr_diff` dispatches to `gh pr diff`/`glab mr diff` and parses the same
    // git-format output through the shared `vcs-diff` parser — a plain forward,
    // no facade-specific mapping.
    #[tokio::test]
    async fn pr_diff_dispatches_and_parses_per_backend() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";

        let forge = github(ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(out)));
        let files = forge.pr_diff(1).await.expect("github pr_diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("m"));
        assert_eq!(files[0].change, ChangeKind::Modified);

        let forge = gitlab(ScriptedRunner::new().on(["glab", "mr", "diff"], Reply::ok(out)));
        let files = forge.pr_diff(1).await.expect("gitlab pr_diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("m"));
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
