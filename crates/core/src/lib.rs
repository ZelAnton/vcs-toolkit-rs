//! `vcs-core` — a unified facade over [`vcs-git`](vcs_git) and [`vcs-jj`](vcs_jj).
//!
//! Two pieces both downstream tools kept re-implementing:
//!
//! * [`detect`] — walk up from a directory to find a `.git`/`.jj` repository
//!   (jj wins when colocated), returning the [`BackendKind`] and root.
//! * [`Repo`] — a cwd-bound handle that dispatches the *common* VCS operations
//!   (status, diff stat, partial commit, worktree create/remove, …) to whichever
//!   backend is present, returning backend-agnostic DTOs. Open it
//!   once with [`Repo::open`]; re-anchor it to another directory with
//!   [`Repo::at`] without threading a `dir` argument through every call.
//!
//! Tool-specific operations stay on the underlying typed clients, reachable via
//! the cwd-bound [`Repo::git_at`] / [`Repo::jj_at`] handles (or the raw
//! [`Repo::git`] / [`Repo::jj`] escape hatches). Some operations are deliberately
//! *not* on the common surface because the backends model them too differently to
//! unify without lying: a full `merge` (jj composes `new` + `squash` + bookmark
//! moves — though the conflict *probe* is unified as [`Repo::try_merge`]),
//! operation rollback (jj's `op restore` has no faithful git analogue; see
//! [`Jj::transaction`](vcs_jj::Jj::transaction) on the jj client), and
//! range/revset-scoped queries
//! (`commit_count`, diff stats over a range — git's `a..b` and jj's revsets
//! aren't interchangeable). Reach those through the bound handles.
//!
//! ```no_run
//! use vcs_core::Repo;
//! # fn run() -> vcs_core::Result<()> {
//! let repo = Repo::open(".")?;
//! # let _ = repo.kind();
//! # Ok(()) }
//! ```
//!
//! The handle is generic over the [`ProcessRunner`] so tests can inject a fake;
//! [`Repo::open`] uses the real job-backed runner.

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
    WorktreeInfo,
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
    /// on jj it shows in both.
    pub async fn diff_stat(&self) -> Result<DiffStat> {
        match &self.backend {
            Backend::Git(g) => git_backend::diff_stat(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::diff_stat(j, &self.cwd).await,
        }
    }

    /// Commit exactly `paths` with `message` (git `commit --only`, jj
    /// `commit <filesets>`). Paths are repo-relative.
    pub async fn commit_paths(&self, paths: &[String], message: &str) -> Result<()> {
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
    /// [`Error::is_conflict`] (or as `Conflict` from
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
    /// workspace name by matching `path`, deletes the directory, then forgets it.
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

/// The backend-agnostic common surface of [`Repo`], as a trait — so a consumer can
/// hold a `Box<dyn VcsRepo>` / `&dyn VcsRepo` and code against the operations
/// without naming the [`ProcessRunner`] generic or wrapping `Repo` themselves.
///
/// Every method mirrors the like-named inherent method on [`Repo`]; the trait adds
/// nothing but the abstraction boundary. Tool-specific operations stay off it (see
/// the crate docs) — reach those through the concrete [`Repo`] and its bound
/// handles. For hermetic tests, build a `Repo` over a fake runner with
/// [`Repo::from_git`] / [`Repo::from_jj`] rather than mocking this trait.
#[async_trait::async_trait]
pub trait VcsRepo: Send + Sync {
    /// Which backend drives this handle.
    fn kind(&self) -> BackendKind;
    /// The repository root detected at open time.
    fn root(&self) -> &Path;
    /// The directory operations run against.
    fn cwd(&self) -> &Path;

    /// See [`Repo::current_branch`].
    async fn current_branch(&self) -> Result<Option<String>>;
    /// See [`Repo::trunk`].
    async fn trunk(&self) -> Result<Option<String>>;
    /// See [`Repo::local_branches`].
    async fn local_branches(&self) -> Result<Vec<String>>;
    /// See [`Repo::branch_exists`].
    async fn branch_exists(&self, name: &str) -> Result<bool>;
    /// See [`Repo::has_uncommitted_changes`].
    async fn has_uncommitted_changes(&self) -> Result<bool>;
    /// See [`Repo::has_tracked_changes`].
    async fn has_tracked_changes(&self) -> Result<bool>;
    /// See [`Repo::conflicted_files`].
    async fn conflicted_files(&self) -> Result<Vec<String>>;
    /// See [`Repo::delete_branch`].
    async fn delete_branch(&self, name: &str, force: bool) -> Result<()>;
    /// See [`Repo::rename_branch`].
    async fn rename_branch(&self, old: &str, new: &str) -> Result<()>;
    /// See [`Repo::changed_files`].
    async fn changed_files(&self) -> Result<Vec<FileChange>>;
    /// See [`Repo::diff_stat`].
    async fn diff_stat(&self) -> Result<DiffStat>;
    /// See [`Repo::commit_paths`].
    async fn commit_paths(&self, paths: &[String], message: &str) -> Result<()>;
    /// See [`Repo::fetch`].
    async fn fetch(&self) -> Result<()>;
    /// See [`Repo::fetch_from`].
    async fn fetch_from(&self, remote: &str) -> Result<()>;
    /// See [`Repo::fetch_remote_branch`].
    async fn fetch_remote_branch(&self, branch: &str) -> Result<()>;
    /// See [`Repo::checkout`].
    async fn checkout(&self, reference: &str) -> Result<()>;
    /// See [`Repo::rebase`].
    async fn rebase(&self, onto: &str) -> Result<()>;
    /// See [`Repo::try_merge`].
    async fn try_merge(&self, source: &str) -> Result<MergeProbe>;
    /// See [`Repo::abort_in_progress`].
    async fn abort_in_progress(&self) -> Result<OperationState>;
    /// See [`Repo::continue_in_progress`].
    async fn continue_in_progress(&self) -> Result<OperationState>;
    /// See [`Repo::in_progress_state`].
    async fn in_progress_state(&self) -> Result<OperationState>;
    /// See [`Repo::list_worktrees`].
    async fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>>;
    /// See [`Repo::create_worktree`].
    async fn create_worktree(&self, path: &Path, branch: &str, base: &str)
    -> Result<CreateOutcome>;
    /// See [`Repo::remove_worktree`].
    async fn remove_worktree(&self, path: &Path, force: bool) -> Result<()>;
    /// See [`Repo::cleanup_worktree_blocking`].
    fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()>;
}

// Delegates to the inherent methods, which method resolution prefers — so these
// bodies dispatch through `Repo`'s real implementations, not back into the trait.
#[async_trait::async_trait]
impl<R: ProcessRunner> VcsRepo for Repo<R> {
    fn kind(&self) -> BackendKind {
        self.kind()
    }
    fn root(&self) -> &Path {
        self.root()
    }
    fn cwd(&self) -> &Path {
        self.cwd()
    }
    async fn current_branch(&self) -> Result<Option<String>> {
        self.current_branch().await
    }
    async fn trunk(&self) -> Result<Option<String>> {
        self.trunk().await
    }
    async fn local_branches(&self) -> Result<Vec<String>> {
        self.local_branches().await
    }
    async fn branch_exists(&self, name: &str) -> Result<bool> {
        self.branch_exists(name).await
    }
    async fn has_uncommitted_changes(&self) -> Result<bool> {
        self.has_uncommitted_changes().await
    }
    async fn has_tracked_changes(&self) -> Result<bool> {
        self.has_tracked_changes().await
    }
    async fn conflicted_files(&self) -> Result<Vec<String>> {
        self.conflicted_files().await
    }
    async fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.delete_branch(name, force).await
    }
    async fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.rename_branch(old, new).await
    }
    async fn changed_files(&self) -> Result<Vec<FileChange>> {
        self.changed_files().await
    }
    async fn diff_stat(&self) -> Result<DiffStat> {
        self.diff_stat().await
    }
    async fn commit_paths(&self, paths: &[String], message: &str) -> Result<()> {
        self.commit_paths(paths, message).await
    }
    async fn fetch(&self) -> Result<()> {
        self.fetch().await
    }
    async fn fetch_from(&self, remote: &str) -> Result<()> {
        self.fetch_from(remote).await
    }
    async fn fetch_remote_branch(&self, branch: &str) -> Result<()> {
        self.fetch_remote_branch(branch).await
    }
    async fn checkout(&self, reference: &str) -> Result<()> {
        self.checkout(reference).await
    }
    async fn rebase(&self, onto: &str) -> Result<()> {
        self.rebase(onto).await
    }
    async fn try_merge(&self, source: &str) -> Result<MergeProbe> {
        self.try_merge(source).await
    }
    async fn abort_in_progress(&self) -> Result<OperationState> {
        self.abort_in_progress().await
    }
    async fn continue_in_progress(&self) -> Result<OperationState> {
        self.continue_in_progress().await
    }
    async fn in_progress_state(&self) -> Result<OperationState> {
        self.in_progress_state().await
    }
    async fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        self.list_worktrees().await
    }
    async fn create_worktree(
        &self,
        path: &Path,
        branch: &str,
        base: &str,
    ) -> Result<CreateOutcome> {
        self.create_worktree(path, branch, base).await
    }
    async fn remove_worktree(&self, path: &Path, force: bool) -> Result<()> {
        self.remove_worktree(path, force).await
    }
    fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()> {
        self.cleanup_worktree_blocking(path)
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
            // parallel tests never collide.
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
        let repo = git_repo(ScriptedRunner::new().on(["rev-parse"], Reply::ok("main\n")));
        let dynamic: &dyn VcsRepo = &repo;
        assert_eq!(dynamic.kind(), BackendKind::Git);
        assert_eq!(
            dynamic.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
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
        assert!(conflict.is_conflict());
        assert!(!conflict.is_nothing_to_commit());
        // A non-Vcs error classifies as none of them.
        assert!(!Error::NotARepository("/x".into()).is_conflict());
    }
}
