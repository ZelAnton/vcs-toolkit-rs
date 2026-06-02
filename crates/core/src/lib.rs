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
//! the [`Repo::git`] / [`Repo::jj`] escape hatches.
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
use vcs_git::Git;
use vcs_jj::Jj;

mod dto;
mod error;
mod git_backend;
mod jj_backend;

pub use dto::{BackendKind, ChangeKind, CreateOutcome, DiffStat, FileChange, WorktreeInfo};
pub use error::{Error, Result};

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

    /// The current branch (git) or bookmark (jj); `None` when detached / no
    /// bookmark on the working copy.
    pub async fn current_branch(&self) -> Result<Option<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::current_branch(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::current_branch(j, &self.cwd).await,
        }
    }

    /// The trunk branch/bookmark; `None` when it can't be resolved.
    pub async fn trunk(&self) -> Result<Option<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::trunk(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::trunk(j, &self.cwd).await,
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

    #[tokio::test]
    async fn kind_and_escape_hatches_reflect_backend() {
        let repo = git_repo(ScriptedRunner::new());
        assert_eq!(repo.kind(), BackendKind::Git);
        assert!(repo.git().is_some());
        assert!(repo.jj().is_none());
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
    async fn at_rebinds_cwd_and_shares_backend() {
        let repo = git_repo(ScriptedRunner::new());
        let moved = repo.at("/repo/sub");
        assert_eq!(moved.cwd(), Path::new("/repo/sub"));
        assert_eq!(moved.root(), Path::new("/repo"));
        assert_eq!(moved.kind(), BackendKind::Git);
    }
}
