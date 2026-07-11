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
//! let repo = Repo::discover(".")?;        // walks up, detects git vs jj
//! let s = repo.snapshot().await?;         // a few spawns, not a call per field
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
//! - **[`discover`]** — walk up from a directory to the filesystem root for a
//!   `.git`/`.jj` repo (jj wins when colocated — it's the tool driving the working
//!   copy). Pure filesystem probing, no subprocess; yields a [`Located`]
//!   ([`BackendKind`] + worktree root).
//! - **[`Repo`]** — the cwd-bound facade handle, the thing you hold. Open one with
//!   [`Repo::discover`] (walks up to find the repo; real job-backed runner) or
//!   [`Repo::open`] (strict — exactly `dir`, no walking up), or build it over an
//!   explicit client with [`Repo::from_git`] / [`Repo::from_jj`] (the test seam).
//!   Re-anchor it to another directory cheaply with [`Repo::at`] — the backend is
//!   shared behind an `Arc`, so threading work across worktrees never re-detects
//!   or rebuilds the client. Inspect it with [`kind`](Repo::kind) /
//!   [`root`](Repo::root) / [`cwd`](Repo::cwd).
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
//!   [`fetch_branch`](Repo::fetch_branch) /
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
//! use vcs_core::{MergeProbe, Repo, WorktreeCreate};
//! # async fn demo(repo: &Repo) -> vcs_core::Result<()> {
//! match repo.try_merge("feature").await? {
//!     MergeProbe::Clean            => println!("merges cleanly"),
//!     MergeProbe::Conflicts(paths) => println!("would conflict in {paths:?}"),
//!     _                            => {} // #[non_exhaustive]
//! }
//! let wt = repo
//!     .create_worktree(WorktreeCreate::new(Path::new("/tmp/feat"), "feature").base("main"))
//!     .await?;
//! # let _ = wt;
//! # Ok(()) }
//! ```
//!
//! # Testing
//!
//! There is **no mock feature** on the facade traits — the runner is the seam.
//! Build a [`Repo`] over a fake [`ProcessRunner`] with [`Repo::from_git`] /
//! [`Repo::from_jj`] (e.g. a [`ScriptedRunner`](processkit::testing::ScriptedRunner)
//! replying to canned argv), so the *real* per-backend dispatch, argv-building and
//! parsing run against canned output — exactly what a mocked `VcsRepo` would skip.
//! The cross-cutting patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! ```no_run
//! use processkit::testing::{Reply, ScriptedRunner};
//! use vcs_core::{vcs_git::Git, Repo};
//! # async fn demo() -> vcs_core::Result<()> {
//! let runner = ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0"));
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

use std::fmt::{self, Debug, Formatter};
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
    BackendKind, BranchDelete, ChangeKind, Commit, CreateOutcome, DiffStat, FileChange, MergeProbe,
    OperationState, RepoSnapshot, UpstreamTracking, WorktreeCreate, WorktreeCreatePartial,
    WorktreeInfo, WorktreeRemove,
};
pub use error::{Error, Result};
// The shared output-budget knob (from the CLI-support plumbing, via `vcs-git`): a
// per-client default ([`Repo::from_git`]/[`from_jj`] over a client built with
// `default_output_budget`) or a per-call override
// ([`Repo::show_file_within`](Repo::show_file_within)) for the content read this
// facade exposes. `vcs-git` and `vcs-jj` re-export the same type.
pub use vcs_git::OutputBudget;

// Re-export the underlying typed clients so a consumer depending only on
// `vcs-core` can still reach raw, tool-specific operations — and their types
// (`GitApi`, `JjApi`, `WorktreeAdd`, `JjFileset`, …) — without adding `vcs-git`
// / `vcs-jj` as separate dependencies. [`Repo::git`] / [`Repo::jj`] hand out
// borrows of these clients; the consumer decides, per call, whether to go
// through the facade or straight to the tool.
pub use vcs_git;
pub use vcs_jj;
// Re-export `processkit` itself so a `vcs-core`-only consumer can name the
// wrapped error directly — `match err { Error::Vcs(vcs_core::processkit::Error::
// Timeout { .. }) => … }` — and reach `Outcome`/`CancellationToken`/… without
// adding `processkit` as a separate dependency. (`Error::Vcs` carries a
// `processkit::Error`; the classifiers below cover the common branches.)
pub use processkit;
// Also surfaced at the crate root so the token a `default_cancel_on` client takes
// (built via `Git`/`Jj`, then passed to `Repo::from_git`/`from_jj`) is one name
// away. (Cancellation is core in processkit 0.10 — always available, no feature.)
pub use processkit::CancellationToken;

/// The result of [`discover`]: which backend, and the repository root it was
/// found at.
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
/// and only its own directory is checked. ([`Repo::discover`] absolutises for
/// you.) See [`Repo::open`] for a strict, non-walking check of exactly one
/// directory.
pub fn discover(start: &Path) -> Option<Located> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if is_jj_marker(&dir.join(".jj")) {
            return Some(Located {
                kind: BackendKind::Jj,
                root: dir.to_path_buf(),
            });
        }
        if is_git_marker(&dir.join(".git")) {
            return Some(Located {
                kind: BackendKind::Git,
                root: dir.to_path_buf(),
            });
        }
        current = dir.parent();
    }
    None
}

/// Whether `path` (a candidate `.jj`) is a real jj repository marker — a `.jj`
/// **directory** that contains a **`repo`** entry (the store: a *directory* in a
/// repo's main workspace / a colocated repo, a *file* pointer in a secondary
/// workspace). A stray/empty directory merely *named* `.jj` (e.g. a leftover
/// `mkdir .jj`) has no `repo` entry, so it can't shadow a healthy `.git` repo in the
/// same or a higher directory (M19). Symmetric with [`is_git_marker`]: both require a
/// *valid* marker, not mere existence.
fn is_jj_marker(path: &Path) -> bool {
    path.is_dir() && path.join("repo").exists()
}

/// Whether `path` (a candidate `.git`) is a real git repository marker — a `.git`
/// **directory**, or a **gitlink file** (a linked worktree / submodule) whose
/// content starts with `gitdir:`. A stray/garbage file merely *named* `.git` is
/// rejected, so it can't shadow a real repository higher up the tree, and a binary
/// or unreadable file is rejected too (the read fails → `false`). Symmetric with
/// [`is_jj_marker`]: both require a *valid* marker, not mere existence.
fn is_git_marker(path: &Path) -> bool {
    use std::io::Read;
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => true,
        Ok(meta) if meta.is_file() => {
            // A gitlink file is tiny (`gitdir: <path>\n`), so read only a small
            // prefix: `discover` walks *up to the filesystem root*, so a huge/garbage
            // file merely named `.git` in an ancestor we don't own must not force an
            // unbounded read. `read_to_end` loops over short reads (unlike a single
            // `read`, which the `Read` contract lets return fewer bytes), and
            // `from_utf8_lossy` tolerates a binary file or a multibyte char split at
            // the cap — the `gitdir:` marker is ASCII and within the first bytes.
            let Ok(file) = std::fs::File::open(path) else {
                return false;
            };
            let mut buf = Vec::new();
            let _ = file.take(32).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf)
                .trim_start()
                .starts_with("gitdir:")
        }
        _ => false,
    }
}

/// Whether `dir` (the candidate itself, not a `.git` beneath it) is a **bare**
/// git repository — created with `git init --bare` (or an equivalent bare
/// clone): `HEAD`/`config`/`objects`/`refs` sit directly in `dir`, with no
/// `.git` subdirectory. Requires all four markers together (`HEAD` a file,
/// `config` a file, `objects`/`refs` directories) so a directory that merely
/// happens to contain one or two similarly-named entries isn't misdetected —
/// symmetric with [`is_jj_marker`]/[`is_git_marker`]: a *valid* marker, not
/// mere partial name overlap. Used to give bare repositories their own
/// [`Error::BareRepository`](crate::Error::BareRepository) instead of the
/// generic [`Error::NotARepository`](crate::Error::NotARepository) (issue #6).
fn is_bare_git_repo_marker(dir: &Path) -> bool {
    dir.join("HEAD").is_file()
        && dir.join("config").is_file()
        && dir.join("objects").is_dir()
        && dir.join("refs").is_dir()
}

/// Walk up from `start` to the filesystem root looking for a **bare** git
/// repository marker (see [`is_bare_git_repo_marker`]). Only called after
/// [`discover`] has already walked the same chain and found no `.jj`/`.git`, so
/// any hit here is unambiguous — no real (non-bare) repository intervenes
/// between `start` and the bare repository root.
fn find_bare_git_repo(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if is_bare_git_repo_marker(dir) {
            return Some(dir.to_path_buf());
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

impl<R: ProcessRunner> Debug for Backend<R> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let variant_name = match self {
            Backend::Git(_) => "Git",
            Backend::Jj(_) => "Jj",
        };
        f.debug_tuple(variant_name).finish_non_exhaustive()
    }
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
// need a manual impl to avoid `R: Debug` bound.
impl<R: ProcessRunner> Debug for Repo<R> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let Repo { root, cwd, backend } = self;
        f.debug_struct("Repo")
            .field("root", root)
            .field("cwd", cwd)
            .field("backend", backend)
            .finish()
    }
}

