#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-core` — write code against "the repository" without caring whether it's
//! git or jj.
//!
//! You hold one handle, [`Repo`], that auto-detects whether a directory is a git or
//! a jj checkout and runs whatever operations *both* tools support — handing back
//! plain result types ([`RepoSnapshot`], [`FileChange`], [`MergeProbe`], …) that
//! don't mention the backend (whether the repo is git or jj). Async, structured
//! errors, and every subprocess
//! inherits the underlying client's OS-**job** containment (an OS-level container
//! that kills the whole process tree if your program exits, via [`processkit`]) so
//! no `git`/`jj` tree is orphaned.
//!
//! # What you can do
//!
//! From one [`Repo`] handle: read the current branch and a batched status
//! [`snapshot`](Repo::snapshot) · list & diff changed files · commit paths · fetch
//! / push / checkout / rebase · probe a merge for conflicts
//! ([`try_merge`](Repo::try_merge)) · drive in-progress merge/rebase state · manage
//! worktrees. Open one and read a prompt line:
//!
//! ```no_run
//! use vcs_core::Repo;
//! # async fn demo() -> vcs_core::Result<()> {
//! let repo = Repo::open(".")?;            // detects git vs jj
//! let s = repo.snapshot().await?;         // one or two spawns, not a call per field
//! let branch = s.branch.as_deref().unwrap_or("(detached)");
//! println!("{branch} {}", if s.dirty { "*" } else { "" });
//! # Ok(()) }
//! ```
//!
//! **It's a thin common layer, not a god-object.** The shared surface carries only
//! what unifies *without lying*; the few operations the two tools model too
//! differently (a full `merge`, jj's `op restore`, range/revset queries) stay on
//! the raw `git`/`jj` handle rather than being faked (see
//! [below](#whats-deliberately-not-unified)). Reach for the unified handle when code
//! must work on both backends; drop to the raw client when you need power only one
//! of them offers.
//!
//! # Mental model (engineering reference)
//!
//! The surface is three layers, narrowing from "which tool is this?" to "do the
//! thing":
//!
//! - **[`detect`]** — walk up from a directory to the filesystem root for a
//!   `.git`/`.jj` repo (jj wins when colocated — it's the tool driving the working
//!   copy). Pure filesystem probing, no subprocess; yields a [`Located`]
//!   ([`BackendKind`] + worktree root).
//! - **[`Repo`]** — the cwd-bound facade handle, the thing you hold. Open one with
//!   [`Repo::open`] (real job-backed runner) or build it over an explicit client
//!   with [`Repo::from_git`] / [`Repo::from_jj`] (the test seam). Re-anchor it to
//!   another directory cheaply with [`Repo::at`] — the backend is shared behind an
//!   `Arc`, so threading work across worktrees never re-detects or rebuilds the
//!   client. Inspect it with [`kind`](Repo::kind) / [`root`](Repo::root) /
//!   [`cwd`](Repo::cwd).
//! - **[`VcsRepo`]** — the same common surface as an object-safe trait, so a
//!   consumer can hold a `Box<dyn VcsRepo>` / `&dyn VcsRepo` without naming the
//!   [`ProcessRunner`] generic. Every method mirrors the like-named inherent method
//!   on [`Repo`]; it adds nothing but the abstraction boundary.
//!
//! ## The common operations
//!
//! All on [`Repo`] (and [`VcsRepo`]), dir-free, dispatched per backend:
//!
//! - **Refs** — [`current_branch`](Repo::current_branch),
//!   [`trunk`](Repo::trunk), [`local_branches`](Repo::local_branches),
//!   [`branch_exists`](Repo::branch_exists),
//!   [`delete_branch`](Repo::delete_branch),
//!   [`rename_branch`](Repo::rename_branch) (branch on git, bookmark on jj).
//! - **Status** — [`changed_files`](Repo::changed_files),
//!   [`diff_stat`](Repo::diff_stat),
//!   [`has_uncommitted_changes`](Repo::has_uncommitted_changes),
//!   [`has_tracked_changes`](Repo::has_tracked_changes),
//!   [`conflicted_files`](Repo::conflicted_files), and
//!   [`snapshot`](Repo::snapshot) — a **batched** prompt/status-bar read of the
//!   lot in one or two spawns.
//! - **Mutations** — [`commit_paths`](Repo::commit_paths) (partial commit),
//!   [`fetch`](Repo::fetch) / [`fetch_from`](Repo::fetch_from) /
//!   [`fetch_remote_branch`](Repo::fetch_remote_branch) /
//!   [`push`](Repo::push), [`checkout`](Repo::checkout),
//!   [`rebase`](Repo::rebase).
//! - **Merge & operation state** — [`try_merge`](Repo::try_merge) (a
//!   trace-free conflict probe → [`MergeProbe`]),
//!   [`in_progress_state`](Repo::in_progress_state) /
//!   [`abort_in_progress`](Repo::abort_in_progress) /
//!   [`continue_in_progress`](Repo::continue_in_progress) → [`OperationState`].
//! - **Worktrees / workspaces** — [`list_worktrees`](Repo::list_worktrees),
//!   [`create_worktree`](Repo::create_worktree),
//!   [`remove_worktree`](Repo::remove_worktree), and the **synchronous**
//!   [`cleanup_worktree_blocking`](Repo::cleanup_worktree_blocking) for a `Drop`
//!   guard that cannot `.await`.
//!
//! Because the backends genuinely diverge in places, several common methods carry
//! a documented asymmetry (e.g. `upstream`/`ahead`/`behind` are always `None` on
//! jj; [`diff_stat`](Repo::diff_stat) excludes untracked files on git but not jj;
//! [`in_progress_state`](Repo::in_progress_state) never returns `Conflict` on git).
//! The method docs spell each one out — the facade unifies the *shape*, not away
//! the truth.
//!
//! ## The escape hatches
//!
//! Tool-specific work reaches the underlying typed clients without adding
//! `vcs-git`/`vcs-jj` as separate dependencies (both are re-exported):
//! [`git_at`](Repo::git_at) / [`jj_at`](Repo::jj_at) hand out a cwd-bound view
//! ([`GitAt`] / [`JjAt`], `dir` dropped); the raw
//! [`git`](Repo::git) / [`jj`](Repo::jj) hand out a borrow of the client itself.
//! Each returns `None` for the other backend.
//!
//! ## What's deliberately *not* unified
//!
//! Three families stay off the common surface because no honest single shape
//! exists — reach them through the bound handles:
//!
//! - **Full `merge`** — jj composes `new` + `squash` + bookmark moves; git runs a
//!   single command. Only the *conflict probe* unifies, as
//!   [`try_merge`](Repo::try_merge).
//! - **Operation rollback** — jj's `op restore` has no faithful git analogue; use
//!   [`Jj::transaction`](vcs_jj::Jj::transaction) on the jj client.
//! - **Range / revset queries** — commit counts and diff stats over a range: git's
//!   `a..b` and jj's revsets aren't interchangeable, so neither is forced onto a
//!   shared signature.
//!
//! # Recipes
//!
//! Probe a merge for conflicts (trace-free), or spin up a worktree:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_core::{MergeProbe, Repo};
//! # async fn demo(repo: &Repo) -> vcs_core::Result<()> {
//! match repo.try_merge("feature").await? {
//!     MergeProbe::Clean            => println!("merges cleanly"),
//!     MergeProbe::Conflicts(paths) => println!("would conflict in {paths:?}"),
//!     _                            => {} // #[non_exhaustive]
//! }
//! let wt = repo.create_worktree(Path::new("/tmp/feat"), "feature", "main").await?;
//! # let _ = wt;
//! # Ok(()) }
//! ```
//!
//! # Testing
//!
//! There is **no mock feature** on the facade traits — the runner is the seam.
//! Build a [`Repo`] over a fake [`ProcessRunner`] with [`Repo::from_git`] /
//! [`Repo::from_jj`] (e.g. a [`ScriptedRunner`](processkit::ScriptedRunner)
//! replying to canned argv), so the *real* per-backend dispatch, argv-building and
//! parsing run against canned output — exactly what a mocked `VcsRepo` would skip.
//! The cross-cutting patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! ```no_run
//! use processkit::{Reply, ScriptedRunner};
//! use vcs_core::{vcs_git::Git, Repo};
//! # async fn demo() -> vcs_core::Result<()> {
//! let runner = ScriptedRunner::new().on(["status"], Reply::ok(" M a.rs\0"));
//! let repo = Repo::from_git("/repo", "/repo", Git::with_runner(runner));
//! assert!(repo.has_uncommitted_changes().await?);
//! # Ok(()) }
//! ```
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module, which walks every operation in depth
//! and hosts the cross-cutting sub-guides: a [`cookbook`](guide::cookbook) of
//! end-to-end flows, the [`process_model`](guide::process_model) (job containment,
//! errors, cancellation), [`positioning`](guide::positioning) (facade-vs-raw-client
//! and the three call shapes), and the [`stability`](guide::stability) contract.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use processkit::{JobRunner, ProcessRunner};
use vcs_git::{Git, GitAt};
use vcs_jj::{Jj, JjAt};

mod dto;
mod error;
mod git_backend;
mod jj_backend;

pub use dto::{
    BackendKind, ChangeKind, CreateOutcome, DiffStat, FileChange, MergeProbe, OperationState,
    RepoSnapshot, WorktreeInfo,
};
pub use error::{Error, Result};

// Re-export the underlying typed clients so a consumer depending only on
// `vcs-core` can still reach raw, tool-specific operations — and their types
// (`GitApi`, `JjApi`, `WorktreeAdd`, `JjFileset`, …) — without adding `vcs-git`
// / `vcs-jj` as separate dependencies. [`Repo::git`] / [`Repo::jj`] hand out
// borrows of these clients; the consumer decides, per call, whether to go
// through the facade or straight to the tool.
pub use vcs_git;
pub use vcs_jj;
// Re-exported under the `cancellation` feature so a `vcs-core`-only consumer can
// name the token for a `default_cancel_on` client (built via `Git`/`Jj`, then
// passed to `Repo::from_git`/`from_jj`) without a direct `processkit` dependency.
#[cfg(feature = "cancellation")]
#[cfg_attr(docsrs, doc(cfg(feature = "cancellation")))]
pub use processkit::CancellationToken;

/// The result of [`detect`]: which backend, and the repository root it was found
/// at.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Located {
    /// The detected backend.
    pub kind: BackendKind,
    /// The directory holding `.git`/`.jj` — the worktree root.
    pub root: PathBuf,
}

/// Walk up from `start` to the filesystem root looking for a repository. A `.jj`
/// directory wins over `.git` (colocated repos are driven through jj); `.git` may
/// be a directory or a gitlink file (a linked worktree/submodule). Pure
/// filesystem probing — no subprocess.
///
/// `start` is walked exactly as given via [`Path::parent`], so pass an **absolute**
/// path to search ancestors — a relative path like `"."` has no ancestor chain
/// and only its own directory is checked. ([`Repo::open`] absolutises for you.)
pub fn detect(start: &Path) -> Option<Located> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".jj").is_dir() {
            return Some(Located {
                kind: BackendKind::Jj,
                root: dir.to_path_buf(),
            });
        }
        if dir.join(".git").exists() {
            return Some(Located {
                kind: BackendKind::Git,
                root: dir.to_path_buf(),
            });
        }
        current = dir.parent();
    }
    None
}

/// The per-tool client behind a [`Repo`]. Shared via `Arc` so [`Repo::at`] can
/// re-anchor the cwd cheaply without rebuilding the client.
enum Backend<R: ProcessRunner> {
    Git(Arc<Git<R>>),
    Jj(Arc<Jj<R>>),
}

impl<R: ProcessRunner> Backend<R> {
    fn shared(&self) -> Self {
        match self {
            Backend::Git(g) => Backend::Git(Arc::clone(g)),
            Backend::Jj(j) => Backend::Jj(Arc::clone(j)),
        }
    }
}

/// A cwd-bound, backend-agnostic VCS handle. Operations run against the bound
/// directory ([`cwd`](Repo::cwd)); use [`at`](Repo::at) to get a sibling handle
/// bound elsewhere.
pub struct Repo<R: ProcessRunner = JobRunner> {
    root: PathBuf,
    cwd: PathBuf,
    backend: Backend<R>,
}

impl Repo<JobRunner> {
    /// Detect the repository at or above `dir` and open a handle bound to `dir`,
    /// using the real job-backed runner. Errors with
    /// [`Error::NotARepository`] when no `.git`/`.jj` is found.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        // Absolutise first: `detect` walks parents, and a relative path like "."
        // has no real ancestor chain (`Path::new(".").parent()` is `""`, then
        // `None`), so a relative input would never find a repo above the cwd.
        let dir = std::path::absolute(dir.as_ref())?;
        let located = detect(&dir).ok_or_else(|| Error::NotARepository(dir.clone()))?;
        let backend = match located.kind {
            BackendKind::Git => Backend::Git(Arc::new(Git::new())),
            BackendKind::Jj => Backend::Jj(Arc::new(Jj::new())),
        };
        Ok(Repo {
            root: located.root,
            cwd: dir,
            backend,
        })
    }
}

impl<R: ProcessRunner> Repo<R> {
    /// Build a git-backed handle from an explicit client — for a custom runner
    /// (e.g. a test seam) or a pre-configured [`Git`].
    pub fn from_git(root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Git<R>) -> Self {
        Repo {
            root: root.into(),
            cwd: cwd.into(),
            backend: Backend::Git(Arc::new(client)),
        }
    }

    /// Build a jj-backed handle from an explicit client.
    pub fn from_jj(root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Jj<R>) -> Self {
        Repo {
            root: root.into(),
            cwd: cwd.into(),
            backend: Backend::Jj(Arc::new(client)),
        }
    }

    /// Which backend drives this handle.
    pub fn kind(&self) -> BackendKind {
        match &self.backend {
            Backend::Git(_) => BackendKind::Git,
            Backend::Jj(_) => BackendKind::Jj,
        }
    }

    /// The repository root detected at open time.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory operations run against.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// A sibling handle bound to `dir`, sharing this handle's client and root.
    pub fn at(&self, dir: impl Into<PathBuf>) -> Self {
        Repo {
            root: self.root.clone(),
            cwd: dir.into(),
            backend: self.backend.shared(),
        }
    }

    /// The underlying [`Git`] client, or `None` when jj-backed — an escape hatch
    /// to git-only operations not on the common surface.
    pub fn git(&self) -> Option<&Git<R>> {
        match &self.backend {
            Backend::Git(g) => Some(g.as_ref()),
            Backend::Jj(_) => None,
        }
    }

    /// The underlying [`Jj`] client, or `None` when git-backed.
    pub fn jj(&self) -> Option<&Jj<R>> {
        match &self.backend {
            Backend::Jj(j) => Some(j.as_ref()),
            Backend::Git(_) => None,
        }
    }