impl Repo<JobRunner> {
    /// Discover the repository at or above `dir` and open a handle bound to
    /// `dir`, using the real job-backed runner. Walks up from `dir` toward the
    /// filesystem root — see [`discover`] — so it finds a repository whose root
    /// is `dir` itself or any ancestor. Errors with [`Error::NotARepository`]
    /// when no `.git`/`.jj` is found, or with [`Error::BareRepository`] when the
    /// walk instead reaches a **bare** git repository (`git init --bare`) before
    /// any `.jj`/`.git` — a bare repo has no working tree for this facade to
    /// drive (issue #6).
    ///
    /// For a strict check of exactly `dir` — no walking up — see [`Repo::open`].
    pub fn discover(dir: impl AsRef<Path>) -> Result<Self> {
        // Absolutise first: `discover` walks parents, and a relative path like "."
        // has no real ancestor chain (`Path::new(".").parent()` is `""`, then
        // `None`), so a relative input would never find a repo above the cwd.
        let dir = std::path::absolute(dir.as_ref())?;
        let located = match discover(&dir) {
            Some(located) => located,
            None => {
                // `discover` already walked the full chain and found nothing — a
                // second, cheap walk tells us whether the reason is "no
                // repository at all" or "a bare git repository sits in the
                // way", so the caller gets the more precise error.
                return Err(match find_bare_git_repo(&dir) {
                    Some(bare_root) => Error::BareRepository(bare_root),
                    None => Error::NotARepository(dir),
                });
            }
        };
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

    /// Open the repository at **exactly** `dir` — unlike [`Repo::discover`],
    /// this does **not** walk up through parent directories: `dir` itself must
    /// hold the `.jj`/`.git` marker (a `.jj` directory with a `repo` entry, or a
    /// `.git` directory / gitlink file — the same validated markers [`discover`]
    /// uses), or this errors with
    /// [`Error::NotARepository(dir)`](Error::NotARepository)
    /// even if a repository exists somewhere above `dir`. Mirrors the
    /// discover-vs-open split in gitoxide (`gix::discover` vs `gix::open`) and
    /// libgit2 (`git_repository_discover` vs `git_repository_open`) — see
    /// issue #8.
    ///
    /// If `dir` itself is a **bare** git repository (`git init --bare`: no
    /// `.git` subdirectory, just `HEAD`/`config`/`objects`/`refs` directly in
    /// `dir` — see `is_bare_git_repo_marker`), this errors with
    /// [`Error::BareRepository(dir)`](Error::BareRepository) instead of the
    /// generic `NotARepository`, matching what [`Repo::discover`] reports for
    /// the same directory (issue #6) — `open` still never walks up, so this
    /// only applies to `dir` itself, not an ancestor.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        // Absolutise so the bound `cwd`/`root` are consistent with `discover`'s
        // and so a relative "." names the actual directory, not an empty path.
        let dir = std::path::absolute(dir.as_ref())?;
        let kind = if is_jj_marker(&dir.join(".jj")) {
            BackendKind::Jj
        } else if is_git_marker(&dir.join(".git")) {
            BackendKind::Git
        } else if is_bare_git_repo_marker(&dir) {
            return Err(Error::BareRepository(dir));
        } else {
            return Err(Error::NotARepository(dir));
        };
        let backend = match kind {
            BackendKind::Git => Backend::Git(Arc::new(Git::new())),
            BackendKind::Jj => Backend::Jj(Arc::new(Jj::new())),
        };
        Ok(Repo {
            root: dir.clone(),
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

    /// The current branch (git) or bookmark (jj). On jj this is the nearest
    /// bookmark reachable from the working copy (`heads(::@ & bookmarks())`),
    /// so it stays set across a `jj describe`/`jj new`/`jj commit` — which leave
    /// the bookmark on the described parent while the new change carries none —
    /// matching git's "still on my branch" reporting. When several bookmarks are
    /// equally near `@`, the lexicographically-smallest name is returned
    /// (deterministic). `None` only when detached / no bookmark on or above `@`.
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
    ///
    /// Backend divergence: on **jj**, a bookmark deleted locally but still **tracked**
    /// on a remote lingers as a *tombstone* row (jj keeps it so the deletion can be
    /// propagated) until the deletion is pushed — so this can list a name a
    /// `delete_branch` just removed, unlike git. (The tombstone is not filtered here
    /// because jj renders it and a *conflicted* bookmark identically — filtering would
    /// also hide a real, conflicted bookmark; M21.)
    pub async fn local_branches(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::local_branches(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::local_branches(j, &self.cwd).await,
        }
    }

    /// A **read-only** [`local_branches`](Repo::local_branches): the same result,
    /// but on **jj** it passes `--ignore-working-copy`, so listing the bookmarks
    /// records no jj operation and never moves `@`. On **git** it is exactly
    /// [`local_branches`](Repo::local_branches) — git's branch listing records no
    /// operation and moves no ref, so there is nothing to make read-only.
    ///
    /// Use it (with [`snapshot_readonly`](Repo::snapshot_readonly)) from an
    /// *observer* — a watcher or a prompt refresh — that must not perturb the
    /// state it reads. See [`snapshot_readonly`](Repo::snapshot_readonly) for the
    /// jj working-copy trade-off this shares.
    pub async fn local_branches_readonly(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Git(g) => git_backend::local_branches(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::local_branches_readonly(j, &self.cwd).await,
        }
    }

    /// Whether a local branch/bookmark named `name` exists. See
    /// [`local_branches`](Repo::local_branches) for the jj deleted-but-tracked
    /// *tombstone* divergence (a just-deleted tracked bookmark can still read as
    /// existing until the deletion is pushed).
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
    /// Empty when there are none. Each path is a [`PathBuf`] carried losslessly from
    /// the backend, so a non-UTF-8 conflicted filename (legal on Unix) is not
    /// corrupted to `U+FFFD`.
    pub async fn conflicted_files(&self) -> Result<Vec<PathBuf>> {
        match &self.backend {
            Backend::Git(g) => git_backend::conflicted_files(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::conflicted_files(j, &self.cwd).await,
        }
    }

    /// Delete a local branch (git) / bookmark (jj). The [`BranchDelete`] spec's
    /// [`force`](BranchDelete::force) applies to git only (`branch -D` vs `-d`); jj
    /// has no force and ignores it.
    pub async fn delete_branch(&self, spec: BranchDelete) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => {
                git_backend::delete_branch(g, &self.cwd, &spec.name, spec.force).await
            }
            Backend::Jj(j) => jj_backend::delete_branch(j, &self.cwd, &spec.name).await,
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

    /// Recent history: up to `max` commits reachable from `revspec_or_revset`
    /// (git revspec / jj revset), most-recent-first (git `log`'s default order /
    /// jj `log`'s topological order).
    ///
    /// Backend nuance: [`Commit::author`]/[`Commit::date`] are `Some` only on
    /// git — jj's typed log doesn't currently surface authorship or a
    /// timestamp, so they're `None` there rather than guessed (see the
    /// [`Commit`] type docs).
    pub async fn log(&self, revspec_or_revset: &str, max: usize) -> Result<Vec<Commit>> {
        match &self.backend {
            Backend::Git(g) => git_backend::log(g, &self.cwd, revspec_or_revset, max).await,
            Backend::Jj(j) => jj_backend::log(j, &self.cwd, revspec_or_revset, max).await,
        }
    }

    /// The content of `path` as it exists at `rev` (git revspec / jj revset), e.g.
    /// `HEAD:src/lib.rs` on git or `@-` + a fileset on jj — both normalise
    /// backslash path separators and return the file's bytes verbatim (including
    /// any trailing newline).
    pub async fn show_file(&self, rev: &str, path: &str) -> Result<String> {
        match &self.backend {
            Backend::Git(g) => git_backend::show_file(g, &self.cwd, rev, path).await,
            Backend::Jj(j) => jj_backend::show_file(j, &self.cwd, rev, path).await,
        }
    }

    /// [`show_file`](Repo::show_file) with an explicit per-call [`OutputBudget`],
    /// instead of the budget the backend client was built with
    /// ([`default_output_budget`](vcs_git::Git::default_output_budget), inherited
    /// through [`from_git`](Repo::from_git)/[`from_jj`](Repo::from_jj)). Reads the
    /// blob under `budget`: past the ceiling it errors with an
    /// [`OutputTooLarge`](processkit::Error::OutputTooLarge)-carrying
    /// [`Error::Vcs`] (actual and allowed sizes) rather than buffering an unbounded
    /// file — use it to read a legitimately large file
    /// ([`OutputBudget::unlimited`], or a higher cap) or to tighten the cap for one
    /// call. A truncated blob is never returned as if complete.
    pub async fn show_file_within(
        &self,
        rev: &str,
        path: &str,
        budget: OutputBudget,
    ) -> Result<String> {
        match &self.backend {
            Backend::Git(g) => git_backend::show_file_within(g, &self.cwd, rev, path, budget).await,
            Backend::Jj(j) => jj_backend::show_file_within(j, &self.cwd, rev, path, budget).await,
        }
    }

    /// A batched [`RepoSnapshot`] of the common repo state — branch, upstream,
    /// ahead/behind, dirtiness, change count, and operation state — in a **small
    /// fixed** number of spawns instead of a call per field (git: `status
    /// --porcelain=v2 --branch` + the in-progress probe; jj: a `log -r @`
    /// template for head/empty/conflict, a `reachable_bookmarks` query for
    /// `branch`, and a change count only when dirty). Built for prompt/status-bar/
    /// TUI refreshes. Note the asymmetry: [`tracking`](RepoSnapshot::tracking)
    /// (the upstream ref + ahead/behind) is always `None` on jj, which has no
    /// git-style upstream tracking.
    pub async fn snapshot(&self) -> Result<RepoSnapshot> {
        match &self.backend {
            Backend::Git(g) => git_backend::snapshot(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::snapshot(j, &self.cwd).await,
        }
    }

    /// A **read-only** [`snapshot`](Repo::snapshot): the same [`RepoSnapshot`],
    /// but on **jj** it never snapshots the working copy — every underlying query
    /// passes `--ignore-working-copy`, so the batched read records **no** jj
    /// operation and never moves `@`. On **git** it is exactly
    /// [`snapshot`](Repo::snapshot) (git's status query records no operation and
    /// moves no ref).
    ///
    /// Use it for an *observer* — a repository watcher, a prompt/status-bar
    /// refresh — that must not perturb the state it reports: an ordinary jj query
    /// snapshots the working copy as a side effect (taking the lock, recording an
    /// operation, possibly moving `@`), so the observer would otherwise *mutate*
    /// the repo it merely means to read.
    ///
    /// **jj trade-off:** because the working copy isn't snapshotted, a bare
    /// working-tree edit that no jj command has recorded yet is **not** reflected
    /// — [`dirty`](RepoSnapshot::dirty)/[`head`](RepoSnapshot::head) are as of the
    /// last recorded operation. To observe such unsnapshotted edits, accept the
    /// mutation and call [`snapshot`](Repo::snapshot).
    pub async fn snapshot_readonly(&self) -> Result<RepoSnapshot> {
        match &self.backend {
            Backend::Git(g) => git_backend::snapshot(g, &self.cwd).await,
            Backend::Jj(j) => jj_backend::snapshot_readonly(j, &self.cwd).await,
        }
    }

    /// Commit exactly `paths` with `message` (git `commit --only`, jj
    /// `commit <filesets>`). Paths are repo-relative. `paths` must be non-empty:
    /// an empty set is refused up front, because the backends would diverge
    /// dangerously — git errors out, while jj's `commit` with no filesets would
    /// silently commit the **entire** working copy.
    ///
    /// Takes [`PathBuf`]s so a path obtained from [`changed_files`](Self::changed_files)
    /// / [`conflicted_files`](Self::conflicted_files) round-trips **losslessly** — on
    /// git a non-UTF-8 path (legal on Unix) reaches the commit unchanged via the
    /// NUL-safe pathspec transport; on jj the fileset language is text, so jj's own
    /// (non-UTF-8-incapable) fileset handling applies.
    pub async fn commit_paths(&self, paths: &[PathBuf], message: &str) -> Result<()> {
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
    /// (git `fetch_branch` / jj `git fetch -b`). Transient network failures
    /// are retried by the underlying client.
    pub async fn fetch_branch(&self, branch: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::fetch_branch(g, &self.cwd, branch).await,
            Backend::Jj(j) => jj_backend::fetch_branch(j, &self.cwd, branch).await,
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
    ///
    /// ⚠ **Backend divergence — this is not "detach and build on top" on jj.** On
    /// **git**, a subsequent commit *appends* on top of `reference` (its tip is
    /// untouched). On **jj**, `checkout` maps to `jj edit`, which makes `reference`'s
    /// commit *itself* the working-copy change — so a following
    /// [`commit_paths`](Repo::commit_paths) (or any edit) **rewrites that commit in
    /// place** (a new change-id, a replaced
    /// description), silently amending a possibly-already-pushed commit rather than
    /// adding a new one.
    ///
    /// So backend-agnostic "start fresh work on top of `main`" code must **not** rely
    /// on `checkout` alone. If you want git-like append-on-top semantics on both
    /// backends, use [`new_child`](Repo::new_child), which maps to `jj new
    /// <reference>` on jj and to `checkout <reference>` on git.
    pub async fn checkout(&self, reference: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::checkout(g, &self.cwd, reference).await,
            Backend::Jj(j) => jj_backend::checkout(j, &self.cwd, reference).await,
        }
    }

    /// Start new work on top of `reference` without modifying it.
    ///
    /// On git this checks out `reference`; the next commit naturally appends on top.
    /// On jj this runs `jj new <reference>`, creating an undescribed child change.
    pub async fn new_child(&self, reference: &str) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => git_backend::new_child(g, &self.cwd, reference).await,
            Backend::Jj(j) => jj_backend::new_child(j, &self.cwd, reference).await,
        }
    }

    /// Rebase the current line onto `onto`. The two backends **diverge** on
    /// non-linear layouts, so this is a documented least-common-denominator:
    /// - **git** (`rebase <onto>` = `merge-base(HEAD,onto)..HEAD`) moves only
    ///   `HEAD`'s own ancestor line; commits stacked on `HEAD` stay put.
    /// - **jj** (`rebase -d <onto>` = the default `-b @` = `(onto..@)::`) moves
    ///   that line *and its whole descendant closure* — anything stacked on `@`,
    ///   and any sibling off an *intermediate* commit of the line, move too.
    ///
    /// They agree on a linear `HEAD`/`@`; on a **stacked or intermediate-fork**
    /// layout jj moves strictly more. A sibling that shares only the fork point is
    /// moved by neither. `onto` is a branch/bookmark name or revision the backend
    /// understands.
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
    /// - **Cancellation-safe rollback:** on **both** backends the *whole* rollback
    ///   path — the decision of whether to roll back **and** the command that
    ///   performs it — runs on a fresh cancellation context with its own bounded
    ///   deadline (git: `Git::is_merge_in_progress_detached` + `merge --abort` via
    ///   `Git::merge_abort_detached`; jj: the op-log probe + `op restore` via
    ///   `Jj::rollback_to`), so a `default_cancel_on` token (the `cancellation`
    ///   feature) that fires during the probe no longer cancels the rollback too —
    ///   not even by cancelling the "is a trial merge still staged?" check before
    ///   the abort is reached. The trial merge is still undone rather than left
    ///   staged, closing the gap where a cancelled probe abandoned it on git. (A
    ///   rollback that fails for another reason still propagates per the bullet
    ///   above.)
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
    /// then `bookmark create`) and is not atomic, but a failed bookmark step
    /// **rolls back**: the workspace directory is removed only when `workspace add`
    /// created it (a pre-existing directory the caller already had is left intact),
    /// then the workspace is forgotten. Residue is no longer swallowed: if the
    /// rollback can't remove that directory or can't `forget` the workspace, the call
    /// fails with a composite [`Error::Io`] naming what still needs cleaning up (and
    /// is safe to re-run); a clean rollback instead surfaces the original
    /// bookmark-step error unchanged (its [`Error::Vcs`] classification) — so a failed
    /// call never silently leaks a half-made worktree.
    pub async fn create_worktree(&self, spec: WorktreeCreate) -> Result<CreateOutcome> {
        let WorktreeCreate { path, branch, base } = &spec;
        match &self.backend {
            Backend::Git(g) => git_backend::create_worktree(g, &self.cwd, path, branch, base).await,
            Backend::Jj(j) => jj_backend::create_worktree(j, &self.cwd, path, branch, base).await,
        }
    }

    /// Remove the worktree/workspace at `path`. For jj this resolves the
    /// workspace name by matching `path`, deletes the directory, then forgets it;
    /// a `path` that matches none of the **resolvable** jj workspaces returns
    /// [`Error::WorktreeNotFound`], but when some registered workspace can't be
    /// resolved via `jj workspace root --name` the path's absence is unprovable, so a
    /// distinct diagnosable [`Error::Io`] (naming the unresolved workspaces;
    /// [`is_resource_not_found`](Error::is_resource_not_found) stays `false`) is
    /// returned instead. A directory that can't be deleted is likewise surfaced (an
    /// [`Error::Io`] naming the still-registered workspace, with the `forget` left for
    /// the retry). (For the short-lived, blocking `Drop`-path variant, see
    /// [`cleanup_worktree_blocking`](Self::cleanup_worktree_blocking).)
    ///
    /// The [`WorktreeRemove`] spec's [`force`](WorktreeRemove::force) mirrors git's
    /// `worktree remove`: without it a worktree that still has **uncommitted changes**
    /// is refused (`Err`) rather than deleted, so a stray edit isn't silently lost —
    /// build `WorktreeRemove::new(path).force()` to remove it anyway. On **jj** the
    /// changes are snapshotted into the op log before the check, so a refusal keeps
    /// them recoverable; note that checking spawns a jj command in the target
    /// workspace, so a genuinely stale working copy can surface an error without
    /// `force` (use `.force()` there). The repository's **main** workspace is always
    /// refused (it can't be removed without destroying the repo), regardless of `force`.
    pub async fn remove_worktree(&self, spec: WorktreeRemove) -> Result<()> {
        match &self.backend {
            Backend::Git(g) => {
                git_backend::remove_worktree(g, &self.cwd, &spec.path, spec.force).await
            }
            Backend::Jj(j) => {
                jj_backend::remove_worktree(j, &self.cwd, &spec.path, spec.force).await
            }
        }
    }

    /// **Synchronous** worktree cleanup for a context that cannot `.await` —
    /// chiefly a `Drop` guard. Force-removes the worktree at `path` (git:
    /// `worktree remove --force`; jj: resolve the workspace name by `path`, delete
    /// the directory, then `workspace forget`). Short-lived and shells out directly
    /// (no job-containment), but not error-swallowing: a jj `path` that genuinely
    /// matches no workspace is an `Ok` no-op, yet a probe failure (the `workspace
    /// list`, or a registered workspace that won't resolve) and a `remove_dir_all`
    /// failure are surfaced as `Err` (the `forget` is skipped on a failed removal, so
    /// a surviving directory isn't orphaned). Like the async
    /// [`remove_worktree`](Self::remove_worktree), it **refuses the repository's
    /// main workspace** (whose directory is the main working copy) — deleting it
    /// would wipe the repo — even on this force-by-contract path.
    pub fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()> {
        match &self.backend {
            Backend::Git(_) => vcs_git::blocking::worktree_remove(
                &self.cwd,
                vcs_git::WorktreeRemove::new(path).force(),
            )
            .map_err(Error::Io),
            Backend::Jj(_) => {
                // jj resolves a relative worktree path against the repo dir (its
                // cwd), so resolve it the same way here — the lookup and the dir
                // removal must target the location jj used, not one under the process
                // cwd (which may differ from `self.cwd`).
                let abs_path = self.cwd.join(path);
                // Tell a genuine "no such workspace" (`Ok(None)` → nothing to clean
                // up, a no-op) apart from a probe failure (`Err` → surfaced, not
                // silently treated as a no-op): the blocking resolver no longer folds
                // both into `None`.
                match vcs_jj::blocking::workspace_name_for_path(&self.cwd, &abs_path)
                    .map_err(Error::Io)?
                {
                    Some(name) => {
                        // Same main-workspace guard as the async `remove_worktree`
                        // (jj_backend.rs): never `remove_dir_all` the repository's
                        // main working copy — its directory owns the object store, so
                        // deleting it wipes the whole repo. The `default` name and the
                        // store-owning `.jj/repo` *directory* (a secondary's is a file
                        // pointer) both flag it, so a `jj workspace rename` can't
                        // bypass it. Force is implied on this Drop path, but this guard
                        // is unconditional — a repo-wipe is never the intent.
                        if name == "default" || abs_path.join(".jj").join("repo").is_dir() {
                            return Err(Error::Io(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "refusing to remove the repository's main workspace",
                            )));
                        }
                        // Delete the on-disk dir first (jj `forget` leaves it), then
                        // drop jj's record of the workspace. A removal failure is
                        // SURFACED (not swallowed with `let _ =`) and the forget is
                        // skipped: forgetting a workspace whose directory survived
                        // would orphan that dir — worse than a still-attached workspace
                        // — and the reported error names what is still registered so
                        // the cleanup can be safely re-run once the directory is free.
                        if abs_path.exists() {
                            std::fs::remove_dir_all(&abs_path).map_err(|e| {
                                Error::Io(std::io::Error::new(
                                    e.kind(),
                                    format!(
                                        "failed to remove the worktree directory {} ({e}); the jj \
                                         workspace `{name}` is still registered — free the \
                                         directory and retry the cleanup",
                                        abs_path.display()
                                    ),
                                ))
                            })?;
                        }
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
/// `vcs-forge` used to carry a near-identical copy of this macro, kept
/// deliberately unshared (separate crates, ~40-line macro — duplication beats a
/// new dependency); it was removed there in v0.1.1 when new trait methods needed
/// default bodies the macro couldn't express, so `vcs-forge`'s facade trait and
/// impl are now hand-maintained (see the removal note in `vcs-forge`'s
/// `src/lib.rs`). This crate is still v0.x and doesn't need that, so the
/// original signature-table macro remains the right shape here.
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
        fn local_branches_readonly() -> Result<Vec<String>>;
        fn branch_exists(name: &str) -> Result<bool>;
        fn has_uncommitted_changes() -> Result<bool>;
        fn has_tracked_changes() -> Result<bool>;
        fn conflicted_files() -> Result<Vec<PathBuf>>;
        fn delete_branch(spec: BranchDelete) -> Result<()>;
        fn rename_branch(old: &str, new: &str) -> Result<()>;
        fn changed_files() -> Result<Vec<FileChange>>;
        fn diff_stat() -> Result<DiffStat>;
        fn log(revspec_or_revset: &str, max: usize) -> Result<Vec<Commit>>;
        fn show_file(rev: &str, path: &str) -> Result<String>;
        fn show_file_within(rev: &str, path: &str, budget: OutputBudget) -> Result<String>;
        fn snapshot() -> Result<RepoSnapshot>;
        fn snapshot_readonly() -> Result<RepoSnapshot>;
        fn commit_paths(paths: &[PathBuf], message: &str) -> Result<()>;
        fn fetch() -> Result<()>;
        fn fetch_from(remote: &str) -> Result<()>;
        fn fetch_branch(branch: &str) -> Result<()>;
        fn push(branch: &str) -> Result<()>;
        fn checkout(reference: &str) -> Result<()>;
        fn new_child(reference: &str) -> Result<()>;
        fn rebase(onto: &str) -> Result<()>;
        fn try_merge(source: &str) -> Result<MergeProbe>;
        fn abort_in_progress() -> Result<OperationState>;
        fn continue_in_progress() -> Result<OperationState>;
        fn in_progress_state() -> Result<OperationState>;
        fn list_worktrees() -> Result<Vec<WorktreeInfo>>;
        fn create_worktree(spec: WorktreeCreate) -> Result<CreateOutcome>;
        fn remove_worktree(spec: WorktreeRemove) -> Result<()>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{Reply, ScriptedRunner};
    // The shared sandbox fixture — a unique temp dir removed on drop. Using the
    // testkit's one impl instead of a private copy means the wrappers/facades
    // don't each carry a fixture that could drift.
    use vcs_testkit::TempDir;

    // --- discover ------------------------------------------------------------

    #[test]
    fn discover_finds_git_and_jj_and_prefers_jj() {
        let tmp = TempDir::new("discover");
        let root = tmp.path();

        // Plain git repo.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let located = discover(root).expect("git detected");
        assert_eq!(located.kind, BackendKind::Git);
        assert_eq!(located.root, root);

        // Colocated: adding a *valid* .jj (with its `repo` store) makes jj win.
        std::fs::create_dir_all(root.join(".jj").join("repo")).unwrap();
        assert_eq!(discover(root).unwrap().kind, BackendKind::Jj);
    }

    // M19: a stray/empty `.jj` directory (no `repo` store — e.g. a leftover
    // `mkdir .jj`) is NOT a jj marker and must not shadow a healthy `.git` repo in the
    // same directory. A valid `.jj` (with `repo`, dir or file) still wins.
    #[test]
    fn discover_ignores_a_dotjj_without_a_repo_store() {
        let tmp = TempDir::new("stray-jj");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".jj")).unwrap(); // empty — no `repo`
        assert_eq!(
            discover(root).expect("git still detected").kind,
            BackendKind::Git,
            "an empty .jj must not shadow a real .git"
        );

        // A secondary workspace's `.jj/repo` is a *file* pointer — still valid.
        let sec = TempDir::new("jj-secondary");
        std::fs::create_dir_all(sec.path().join(".jj")).unwrap();
        std::fs::write(sec.path().join(".jj").join("repo"), b"/path/to/store\n").unwrap();
        assert_eq!(discover(sec.path()).unwrap().kind, BackendKind::Jj);
    }

    #[test]
    fn discover_walks_up_to_ancestor() {
        let tmp = TempDir::new("walkup");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let located = discover(&nested).expect("found via ancestor walk");
        assert_eq!(located.kind, BackendKind::Git);
        assert_eq!(located.root, root);
    }

    #[test]
    fn discover_returns_none_outside_repo() {
        let tmp = TempDir::new("norepo");
        assert!(discover(tmp.path()).is_none());
    }

    // A gitlink `.git` *file* (a linked worktree / submodule) is a valid git marker;
    // a stray file merely named `.git` is NOT — so it can't shadow a real repo above.
    #[test]
    fn discover_validates_dotgit_file_is_a_gitlink() {
        let tmp = TempDir::new("gitlink");
        let root = tmp.path();

        // A gitlink file → detected as a git repo at this dir.
        std::fs::write(root.join(".git"), "gitdir: /somewhere/.git/worktrees/wt\n").unwrap();
        assert_eq!(
            discover(root).expect("gitlink detected").kind,
            BackendKind::Git
        );

        // A garbage file named `.git` (not a gitlink) is rejected — and must NOT
        // shadow a real `.git` directory in the parent.
        let parent = TempDir::new("gitlink-parent");
        std::fs::create_dir_all(parent.path().join(".git")).unwrap();
        let child = parent.path().join("sub");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join(".git"), "not a gitlink, just noise\n").unwrap();
        let located = discover(&child).expect("walks up past the bogus .git file");
        assert_eq!(located.root, parent.path(), "the real repo is the parent");

        // An empty `.git` file is not a marker.
        let empty = TempDir::new("gitlink-empty");
        std::fs::write(empty.path().join(".git"), "").unwrap();
        assert!(discover(empty.path()).is_none(), "empty .git is not a repo");

        // Leading whitespace before `gitdir:` is tolerated (the `trim_start`).
        let spaced = TempDir::new("gitlink-spaced");
        std::fs::write(
            spaced.path().join(".git"),
            "  gitdir: /x/.git/worktrees/w\n",
        )
        .unwrap();
        assert_eq!(
            discover(spaced.path())
                .expect("spaced gitlink detected")
                .kind,
            BackendKind::Git
        );
    }

    // --- bare git repository (issue #6) -------------------------------------

    // The issue #6 repro: a `git init --bare` directory (no `.git` subdir, just
    // `HEAD`/`config`/`objects`/`refs` in the root) must open as
    // `Error::BareRepository`, not the generic `Error::NotARepository` — matched
    // by variant, not by message substring, so the distinction can't silently
    // regress into the old generic error.
    #[test]
    fn discover_reports_bare_repository_not_generic_not_a_repository() {
        let tmp = TempDir::new("bare-repo");
        let root = tmp.path();
        std::fs::write(root.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join("config"), "[core]\n\tbare = true\n").unwrap();
        std::fs::create_dir_all(root.join("objects")).unwrap();
        std::fs::create_dir_all(root.join("refs")).unwrap();

        match Repo::discover(root) {
            Err(Error::BareRepository(p)) => assert_eq!(p, root),
            other => panic!("expected Error::BareRepository, got {other:?}"),
        }

        // The strict, non-walking `open`, called directly on the bare repo's own
        // root, also special-cases it via `is_bare_git_repo_marker` — mirroring
        // `discover`'s classification for this same directory (issue #6/#8
        // symmetry), even though `open` itself never walks up.
        match Repo::open(root) {
            Err(Error::BareRepository(p)) => assert_eq!(p, root),
            other => panic!("expected Error::BareRepository, got {other:?}"),
        }
    }

    // A bare repository nested a few levels below `dir` is still found by
    // walking up — mirrors `discover_walks_up_to_ancestor` for the bare case.
    #[test]
    fn discover_finds_bare_repository_via_ancestor_walk() {
        let tmp = TempDir::new("bare-walkup");
        let root = tmp.path();
        std::fs::write(root.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join("config"), "[core]\n\tbare = true\n").unwrap();
        std::fs::create_dir_all(root.join("objects")).unwrap();
        std::fs::create_dir_all(root.join("refs")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        match Repo::discover(&nested) {
            Err(Error::BareRepository(p)) => assert_eq!(p, root),
            other => panic!("expected Error::BareRepository, got {other:?}"),
        }

        // The strict `open` never walks up, so it reports `NotARepository` on
        // the nested dir regardless of what sits above it.
        match Repo::open(&nested) {
            Err(Error::NotARepository(p)) => assert_eq!(p, nested),
            other => panic!("expected Error::NotARepository, got {other:?}"),
        }
    }

    // A directory that merely happens to hold some, but not all four, of the
    // bare-repo marker entries must NOT be misdetected as a bare repository —
    // it's just an ordinary non-repository directory.
    #[test]
    fn discover_does_not_misdetect_partial_bare_markers_as_bare_repository() {
        let tmp = TempDir::new("bare-partial");
        let root = tmp.path();
        // Only `HEAD` and `config` — no `objects`/`refs` directories.
        std::fs::write(root.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join("config"), "[core]\n\tbare = true\n").unwrap();

        match Repo::discover(root) {
            Err(Error::NotARepository(p)) => assert_eq!(p, root),
            other => panic!("expected Error::NotARepository, got {other:?}"),
        }
    }

    // A real (non-bare) git repository — `.git` subdirectory present — must
    // keep opening as before, not get swept up by the new bare-detection path.
    #[test]
    fn open_still_opens_a_normal_git_repository() {
        let tmp = TempDir::new("normal-git");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let repo = Repo::open(root).expect("normal git repo still opens");
        assert_eq!(repo.kind(), BackendKind::Git);
        assert_eq!(repo.root(), root);
    }

    // A real jj repository must also keep opening as before.
    #[test]
    fn open_still_opens_a_normal_jj_repository() {
        let tmp = TempDir::new("normal-jj");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".jj").join("repo")).unwrap();

        let repo = Repo::open(root).expect("normal jj repo still opens");
        assert_eq!(repo.kind(), BackendKind::Jj);
        assert_eq!(repo.root(), root);
    }

    // A directory that is neither a repo nor a bare repo still reports the
    // generic `NotARepository`.
    #[test]
    fn open_reports_not_a_repository_when_nothing_found() {
        let tmp = TempDir::new("norepo-open");
        match Repo::open(tmp.path()) {
            Err(Error::NotARepository(p)) => assert_eq!(p, tmp.path()),
            other => panic!("expected Error::NotARepository, got {other:?}"),
        }
    }

    // Unlike `discover`, the strict `open` never walks up — a repository at an
    // ancestor of `dir` must NOT make `open(dir)` succeed, even though
    // `discover(dir)` would find it.
    #[test]
    fn open_does_not_walk_up_even_though_discover_would() {
        let tmp = TempDir::new("open-no-walkup");
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        match Repo::open(&nested) {
            Err(Error::NotARepository(p)) => assert_eq!(p, nested),
            other => panic!("expected Error::NotARepository, got {other:?}"),
        }
        // `discover` from the same nested dir finds the repo at `root`.
        assert_eq!(
            Repo::discover(&nested).expect("discover walks up").root(),
            root
        );
    }

    // --- dispatch (hermetic, ScriptedRunner-backed) ------------------------

    fn git_repo(runner: ScriptedRunner) -> Repo<ScriptedRunner> {
        Repo::from_git("/repo", "/repo", Git::with_runner(runner))
    }

    fn jj_repo(runner: ScriptedRunner) -> Repo<ScriptedRunner> {
        Repo::from_jj("/repo", "/repo", Jj::with_runner(runner))
    }

    // --- Debug -------------------------------------------------------------
    //
    // Regression tests for the `Repo`/`Backend` `Debug` impl (PR #7): formatting
    // the facade must show the elided shape (`Repo { .. }` with a `Git(..)`/
    // `Jj(..)` backend) but never expose the wrapped CLI client — and therefore
    // never a credential token that client might hold. These exist to catch a
    // future refactor that accidentally starts formatting the client (e.g.
    // deriving `Debug` on `Backend` directly, or dropping `finish_non_exhaustive`).

    // A git-backed `Repo` built over a `Git` client holding a token via
    // `with_token` must format to the expected elided shape and must NOT leak
    // the token (or any other inner-client internal) through `{:?}`.
    #[test]
    fn debug_output_shows_elided_git_backend_and_never_leaks_the_token() {
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()).with_token("ghp_super_secret_token"),
        );
        let out = format!("{repo:?}");
        assert!(out.contains("Repo {"), "{out}");
        assert!(out.contains("root"), "{out}");
        assert!(out.contains("cwd"), "{out}");
        assert!(out.contains("Git(.."), "{out}");
        assert!(
            !out.contains("ghp_super_secret_token"),
            "token must not leak through Debug: {out}"
        );
        // Nothing from the inner `Git`/`ManagedClient`/`CliClient` internals
        // (e.g. its env-var bookkeeping) should surface either — the backend
        // must render as a bare, elided discriminant.
        assert!(!out.contains("ManagedClient"), "{out}");
        assert!(!out.contains("CliClient"), "{out}");
    }

    // A jj-backed `Repo` (jj is ambient-auth-only — no `with_token`) must format
    // to the analogous elided shape, with the `Jj(..)` discriminant and no inner
    // client internals.
    #[test]
    fn debug_output_shows_elided_jj_backend() {
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(ScriptedRunner::new()));
        let out = format!("{repo:?}");
        assert!(out.contains("Repo {"), "{out}");
        assert!(out.contains("Jj(.."), "{out}");
        assert!(!out.contains("ManagedClient"), "{out}");
        assert!(!out.contains("CliClient"), "{out}");
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
                .on(["git", "status", "--porcelain=v2"], Reply::ok(v2))
                .on(
                    ["git", "rev-parse", "--git-dir"],
                    Reply::ok(gitdir.path().to_str().unwrap()),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        let tracking = s.tracking.as_ref().expect("upstream tracking");
        assert_eq!(tracking.branch, "origin/main");
        assert_eq!((tracking.ahead, tracking.behind), (Some(2), Some(0)));
        assert!(s.dirty);
        assert_eq!(s.change_count, 2, "1 tracked + 1 untracked");
        assert!(!s.conflicted);
        assert_eq!(s.operation, OperationState::Clear);
    }

    // M20 (whole-solution): `snapshot()` has its OWN operation probe (separate from
    // `in_progress_state`); it too must report a `git am` as `ApplyMailbox`, not
    // `Rebase` — otherwise the new variant is dead on the snapshot → watch → mcp path.
    #[tokio::test]
    async fn git_snapshot_reports_git_am_as_apply_mailbox() {
        let v2 = concat!("# branch.oid abc\0", "# branch.head main\0");
        let gitdir = TempDir::new("snap-git-am");
        // A `git am` in progress: `rebase-apply/` WITH the `applying` marker.
        let apply = gitdir.path().join("rebase-apply");
        std::fs::create_dir_all(&apply).unwrap();
        std::fs::write(apply.join("applying"), b"").unwrap();
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["git", "status", "--porcelain=v2"], Reply::ok(v2))
                .on(
                    ["git", "rev-parse", "--git-dir"],
                    Reply::ok(gitdir.path().to_str().unwrap()),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(
            s.operation,
            OperationState::ApplyMailbox,
            "a git am must not read as Rebase in snapshot()"
        );
    }

    // git with NO upstream configured: porcelain v2 omits the `# branch.upstream`
    // and `# branch.ab` lines, so `tracking` is None (the all-or-nothing invariant —
    // git is the only backend that can produce either) — mirrors the jj None case.
    #[tokio::test]
    async fn git_snapshot_without_upstream_has_no_tracking() {
        let v2 = concat!("# branch.oid abc123\0", "# branch.head main\0");
        let gitdir = TempDir::new("snap-git-noup");
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["git", "status", "--porcelain=v2"], Reply::ok(v2))
                .on(
                    ["git", "rev-parse", "--git-dir"],
                    Reply::ok(gitdir.path().to_str().unwrap()),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.tracking.is_none(), "no upstream → no tracking");
    }

    // M17: an upstream that is SET but GONE (deleted on the remote, or not yet
    // fetched) — porcelain v2 emits `# branch.upstream` but OMITS `# branch.ab`, so the
    // counts are uncountable. `tracking` must be `Some { branch, ahead: None, behind:
    // None }` (tracking configured but uncountable), NOT a fabricated in-sync `0`/`0`.
    #[tokio::test]
    async fn git_snapshot_upstream_set_but_gone_is_uncountable() {
        let v2 = concat!(
            "# branch.oid abc123\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0", // upstream named…
                                               // …but no `# branch.ab` line — it doesn't resolve.
        );
        let gitdir = TempDir::new("snap-git-gone");
        let repo = git_repo(
            ScriptedRunner::new()
                .on(["git", "status", "--porcelain=v2"], Reply::ok(v2))
                .on(
                    ["git", "rev-parse", "--git-dir"],
                    Reply::ok(gitdir.path().to_str().unwrap()),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        let tracking = s.tracking.as_ref().expect("upstream is set");
        assert_eq!(tracking.branch, "origin/main");
        assert_eq!(
            (tracking.ahead, tracking.behind),
            (None, None),
            "a gone upstream is uncountable, not in-sync 0/0"
        );
    }

    // jj: one template row + a status count; a conflicted @ maps to Conflict; no
    // git-style upstream/ahead/behind.
    #[tokio::test]
    async fn jj_snapshot_dirty_with_change_count() {
        let repo = jj_repo(
            ScriptedRunner::new()
                // snapshot template (`jj log -r @`): commit_id \t empty \t conflict
                .on(["jj", "log", "-r", "@"], Reply::ok("deadbeef\t0\t1\n")) // empty=0 dirty, conflict=1
                // `branch` via `current_branch` → `reachable_bookmarks`
                // (`jj log -r heads(::@ & bookmarks())`): bookmarks \t commit
                .on(
                    ["jj", "log", "-r", "heads(::@ & bookmarks())"],
                    Reply::ok("\"main\"\tdeadbeef\n"),
                )
                .on(["jj", "root"], Reply::ok("/repo\n"))
                .on(["jj", "diff"], Reply::ok("M a.rs\nA b.rs\n")), // status -r @ --summary → 2
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.head.as_deref(), Some("deadbeef"));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert!(s.dirty);
        assert_eq!(s.change_count, 2);
        assert!(s.conflicted);
        assert_eq!(s.operation, OperationState::Conflict);
        assert!(s.tracking.is_none(), "jj has no upstream tracking");
    }

    // jj: a clean `@` (empty=1) skips the change-count spawn entirely — the test
    // scripts NO `diff` rule, so calling `status` would error.
    #[tokio::test]
    async fn jj_snapshot_clean_skips_change_count() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(["jj", "log", "-r", "@"], Reply::ok("c0ffee\t1\t0\n"))
                .on(
                    ["jj", "log", "-r", "heads(::@ & bookmarks())"],
                    Reply::ok(""),
                ),
        );
        let s = repo.snapshot().await.unwrap();
        assert_eq!(s.head.as_deref(), Some("c0ffee"));
        assert_eq!(s.branch, None, "no bookmark");
        assert!(!s.dirty);
        assert_eq!(s.change_count, 0);
        assert!(!s.conflicted);
        assert_eq!(s.operation, OperationState::Clear);
    }

    // jj: a conflicted `@` that jj marks `empty` (conflict but no net content change)
    // is still reported `dirty` — the conflict is uncommitted state needing
    // resolution — so the count runs and the snapshot is coherent (no
    // `conflicted: true` next to `dirty: false`), mirroring git's conflict handling.
    #[tokio::test]
    async fn jj_snapshot_conflicted_empty_change_is_dirty() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(["jj", "log", "-r", "@"], Reply::ok("c0ffee\t1\t1\n")) // empty=1, conflict=1
                .on(
                    ["jj", "log", "-r", "heads(::@ & bookmarks())"],
                    Reply::ok(""),
                ) // no bookmark
                .on(["jj", "root"], Reply::ok("/repo\n"))
                .on(["jj", "diff"], Reply::ok("M conflicted.rs\n")), // status → 1
        );
        let s = repo.snapshot().await.unwrap();
        assert!(s.conflicted);
        assert!(s.dirty, "a conflicted change is a dirty working copy");
        assert_eq!(s.change_count, 1);
        assert_eq!(s.operation, OperationState::Conflict);
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
                    ["jj", "workspace", "list"],
                    Reply::ok("\"default\"\tc0ffee\t\"main\"\n\"ws1\"\tdecaf0\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "default",
                    ],
                    Reply::ok("/repo\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "ws1",
                    ],
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
                    ["jj", "workspace", "list"],
                    Reply::ok("\"default\"\tc0ffee\t\"main\"\n\"gone\"\tdecaf0\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "default",
                    ],
                    Reply::ok("/repo\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "gone",
                    ],
                    Reply::fail(1, "Error: No such workspace"),
                ),
        );
        let worktrees = repo.list_worktrees().await.expect("list_worktrees");
        assert_eq!(worktrees.len(), 1, "the unresolvable workspace is skipped");
        assert_eq!(worktrees[0].path, Path::new("/repo"));
    }

    // remove_worktree surfaces a `workspace forget` failure rather than swallowing
    // it — name resolution already proved the workspace is registered, so a forget
    // error is a real dangling-registration the caller should see.
    #[tokio::test]
    async fn jj_remove_worktree_surfaces_forget_error() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["jj", "workspace", "list"],
                    Reply::ok("\"ws1\"\tc0ffee\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "ws1",
                    ],
                    Reply::ok("/repo/ws1\n"),
                )
                .on(
                    ["jj", "workspace", "forget"],
                    Reply::fail(1, "Error: cannot forget workspace"),
                ),
        );
        // `/repo/ws1` does not exist on disk, so the dir-removal step is skipped and
        // the forget error is the sole outcome.
        let res = repo.remove_worktree(WorktreeRemove::new("/repo/ws1")).await;
        assert!(res.is_err(), "a forget failure is surfaced, not swallowed");
    }

    // Windows-like removal failure: `remove_worktree` surfaces a `remove_dir_all`
    // failure and names what remains (the still-registered workspace) rather than
    // swallowing it. A *file* sits where the workspace dir should be, so
    // `remove_dir_all` errors deterministically on every platform.
    #[tokio::test]
    async fn jj_remove_worktree_surfaces_dir_removal_failure() {
        let tmp = TempDir::new("rmw-rmdir-fail");
        let ws = tmp.path().join("ws1");
        std::fs::write(&ws, b"not a dir").expect("write file where the dir should be");
        let root = tmp.path().to_string_lossy().into_owned();
        let ws_str = ws.to_string_lossy().into_owned();
        let repo = Repo::from_jj(
            &root,
            &root,
            Jj::with_runner(
                ScriptedRunner::new()
                    .on(
                        ["jj", "workspace", "list"],
                        Reply::ok("\"ws1\"\tc0ffee\t\n"),
                    )
                    .on(
                        [
                            "jj",
                            "--ignore-working-copy",
                            "workspace",
                            "root",
                            "--name",
                            "ws1",
                        ],
                        Reply::ok(format!("{ws_str}\n")),
                    ),
            ),
        );
        // force skips the dirty check, so the removal step is reached directly.
        let err = repo
            .remove_worktree(WorktreeRemove::new(ws.clone()).force())
            .await
            .expect_err("a dir-removal failure must be surfaced");
        let msg = err.to_string();
        assert!(
            msg.contains("still registered") && msg.contains("ws1"),
            "the failure must name what remains to clean up: {msg}"
        );
        assert!(
            ws.exists(),
            "the undeletable path must survive the failed removal"
        );
    }

    // Compatible fallback / diagnosable error: when a registered workspace's root
    // can't be resolved via `workspace root --name`, a path matching none of the
    // resolvable ones is NOT reported as a clean `WorktreeNotFound` — absence can't be
    // proven, so a distinct diagnosable error naming the unresolved workspace is
    // raised instead (so a real-but-unresolvable workspace isn't misreported).
    #[tokio::test]
    async fn jj_remove_worktree_reports_unresolvable_workspaces() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["jj", "workspace", "list"],
                    Reply::ok("\"ws1\"\tc0ffee\t\n\"gone\"\tdecaf0\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "ws1",
                    ],
                    Reply::ok("/repo/ws1\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "gone",
                    ],
                    Reply::fail(1, "Error: No such workspace"),
                ),
        );
        let err = repo
            .remove_worktree(WorktreeRemove::new("/repo/missing"))
            .await
            .expect_err("an unresolvable workspace must not be reported as a clean not-found");
        assert!(
            !err.is_resource_not_found(),
            "a partial resolution is not a clean WorktreeNotFound: {err}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("could not resolve") && msg.contains("gone"),
            "the diagnosable error must name the unresolved workspace: {msg}"
        );
    }

    // Repeated cleanup is idempotent: after a first pass removed the directory but its
    // `workspace forget` failed, a retry finds the dir already gone, re-resolves the
    // still-registered workspace by name, and completes the forget — no error.
    #[tokio::test]
    async fn jj_remove_worktree_retry_after_dir_gone_forgets_cleanly() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["jj", "workspace", "list"],
                    Reply::ok("\"ws1\"\tc0ffee\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "ws1",
                    ],
                    Reply::ok("/repo/ws1\n"),
                )
                .on(["jj", "workspace", "forget"], Reply::ok("")),
        );
        // `/repo/ws1` does not exist on disk (a prior pass removed it), so the removal
        // step is skipped and the forget clears the dangling registration.
        repo.remove_worktree(WorktreeRemove::new("/repo/ws1"))
            .await
            .expect("a retry with the dir already gone completes the forget");
    }

    // C1: the default workspace resolves at the repo root; removing it would wipe
    // the whole repository, so it is refused even with force = true and WITHOUT
    // running `workspace forget` (no such cassette rule — a miss would also error,
    // so we assert the *refusal* message to prove the guard, not a fallthrough).
    #[tokio::test]
    async fn jj_remove_worktree_refuses_the_main_workspace() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(
                    ["jj", "workspace", "list"],
                    Reply::ok("\"default\"\tc0ffee\t\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "default",
                    ],
                    Reply::ok("/repo\n"),
                ),
        );
        let err = repo
            .remove_worktree(WorktreeRemove::new("/repo").force())
            .await
            .expect_err("the main workspace must be refused");
        assert!(
            err.to_string().contains("main workspace"),
            "refusal message, not a cassette miss: {err}"
        );
    }

    // C1: a secondary workspace with un-snapshotted edits (`current_change` reports
    // non-empty) is refused under force = false, and its directory is NOT deleted.
    #[tokio::test]
    async fn jj_remove_worktree_refuses_dirty_workspace_without_force() {
        let tmp = TempDir::new("rmw-dirty");
        let root = tmp.path().to_string_lossy().into_owned();
        let repo = Repo::from_jj(
            &root,
            &root,
            Jj::with_runner(
                ScriptedRunner::new()
                    .on(
                        ["jj", "workspace", "list"],
                        Reply::ok("\"ws1\"\tc0ffee\t\n"),
                    )
                    .on(
                        [
                            "jj",
                            "--ignore-working-copy",
                            "workspace",
                            "root",
                            "--name",
                            "ws1",
                        ],
                        Reply::ok(format!("{root}\n")),
                    )
                    // `current_change` → 3rd field `false` = not empty = dirty.
                    .on(["jj", "log"], Reply::ok("aaa\tbbb\tfalse\t\"work\"\n")),
            ),
        );
        let err = repo
            .remove_worktree(WorktreeRemove::new(tmp.path()))
            .await
            .expect_err("a dirty workspace must be refused without force");
        assert!(
            err.to_string().contains("uncommitted changes"),
            "refusal message: {err}"
        );
        assert!(
            tmp.path().exists(),
            "the workspace directory must survive a refusal"
        );
    }

    // C1: force = true skips the dirty check and removes the directory (no
    // `current_change` rule is scripted, proving the check is bypassed).
    #[tokio::test]
    async fn jj_remove_worktree_with_force_removes_the_dir() {
        let tmp = TempDir::new("rmw-force");
        let ws = tmp.path().join("ws1");
        std::fs::create_dir_all(&ws).expect("mkdir ws");
        let root = tmp.path().to_string_lossy().into_owned();
        let ws_str = ws.to_string_lossy().into_owned();
        let repo = Repo::from_jj(
            &root,
            &root,
            Jj::with_runner(
                ScriptedRunner::new()
                    .on(
                        ["jj", "workspace", "list"],
                        Reply::ok("\"ws1\"\tc0ffee\t\n"),
                    )
                    .on(
                        [
                            "jj",
                            "--ignore-working-copy",
                            "workspace",
                            "root",
                            "--name",
                            "ws1",
                        ],
                        Reply::ok(format!("{ws_str}\n")),
                    )
                    .on(["jj", "workspace", "forget"], Reply::ok("")),
            ),
        );
        repo.remove_worktree(WorktreeRemove::new(ws.clone()).force())
            .await
            .expect("force removes a dirty worktree");
        assert!(!ws.exists(), "the worktree directory was removed");
    }

    // C1: the main-workspace guard's store-directory branch — a workspace whose
    // name was changed away from `default` (via `jj workspace rename`) still owns
    // the object store (`.jj/repo` is a *directory*, not a secondary's file
    // pointer), so removal is refused even with force = true, and the dir survives.
    // Exercises the `|| .jj/repo.is_dir()` half of the guard (the name is not
    // `default`), which the name-based test can't reach.
    #[tokio::test]
    async fn jj_remove_worktree_refuses_renamed_store_owning_workspace() {
        let tmp = TempDir::new("rmw-store");
        std::fs::create_dir_all(tmp.path().join(".jj").join("repo")).expect("mk .jj/repo dir");
        let root = tmp.path().to_string_lossy().into_owned();
        let repo = Repo::from_jj(
            &root,
            &root,
            Jj::with_runner(
                ScriptedRunner::new()
                    .on(
                        ["jj", "workspace", "list"],
                        Reply::ok("\"mainws\"\tc0ffee\t\n"),
                    )
                    .on(
                        [
                            "jj",
                            "--ignore-working-copy",
                            "workspace",
                            "root",
                            "--name",
                            "mainws",
                        ],
                        Reply::ok(format!("{root}\n")),
                    ),
            ),
        );
        let err = repo
            .remove_worktree(WorktreeRemove::new(tmp.path()).force())
            .await
            .expect_err("a renamed store-owning workspace is still refused");
        assert!(
            err.to_string().contains("main workspace"),
            "refusal message: {err}"
        );
        assert!(
            tmp.path().exists(),
            "the store-owning directory must not be deleted"
        );
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
        // git's `current_branch` now runs `symbolic-ref --quiet --short HEAD`:
        // exit 0 → the branch name, exit 1 → detached HEAD → None.
        let named =
            git_repo(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")));
        assert_eq!(
            named.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
        let detached =
            git_repo(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::fail(1, "")));
        assert!(detached.current_branch().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn changed_files_maps_git_status() {
        let repo = git_repo(ScriptedRunner::new().on(
            ["git", "status"],
            Reply::ok(" M a.rs\0?? b.rs\0R  new.rs\0old.rs\0"),
        ));
        let changes = repo.changed_files().await.unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[1].kind, ChangeKind::Added);
        assert_eq!(changes[2].kind, ChangeKind::Renamed);
        assert_eq!(changes[2].old_path.as_deref(), Some(Path::new("old.rs")));
    }

    #[tokio::test]
    async fn local_branches_maps_git_branch_output() {
        let repo =
            git_repo(ScriptedRunner::new().on(["git", "branch"], Reply::ok("* main\n  feat\n")));
        assert_eq!(repo.local_branches().await.unwrap(), ["main", "feat"]);
    }

    #[tokio::test]
    async fn branch_exists_reads_show_ref_exit() {
        let yes = git_repo(ScriptedRunner::new().on(["git", "show-ref"], Reply::ok("")));
        assert!(yes.branch_exists("main").await.unwrap());
        let no = git_repo(ScriptedRunner::new().on(["git", "show-ref"], Reply::fail(1, "")));
        assert!(!no.branch_exists("nope").await.unwrap());
    }

    #[tokio::test]
    async fn has_uncommitted_changes_reflects_status() {
        let dirty = git_repo(ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0")));
        assert!(dirty.has_uncommitted_changes().await.unwrap());
        let clean = git_repo(ScriptedRunner::new().on(["git", "status"], Reply::ok("")));
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
        // current_branch derives from `reachable_bookmarks`, whose template is
        // `<bookmarks space-joined>\t<commit>` — distinct from the strict
        // `current_bookmark(@)` comma-joined template.
        let repo =
            jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("\"main\"\t53e4e879\n")));
        assert_eq!(
            repo.current_branch().await.unwrap().as_deref(),
            Some("main")
        );
    }

    #[tokio::test]
    async fn jj_current_branch_persists_across_commit() {
        // After a jj commit the new working-copy change carries no bookmark, but
        // the described parent does. `reachable_bookmarks` resolves the nearest
        // bookmarked ancestor, so the facade still reports it — git-like "I'm
        // still on my branch". Under the old strict `current_bookmark(@)` rule
        // this returned `None`; feeding the reachable template (`feat\t…`,
        // unparseable as a comma-joined bookmark name) pins the new derivation.
        let repo =
            jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("\"feat\"\tc8d49332\n")));
        assert_eq!(
            repo.current_branch().await.unwrap().as_deref(),
            Some("feat")
        );
    }

    #[tokio::test]
    async fn jj_current_branch_tie_break_is_deterministic() {
        // `heads(::@ & bookmarks())` can yield several equally-near bookmarks —
        // a merge of two bookmarked lines (one row each) or one commit carrying
        // several (one row, space-joined). current_branch returns the
        // lexicographically-smallest name regardless of jj's row order, so the
        // result is stable. Here: rows `zeta` then `alpha beta` ⇒ `alpha`.
        let repo = jj_repo(ScriptedRunner::new().on(
            ["jj", "log"],
            Reply::ok("\"zeta\"\tabc1234\n\"alpha\" \"beta\"\tdef5678\n"),
        ));
        assert_eq!(
            repo.current_branch().await.unwrap().as_deref(),
            Some("alpha")
        );
    }

    #[tokio::test]
    async fn jj_local_branches_maps_bookmark_list() {
        // BOOKMARK_LIST_TEMPLATE rows: `<present>\t<remote>\t"<name>"\t<commit>`.
        let repo = jj_repo(ScriptedRunner::new().on(
            ["jj", "bookmark", "list"],
            Reply::ok("1\t\t\"main\"\tcmt\n1\t\t\"feat\"\tm2\n"),
        ));
        assert_eq!(repo.local_branches().await.unwrap(), ["main", "feat"]);
    }

    #[tokio::test]
    async fn jj_branch_exists_scans_bookmarks() {
        let repo = jj_repo(ScriptedRunner::new().on(
            ["jj", "bookmark", "list"],
            Reply::ok("1\t\t\"main\"\tcmt\n"),
        ));
        assert!(repo.branch_exists("main").await.unwrap());
        let repo2 = jj_repo(ScriptedRunner::new().on(
            ["jj", "bookmark", "list"],
            Reply::ok("1\t\t\"main\"\tcmt\n"),
        ));
        assert!(!repo2.branch_exists("missing").await.unwrap());
    }

    #[tokio::test]
    async fn jj_has_uncommitted_changes_reads_empty_flag() {
        // CHANGE_TEMPLATE row: change_id \t commit_id \t empty \t description
        let dirty =
            jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("kz\t38\tfalse\t\"wip\"\n")));
        assert!(dirty.has_uncommitted_changes().await.unwrap());
        let clean =
            jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("kz\t38\ttrue\t\"\"\n")));
        assert!(!clean.has_uncommitted_changes().await.unwrap());
    }

    // M18: a conflicted-but-**empty** `@` is uncommitted state (it needs resolution),
    // so `has_uncommitted_changes` returns true — agreeing with `snapshot().dirty`,
    // which already treats `conflict ⇒ dirty`. First `jj log` = current_change (empty),
    // second = is_conflicted (`"1"`).
    #[tokio::test]
    async fn jj_has_uncommitted_changes_true_when_conflicted_even_if_empty() {
        let repo = jj_repo(ScriptedRunner::new().on_sequence(
            ["jj", "log"],
            [
                Reply::ok("kz\t38\ttrue\t\"\"\n"), // current_change: empty = true
                Reply::ok("1\n"),                  // is_conflicted: conflicted
            ],
        ));
        assert!(
            repo.has_uncommitted_changes().await.unwrap(),
            "a conflicted empty @ is dirty"
        );
    }

    #[tokio::test]
    async fn jj_changed_files_maps_diff_summary() {
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(["jj", "root"], Reply::ok("/repo\n"))
                .on(["jj", "diff"], Reply::ok("M src/a.rs\nA b.rs\nD gone.rs\n")),
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
        let repo = jj_repo(
            ScriptedRunner::new()
                .on(["jj", "root"], Reply::ok("/repo\n"))
                .on(["jj", "diff"], Reply::ok("R src/{old.rs => new.rs}\n")),
        );
        let changes = repo.changed_files().await.unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Renamed);
        assert_eq!(changes[0].path, Path::new("src/new.rs"));
        assert_eq!(
            changes[0].old_path.as_deref(),
            Some(Path::new("src/old.rs"))
        );
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
        use processkit::testing::RecordingRunner;
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
        use processkit::testing::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .checkout("feat")
            .await
            .unwrap();
        // Trailing `--` so a path-like ref can't fall into pathspec mode (C2).
        assert_eq!(grec.only_call().args_str(), ["checkout", "feat", "--"]);

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
    async fn new_child_dispatches_per_backend() {
        use processkit::testing::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .new_child("feat")
            .await
            .unwrap();
        assert_eq!(grec.only_call().args_str(), ["checkout", "feat", "--"]);

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .new_child("feat")
            .await
            .unwrap();
        assert_eq!(
            jrec.only_call().args_str(),
            ["new", "feat", "--color", "never"]
        );
    }

    // A1: `delete_branch` takes a `BranchDelete` spec; `.force()` threads through to
    // git's `-D` (vs `-d`), and jj ignores it (its `bookmark delete` has no force).
    #[tokio::test]
    async fn delete_branch_spec_threads_force_to_git_only() {
        use processkit::testing::RecordingRunner;
        let forced = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&forced))
            .delete_branch(BranchDelete::new("feat").force())
            .await
            .unwrap();
        assert!(
            forced.only_call().args_str().iter().any(|a| a == "-D"),
            "force → branch -D"
        );

        let unforced = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&unforced))
            .delete_branch(BranchDelete::new("feat"))
            .await
            .unwrap();
        assert!(
            unforced.only_call().args_str().iter().any(|a| a == "-d"),
            "no force → branch -d"
        );

        let jj = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jj))
            .delete_branch(BranchDelete::new("feat").force())
            .await
            .unwrap();
        assert!(
            !jj.only_call()
                .args_str()
                .iter()
                .any(|a| a == "-D" || a == "--force"),
            "jj bookmark delete has no force flag"
        );
    }

    #[tokio::test]
    async fn fetch_branch_dispatches_per_backend() {
        use processkit::testing::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .fetch_branch("main")
            .await
            .unwrap();
        assert!(
            grec.only_call()
                .args_str()
                .starts_with(&["fetch".to_string()])
        );

        let jrec = RecordingRunner::replying(Reply::ok(""));
        Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .fetch_branch("main")
            .await
            .unwrap();
        let args = jrec.only_call().args_str();
        assert_eq!(&args[..2], &["git", "fetch"]);
    }

    // The facade push is the honest LCD: git pushes the ref with `-u origin`,
    // jj pushes the bookmark's state with `-b`. Argv pinned on both backends.
    #[tokio::test]
    async fn push_dispatches_per_backend() {
        use processkit::testing::RecordingRunner;
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
        // `exact:` disables jj's glob matching so a `*` can't push every bookmark (H1).
        assert_eq!(&args[..4], &["git", "push", "-b", "exact:feature"]);
    }

    // A flag-like branch is now rejected the same way on BOTH backends: the
    // facade converts the branch string into the validated newtype at the
    // boundary (`vcs_git::RefName` / `vcs_jj::BookmarkName`), so `--force` is
    // refused with a classifiable input-validation error BEFORE any process
    // spawns — no longer a per-backend difference.
    #[tokio::test]
    async fn push_flag_like_branch_rejected_before_spawn_on_both_backends() {
        use processkit::testing::RecordingRunner;
        let grec = RecordingRunner::replying(Reply::ok(""));
        let err = Repo::from_git("/repo", "/repo", Git::with_runner(&grec))
            .push("--force")
            .await
            .unwrap_err();
        assert!(err.is_invalid_input(), "git: got {err:?}");
        assert_eq!(grec.calls().len(), 0, "git: no process must have spawned");

        let jrec = RecordingRunner::replying(Reply::ok(""));
        let err = Repo::from_jj("/repo", "/repo", Jj::with_runner(&jrec))
            .push("--force")
            .await
            .unwrap_err();
        assert!(err.is_invalid_input(), "jj: got {err:?}");
        assert_eq!(jrec.calls().len(), 0, "jj: no process must have spawned");
    }

    #[tokio::test]
    async fn fetch_from_names_the_remote_on_both_backends() {
        use processkit::testing::RecordingRunner;
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
        // `exact:` disables jj's glob matching on the remote name (H1).
        assert_eq!(&args[..4], &["git", "fetch", "--remote", "exact:upstream"]);
    }

    // git: untracked files count as uncommitted but not as *tracked* changes.
    #[tokio::test]
    async fn git_has_tracked_changes_ignores_untracked() {
        let dirty = git_repo(ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0")));
        assert!(dirty.has_tracked_changes().await.unwrap());
        // `--untracked-files=no` means git itself omits `??` entries; an empty
        // reply is what a tracked-clean tree returns.
        let clean = git_repo(ScriptedRunner::new().on(["git", "status"], Reply::ok("")));
        assert!(!clean.has_tracked_changes().await.unwrap());
    }

    // jj has no untracked concept — `has_tracked_changes` follows `@`'s emptiness.
    #[tokio::test]
    async fn jj_has_tracked_changes_follows_working_copy() {
        let dirty =
            jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("kz\t38\tfalse\t\"wip\"\n")));
        assert!(dirty.has_tracked_changes().await.unwrap());
    }

    #[tokio::test]
    async fn conflicted_files_dispatches_per_backend() {
        let git =
            git_repo(ScriptedRunner::new().on(["git", "diff"], Reply::ok("a.rs\0b dir/c.rs\0")));
        assert_eq!(
            git.conflicted_files().await.unwrap(),
            [PathBuf::from("a.rs"), PathBuf::from("b dir/c.rs")]
        );

        let jj = jj_repo(
            ScriptedRunner::new().on(["jj", "resolve"], Reply::ok("a.rs    2-sided conflict\n")),
        );
        assert_eq!(
            jj.conflicted_files().await.unwrap(),
            [PathBuf::from("a.rs")]
        );
        // The benign "no conflicts" non-zero exit still reads as an empty list.
        let clean = jj_repo(ScriptedRunner::new().on(
            ["jj", "resolve"],
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
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "merge"], Reply::ok("Already up to date.\n"))
                .on(["git", "rev-parse"], Reply::ok("/vcs-core-no-such-git-dir")),
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
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                // Order matters: ["merge","--abort"] must outrank the ["merge"] rule.
                .on(["git", "merge", "--abort"], Reply::ok(""))
                .on(
                    ["git", "merge"],
                    Reply::fail(1, "CONFLICT (content): Merge conflict in a.rs"),
                )
                .on(["git", "diff"], Reply::ok("a.rs\0")),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        assert_eq!(
            repo.try_merge("other").await.unwrap(),
            MergeProbe::Conflicts(vec![PathBuf::from("a.rs")])
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
                    ["git", "merge", "--abort"],
                    Reply::fail(128, "fatal: cannot abort"),
                )
                .on(["git", "merge"], Reply::ok(""))
                .on(
                    ["git", "rev-parse"],
                    Reply::ok(tmp.path().to_str().unwrap()),
                ),
        );
        assert!(repo.try_merge("other").await.is_err());
    }

    // A thin shim over the standard `RecordingRunner`/`ScriptedRunner` that fires a
    // cancellation token the instant a command whose argv satisfies `trip` is
    // dispatched — modelling the client's `default_cancel_on` firing at a *precise*
    // point during `try_merge`. Needed because a plain scripted reply cannot express
    // this: an already-fired token short-circuits the *first* command (so the later
    // stages are never reached), and the harness has no mid-sequence hook. Choosing
    // `trip` pins the exact moment — at the rollback abort, or earlier, at the
    // in-progress probe.
    struct CancelWhen<R: ProcessRunner> {
        inner: R,
        token: CancellationToken,
        trip: fn(&processkit::Command) -> bool,
    }

    #[async_trait::async_trait]
    impl<R: ProcessRunner> ProcessRunner for CancelWhen<R> {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            // Fire the client token as `trip` selects. A detached cleanup command
            // (fresh token) survives it; a token-inheriting one is cancelled. Firing
            // during command N's dispatch leaves the token fired for command N+1,
            // which `ScriptedRunner` short-circuits when the token is inherited.
            if (self.trip)(command) {
                self.token.cancel();
            }
            self.inner.output_string(command).await
        }
    }

    fn arg_present(command: &processkit::Command, needle: &str) -> bool {
        command
            .arguments()
            .iter()
            .any(|a| a.to_str() == Some(needle))
    }

    // The facade `try_merge`'s rollback must survive the client's cancellation
    // firing as it reaches cleanup — the git analogue of jj's unit test
    // `rollback_to_survives_fired_cancellation`. With the fix, the cleanup
    // `merge --abort` runs on a FRESH cancel token and completes (→ `Clean`); the
    // old token-inheriting abort would be cancelled and surface `Error::Cancelled`,
    // abandoning the staged trial merge — so this test fails on the pre-fix code.
    #[tokio::test]
    async fn git_try_merge_cleanup_survives_cancellation_fired_at_rollback() {
        use processkit::testing::RecordingRunner;
        let tmp = TempDir::new("probe-cancel");
        std::fs::write(tmp.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let token = CancellationToken::new();
        let runner = CancelWhen {
            inner: RecordingRunner::new(
                ScriptedRunner::new()
                    .on(["git", "merge", "--abort"], Reply::ok(""))
                    .on(["git", "merge"], Reply::ok(""))
                    .on(
                        ["git", "rev-parse"],
                        Reply::ok(tmp.path().to_str().unwrap()),
                    ),
            ),
            token: token.clone(),
            // Fire exactly as the rollback abort is issued.
            trip: |c| arg_present(c, "--abort"),
        };
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(&runner).default_cancel_on(token),
        );
        // The probe merge is clean and MERGE_HEAD is present, so `try_merge` reaches
        // the cleanup abort — which runs on a fresh cancel token and completes
        // despite the client cancellation the shim fires at that exact moment.
        assert_eq!(repo.try_merge("other").await.unwrap(), MergeProbe::Clean);
        // ...and the detached abort really was issued, not skipped.
        assert!(
            runner
                .inner
                .calls()
                .iter()
                .any(|c| c.args_str().contains(&"--abort".to_string())),
            "the cleanup abort must have run: {:?}",
            runner.inner.calls()
        );
    }

    // The gap R-01 caught: the cleanup DECISION — `is_merge_in_progress`, whose
    // `rev-parse --git-dir` used to inherit the client token — must also survive a
    // cancellation that fires DURING the probe, before the abort is ever reached.
    // Here the token fires as that `rev-parse --git-dir` is dispatched (the Ok
    // branch: the `--no-ff` probe merge staged a real merge, then the deadline
    // hit). With the fix the probe runs detached (fresh token) → sees MERGE_HEAD →
    // the detached abort runs → `Clean`. On the pre-fix code the probe's `?`
    // propagated `Cancelled` and the abort was skipped, abandoning the staged trial
    // merge — so this test fails there. Firing at the probe, not at `--abort`, is
    // exactly what the older `..._fired_at_rollback` test could not cover.
    #[tokio::test]
    async fn git_try_merge_cleanup_survives_cancellation_fired_at_probe() {
        use processkit::testing::RecordingRunner;
        let tmp = TempDir::new("probe-cancel-at-probe");
        std::fs::write(tmp.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let token = CancellationToken::new();
        let runner = CancelWhen {
            inner: RecordingRunner::new(
                ScriptedRunner::new()
                    .on(["git", "merge", "--abort"], Reply::ok(""))
                    .on(["git", "merge"], Reply::ok(""))
                    .on(
                        ["git", "rev-parse"],
                        Reply::ok(tmp.path().to_str().unwrap()),
                    ),
            ),
            token: token.clone(),
            // Fire as the in-progress probe's `rev-parse --git-dir` is dispatched —
            // strictly BEFORE the abort, unlike `..._fired_at_rollback`.
            trip: |c| arg_present(c, "--git-dir"),
        };
        let repo = Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(&runner).default_cancel_on(token),
        );
        // Decision + abort both run on fresh tokens, so the probe still reports the
        // staged merge and the abort still undoes it despite the fired client token.
        assert_eq!(repo.try_merge("other").await.unwrap(), MergeProbe::Clean);
        assert!(
            runner
                .inner
                .calls()
                .iter()
                .any(|c| c.args_str().contains(&"--abort".to_string())),
            "cleanup abort must run even when cancellation fires at the probe: {:?}",
            runner.inner.calls()
        );
    }

    // jj try_merge: op head captured first, probe runs, op restore always runs.
    #[tokio::test]
    async fn jj_try_merge_probes_and_restores() {
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["jj", "op", "log"], Reply::ok("op42\n"))
                .on(["jj", "op", "restore"], Reply::ok(""))
                .on(["jj", "new"], Reply::ok(""))
                .on(["jj", "log"], Reply::ok("1\n")) // is_conflicted → true
                .on(["jj", "resolve"], Reply::ok("a.rs    2-sided conflict\n")),
        );
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(&rec));
        assert_eq!(
            repo.try_merge("feature").await.unwrap(),
            MergeProbe::Conflicts(vec![PathBuf::from("a.rs")])
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
                .on(["jj", "op", "log"], Reply::ok("op42\n"))
                .on(["jj", "op", "restore"], Reply::ok(""))
                .on(["jj", "new"], Reply::ok(""))
                .on(["jj", "log"], Reply::ok("0\n")),
        );
        assert_eq!(clean.try_merge("feature").await.unwrap(), MergeProbe::Clean);

        // A failing op restore breaks the rollback guarantee → error, not Clean.
        let broken = jj_repo(
            ScriptedRunner::new()
                .on(["jj", "op", "log"], Reply::ok("op42\n"))
                .on(["jj", "op", "restore"], Reply::fail(1, "op not found"))
                .on(["jj", "new"], Reply::ok(""))
                .on(["jj", "log"], Reply::ok("0\n")),
        );
        assert!(broken.try_merge("feature").await.is_err());
    }

    // jj try_merge shares `Jj::rollback_to`'s concurrency guard: if a concurrent jj
    // process advances the op log during the trial merge (jj records a `>= 2`-parent
    // "reconcile divergent operations" merge), the rollback is REFUSED rather than
    // clobbering that work — try_merge surfaces `Error::Rollback` instead of a stale,
    // untrustworthy `Clean`, and issues no `op restore`.
    #[tokio::test]
    async fn jj_try_merge_refuses_rollback_on_op_log_divergence() {
        use processkit::testing::RecordingRunner;
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on_sequence(
                    ["jj", "op", "log"],
                    [
                        Reply::ok("op42\n"),              // capture → pre
                        Reply::ok("merge\t2\nop42\t1\n"), // probe → foreign reconcile merge
                    ],
                )
                .on(["jj", "op", "restore"], Reply::ok(""))
                .on(["jj", "new"], Reply::ok(""))
                .on(["jj", "log"], Reply::ok("0\n")),
        );
        let repo = Repo::from_jj("/repo", "/repo", Jj::with_runner(&rec));
        let err = repo
            .try_merge("feature")
            .await
            .expect_err("a divergence must error, not report a stale Clean");
        assert!(
            matches!(err, Error::Rollback(vcs_jj::Rollback::SkippedDiverged)),
            "expected Error::Rollback(SkippedDiverged), got {err:?}"
        );
        assert!(
            rec.calls()
                .iter()
                .all(|c| c.args_str()[..2] != ["op", "restore"]),
            "the concurrent op must not be clobbered by a restore: {:?}",
            rec.calls()
        );
    }

    // continue_in_progress with unresolved paths reports `Conflict` and must NOT
    // attempt the continue (git would hard-error).
    #[tokio::test]
    async fn git_continue_blocked_by_conflicts_does_not_act() {
        use processkit::testing::RecordingRunner;
        let rec =
            RecordingRunner::new(ScriptedRunner::new().on(["git", "diff"], Reply::ok("a.rs\0")));
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
                .on(["git", "diff"], Reply::ok(""))
                .on(
                    ["git", "rev-parse"],
                    Reply::ok(tmp.path().to_str().unwrap()),
                )
                .on(
                    ["git", "rebase", "--continue"],
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
        use processkit::testing::RecordingRunner;
        let tmp = TempDir::new("abort");
        std::fs::write(tmp.path().join("MERGE_HEAD"), "deadbeef\n").unwrap();
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(
                    ["git", "rev-parse"],
                    Reply::ok(tmp.path().to_str().unwrap()),
                )
                .on(["git", "merge", "--abort"], Reply::ok("")),
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
            Git::with_runner(ScriptedRunner::new().on(
                ["git", "rev-parse"],
                Reply::ok(merging.path().to_str().unwrap()),
            )),
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
            Git::with_runner(ScriptedRunner::new().on(
                ["git", "rev-parse"],
                Reply::ok(rebasing.path().to_str().unwrap()),
            )),
        );
        assert_eq!(
            rebase_repo.in_progress_state().await.unwrap(),
            OperationState::Rebase
        );
    }

    // T-044: the sequencer states are read from their own git-dir markers and,
    // crucially, a cherry-pick/revert marker is NOT mistaken for a merge (which
    // would then dispatch `merge --abort`). `snapshot().operation` must agree with
    // `in_progress_state`, since the watcher diffs the snapshot.
    #[tokio::test]
    async fn git_in_progress_state_maps_cherry_pick_revert_and_bisect() {
        for (marker, expected) in [
            ("CHERRY_PICK_HEAD", OperationState::CherryPick),
            ("REVERT_HEAD", OperationState::Revert),
            ("BISECT_LOG", OperationState::Bisect),
        ] {
            let gd = TempDir::new("inprog-seq");
            std::fs::write(gd.path().join(marker), "deadbeef\n").unwrap();
            let repo = Repo::from_git(
                "/repo",
                "/repo",
                // `snapshot` also runs `status --porcelain=v2 --branch`; a clean reply
                // lets it reach the operation probe. Both methods resolve the git dir
                // via `rev-parse`.
                Git::with_runner(
                    ScriptedRunner::new()
                        .on(["git", "status"], Reply::ok(""))
                        .on(["git", "rev-parse"], Reply::ok(gd.path().to_str().unwrap())),
                ),
            );
            assert_eq!(
                repo.in_progress_state().await.unwrap(),
                expected,
                "{marker} must read as {expected:?}"
            );
            assert_eq!(
                repo.snapshot().await.unwrap().operation,
                expected,
                "snapshot().operation must agree for {marker}"
            );
        }
    }

    // T-044: abort dispatches the state's OWN git command — the whole point of
    // keeping the states distinct. A cherry-pick must abort with `cherry-pick
    // --abort`, never `merge --abort`.
    #[tokio::test]
    async fn git_abort_dispatches_each_sequencer_command() {
        use processkit::testing::RecordingRunner;
        for (marker, argv) in [
            ("CHERRY_PICK_HEAD", vec!["cherry-pick", "--abort"]),
            ("REVERT_HEAD", vec!["revert", "--abort"]),
            ("BISECT_LOG", vec!["bisect", "reset"]),
        ] {
            let gd = TempDir::new("abort-seq");
            let marker_path = gd.path().join(marker);
            std::fs::write(&marker_path, "x\n").unwrap();
            // The abort command's ScriptedRunner side-effect: remove the marker so the
            // *post-call* `in_progress_state` re-probe reads `Clear`.
            let mp = marker_path.clone();
            let rec = RecordingRunner::new(
                ScriptedRunner::new()
                    .on(["git", "rev-parse"], Reply::ok(gd.path().to_str().unwrap()))
                    .when(
                        move |cmd| {
                            let a0 = cmd.arguments().first().and_then(|a| a.to_str());
                            let is_abort = matches!(a0, Some("cherry-pick" | "revert" | "bisect"));
                            if is_abort {
                                let _ = std::fs::remove_file(&mp);
                            }
                            is_abort
                        },
                        Reply::ok(""),
                    ),
            );
            let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
            assert_eq!(
                repo.abort_in_progress().await.unwrap(),
                OperationState::Clear,
                "{marker} abort must leave the repo Clear"
            );
            assert!(
                rec.calls().iter().any(|c| c.args_str() == argv),
                "{marker} must dispatch {argv:?}, got {:?}",
                rec.calls()
            );
        }
    }

    // T-044: a bisect has no continue step — `continue_in_progress` must refuse it
    // with `Error::Unsupported`, not silently report it still in progress. And no
    // git mutation may run (only the conflict probe + git-dir resolution).
    #[tokio::test]
    async fn git_continue_on_bisect_is_unsupported_and_inert() {
        use processkit::testing::RecordingRunner;
        let gd = TempDir::new("continue-bisect");
        std::fs::write(gd.path().join("BISECT_LOG"), "x\n").unwrap();
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "diff"], Reply::ok("")) // no conflicted paths
                .on(["git", "rev-parse"], Reply::ok(gd.path().to_str().unwrap())),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        let err = repo
            .continue_in_progress()
            .await
            .expect_err("bisect continue must be refused");
        assert!(err.is_unsupported(), "expected Unsupported, got {err:?}");
        assert!(
            rec.calls()
                .iter()
                .all(|c| matches!(c.args_str()[0].as_str(), "diff" | "rev-parse")),
            "no git mutation may run for an unsupported continue: {:?}",
            rec.calls()
        );
    }

    // T-044: a cherry-pick that continues cleanly commits and reports the post-call
    // state; the routing calls `cherry-pick --continue`, not a merge/rebase continue.
    #[tokio::test]
    async fn git_continue_dispatches_cherry_pick_continue() {
        use processkit::testing::RecordingRunner;
        let gd = TempDir::new("continue-cp");
        let marker = gd.path().join("CHERRY_PICK_HEAD");
        std::fs::write(&marker, "x\n").unwrap();
        let mp = marker.clone();
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "diff"], Reply::ok("")) // nothing conflicted → not blocked
                .on(["git", "rev-parse"], Reply::ok(gd.path().to_str().unwrap()))
                .when(
                    move |cmd| {
                        let is_cont =
                            cmd.arguments().first().and_then(|a| a.to_str()) == Some("cherry-pick");
                        if is_cont {
                            let _ = std::fs::remove_file(&mp); // completes the pick
                        }
                        is_cont
                    },
                    Reply::ok(""),
                ),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        assert_eq!(
            repo.continue_in_progress().await.unwrap(),
            OperationState::Clear
        );
        assert!(
            rec.calls()
                .iter()
                .any(|c| c.args_str() == ["cherry-pick", "--continue"]),
            "must dispatch cherry-pick --continue: {:?}",
            rec.calls()
        );
    }

    // On an unborn git repo (no commits) diff_stat probes is_unborn and stats
    // against the empty tree instead of the unresolvable HEAD, so a fresh working
    // tree reports its additions rather than erroring. The empty-tree id is
    // resolved from git (`hash-object`), so it tracks the repo's object format
    // rather than being a hard-coded SHA-1 value.
    #[tokio::test]
    async fn git_diff_stat_unborn_uses_empty_tree() {
        use processkit::testing::RecordingRunner;
        // A SHA-256 repo's empty-tree id (64 hex): the value `hash-object` returns,
        // which `diff_stat` must then target verbatim.
        let oid = "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321";
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "rev-parse"], Reply::fail(1, "")) // HEAD unborn
                .on(["git", "hash-object"], Reply::ok(format!("{oid}\n")))
                .on(
                    ["git", "diff", "--shortstat"],
                    Reply::ok(" 1 file changed, 2 insertions(+)\n"),
                ),
        );
        let repo = Repo::from_git("/repo", "/repo", Git::with_runner(&rec));
        let stat = repo.diff_stat().await.unwrap();
        assert_eq!(stat.insertions, 2);
        assert!(
            rec.calls()
                .iter()
                .any(|c| c.args_str() == ["diff", "--shortstat", oid, "--"]),
            "diff_stat should target the resolved empty tree on an unborn repo: {:?}",
            rec.calls()
        );
    }

    // `Repo::log` on git maps `GitApi::log`'s typed `Commit` (hash/author/date/
    // subject) onto the facade `Commit`, with author/date populated.
    #[tokio::test]
    async fn git_log_maps_commit_fields() {
        let repo = git_repo(ScriptedRunner::new().on(
            ["git", "log"],
            Reply::ok("deadbeef\u{1f}dead\u{1f}Jane\u{1f}2026-05-31T10:00:00+00:00\u{1f}Fix bug\0"),
        ));
        let commits = repo.log("HEAD", 10).await.unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].id, "deadbeef");
        assert_eq!(commits[0].description, "Fix bug");
        assert_eq!(commits[0].author.as_deref(), Some("Jane"));
        assert_eq!(
            commits[0].date.as_deref(),
            Some("2026-05-31T10:00:00+00:00")
        );
    }

    // `Repo::log` on jj maps `JjApi::log`'s typed `Change` (change-id/commit-id/
    // empty/description) onto the facade `Commit` — author/date stay `None`, since
    // jj's typed log doesn't surface them.
    #[tokio::test]
    async fn jj_log_maps_change_with_no_author_or_date() {
        let repo = jj_repo(ScriptedRunner::new().on(
            ["jj", "log"],
            Reply::ok("kztuxlro\t38e00654\tfalse\t\"wip\"\n"),
        ));
        let commits = repo.log("@", 10).await.unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].id, "38e00654");
        assert_eq!(commits[0].description, "wip");
        assert_eq!(commits[0].author, None);
        assert_eq!(commits[0].date, None);
    }

    // `Repo::show_file` on git dispatches to `GitApi::show_file` and forwards its
    // content verbatim.
    #[tokio::test]
    async fn git_show_file_dispatches_to_git_backend() {
        let repo = git_repo(ScriptedRunner::new().on(["git", "show"], Reply::ok("fn main() {}\n")));
        let content = repo.show_file("HEAD", "src/main.rs").await.unwrap();
        assert_eq!(content, "fn main() {}\n");
    }

    // `Repo::show_file` on jj dispatches to `JjApi::file_show` and forwards its
    // content verbatim.
    #[tokio::test]
    async fn jj_show_file_dispatches_to_jj_backend() {
        let repo =
            jj_repo(ScriptedRunner::new().on(["jj", "file", "show"], Reply::ok("fn main() {}\n")));
        let content = repo.show_file("@-", "src/main.rs").await.unwrap();
        assert_eq!(content, "fn main() {}\n");
    }

    // On jj, abort/continue are reporting no-ops (nothing is ever paused).
    #[tokio::test]
    async fn jj_abort_and_continue_are_reporting_noops() {
        let conflicted = jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("1\n")));
        assert_eq!(
            conflicted.abort_in_progress().await.unwrap(),
            OperationState::Conflict
        );
        let clear = jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("0\n")));
        assert_eq!(
            clear.continue_in_progress().await.unwrap(),
            OperationState::Clear
        );
    }

    // jj records conflicts on the change; the facade maps that to `Conflict`.
    #[tokio::test]
    async fn jj_in_progress_state_maps_conflict() {
        let conflicted = jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("1\n")));
        assert_eq!(
            conflicted.in_progress_state().await.unwrap(),
            OperationState::Conflict
        );
        let clear = jj_repo(ScriptedRunner::new().on(["jj", "log"], Reply::ok("0\n")));
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
                .on(["git", "symbolic-ref"], Reply::ok("main\n"))
                .on(["git", "show-ref"], Reply::ok("")),
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
                .on(["git", "symbolic-ref"], Reply::fail(1, "")) // origin/HEAD unset → None
                .on(["git", "show-ref"], Reply::ok("")), // branch_exists("main") → exit 0
        );
        assert_eq!(repo.trunk().await.unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn error_classifiers_recognise_markers() {
        let conflict = Error::Vcs(processkit::Error::exit(
            "git",
            1,
            "CONFLICT (content): Merge conflict in a.rs",
            "",
        ));
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