    /// The git client bound to this handle's [`cwd`](Repo::cwd) — a [`GitAt`] whose
    /// methods omit the `dir` argument — or `None` when jj-backed. The dir-free
    /// counterpart of [`git`](Repo::git): `repo.git_at()?.merge_continue().await?`.
    ///
    /// The returned view borrows `self`. To work in another worktree, **bind the
    /// re-anchored handle first** (the view can't outlive a temporary
    /// [`at`](Repo::at)):
    ///
    /// ```no_run
    /// # async fn f(repo: vcs_core::Repo, wt: &std::path::Path) -> vcs_core::Result<()> {
    /// let wt = repo.at(wt);          // owns the re-anchored handle
    /// let git = wt.git_at().unwrap();
    /// git.fetch().await?;
    /// # Ok(()) }
    /// ```
    pub fn git_at(&self) -> Option<GitAt<'_, R>> {
        match &self.backend {
            Backend::Git(g) => Some(g.at(&self.cwd)),
            Backend::Jj(_) => None,
        }
    }

    /// The jj client bound to this handle's [`cwd`](Repo::cwd) — a [`JjAt`] whose
    /// methods omit the `dir` argument — or `None` when git-backed. The dir-free
    /// counterpart of [`jj`](Repo::jj). For another workspace, bind the re-anchored
    /// handle first (`let ws = repo.at(path); ws.jj_at()…`) — see [`git_at`](Repo::git_at).
    pub fn jj_at(&self) -> Option<JjAt<'_, R>> {
        match &self.backend {
            Backend::Jj(j) => Some(j.at(&self.cwd)),
            Backend::Git(_) => None,
        }
    }

    /// The current branch (git) or bookmark (jj); `None` when detached / no
    /// bookmark on the working copy.
    pub async fn current_branch(&self) -> Result<Option<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::current_branch(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::current_branch(j, &self.cwd).await,
        }
    }

    /// The trunk branch/bookmark. Resolution order: the backend's own notion
    /// (git's `origin/HEAD`, jj's `trunk()` revset), then a fallback to a local
    /// `main`, then `master`; `None` when none of those resolve.
    pub async fn trunk(&self) -> Result<Option<String>> {
        let native = match &self.backend {
            Backend::Git(g) => git_backend::trunk(g, &self.cwd).await?,
            Backend::Jj(j) => jj_backend::trunk(j, &self.cwd).await?,
        };
        if native.is_some() {
            return Ok(native);
        }
        for candidate in ["main", "master"] {
            if self.branch_exists(candidate).await? {
                return Ok(Some(candidate.to_string()));
            }
        }
        Ok(None)
    }

    /// Local branch (git) / bookmark (jj) names.
    pub async fn local_branches(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::local_branches(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::local_branches(j, &self.cwd).await,
        }
    }

    /// Whether a local branch/bookmark named `name` exists.
    pub async fn branch_exists(&self, name: &str) -> Result<bool> {
        match &self.backend {
            Backend::Git(g) => git_backend::branch_exists(g, &self.cwd, name).await,
            Backend::Jj(j) => jj_backend::branch_exists(j, &self.cwd, name).await,
        }
    }

    /// Whether the working copy has uncommitted changes (git: a non-empty
    /// `status`; jj: a non-empty working-copy change `@`).
    pub async fn has_uncommitted_changes(&self) -> Result<bool> {
        match &self.backend {
            Backend::Git(g) => git_backend::has_uncommitted_changes(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::has_uncommitted_changes(j, &self.cwd).await,
        }
    }

    /// Whether the working copy has uncommitted changes to *tracked* files.
    ///
    /// Backend nuance: git ignores untracked files here
    /// (`status --untracked-files=no`); jj auto-tracks new files, so there is no
    /// untracked concept and this equals
    /// [`has_uncommitted_changes`](Self::has_uncommitted_changes).
    pub async fn has_tracked_changes(&self) -> Result<bool> {
        match &self.backend {
            Backend::Git(g) => git_backend::has_tracked_changes(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::has_uncommitted_changes(j, &self.cwd).await,
        }
    }

    /// Paths with unresolved merge conflicts in the working copy, repo-relative
    /// with `/` separators (git `diff --diff-filter=U` / jj `resolve --list -r @`).
    /// Empty when there are none.
    pub async fn conflicted_files(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::conflicted_files(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::conflicted_files(j, &self.cwd).await,
        }
    }

    /// Delete a local branch (git) / bookmark (jj). `force` applies to git only
    /// (`branch -D` vs `-d`); jj has no force and ignores it.
    pub async fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::delete_branch(g, &self.cwd, name, force).await,
            Backend::Jj(j) => jj_backend::delete_branch(j, &self.cwd, name).await,
        }
    }

    /// Rename a local branch (git) / bookmark (jj).
    pub async fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::rename_branch(g, &self.cwd, old, new).await,
            Backend::Jj(j) => jj_backend::rename_branch(j, &self.cwd, old, new).await,
        }
    }

    /// The working-copy changes (git `status` / jj `diff -r @ --summary`).
    pub async fn changed_files(&self) -> Result<Vec<FileChange>> {
        match &self.backend {
            Backend::Git(g) => git_backend::changed_files(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::changed_files(j, &self.cwd).await,
        }
    }

    /// Aggregate insertion/deletion counts for the working copy.
    ///
    /// Backend nuance: git counts the working tree against `HEAD` (`git diff`,
    /// which **excludes untracked files**), while jj counts the `@` change against
    /// its parent (which **includes** newly-added files). So on git a brand-new
    /// file shows in [`changed_files`](Self::changed_files) but not here, whereas
    /// on jj it shows in both. On an unborn git repo (no commits yet) the count is
    /// taken against the empty tree, so a pre-first-commit working tree stats
    /// instead of erroring.
    pub async fn diff_stat(&self) -> Result<DiffStat> {
        match &self.backend {
            Backend::Git(g) => git_backend::diff_stat(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::diff_stat(j, &self.cwd).await,
        }
    }

    /// A batched [`RepoSnapshot`] of the common repo state — branch, upstream,
    /// ahead/behind, dirtiness, change count, and operation state — in **one or
    /// two** spawns instead of a call per field (git: `status --porcelain=v2
    /// --branch` + the in-progress probe; jj: one `log -r @` template + a change
    /// count). Built for prompt/status-bar/TUI refreshes. Note the asymmetry:
    /// `upstream`/`ahead`/`behind` are always `None` on jj.
    pub async fn snapshot(&self) -> Result<RepoSnapshot> {
        match &self.backend {
            Backend::Git(g) => git_backend::snapshot(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::snapshot(j, &self.cwd).await,
        }
    }

    /// Commit exactly `paths` with `message` (git `commit --only`, jj
    /// `commit <filesets>`). Paths are repo-relative. `paths` must be non-empty:
    /// an empty set is refused up front, because the backends would diverge
    /// dangerously — git errors out, while jj's `commit` with no filesets would
    /// silently commit the **entire** working copy.
    pub async fn commit_paths(&self, paths: &[String], message: &str) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "commit_paths requires at least one path: an empty set would error \
                 on git but commit the entire working copy on jj",
            )));
        }
        match &self.backend {
            Backend::Git(g) => git_backend::commit_paths(g, &self.cwd, paths, message).await,
            Backend::Jj(j) => jj_backend::commit_paths(j, &self.cwd, paths, message).await,
        }
    }

    /// Fetch from the default remote (git `fetch` / jj `git fetch`).
    pub async fn fetch(&self) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::fetch(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::fetch(j, &self.cwd).await,
        }
    }

    /// Fetch from a *named* remote (git `fetch <remote>` / jj
    /// `git fetch --remote <remote>`). Transient network failures are retried by
    /// the underlying client.
    pub async fn fetch_from(&self, remote: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::fetch_from(g, &self.cwd, remote).await,
            Backend::Jj(j) => jj_backend::fetch_from(j, &self.cwd, remote).await,
        }
    }

    /// Fetch a single branch/bookmark from `origin` into its remote-tracking ref
    /// (git `fetch_remote_branch` / jj `git fetch -b`). Transient network failures
    /// are retried by the underlying client.
    pub async fn fetch_remote_branch(&self, branch: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::fetch_remote_branch(g, &self.cwd, branch).await,
            Backend::Jj(j) => jj_backend::fetch_remote_branch(j, &self.cwd, branch).await,
        }
    }

    /// Push `branch` to `origin` (git `push -u origin <branch>` / jj
    /// `git push -b <branch>`).
    ///
    /// The branch (jj: bookmark) must already exist locally. The two backends
    /// honestly differ in what "push" means: git pushes the *ref* and records
    /// the upstream (`-u`; idempotent on repeat pushes), while jj pushes the
    /// *bookmark's state* — including deleting the remote branch if the
    /// bookmark was deleted locally. Renamed refspecs (`local:remote`) and
    /// non-`origin` remotes are git-only concepts; use the
    /// [`git()`](Repo::git) escape hatch ([`vcs_git::GitPush`]) for those.
    pub async fn push(&self, branch: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::push(g, &self.cwd, branch).await,
            Backend::Jj(j) => jj_backend::push(j, &self.cwd, branch).await,
        }
    }

    /// Switch the working copy to `reference` (git `checkout` / jj `edit`).
    pub async fn checkout(&self, reference: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::checkout(g, &self.cwd, reference).await,
            Backend::Jj(j) => jj_backend::checkout(j, &self.cwd, reference).await,
        }
    }

    /// Rebase the current work onto `onto` (git `rebase` / jj `rebase -d`). The
    /// `onto` is a branch/bookmark name or revision the backend understands.
    pub async fn rebase(&self, onto: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::rebase(g, &self.cwd, onto).await,
            Backend::Jj(j) => jj_backend::rebase(j, &self.cwd, onto).await,
        }
    }

    /// Probe whether merging `source` into the current work would conflict,
    /// **without leaving any trace**: the probe is rolled back before returning
    /// (git: `merge --no-commit --no-ff` then `merge --abort`; jj: a merge
    /// change probed and undone via `op restore`).
    ///
    /// Preconditions/behaviour:
    /// - git: requires a clean-enough working tree — a dirty-tree refusal
    ///   propagates as a plain error, not as [`MergeProbe::Conflicts`].
    /// - A failing rollback **propagates as an error** rather than returning a
    ///   result that misdescribes the on-disk state.
    /// - **Cancellation caveat:** the rollback runs on the same client, so if the
    ///   client carries a `default_cancel_on` token (the `cancellation` feature)
    ///   that fires during the probe, the rollback command is cancelled too and the
    ///   probe change may be left behind (`Error::Cancelled` surfaces). Re-probe and
    ///   reset with an un-cancelled client if you need a clean tree.
    pub async fn try_merge(&self, source: &str) -> Result<MergeProbe> {
        match &self.backend {
            Backend::Git(g) => git_backend::try_merge(g, &self.cwd, source).await,
            Backend::Jj(j) => jj_backend::try_merge(j, &self.cwd, source).await,
        }
    }

    /// Abort the in-progress operation, if any (git: `merge --abort` /
    /// `rebase --abort`; jj: a no-op — there are no paused operations, roll back
    /// explicitly via `Jj::transaction` / `op_restore`). Returns the fresh
    /// *post-call* [`OperationState`]; `Clear` when nothing was (or remains) in
    /// progress.
    pub async fn abort_in_progress(&self) -> Result<OperationState> {
        match &self.backend {
            Backend::Git(g) => git_backend::abort_in_progress(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::abort_in_progress(j, &self.cwd).await,
        }
    }

    /// Continue the in-progress operation after conflict resolution (git:
    /// `commit --no-edit` for a merge / `rebase --continue`; jj: a no-op —
    /// resolving the files *is* the continuation). Returns the fresh *post-call*
    /// [`OperationState`]:
    /// - `Conflict` when unresolved paths still block continuing (also on git —
    ///   unlike [`in_progress_state`](Self::in_progress_state), this method
    ///   *does* report `Conflict` for git), or when a continued rebase stops on
    ///   the next patch's conflict.
    /// - `Clear` when the operation finished.
    pub async fn continue_in_progress(&self) -> Result<OperationState> {
        match &self.backend {
            Backend::Git(g) => git_backend::continue_in_progress(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::continue_in_progress(j, &self.cwd).await,
        }
    }

    /// Whether the working copy is mid-operation or conflicted — see
    /// [`OperationState`]. Lets a caller decide between abort/continue without
    /// knowing the backend's model. Note the asymmetry: *this method* reports
    /// `Merge`/`Rebase` (never `Conflict`) on git — a git conflict *is* that
    /// paused state, and the conflict itself surfaces on the failed op via
    /// [`Error::is_merge_conflict`] (or as `Conflict` from
    /// [`continue_in_progress`](Self::continue_in_progress)) — while jj has no
    /// paused op and reports `Conflict` directly.
    pub async fn in_progress_state(&self) -> Result<OperationState> {
        match &self.backend {
            Backend::Git(g) => git_backend::in_progress_state(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::in_progress_state(j, &self.cwd).await,
        }
    }

    /// List attached worktrees (git) / workspaces (jj).
    pub async fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        match &self.backend {
            Backend::Git(g) => git_backend::list_worktrees(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::list_worktrees(j, &self.cwd).await,
        }
    }

    /// Create a worktree/workspace at `path` on a **new** `branch` based on
    /// `base`. Always [`CreateOutcome::Plain`]; a copy-on-write strategy stays in
    /// the consumer.
    ///
    /// `branch` must not already exist. The jj path is two steps (`workspace add`
    /// then `bookmark create`) and is not atomic: if the bookmark step fails, the
    /// freshly-added workspace is left in place for the caller to clean up. A
    /// consumer needing resume-existing or rollback semantics should drive the
    /// underlying client via [`jj`](Repo::jj) / [`git`](Repo::git).
    pub async fn create_worktree(
        &self,
        path: &Path,
        branch: &str,
        base: &str,
    ) -> Result<CreateOutcome> {
        match &self.backend {
            Backend::Git(g) => git_backend::create_worktree(g, &self.cwd, path, branch, base).await,
            Backend::Jj(j) => jj_backend::create_worktree(j, &self.cwd, path, branch, base).await,
        }
    }

    /// Remove the worktree/workspace at `path`. For jj this resolves the
    /// workspace name by matching `path`, deletes the directory, then forgets it;
    /// a `path` that matches no attached jj workspace returns
    /// [`Error::WorktreeNotFound`]. (For the best-effort, never-erroring variant,
    /// see [`cleanup_worktree_blocking`](Self::cleanup_worktree_blocking).)
    pub async fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::remove_worktree(g, &self.cwd, path, force).await,
            Backend::Jj(j) => jj_backend::remove_worktree(j, &self.cwd, path, force).await,
        }
    }

    /// **Synchronous** worktree cleanup for a context that cannot `.await` —
    /// chiefly a `Drop` guard. Force-removes the worktree at `path` (git:
    /// `worktree remove --force`; jj: resolve the workspace name by `path`, delete
    /// the directory, then `workspace forget`). Best-effort and short-lived: it
    /// shells out directly (no job-containment); a jj `path` that matches no
    /// workspace is a no-op (`Ok`).
    pub fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()> {
        match &self.backend {
            Backend::Git(_) => {
                vcs_git::blocking::worktree_remove(&self.cwd, path, true).map_err(Error::Io)
            }
            Backend::Jj(_) => {
                match vcs_jj::blocking::workspace_name_for_path(&self.cwd, path) {
                    Some(name) => {
                        // Delete the on-disk dir first (jj `forget` leaves it), then
                        // drop jj's record of the workspace.
                        let _ = std::fs::remove_dir_all(path);
                        vcs_jj::blocking::workspace_forget(&self.cwd, &name).map_err(Error::Io)
                    }
                    None => Ok(()),
                }
            }
        }
    }
}

/// Generate a facade trait from one signature table: the `#[async_trait]` trait
/// declaration *and* the delegating `impl … for $Ty<R>`, so the two can never drift
/// out of sync (a hazard when each is hand-maintained). Every generated body is a
/// trivial delegation to the like-named inherent method — which method resolution
/// prefers, so this never recurses; the real backend-`match` dispatch stays
/// hand-written on the inherent `impl`. `async` methods doc-link to their inherent
/// twin; `sync` methods carry an explicit doc string (their docs aren't uniform).
///
/// A near-identical copy lives in `vcs-forge`; the two are deliberately not shared
/// (separate crates, ~40-line macro — duplication beats a new dependency).
///
/// Signatures only: each entry is a bare `&self` (or sync) method — no method-level
/// generics, no `&mut self`, no default bodies (a new method shaped that way needs a
/// grammar tweak, not just a table row).
///
/// No `mockall::automock`: a Wave-S spike proved it can't process a trait whose
/// signatures come from `macro_rules!`. Captured `$_:ty` fragments reach `automock`
/// as opaque nonterminal token groups; its `syn` parser rejects them ("unsupported
/// type in this position"), whereas `#[async_trait]` tolerates them. So the facade
/// traits stay test-seam-tested (build a handle over a fake runner — see the trait
/// docs), which is also what their docs already recommend over mocking.
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
    /// The backend-agnostic common surface of [`Repo`], as a trait — so a consumer can
    /// hold a `Box<dyn VcsRepo>` / `&dyn VcsRepo` and code against the operations
    /// without naming the [`ProcessRunner`] generic or wrapping `Repo` themselves.
    ///
    /// Every method mirrors the like-named inherent method on [`Repo`]; the trait adds
    /// nothing but the abstraction boundary. Tool-specific operations stay off it (see
    /// the crate docs) — reach those through the concrete [`Repo`] and its bound
    /// handles. For hermetic tests, build a `Repo` over a fake runner with
    /// [`Repo::from_git`] / [`Repo::from_jj`] rather than mocking this trait.
    trait VcsRepo for Repo;
    sync {
        #[doc = "Which backend drives this handle."]
        fn kind() -> BackendKind;
        #[doc = "The repository root detected at open time."]
        fn root() -> &Path;
        #[doc = "The directory operations run against."]
        fn cwd() -> &Path;
        #[doc = "See [`Repo::cleanup_worktree_blocking`]."]
        fn cleanup_worktree_blocking(path: &Path) -> Result<()>;
    }
    async {
        fn current_branch() -> Result<Option<String>>;
        fn trunk() -> Result<Option<String>>;
        fn local_branches() -> Result<Vec<String>>;
        fn branch_exists(name: &str) -> Result<bool>;
        fn has_uncommitted_changes() -> Result<bool>;
        fn has_tracked_changes() -> Result<bool>;
        fn conflicted_files() -> Result<Vec<String>>;
        fn delete_branch(name: &str, force: bool) -> Result<()>;
        fn rename_branch(old: &str, new: &str) -> Result<()>;
        fn changed_files() -> Result<Vec<FileChange>>;
        fn diff_stat() -> Result<DiffStat>;
        fn snapshot() -> Result<RepoSnapshot>;
        fn commit_paths(paths: &[String], message: &str) -> Result<()>;
        fn fetch() -> Result<()>;
        fn fetch_from(remote: &str) -> Result<()>;
        fn fetch_remote_branch(branch: &str) -> Result<()>;
        fn push(branch: &str) -> Result<()>;
        fn checkout(reference: &str) -> Result<()>;
        fn rebase(onto: &str) -> Result<()>;
        fn try_merge(source: &str) -> Result<MergeProbe>;
        fn abort_in_progress() -> Result<OperationState>;
        fn continue_in_progress() -> Result<OperationState>;
        fn in_progress_state() -> Result<OperationState>;
        fn list_worktrees() -> Result<Vec<WorktreeInfo>>;
        fn create_worktree(path: &Path, branch: &str, base: &str) -> Result<CreateOutcome>;
        fn remove_worktree(path: &Path, force: bool) -> Result<()>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{Reply, ScriptedRunner};

    // --- detect ------------------------------------------------------------

    /// A unique temp directory, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            // Unique without a temp crate: process id + a monotonic counter, so
            // parallel tests never collide. Kept short — a long prefix can tip a
            // nested jj `op_store` path over Windows' MAX_PATH.
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("vcs-core-{tag}-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn detect_finds_git_and_jj_and_prefers_jj() {
        let tmp = TempDir::new("detect");
        let root = tmp.path();

        // Plain git repo.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let located = detect(root).expect("git detected");
        assert_eq!(located.kind, BackendKind::Git);
        assert_eq!(located.root, root);

        // Colocated: adding .jj makes jj win.
        std::fs::create_dir_all(root.join(".jj")).unwrap();
        assert_eq!(detect(root).unwrap().kind, BackendKind::Jj);
    }

    #[test]
    fn detect_walks_up_to_ancestor() {
        let tmp = TempDir::new("walkup");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let located = detect(&nested).expect("found via ancestor walk");
        assert_eq!(located.kind, BackendKind::Git);
        assert_eq!(located.root, root);
    }

    #[test]
    fn detect_returns_none_outside_repo() {
        let tmp = TempDir::new("norepo");
        assert!(detect(tmp.path()).is_none());
    }

    // --- dispatch (hermetic, ScriptedRunner-backed) ------------------------

    fn git_repo(runner: ScriptedRunner) -> Repo<ScriptedRunner> {
        Repo::from_git("/repo", "/repo", Git::with_runner(runner))
    }

    fn jj_repo(runner: ScriptedRunner) -> Repo<ScriptedRunner> {
        Repo::from_jj("/repo", "/repo", Jj::with_runner(runner))
    }

    // --- snapshot ----------------------------------------------------------

    // git: one porcelain-v2 call + a git-dir probe → a combined RepoSnapshot.
    #[tokio::test]
    async fn git_snapshot_combines_v2_status_and_op_state() {
        let v2 = concat!(
            "# branch.oid abc123\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -0\0",
            "1 .M N... 100644 100644 100644 1 2 a.rs\0",
            "? new.txt\0",
        );
        // An empty git dir → no MERGE_HEAD / rebase dir → Clear.
        let gitdir = TempDir::new("snap-git");
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["status", "--porcelain=v2"], Reply::ok(v2))
                .on(
                    ["rev-parse", "--git-dir"],
                    Reply::ok(gitdir.path().to_str().unwrap()),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (Some(2), Some(0)));
        assert!(s.dirty);
        assert_eq!(s.change_count, 2, "1 tracked + 1 untracked");
        assert!(!s.conflicted);
        assert_eq!(s.operation, OperationState::Clear);
    }

    // jj: one template row + a status count; a conflicted @ maps to Conflict; no
    // git-style upstream/ahead/behind.
    #[tokio::test]
    async fn jj_snapshot_from_template_with_change_count() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(["log"], Reply::ok("deadbeef\tmain\t0\t1\n")) // empty=0 dirty, conflict=1
                .on(["diff"], Reply::ok("M a.rs\nA b.rs\n")), // status -r @ --summary → 2
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.head.as_deref(), Some("deadbeef"));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.dirty);
        assert_eq!(s.change_count, 2);
        assert!(s.conflicted);
        assert_eq!(s.operation, OperationState::Conflict);
        assert_eq!(s.upstream, None);
        assert_eq!((s.ahead, s.behind), (None, None));
    }

    // jj: a clean `@` (empty=1) skips the change-count spawn entirely — the test
    // scripts NO `diff` rule, so calling `status` would error.
    #[tokio::test]
    async fn jj_snapshot_clean_skips_change_count() {
        let repo = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("c0ffee\t\t1\t0\n")));
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.head.as_deref(), Some("c0ffee"));
        assert_eq!(s.branch, None, "no bookmark");
        assert!(!s.dirty);
        assert_eq!(s.change_count, 0);
        assert!(!s.conflicted);
        assert_eq!(s.operation, OperationState::Clear);
    }

    // jj `list_worktrees` resolves each workspace's root via the batched
    // `workspace_roots` fan-out (one `workspace root --name <n>` per `workspace
    // list` row), then builds a `WorktreeInfo` per workspace. Hermetic: scripts the
    // template rows + the per-name root replies — the backend glue that the
    // `#[ignore]` integration tests otherwise cover only with a real `jj`.
    #[tokio::test]
    async fn jj_list_worktrees_batches_root_lookups() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["workspace", "list"],
                    Reply::ok("default\tc0ffee\tmain\nws1\tdecaf0\t\n"),
                )
                .on(
                    ["workspace", "root", "--name", "default"],
                    Reply::ok("/repo\n"),
                )
                .on(
                    ["workspace", "root", "--name", "ws1"],
                    Reply::ok("/repo/ws1\n"),
                ),
        );
        let worktrees = repo.list_worktrees().await.expect("list_worktrees");
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, Path::new("/repo"));
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert_eq!(worktrees[1].path, Path::new("/repo/ws1"));
        assert_eq!(worktrees[1].branch, None);
    }

    // A workspace whose `workspace root` lookup errors is skipped (no useful path),
    // mirroring the old sequential loop — the batch maps that slot to `Err`.
    #[tokio::test]
    async fn jj_list_worktrees_skips_unresolvable_root() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["workspace", "list"],
                    Reply::ok("default\tc0ffee\tmain\ngone\tdecaf0\t\n"),
                )
                .on(
                    ["workspace", "root", "--name", "default"],
                    Reply::ok("/repo\n"),
                )
                .on(
                    ["workspace", "root", "--name", "gone"],
                    Reply::fail(1, "Error: No such workspace"),
                ),
        );
        let worktrees = repo.list_worktrees().await.expect("list_worktrees");
        assert_eq!(worktrees.len(), 1, "the unresolvable workspace is skipped");
        assert_eq!(worktrees[0].path, Path::new("/repo"));
    }

    #[tokio::test]
    async fn kind_and_escape_hatches_reflect_backend() {
        let repo = git_repo(ScriptedRunner::new());
        assert_eq!(repo.kind(), BackendKind::Git);
        assert!(repo.git().is_some());
        assert!(repo.jj().is_none());
    }

    // The cwd-bound views mirror the backend, and `at` re-binds them to another
    // directory without a separate client.
    #[tokio::test]
    async fn bound_views_reflect_backend_and_cwd() {
        let git = git_repo(ScriptedRunner::new());
        assert!(git.git_at().is_some());
        assert!(git.jj_at().is_none());
        // A sibling handle bound elsewhere yields a view rooted at that dir.
        assert_eq!(git.at("/repo/wt").cwd(), Path::new("/repo/wt"));

        let jj = jj_repo(ScriptedRunner::new());
        assert!(jj.jj_at().is_some());
        assert!(jj.git_at().is_none());
    }

    #[tokio::test]
    async fn current_branch_maps_detached_head_to_none() {
        let named = git_repo(ScriptedRunner::new().on(["rev-parse"], Reply::ok("main\n")));
        assert_eq!(
            named.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
        let detached = git_repo(ScriptedRunner::new().on(["rev-parse"], Reply::ok("HEAD\n")));
        assert!(detached.current_branch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn changed_files_maps_git_status() {
        let repo = git_repo(ScriptedRunner::new().on(
            ["status"],
            Reply::ok(" M a.rs\0?? b.rs\0R  new.rs\0old.rs\0"),
        ));
        let changes = repo.changed_files().await.unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[1].kind, ChangeKind::Added);
        assert_eq!(changes[2].kind, ChangeKind::Renamed);
        assert_eq!(changes[2].old_path.as_deref(), Some("old.rs"));
    }

    #[tokio::test]
    async fn local_branches_maps_git_branch_output() {
        let repo = git_repo(ScriptedRunner::new().on(["branch"], Reply::ok("* main\n  feat\n")));
        assert_eq!(repo.local_branches().await.unwrap(), ["main", "feat"]);
    }

    #[tokio::test]
    async fn branch_exists_reads_show_ref_exit() {
        let yes = git_repo(ScriptedRunner::new().on(["show-ref"], Reply::ok("")));
        assert!(yes.branch_exists("main").await.unwrap());
        let no = git_repo(ScriptedRunner::new().on(["show-ref"], Reply::fail(1, "")));
        assert!(!no.branch_exists("nope").await.unwrap());
    }

    #[tokio::test]
    async fn has_uncommitted_changes_reflects_status() {
        let dirty = git_repo(ScriptedRunner::new().on(["status"], Reply::ok(" M a.rs\0")));
        assert!(dirty.has_uncommitted_changes().await.unwrap());
        let clean = git_repo(ScriptedRunner::new().on(["status"], Reply::ok("")));
        assert!(!clean.has_uncommitted_changes().await.unwrap());
    }

    #[tokio::test]
    async fn at_rebinds_cwd_and_shares_backend() {
        let repo = git_repo(ScriptedRunner::new());
        let moved = repo.at("/repo/sub");
        assert_eq!(moved.cwd(), Path::new("/repo/sub"));
        assert_eq!(moved.root(), Path::new("/repo"));
        assert_eq!(moved.kind(), BackendKind::Git);
    }

    // --- dispatch: jj backend (hermetic) -----------------------------------

    #[tokio::test]
    async fn jj_kind_and_escape_hatches_reflect_backend() {
        let repo = jj_repo(ScriptedRunner::new());
        assert_eq!(repo.kind(), BackendKind::Jj);
        assert!(repo.jj().is_some() && repo.git().is_none());
    }

    #[tokio::test]
    async fn jj_current_branch_reads_bookmark() {
        let repo = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("main\n")));
        assert_eq!(
            repo.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
    }

    #[tokio::test]
    async fn jj_local_branches_maps_bookmark_list() {
        let repo = jj_repo(ScriptedRunner::new().on(
            ["bookmark", "list"],
            Reply::ok("main: chg cmt desc\nfeat: c2 m2 d2\n"),
        ));
        assert_eq!(repo.local_branches().await.unwrap(), ["main", "feat"]);
    }

    #[tokio::test]
    async fn jj_branch_exists_scans_bookmarks() {
        let repo = jj_repo(
            ScriptedRunner::new().on(["bookmark", "list"], Reply::ok("main: chg cmt desc\n")),
        );
        assert!(repo.branch_exists("main").await.unwrap());
        let repo2 = jj_repo(
            ScriptedRunner::new().on(["bookmark", "list"], Reply::ok("main: chg cmt desc\n")),
        );
        assert!(!repo2.branch_exists("missing").await.unwrap());
    }

    #[tokio::test]
    async fn jj_has_uncommitted_changes_reads_empty_flag() {
        // CHANGE_TEMPLATE row: change_id \t commit_id \t empty \t description
        let dirty = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("kz\t38\tfalse\twip\n")));
        assert!(dirty.has_uncommitted_changes().await.unwrap());
        let clean = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("kz\t38\ttrue\t\n")));
        assert!(!clean.has_uncommitted_changes().await.unwrap());
    }

    #[tokio::test]
    async fn jj_changed_files_maps_diff_summary() {
        let repo = jj_repo(
            ScriptedRunner::new().on(["diff"], Reply::ok("M src/a.rs\nA b.rs\nD gone.rs\n")),
        );
        let changes = repo.changed_files().await.unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[1].kind, ChangeKind::Added);
        assert_eq!(changes[2].kind, ChangeKind::Deleted);
        assert!(changes.iter().all(|c| c.old_path.is_none()));
    }

    // jj DOES supply the rename's original path (its `{old => new}` summary
    // form) — `old_path` is populated on both backends, as the DTO documents.
    #[tokio::test]
    async fn jj_changed_files_populates_rename_old_path() {
        let repo =
            jj_repo(ScriptedRunner::new().on(["diff"], Reply::ok("R src/{old.rs => new.rs}\n")));
        let changes = repo.changed_files().await.unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Renamed);
        assert_eq!(changes[0].path, "src/new.rs");
        assert_eq!(changes[0].old_path.as_deref(), Some("src/old.rs"));
    }

    // `commit_paths(&[])` is refused up front on BOTH backends: the runners have
    // no rules, so reaching the CLI would error differently — the guard must trip
    // first (on jj an empty fileset would otherwise commit the whole working
    // copy; on git it would exit 128).
    #[tokio::test]
    async fn commit_paths_refuses_an_empty_path_set() {
        for repo in [
            git_repo(ScriptedRunner::new()),
            jj_repo(ScriptedRunner::new()),
        ] {
            let err = repo
                .commit_paths(&[], "msg")
                .await
                .expect_err("empty paths must be refused");
            assert!(
                err.to_string().contains("at least one path"),
                "unexpected error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn jj_rename_branch_builds_bookmark_rename() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::replying(Reply::ok(""));
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(&rec));
        repo.rename_branch("old", "new").await.unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["bookmark", "rename", "old", "new", "--color", "never"]
        );
    }

    // The widened common surface dispatches `checkout` to each backend's verb:
    // git `checkout`, jj `edit`.
    #[tokio::test]
    async fn checkout_dispatches_per_backend() {
        use processkit::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .checkout("feat")
            .await
            .unwrap();
        assert_eq!(grec.only_call().args_str(), ["checkout", "feat"]);

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .checkout("feat")
            .await
            .unwrap();
        assert_eq!(
            jrec.only_call().args_str(),
            ["edit", "feat", "--color", "never"]
        );
    }

    #[tokio::test]
    async fn fetch_remote_branch_dispatches_per_backend() {
        use processkit::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .fetch_remote_branch("main")
            .await
            .unwrap();
        assert!(
            grec.only_call()
                .args_str()
                .starts_with(&["fetch".to_string()])
        );

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .fetch_remote_branch("main")
            .await
            .unwrap();
        let args = jrec.only_call().args_str();
        assert_eq!(&args[..2], &["git", "fetch"]);
    }

    // The facade push is the honest LCD: git pushes the ref with `-u origin`,
    // jj pushes the bookmark's state with `-b`. Argv pinned on both backends.
    #[tokio::test]
    async fn push_dispatches_per_backend() {
        use processkit::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .push("feature")
            .await
            .unwrap();
        assert_eq!(
            grec.only_call().args_str(),
            ["push", "-u", "origin", "feature"]
        );

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .push("feature")
            .await
            .unwrap();
        let args = jrec.only_call().args_str();
        assert_eq!(&args[..4], &["git", "push", "-b", "feature"]);
    }

    // The two backends handle a flag-like branch per the documented guard
    // convention: git rejects it BEFORE spawning (the branch lands in GitPush's
    // bare-positional refspec slot, where `--force` would otherwise be parsed
    // as a flag); jj passes it verbatim in the `-b` flag-VALUE slot, where jj
    // reads it as a bookmark name and errors itself — no flag injection is
    // possible there, so no pre-spawn guard exists (same as rebase/fetch_from).
    #[tokio::test]
    async fn push_flag_like_branch_follows_guard_convention() {
        use processkit::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        let err = Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .push("--force")
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::Vcs(processkit::Error::Spawn { .. })),
            "got: {err:?}"
        );
        assert_eq!(grec.calls().len(), 0, "no process must have spawned");

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .push("--force")
            .await
            .expect("jj path spawns; the value rides -b verbatim");
        assert_eq!(
            &jrec.only_call().args_str()[..4],
            &["git", "push", "-b", "--force"],
            "the flag-like value must ride the -b flag-VALUE slot, not become argv"
        );
    }

    #[tokio::test]
    async fn fetch_from_names_the_remote_on_both_backends() {
        use processkit::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .fetch_from("upstream")
            .await
            .unwrap();
        assert_eq!(
            grec.only_call().args_str(),
            ["fetch", "--quiet", "upstream"]
        );

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .fetch_from("upstream")
            .await
            .unwrap();
        let args = jrec.only_call().args_str();
        assert_eq!(&args[..4], &["git", "fetch", "--remote", "upstream"]);
    }

    // git: untracked files count as uncommitted but not as *tracked* changes.
    #[tokio::test]
    async fn git_has_tracked_changes_ignores_untracked() {
        let dirty = git_repo(ScriptedRunner::new().on(["status"], Reply::ok(" M a.rs\0")));
        assert!(dirty.has_tracked_changes().await.unwrap());
        // `--untracked-files=no` means git itself omits `??` entries; an empty
        // reply is what a tracked-clean tree returns.
        let clean = git_repo(ScriptedRunner::new().on(["status"], Reply::ok("")));
        assert!(!clean.has_tracked_changes().await.unwrap());
    }

    // jj has no untracked concept — `has_tracked_changes` follows `@`'s emptiness.
    #[tokio::test]
    async fn jj_has_tracked_changes_follows_working_copy() {
        let dirty = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("kz\t38\tfalse\twip\n")));
        assert!(dirty.has_tracked_changes().await.unwrap());
    }

    #[tokio::test]
    async fn conflicted_files_dispatches_per_backend() {
        let git = git_repo(ScriptedRunner::new().on(["diff"], Reply::ok("a.rs\0b dir/c.rs\0")));
        assert_eq!(
            git.conflicted_files().await.unwrap(),
            ["a.rs", "b dir/c.rs"]
        );

        let jj =
            jj_repo(ScriptedRunner::new().on(["resolve"], Reply::ok("a.rs    2-sided conflict\n")));
        assert_eq!(jj.conflicted_files().await.unwrap(), ["a.rs"]);
        // The benign "no conflicts" non-zero exit still reads as an empty list.
        let clean = jj_repo(ScriptedRunner::new().on(
            ["resolve"],
            Reply::fail(2, "Error: No conflicts found at this revision"),
        ));
        assert!(clean.conflicted_files().await.unwrap().is_empty());
    }

    #[test]
    fn merge_probe_is_clean() {
        assert!(MergeProbe::Clean.is_clean());
        assert!(!MergeProbe::Conflicts(vec!["a.rs".into()]).is_clean());
    }

    // git try_merge, clean: probe merge, no MERGE_HEAD afterwards (the scripted
    // git-dir doesn't exist) → no abort, `Clean`.
    #[tokio::test]
    async fn git_try_merge_reports_clean_and_skips_needless_abort() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["merge"], Reply::ok("Already up to date.\n"))
                .on(["rev-parse"], Reply::ok("/vcs-core-no-such-git-dir")),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        assert_eq!(repo.try_merge("other").await.unwrap(), MergeProbe::Clean);
        assert!(
            rec.calls()
                .iter()
                .all(|c| !c.args_str().contains(&"--abort".to_string())),
            "no merge to abort"
        );
    }

    // git try_merge, conflict: conflicted paths are read BEFORE the abort (abort
    // clears the unmerged index), then the merge is aborted.
    #[tokio::test]
    async fn git_try_merge_collects_conflicts_then_aborts() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                // Order matters: ["merge","--abort"] must outrank the ["merge"] rule.
                .on(["merge", "--abort"], Reply::ok(""))
                .on(
                    ["merge"],
                    Reply::fail(1, "CONFLICT (content): Merge conflict in a.rs"),
                )
                .on(["diff"], Reply::ok("a.rs\0")),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        assert_eq!(
            repo.try_merge("other").await.unwrap(),
            MergeProbe::Conflicts(vec!["a.rs".to_string()])
        );
        let calls = rec.calls();
        let diff_pos = calls.iter().position(|c| c.args_str()[0] == "diff");
        let abort_pos = calls
            .iter()
            .position(|c| c.args_str().contains(&"--abort".to_string()));
        assert!(diff_pos.unwrap() < abort_pos.unwrap(), "{calls:?}");
    }

    // git try_merge: a failing rollback must propagate, not be reported as a
    // clean/conflicted probe.
    #[tokio::test]
    async fn git_try_merge_propagates_abort_failure() {
        let tmp = TempDir::new("probe-abort");
        std::fs::write(tmp.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let repo = git_repo(
            ScriptedRunner::new()
                .on(
                    ["merge", "--abort"],
                    Reply::fail(128, "fatal: cannot abort"),
                )
                .on(["merge"], Reply::ok(""))
                .on(["rev-parse"], Reply::ok(tmp.path().to_str().unwrap())),
        );
        assert!(repo.try_merge("other").await.is_err());
    }

    // jj try_merge: op head captured first, probe runs, op restore always runs.
    #[tokio::test]
    async fn jj_try_merge_probes_and_restores() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("op42\n"))
                .on(["op", "restore"], Reply::ok(""))
                .on(["new"], Reply::ok(""))
                .on(["log"], Reply::ok("1\n")) // is_conflicted → true
                .on(["resolve"], Reply::ok("a.rs    2-sided conflict\n")),
        );
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(&rec));
        assert_eq!(
            repo.try_merge("feature").await.unwrap(),
            MergeProbe::Conflicts(vec!["a.rs".to_string()])
        );
        let calls = rec.calls();
        assert_eq!(calls[0].args_str()[..2], ["op", "log"]);
        assert_eq!(calls[1].args_str()[0], "new");
        let last = calls.last().unwrap().args_str();
        assert_eq!(last[..3], ["op", "restore", "op42"]);
    }

    #[tokio::test]
    async fn jj_try_merge_clean_and_restore_failure() {
        // Conflict-free probe → Clean (no resolve call needed).
        let clean = jj_repo(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("op42\n"))
                .on(["op", "restore"], Reply::ok(""))
                .on(["new"], Reply::ok(""))
                .on(["log"], Reply::ok("0\n")),
        );
        assert_eq!(clean.try_merge("feature").await.unwrap(), MergeProbe::Clean);

        // A failing op restore breaks the rollback guarantee → error, not Clean.
        let broken = jj_repo(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("op42\n"))
                .on(["op", "restore"], Reply::fail(1, "op not found"))
                .on(["new"], Reply::ok(""))
                .on(["log"], Reply::ok("0\n")),
        );
        assert!(broken.try_merge("feature").await.is_err());
    }

    // continue_in_progress with unresolved paths reports `Conflict` and must NOT
    // attempt the continue (git would hard-error).
    #[tokio::test]
    async fn git_continue_blocked_by_conflicts_does_not_act() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::new(ScriptedRunner::new().on(["diff"], Reply::ok("a.rs\0")));
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        assert_eq!(
            repo.continue_in_progress().await.unwrap(),
            OperationState::Conflict
        );
        assert!(
            rec.calls().iter().all(|c| c.args_str()[0] == "diff"),
            "only the conflict probe may run: {:?}",
            rec.calls()
        );
    }

    // A continued rebase that stops on the NEXT patch's conflict exits non-zero;
    // continue_in_progress must report that as `Conflict`, not as an error. The
    // first conflict probe must see a clean index (else continue is blocked), the
    // post-continue probe must see the new conflict — a stateful predicate
    // sequences the two `diff` replies.
    #[tokio::test]
    async fn git_continue_maps_rebase_re_conflict() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let tmp = TempDir::new("rebase-restop");
        std::fs::create_dir_all(tmp.path().join("rebase-merge")).unwrap();
        let seen_first_diff = StdArc::new(AtomicBool::new(false));
        let flag = StdArc::clone(&seen_first_diff);
        let repo = git_repo(
            ScriptedRunner::new()
                .when(
                    move |cmd| {
                        cmd.arguments().first().and_then(|a| a.to_str()) == Some("diff")
                            && flag.swap(true, Ordering::SeqCst)
                    },
                    Reply::ok("a.rs\0"),
                )
                .on(["diff"], Reply::ok(""))
                .on(["rev-parse"], Reply::ok(tmp.path().to_str().unwrap()))
                .on(
                    ["rebase", "--continue"],
                    Reply::fail(1, "CONFLICT (content): Merge conflict in a.rs"),
                ),
        );
        assert_eq!(
            repo.continue_in_progress().await.unwrap(),
            OperationState::Conflict
        );
    }

    // abort_in_progress dispatches to `merge --abort` when MERGE_HEAD is present.
    #[tokio::test]
    async fn git_abort_dispatches_on_merge_in_progress() {
        use processkit::RecordingRunner;
        let tmp = TempDir::new("abort");
        std::fs::write(tmp.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["rev-parse"], Reply::ok(tmp.path().to_str().unwrap()))
                .on(["merge", "--abort"], Reply::ok("")),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        repo.abort_in_progress().await.unwrap();
        assert!(
            rec.calls()
                .iter()
                .any(|c| c.args_str() == ["merge", "--abort"]),
            "{:?}",
            rec.calls()
        );
    }

    // git surfaces an interrupted op as on-disk state: in_progress_state returns
    // Merge when MERGE_HEAD is present and Rebase when a rebase dir is — the
    // documented asymmetry (git's conflict IS that paused state, never `Conflict`
    // from this method).
    #[tokio::test]
    async fn git_in_progress_state_maps_merge_and_rebase() {
        let merging = TempDir::new("inprog-merge");
        std::fs::write(merging.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let merge_repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(
                ScriptedRunner::new()
                    .on(["rev-parse"], Reply::ok(merging.path().to_str().unwrap())),
            ),
        );
        assert_eq!(
            merge_repo.in_progress_state().await.unwrap(),
            OperationState::Merge
        );

        let rebasing = TempDir::new("inprog-rebase");
        std::fs::create_dir_all(rebasing.path().join("rebase-merge")).unwrap();
        let rebase_repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(
                ScriptedRunner::new()
                    .on(["rev-parse"], Reply::ok(rebasing.path().to_str().unwrap())),
            ),
        );
        assert_eq!(
            rebase_repo.in_progress_state().await.unwrap(),
            OperationState::Rebase
        );
    }

    // On an unborn git repo (no commits) diff_stat probes is_unborn and stats
    // against the empty tree instead of the unresolvable HEAD, so a fresh working
    // tree reports its additions rather than erroring.
    #[tokio::test]
    async fn git_diff_stat_unborn_uses_empty_tree() {
        use processkit::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["rev-parse"], Reply::fail(1, "")) // HEAD unborn
                .on(
                    ["diff", "--shortstat"],
                    Reply::ok(" 1 file changed, 2 insertions(+)\n"),
                ),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        let stat = repo.diff_stat().await.unwrap();
        assert_eq!(stat.insertions, 2);
        assert!(
            rec.calls()
                .iter()
                .any(|c| c.args_str() == ["diff", "--shortstat", vcs_git::EMPTY_TREE]),
            "diff_stat should target the empty tree on an unborn repo: {:?}",
            rec.calls()
        );
    }

    // On jj, abort/continue are reporting no-ops (nothing is ever paused).
    #[tokio::test]
    async fn jj_abort_and_continue_are_reporting_noops() {
        let conflicted = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("1\n")));
        assert_eq!(
            conflicted.abort_in_progress().await.unwrap(),
            OperationState::Conflict
        );
        let clear = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("0\n")));
        assert_eq!(
            clear.continue_in_progress().await.unwrap(),
            OperationState::Clear
        );
    }

    // jj records conflicts on the change; the facade maps that to `Conflict`.
    #[tokio::test]
    async fn jj_in_progress_state_maps_conflict() {
        let conflicted = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("1\n")));
        assert_eq!(
            conflicted.in_progress_state().await.unwrap(),
            OperationState::Conflict
        );
        let clear = jj_repo(ScriptedRunner::new().on(["log"], Reply::ok("0\n")));
        assert_eq!(
            clear.in_progress_state().await.unwrap(),
            OperationState::Clear
        );
    }

    // `&dyn VcsRepo` must dispatch through the real inherent methods (a delegating
    // body that recursed would stack-overflow here instead of returning).
    #[tokio::test]
    async fn vcs_repo_trait_object_dispatches() {
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["rev-parse"], Reply::ok("main\n"))
                .on(["show-ref"], Reply::ok("")),
        );
        let dynamic: &dyn VcsRepo = &repo;
        assert_eq!(dynamic.kind(), BackendKind::Git);
        assert_eq!(
            dynamic.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
        // Exercise a reference-argument async method through `&dyn` — pins the
        // async_trait lifetime capture the macro relies on (no-arg calls don't).
        assert!(dynamic.branch_exists("main").await.unwrap());
    }

    // When the backend has no native trunk (git `origin/HEAD` unset), the facade
    // falls back to a local `main`, then `master`.
    #[tokio::test]
    async fn trunk_falls_back_to_main() {
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["symbolic-ref"], Reply::fail(1, "")) // origin/HEAD unset → None
                .on(["show-ref"], Reply::ok("")), // branch_exists("main") → exit 0
        );
        assert_eq!(repo.trunk().await.unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn error_classifiers_recognise_markers() {
        let conflict = Error::Vcs(processkit::Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: "CONFLICT (content): Merge conflict in a.rs".into(),
            stderr: String::new(),
        });
        assert!(conflict.is_merge_conflict());
        assert!(!conflict.is_nothing_to_commit());
        // A non-Vcs error classifies as none of them.
        assert!(!Error::NotARepository("/x".into()).is_merge_conflict());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/core.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {
    #[doc = include_str!("../docs/cookbook.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod cookbook {}
    #[doc = include_str!("../docs/process-model.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod process_model {}
    #[doc = include_str!("../docs/positioning.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod positioning {}
    #[doc = include_str!("../docs/stability.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod stability {}
}
