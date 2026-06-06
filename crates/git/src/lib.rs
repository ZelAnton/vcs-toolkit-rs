//! `vcs-git` — automate Git from Rust through CLI process execution.
//!
//! Async, mockable, and structured-error: consumers depend on the [`GitApi`]
//! trait and substitute a mock for the real [`Git`] client in tests. Commands
//! run inside an OS job (via [`processkit`]) so a `git` subprocess is never
//! orphaned, and honour an optional [timeout](Git::default_timeout).
//!
//! ```no_run
//! use vcs_git::{Git, GitApi};
//! use std::path::Path;
//!
//! # async fn run(git: &dyn GitApi) -> Result<(), processkit::Error> {
//! let branch = git.current_branch(Path::new(".")).await?;
//! # let _ = branch; Ok(()) }
//! ```
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockGitApi`, or inject a fake runner with
//! `Git::with_runner(`[`ScriptedRunner`](processkit::ScriptedRunner)`)`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use processkit::ProcessRunner;
// Re-export the processkit types that appear in this crate's public API, so
// consumers needn't depend on processkit directly. (`Error`/`Result`/`ProcessResult`
// are in scope here too via this `pub use`.)
pub use processkit::{Error, ProcessResult, Result};

pub mod conflict;
mod parse;
pub use parse::{
    BlameLine, Branch, ChangeKind, Commit, DiffLine, DiffStat, FileDiff, GitVersion, Hunk,
    StatusEntry, Worktree,
};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "git";

/// What a [`GitApi::diff`] / [`GitApi::diff_text`] call compares.
///
/// `#[non_exhaustive]` so more comparison shapes can be added later.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DiffSpec {
    /// All tracked working-tree changes vs the last commit (`git diff HEAD`),
    /// staged or not, excluding untracked files.
    WorkingTree,
    /// A specific revision or range, e.g. `main..HEAD` or `HEAD~1` (`git diff <rev>`).
    Rev(String),
}

/// Options for [`GitApi::worktree_add`] (`git worktree add`).
///
/// `#[non_exhaustive]`, so build it through [`WorktreeAdd::checkout`] /
/// [`WorktreeAdd::create_branch`] rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorktreeAdd {
    /// Filesystem path for the new worktree.
    pub path: PathBuf,
    /// Create and check out this new branch (`-b <name>`); `None` checks out an
    /// existing ref.
    pub new_branch: Option<String>,
    /// The commit/branch to base the worktree on; `None` defaults to `HEAD`.
    pub commitish: Option<String>,
    /// Register the worktree without populating its files (`--no-checkout`) — the
    /// caller fills the working tree itself (e.g. a copy-on-write clone).
    pub no_checkout: bool,
}

impl WorktreeAdd {
    /// A worktree at `path` checking out an existing `commitish` (e.g. a branch):
    /// `git worktree add <path> <commitish>`.
    pub fn checkout(path: impl Into<PathBuf>, commitish: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            new_branch: None,
            commitish: Some(commitish.into()),
            no_checkout: false,
        }
    }

    /// A worktree at `path` creating a new branch `name` based on `commitish`:
    /// `git worktree add -b <name> <path> <commitish>`.
    pub fn create_branch(
        path: impl Into<PathBuf>,
        name: impl Into<String>,
        commitish: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            new_branch: Some(name.into()),
            commitish: Some(commitish.into()),
            no_checkout: false,
        }
    }

    /// Register the worktree without checking out its files (`--no-checkout`),
    /// for a caller that populates the working tree itself.
    pub fn no_checkout(mut self) -> Self {
        self.no_checkout = true;
        self
    }
}

/// Options for [`GitApi::push`] (`git push`).
///
/// `#[non_exhaustive]`, so build it through [`GitPush::branch`] /
/// [`GitPush::refspec`] rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GitPush {
    /// Remote to push to (defaults to `origin`).
    pub remote: String,
    /// The refspec — a bare branch name, or `local:remote_branch`.
    pub refspec: String,
    /// Set the pushed branch as the upstream (`-u`).
    pub set_upstream: bool,
}

impl GitPush {
    /// Push branch `name` to `origin` under the same name (`git push origin <name>`).
    pub fn branch(name: impl Into<String>) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: name.into(),
            set_upstream: false,
        }
    }

    /// Push `local` to a differently-named `remote_branch`
    /// (`git push origin <local>:<remote_branch>`).
    pub fn refspec(local: impl AsRef<str>, remote_branch: impl AsRef<str>) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: format!("{}:{}", local.as_ref(), remote_branch.as_ref()),
            set_upstream: false,
        }
    }

    /// Push to a non-default remote.
    pub fn remote(mut self, remote: impl Into<String>) -> Self {
        self.remote = remote.into();
        self
    }

    /// Record the pushed branch as the local branch's upstream (`-u`).
    pub fn set_upstream(mut self) -> Self {
        self.set_upstream = true;
        self
    }
}

/// Options for [`GitApi::clone_repo`] (`git clone`).
///
/// `#[non_exhaustive]`, so build it through [`CloneSpec::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CloneSpec {
    /// Check out this branch instead of the remote's default (`--branch`).
    pub branch: Option<String>,
    /// Shallow-clone to this many commits (`--depth`). git silently ignores
    /// the flag for a plain local-path source (warns, still clones fully);
    /// use a `file://` URL to shallow-clone locally.
    pub depth: Option<u32>,
    /// Create a bare repository (`--bare`).
    pub bare: bool,
}

impl CloneSpec {
    /// A plain full clone of the remote's default branch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check out `branch` instead of the remote's default (`--branch`).
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Shallow-clone to `depth` commits (`--depth`); see the field doc for the
    /// local-path caveat.
    pub fn depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Clone as a bare repository (`--bare`).
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }
}

/// A pre-validated git reference name (branch/tag/remote), for callers that
/// accept names from untrusted input (UIs, bots, agents) and want to fail
/// early with a clear error. The dir-taking methods stay `&str` — they apply
/// the same flag-injection guard internally — so this type is **optional**
/// up-front validation, not a required wrapper.
///
/// Rules follow the load-bearing core of `git check-ref-format`: non-empty,
/// no leading `-` or `.`, no `..`, no control characters or space, none of
/// `~ ^ : ? * [ \`, no trailing `/` or `.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefName(String);

impl RefName {
    /// Validate `name` as a reference name.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let bad = name.is_empty()
            || name.starts_with('-')
            || name.starts_with('.')
            || name.ends_with('/')
            || name.ends_with(".lock")
            || name.contains("..")
            || name
                .chars()
                .any(|c| c.is_control() || " ~^:?*[\\".contains(c));
        if bad {
            return Err(Error::Spawn {
                program: BINARY.to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid git reference name: {name:?}"),
                ),
            });
        }
        Ok(RefName(name))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RefName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A pre-validated revision/range expression (`HEAD~2`, `main..feature`).
/// Deliberately *minimal* — git's revision grammar is too rich to validate
/// here — it only guarantees the expression is non-empty and cannot be parsed
/// as a flag (no leading `-`), matching the internal guard the dir-taking
/// methods apply anyway. Optional up-front validation for untrusted input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevSpec(String);

impl RevSpec {
    /// Validate `rev` as a revision/range expression (non-empty, no leading `-`).
    pub fn new(rev: impl Into<String>) -> Result<Self> {
        let rev = rev.into();
        reject_flag_like("revision", &rev)?;
        Ok(RevSpec(rev))
    }

    /// The validated expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the installed `git` binary supports, probed via
/// [`GitApi::capabilities`]. A value type — the client holds no state, so
/// probe once and keep the result (callers cache it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitCapabilities {
    /// The binary's parsed version.
    pub version: GitVersion,
}

/// The oldest git major this crate is written against. Validated on 2.54;
/// expected to work from ≥ 2.30 — but only the *major* is hard-gated, because
/// a false "unsupported" on an untested-but-fine 2.2x would be worse than the
/// argv error git itself would give. (Contrast vcs-jj, whose floor is precise:
/// its parsers were empirically validated against one jj release.)
const MIN_SUPPORTED_MAJOR: u64 = 2;

impl GitCapabilities {
    /// Whether the binary meets the supported floor (major ≥ 2).
    pub fn is_supported(&self) -> bool {
        self.version.major >= MIN_SUPPORTED_MAJOR
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs git
    /// ≥ 2, found 1.9.5" instead of a cryptic argv failure later.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::Spawn {
            program: BINARY.to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-git requires git >= {MIN_SUPPORTED_MAJOR} (validated on 2.54), \
                     found {}",
                    self.version
                ),
            ),
        })
    }
}

/// The Git operations this crate exposes — the interface consumers code against
/// and mock in tests.
///
/// **Injection safety:** every method that places a caller-supplied name,
/// revision, range, remote, or URL in a positional argv slot rejects a value
/// that is empty or begins with `-` (it would be parsed as a flag) with an
/// [`Error::Spawn`] *before* spawning. Flag-value slots (`-m <msg>`,
/// `--branch <b>`), filesystem path arguments (`--`-separated pathspecs, plus
/// worktree paths and clone destinations — typed `Path`, caller-trusted), and
/// the `run`/`run_raw` escape hatches are not guarded. For eager validation at
/// an input boundary, see [`RefName`] / [`RevSpec`].
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitApi: Send + Sync {
    /// Run `git <args>` in the current directory, returning trimmed stdout
    /// (throws on a non-zero exit). A raw escape hatch for unmodelled commands.
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed Git version (`git --version`).
    async fn version(&self) -> Result<String>;
    /// The installed binary's parsed version, as [`GitCapabilities`]
    /// (`git --version`). A value type — probe once and keep it; an
    /// unrecognisable version string is an [`Error::Parse`].
    async fn capabilities(&self) -> Result<GitCapabilities>;
    /// Working-tree status (`git status --porcelain=v1 -z`).
    async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
    /// Raw porcelain status text (`git status --porcelain=v1`) — the unparsed
    /// counterpart of [`status`](GitApi::status), mirroring `vcs_jj` `status_text`.
    async fn status_text(&self, dir: &Path) -> Result<String>;
    /// Like [`status`](GitApi::status) but ignoring untracked files
    /// (`git status --porcelain=v1 -z --untracked-files=no`) — "is the *tracked*
    /// tree dirty", staged or not.
    async fn status_tracked(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
    /// Paths with unresolved merge conflicts, repo-relative with `/` separators
    /// (`git diff --name-only --diff-filter=U -z`). Empty when there are none.
    async fn conflicted_files(&self, dir: &Path) -> Result<Vec<String>>;
    /// Current branch name (`git rev-parse --abbrev-ref HEAD`).
    async fn current_branch(&self, dir: &Path) -> Result<String>;
    /// Local branches, current one flagged (`git branch`).
    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>>;
    /// Latest `max` commits, newest first (`git log`).
    async fn log(&self, dir: &Path, max: usize) -> Result<Vec<Commit>>;
    /// Commits in `range`, newest first, up to `max` (`git log <range>`).
    async fn log_range(&self, dir: &Path, range: &str, max: usize) -> Result<Vec<Commit>>;
    /// Resolve a revision to a full hash (`git rev-parse <rev>`).
    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String>;
    /// Resolve a revision to its abbreviated hash (`git rev-parse --short <rev>`) —
    /// e.g. to label a detached HEAD.
    async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String>;
    /// Initialise a repository (`git init`).
    async fn init(&self, dir: &Path) -> Result<()>;
    /// Stage `paths` (`git add -- <paths>`).
    async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()>;
    /// Commit staged changes (`git commit -m`).
    async fn commit(&self, dir: &Path, message: &str) -> Result<()>;
    /// Create a branch without switching to it (`git branch <name>`).
    async fn create_branch(&self, dir: &Path, name: &str) -> Result<()>;
    /// Switch to a branch or revision (`git checkout <reference>`).
    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()>;
    /// Check out a commit as a detached HEAD (`git checkout --detach <commit>`).
    async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()>;
    /// Commit exactly `paths`' working-tree content, ignoring the index
    /// (`git commit [--amend] -m <message> --only -- <paths>`).
    async fn commit_paths(
        &self,
        dir: &Path,
        paths: &[PathBuf],
        message: &str,
        amend: bool,
    ) -> Result<()>;
    /// The last commit's full message (`git log -1 --format=%B`) — e.g. to
    /// pre-fill an amend.
    async fn last_commit_message(&self, dir: &Path) -> Result<String>;
    /// Whether `HEAD` is unborn — a fresh repo with no commits yet
    /// (`git rev-parse --verify -q HEAD`, exit-code mapped).
    async fn is_unborn(&self, dir: &Path) -> Result<bool>;
    /// Whether the working tree has no unstaged modifications to **tracked** files
    /// (`git diff --quiet`). Untracked files are *not* counted — this is not a full
    /// "is the working tree clean?" check; use [`status`](GitApi::status) for that.
    async fn diff_is_empty(&self, dir: &Path) -> Result<bool>;

    // --- Discovery / identity ------------------------------------------------

    /// The repository's common git directory (`rev-parse --git-common-dir`) —
    /// stable across linked worktrees.
    async fn common_dir(&self, dir: &Path) -> Result<PathBuf>;
    /// This worktree's git directory (`rev-parse --git-dir`).
    async fn git_dir(&self, dir: &Path) -> Result<PathBuf>;
    /// Resolve a revision to a commit hash, peeling tags
    /// (`rev-parse --verify <rev>^{commit}`).
    async fn resolve_commit(&self, dir: &Path, rev: &str) -> Result<String>;
    /// The remote's default branch from `symbolic-ref refs/remotes/origin/HEAD`
    /// (short name only); `None` when `origin/HEAD` is unset.
    async fn remote_head_branch(&self, dir: &Path) -> Result<Option<String>>;
    /// Whether a local branch exists (`show-ref --verify --quiet refs/heads/<name>`).
    async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
    /// Whether `origin` has `name`, without fetching (`ls-remote origin
    /// refs/heads/<name>` — the fully-qualified ref, so `foo` can't tail-match
    /// `bar/foo`). Runs with `GIT_TERMINAL_PROMPT=0` and a 10s timeout so a missing
    /// credential or a flaky network can't hang the call.
    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
    /// A remote's URL (`remote get-url <remote>`).
    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String>;
    /// The current branch's upstream, e.g. `Some("origin/main")`
    /// (`rev-parse --abbrev-ref --symbolic-full-name @{u}`); `None` when unset.
    async fn upstream(&self, dir: &Path) -> Result<Option<String>>;
    /// Branch names on `remote`, without fetching
    /// (`ls-remote --heads <remote>`).
    async fn remote_branches(&self, dir: &Path, remote: &str) -> Result<Vec<String>>;

    // --- Branches ------------------------------------------------------------

    /// Whether `branch` is fully merged into `target` (`branch --merged <target>`).
    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool>;
    /// Set `branch`'s upstream to `upstream` (e.g. `origin/main`)
    /// (`branch --set-upstream-to=<upstream> <branch>`).
    async fn set_upstream(&self, dir: &Path, branch: &str, upstream: &str) -> Result<()>;
    /// Delete a local branch (`branch -d`, or `-D` when `force`).
    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()>;
    /// Rename a local branch (`branch -m <old> <new>`).
    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
    /// Count commits in a range (`rev-list --count <range>`).
    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize>;
    /// Whether a diff range is empty (`diff --quiet <range>`).
    async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool>;
    /// Aggregate change stats for a range (`diff --shortstat <range>`). Named to
    /// match `vcs_jj::JjApi::diff_stat`.
    async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat>;
    /// Raw git-format unified diff text for `spec`
    /// (`diff <spec> --no-color --no-ext-diff -M`) — stable machine output.
    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
    /// Parsed per-file unified diff for `spec`, layered on [`diff_text`](GitApi::diff_text).
    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;

    // --- In-progress state ---------------------------------------------------

    /// Whether the index has no staged changes (`diff --cached --quiet`).
    async fn staged_is_empty(&self, dir: &Path) -> Result<bool>;
    /// Whether a rebase is in progress (a `rebase-merge`/`rebase-apply` dir exists
    /// under the git dir).
    async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool>;
    /// Whether a merge is in progress (a `MERGE_HEAD` exists under the git dir).
    async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool>;

    // --- Mutations -----------------------------------------------------------

    /// Fetch from the default remote (`fetch --quiet`), with `GIT_TERMINAL_PROMPT=0`.
    /// Transient (network) failures are retried (3 attempts, 500 ms backoff).
    async fn fetch(&self, dir: &Path) -> Result<()>;
    /// Fetch from a *named* remote (`fetch --quiet <remote>`), with
    /// `GIT_TERMINAL_PROMPT=0`. Transient failures are retried like
    /// [`fetch`](GitApi::fetch).
    async fn fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
    /// Fetch a single branch from `origin` into its remote-tracking ref
    /// (`fetch --quiet origin refs/heads/<b>:refs/remotes/origin/<b>`), with
    /// `GIT_TERMINAL_PROMPT=0`. Transient failures are retried (3×, 500 ms).
    async fn fetch_remote_branch(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Push to a remote (`push [-u] <remote> <refspec>`); see [`GitPush`].
    async fn push(&self, dir: &Path, spec: GitPush) -> Result<()>;
    /// Stage a branch's changes without committing (`merge --squash <branch>`).
    async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Merge a branch (`merge [--no-ff] [-m <msg> | --no-edit] <branch>`); with no
    /// message it takes the default merge message non-interactively (`--no-edit`).
    async fn merge_commit(
        &self,
        dir: &Path,
        branch: &str,
        no_ff: bool,
        message: Option<String>,
    ) -> Result<()>;
    /// Merge without committing, for a dry run
    /// (`merge --no-commit [--squash|--no-ff] <branch>`).
    async fn merge_no_commit(
        &self,
        dir: &Path,
        branch: &str,
        squash: bool,
        no_ff: bool,
    ) -> Result<()>;
    /// Abort an in-progress merge (`merge --abort`).
    async fn merge_abort(&self, dir: &Path) -> Result<()>;
    /// Finish a merge after resolving conflicts (`commit --no-edit`).
    async fn merge_continue(&self, dir: &Path) -> Result<()>;
    /// Clear merge state, squash-safe (`reset --merge`).
    async fn reset_merge(&self, dir: &Path) -> Result<()>;
    /// Hard-reset the working tree to a revision (`reset --hard <rev>`).
    async fn reset_hard(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Rebase the current branch onto `onto` (`rebase <onto>`); the editor is
    /// suppressed (`GIT_EDITOR=true`) so it never hangs a headless caller.
    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
    /// Abort an in-progress rebase (`rebase --abort`).
    async fn rebase_abort(&self, dir: &Path) -> Result<()>;
    /// Continue a rebase after resolving conflicts (`rebase --continue`); the
    /// editor is suppressed (`GIT_EDITOR=true`) so the message-confirm never hangs.
    async fn rebase_continue(&self, dir: &Path) -> Result<()>;
    /// Stash the working tree (`stash push`, `--include-untracked` when asked) —
    /// e.g. to save state before a copy-on-write restore.
    async fn stash_push(&self, dir: &Path, include_untracked: bool) -> Result<()>;
    /// Restore the most recent stash and drop it (`stash pop`).
    async fn stash_pop(&self, dir: &Path) -> Result<()>;

    // --- Worktrees -----------------------------------------------------------

    /// List worktrees (`worktree list --porcelain`).
    async fn worktree_list(&self, dir: &Path) -> Result<Vec<Worktree>>;
    /// Add a worktree (`worktree add [-b <branch>] <path> [<commitish>]`).
    async fn worktree_add(&self, dir: &Path, spec: WorktreeAdd) -> Result<()>;
    /// Remove a worktree (`worktree remove [--force] <path>`).
    async fn worktree_remove(&self, dir: &Path, path: &Path, force: bool) -> Result<()>;
    /// Move a worktree (`worktree move <from> <to>`).
    async fn worktree_move(&self, dir: &Path, from: &Path, to: &Path) -> Result<()>;
    /// Prune stale worktree admin entries (`worktree prune`).
    async fn worktree_prune(&self, dir: &Path) -> Result<()>;

    // --- Clone / tags / inspection --------------------------------------------

    /// Clone `url` into `dest` (`git clone <url> <dest>` + [`CloneSpec`] flags).
    /// Runs without a working directory — pass an **absolute** `dest`.
    async fn clone_repo(&self, url: &str, dest: &Path, spec: CloneSpec) -> Result<()>;
    /// Create a lightweight tag at `rev` (`tag <name> [<rev>]`; `None` = HEAD).
    async fn tag_create(&self, dir: &Path, name: &str, rev: Option<String>) -> Result<()>;
    /// Create an annotated tag (`tag -a <name> -m <message> [<rev>]`).
    async fn tag_create_annotated(
        &self,
        dir: &Path,
        name: &str,
        message: &str,
        rev: Option<String>,
    ) -> Result<()>;
    /// Tag names, sorted by git's default ordering (`tag --list`).
    async fn tag_list(&self, dir: &Path) -> Result<Vec<String>>;
    /// Delete a tag (`tag -d <name>`).
    async fn tag_delete(&self, dir: &Path, name: &str) -> Result<()>;
    /// A file's content at a revision (`git show <rev>:<path>`). `path` is
    /// repo-relative; backslashes are normalised to `/` (git requires it).
    /// Content is decoded **lossily** — binary files come back mangled rather
    /// than erroring.
    async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String>;
    /// The value of a config key, or `None` when unset (`config --get <key>`,
    /// whose exit 1 covers both "unset" and "no such section" — git doesn't
    /// distinguish). A multi-valued key errors; read those via `run`.
    async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>>;
    /// Set a config key in the repository's local config (`config <key> <value>`).
    async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()>;
    /// Add a remote (`remote add <name> <url>`).
    async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
    /// Change a remote's URL (`remote set-url <name> <url>`).
    async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
    /// Per-line authorship of `path` (`blame --line-porcelain [<rev>] -- <path>`;
    /// `None` = the working tree's HEAD).
    async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>>;

    // --- Sequencer -------------------------------------------------------------

    /// Apply a commit onto the current branch (`cherry-pick <rev>`). A conflict
    /// surfaces as an error classified by [`is_merge_conflict`].
    async fn cherry_pick(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Revert a commit with the default message (`revert --no-edit <rev>`).
    async fn revert(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Skip the current patch of a paused rebase (`rebase --skip`). Mainly for
    /// the `apply` backend's "nothing to commit" stop — the default `merge`
    /// backend auto-drops emptied patches on `--continue`.
    async fn rebase_skip(&self, dir: &Path) -> Result<()>;
}

processkit::cli_client!(
    /// The real Git client. Generic over the [`ProcessRunner`] so tests can inject
    /// a fake process executor; `Git::new()` uses the real job-backed runner.
    pub struct Git => BINARY
);

#[async_trait::async_trait]
impl<R: ProcessRunner> GitApi for Git<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.text(self.core.command(["--version"])).await
    }

    async fn capabilities(&self) -> Result<GitCapabilities> {
        let raw = self.version().await?;
        let version = parse::parse_git_version(&raw).ok_or_else(|| Error::Parse {
            program: BINARY.to_string(),
            message: format!("unrecognisable `git --version` output: {raw:?}"),
        })?;
        Ok(GitCapabilities { version })
    }

    async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>> {
        self.core
            .parse(
                self.core
                    .command_in(dir, ["status", "--porcelain=v1", "-z"]),
                parse::parse_porcelain,
            )
            .await
    }

    async fn status_text(&self, dir: &Path) -> Result<String> {
        self.core
            .text(self.core.command_in(dir, ["status", "--porcelain=v1"]))
            .await
    }

    async fn status_tracked(&self, dir: &Path) -> Result<Vec<StatusEntry>> {
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    ["status", "--porcelain=v1", "-z", "--untracked-files=no"],
                ),
                parse::parse_porcelain,
            )
            .await
    }

    async fn conflicted_files(&self, dir: &Path) -> Result<Vec<String>> {
        // `-z` keeps special-character paths literal (no C-style quoting).
        self.core
            .parse(
                self.core
                    .command_in(dir, ["diff", "--name-only", "--diff-filter=U", "-z"]),
                parse::parse_nul_paths,
            )
            .await
    }

    async fn current_branch(&self, dir: &Path) -> Result<String> {
        self.core
            .text(
                self.core
                    .command_in(dir, ["rev-parse", "--abbrev-ref", "HEAD"]),
            )
            .await
    }

    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>> {
        self.core
            .parse(self.core.command_in(dir, ["branch"]), parse::parse_branches)
            .await
    }

    async fn log(&self, dir: &Path, max: usize) -> Result<Vec<Commit>> {
        let n = format!("-n{max}");
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    [
                        "log",
                        n.as_str(),
                        "-z",
                        "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s",
                    ],
                ),
                parse::parse_log,
            )
            .await
    }

    async fn log_range(&self, dir: &Path, range: &str, max: usize) -> Result<Vec<Commit>> {
        reject_flag_like("range", range)?;
        let n = format!("-n{max}");
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    [
                        "log",
                        range,
                        n.as_str(),
                        "-z",
                        "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s",
                    ],
                ),
                parse::parse_log,
            )
            .await
    }

    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        self.core
            .text(self.core.command_in(dir, ["rev-parse", rev]))
            .await
    }

    async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        self.core
            .text(self.core.command_in(dir, ["rev-parse", "--short", rev]))
            .await
    }

    async fn init(&self, dir: &Path) -> Result<()> {
        self.core.unit(self.core.command_in(dir, ["init"])).await
    }

    async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()> {
        // `--` separates the pathspecs so a path can never be read as an option.
        let mut command = self.core.command_in(dir, ["add", "--"]);
        for path in paths {
            command = command.arg(path);
        }
        self.core.unit(command).await
    }

    async fn commit(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["commit", "-m", message]))
            .await
    }

    async fn create_branch(&self, dir: &Path, name: &str) -> Result<()> {
        reject_flag_like("branch name", name)?;
        self.core
            .unit(self.core.command_in(dir, ["branch", name]))
            .await
    }

    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()> {
        reject_flag_like("reference", reference)?;
        self.core
            .unit(self.core.command_in(dir, ["checkout", reference]))
            .await
    }

    async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()> {
        reject_flag_like("commit", commit)?;
        self.core
            .unit(self.core.command_in(dir, ["checkout", "--detach", commit]))
            .await
    }

    async fn commit_paths(
        &self,
        dir: &Path,
        paths: &[PathBuf],
        message: &str,
        amend: bool,
    ) -> Result<()> {
        // `--only -- <paths>` commits exactly these paths' working-tree content
        // regardless of the index; `--` keeps a path from being read as an option.
        let mut command = self.core.command_in(dir, ["commit"]);
        if amend {
            command = command.arg("--amend");
        }
        command = command.arg("-m").arg(message).arg("--only").arg("--");
        for path in paths {
            command = command.arg(path);
        }
        self.core.unit(command).await
    }

    async fn last_commit_message(&self, dir: &Path) -> Result<String> {
        self.core
            .text(self.core.command_in(dir, ["log", "-1", "--format=%B"]))
            .await
    }

    async fn is_unborn(&self, dir: &Path) -> Result<bool> {
        // `rev-parse --verify -q HEAD` resolves HEAD quietly: 0 = a commit exists
        // (not unborn), 1 = no commit yet (unborn). `probe` maps those to a bool
        // and surfaces anything else (e.g. 128, not a repo) as `Error::Exit`.
        Ok(!self
            .core
            .probe(
                self.core
                    .command_in(dir, ["rev-parse", "--verify", "-q", "HEAD"]),
            )
            .await?)
    }

    async fn diff_is_empty(&self, dir: &Path) -> Result<bool> {
        // `git diff --quiet` is an exit-code answer: 0 = clean (empty), 1 = dirty;
        // `probe` errors on any other code / timeout / signal.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--quiet"]))
            .await
    }

    async fn common_dir(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core
                .text(self.core.command_in(dir, ["rev-parse", "--git-common-dir"]))
                .await?,
        ))
    }

    async fn git_dir(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core
                .text(self.core.command_in(dir, ["rev-parse", "--git-dir"]))
                .await?,
        ))
    }

    async fn resolve_commit(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        // `^{commit}` peels an annotated tag down to the commit it points at.
        let spec = format!("{rev}^{{commit}}");
        self.core
            .text(
                self.core
                    .command_in(dir, ["rev-parse", "--verify", spec.as_str()]),
            )
            .await
    }

    async fn remote_head_branch(&self, dir: &Path) -> Result<Option<String>> {
        // `--quiet` makes an unset origin/HEAD a silent non-zero exit (no `fatal:`
        // on stderr); that's "no default branch", not an error — so inspect the
        // code rather than `?`.
        let res = self
            .core
            .capture(
                self.core
                    .command_in(dir, ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]),
            )
            .await?;
        if res.code() == Some(0) {
            // "refs/remotes/origin/main" → "main"; strip the whole ref prefix so a
            // slashed default branch (e.g. "release/v2") survives intact.
            let out = res.stdout().trim();
            Ok(Some(
                out.strip_prefix("refs/remotes/origin/")
                    .unwrap_or(out)
                    .to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        let refname = format!("refs/heads/{name}");
        // `show-ref --verify --quiet` is an exit-code answer: 0 = exists, 1 = not.
        self.core
            .probe(
                self.core
                    .command_in(dir, ["show-ref", "--verify", "--quiet", refname.as_str()]),
            )
            .await
    }

    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        // No credential prompt, bounded wait: a missing helper or a flaky network
        // must not hang the call. `capture` reports a timeout as a flagged result
        // (non-zero exit) rather than erroring, so an unreachable remote reads as
        // "absent" (`false`) — the best-effort answer a probe wants. A genuine
        // spawn failure (no `git`) still surfaces as an error.
        //
        // Query the *fully-qualified* ref: `ls-remote origin <name>` tail-matches
        // path components, so a bare `foo` would also match `refs/heads/bar/foo`.
        // `refs/heads/<name>` matches only the exact branch.
        let refname = format!("refs/heads/{name}");
        let cmd = self
            .core
            .command_in(dir, ["ls-remote", "origin", refname.as_str()])
            .env("GIT_TERMINAL_PROMPT", "0")
            .timeout(Duration::from_secs(10));
        let res = self.core.capture(cmd).await?;
        Ok(res.code() == Some(0) && !res.stdout().trim().is_empty())
    }

    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String> {
        reject_flag_like("remote name", remote)?;
        self.core
            .text(self.core.command_in(dir, ["remote", "get-url", remote]))
            .await
    }

    async fn upstream(&self, dir: &Path) -> Result<Option<String>> {
        // `@{u}` resolves the configured upstream; with no upstream the command
        // exits non-zero — surface that as `None` rather than an error.
        match self
            .core
            .capture(self.core.command_in(
                dir,
                ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            ))
            .await?
        {
            res if res.code() == Some(0) => {
                let name = res.stdout().trim();
                Ok((!name.is_empty()).then(|| name.to_string()))
            }
            _ => Ok(None),
        }
    }

    async fn remote_branches(&self, dir: &Path, remote: &str) -> Result<Vec<String>> {
        reject_flag_like("remote name", remote)?;
        // `GIT_TERMINAL_PROMPT=0`: a remote needing credentials must fail fast,
        // never block on an interactive auth prompt.
        let cmd = self
            .core
            .command_in(dir, ["ls-remote", "--heads", remote])
            .env("GIT_TERMINAL_PROMPT", "0");
        self.core.parse(cmd, parse::parse_ls_remote_heads).await
    }

    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool> {
        reject_flag_like("branch", branch)?;
        reject_flag_like("target", target)?;
        let out = self
            .core
            .text(self.core.command_in(dir, ["branch", "--merged", target]))
            .await?;
        // Each line is a fixed 2-column marker (`  `/`* `/`+ `) then the name;
        // drop exactly those two columns rather than trimming a char class (which
        // would over-strip a name that legitimately began with the marker char).
        Ok(out
            .lines()
            .filter_map(|line| line.get(2..))
            .any(|b| b == branch))
    }

    async fn set_upstream(&self, dir: &Path, branch: &str, upstream: &str) -> Result<()> {
        reject_flag_like("branch name", branch)?;
        let flag = format!("--set-upstream-to={upstream}");
        self.core
            .unit(self.core.command_in(dir, ["branch", flag.as_str(), branch]))
            .await
    }

    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()> {
        reject_flag_like("branch name", name)?;
        let flag = if force { "-D" } else { "-d" };
        self.core
            .unit(self.core.command_in(dir, ["branch", flag, name]))
            .await
    }

    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()> {
        reject_flag_like("branch name", old)?;
        reject_flag_like("branch name", new)?;
        self.core
            .unit(self.core.command_in(dir, ["branch", "-m", old, new]))
            .await
    }

    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize> {
        reject_flag_like("range", range)?;
        self.core
            .try_parse(
                self.core.command_in(dir, ["rev-list", "--count", range]),
                |s| {
                    s.trim().parse::<usize>().map_err(|e| Error::Parse {
                        program: BINARY.to_string(),
                        message: e.to_string(),
                    })
                },
            )
            .await
    }

    async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool> {
        reject_flag_like("range", range)?;
        // `diff --quiet <range>`: 0 = empty range, 1 = has changes.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--quiet", range]))
            .await
    }

    async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat> {
        reject_flag_like("range", range)?;
        self.core
            .parse(
                self.core.command_in(dir, ["diff", "--shortstat", range]),
                parse::parse_shortstat,
            )
            .await
    }

    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String> {
        // The target is a single positional arg: `HEAD` for the working tree, or
        // the caller's revision/range. `-M` enables rename detection; `--no-color`
        // / `--no-ext-diff` keep the output stable and machine-parseable.
        let target = match spec {
            DiffSpec::WorkingTree => {
                // On an unborn repo `HEAD` doesn't resolve (`git diff HEAD` errors);
                // diff against the empty tree so a pre-first-commit working tree
                // still yields its additions instead of a hard failure.
                if self.is_unborn(dir).await? {
                    EMPTY_TREE.to_string()
                } else {
                    "HEAD".to_string()
                }
            }
            DiffSpec::Rev(rev) => {
                reject_flag_like("revision", &rev)?;
                rev
            }
        };
        self.core
            .text(self.core.command_in(
                dir,
                ["diff", target.as_str(), "--no-color", "--no-ext-diff", "-M"],
            ))
            .await
    }

    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>> {
        let text = self.diff_text(dir, spec).await?;
        Ok(parse::parse_diff(&text))
    }

    async fn staged_is_empty(&self, dir: &Path) -> Result<bool> {
        // `diff --cached --quiet`: 0 = nothing staged, 1 = staged changes.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--cached", "--quiet"]))
            .await
    }

    async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool> {
        let git_dir = self.resolved_git_dir(dir).await?;
        Ok(git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists())
    }

    async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool> {
        Ok(self
            .resolved_git_dir(dir)
            .await?
            .join("MERGE_HEAD")
            .exists())
    }

    async fn fetch(&self, dir: &Path) -> Result<()> {
        // `GIT_TERMINAL_PROMPT=0` so a remote needing credentials fails fast
        // rather than blocking on an interactive prompt — matching the other
        // remote ops (`fetch_remote_branch`, `push`, `remote_branch_exists`).
        // Fetch is idempotent, so `retry` replays it on a transient failure
        // (DNS/timeout/dropped connection); a non-transient error fails at once.
        let cmd = self
            .core
            .command_in(dir, ["fetch", "--quiet"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.unit(cmd).await
    }

    async fn fetch_from(&self, dir: &Path, remote: &str) -> Result<()> {
        // A leading-`-` remote is a bare positional here — and a flag like
        // `--upload-pack=<cmd>` would run an arbitrary local program for a
        // local/ext transport, so this guard is load-bearing for security.
        reject_flag_like("remote", remote)?;
        // Same containment as `fetch` (prompt off, transient retry), with the
        // remote named explicitly.
        let cmd = self
            .core
            .command_in(dir, ["fetch", "--quiet", remote])
            .env("GIT_TERMINAL_PROMPT", "0")
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.unit(cmd).await
    }

    async fn fetch_remote_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        let cmd = self
            .core
            .command_in(dir, ["fetch", "--quiet", "origin", refspec.as_str()])
            .env("GIT_TERMINAL_PROMPT", "0")
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.unit(cmd).await
    }

    async fn push(&self, dir: &Path, spec: GitPush) -> Result<()> {
        reject_flag_like("remote", &spec.remote)?;
        reject_flag_like("refspec", &spec.refspec)?;
        let mut args: Vec<&str> = vec!["push"];
        if spec.set_upstream {
            args.push("-u");
        }
        args.push(spec.remote.as_str());
        args.push(spec.refspec.as_str());
        let cmd = self
            .core
            .command_in(dir, args)
            .env("GIT_TERMINAL_PROMPT", "0");
        self.core.unit(cmd).await
    }

    async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()> {
        reject_flag_like("branch", branch)?;
        self.core
            .unit(self.core.command_in(dir, ["merge", "--squash", branch]))
            .await
    }

    async fn merge_commit(
        &self,
        dir: &Path,
        branch: &str,
        no_ff: bool,
        message: Option<String>,
    ) -> Result<()> {
        reject_flag_like("branch", branch)?;
        let mut args: Vec<&str> = vec!["merge"];
        if no_ff {
            args.push("--no-ff");
        }
        if let Some(msg) = message.as_deref() {
            args.push("-m");
            args.push(msg);
        } else {
            // No message → take the default merge message non-interactively
            // instead of opening `$EDITOR` (which would hang a headless caller).
            args.push("--no-edit");
        }
        args.push(branch);
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn merge_no_commit(
        &self,
        dir: &Path,
        branch: &str,
        squash: bool,
        no_ff: bool,
    ) -> Result<()> {
        reject_flag_like("branch", branch)?;
        let mut args: Vec<&str> = vec!["merge", "--no-commit"];
        // `--squash` and `--no-ff` are mutually exclusive (git rejects the pair);
        // a squash never fast-forwards anyway, so it takes precedence.
        if squash {
            args.push("--squash");
        } else if no_ff {
            args.push("--no-ff");
        }
        args.push(branch);
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn merge_abort(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["merge", "--abort"]))
            .await
    }

    async fn merge_continue(&self, dir: &Path) -> Result<()> {
        // `--no-edit` already reuses the prepared MERGE_MSG; `no_editor` is a
        // headless backstop so a commit hook re-opening the editor can't hang.
        self.core
            .unit(no_editor(
                self.core.command_in(dir, ["commit", "--no-edit"]),
            ))
            .await
    }

    async fn reset_merge(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["reset", "--merge"]))
            .await
    }

    async fn reset_hard(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        self.core
            .unit(self.core.command_in(dir, ["reset", "--hard", rev]))
            .await
    }

    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()> {
        reject_flag_like("rebase target", onto)?;
        // Force a no-op editor so a rebase that would open `$EDITOR` (reword, or
        // the message-confirm on `--continue`) never hangs a headless caller.
        self.core
            .unit(no_editor(self.core.command_in(dir, ["rebase", onto])))
            .await
    }

    async fn rebase_abort(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["rebase", "--abort"]))
            .await
    }

    async fn rebase_continue(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(no_editor(
                self.core.command_in(dir, ["rebase", "--continue"]),
            ))
            .await
    }

    async fn stash_push(&self, dir: &Path, include_untracked: bool) -> Result<()> {
        let mut command = self.core.command_in(dir, ["stash", "push"]);
        if include_untracked {
            command = command.arg("--include-untracked");
        }
        self.core.unit(command).await
    }

    async fn stash_pop(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["stash", "pop"]))
            .await
    }

    async fn worktree_list(&self, dir: &Path) -> Result<Vec<Worktree>> {
        self.core
            .parse(
                self.core
                    .command_in(dir, ["worktree", "list", "--porcelain"]),
                parse::parse_worktree_porcelain,
            )
            .await
    }

    async fn worktree_add(&self, dir: &Path, spec: WorktreeAdd) -> Result<()> {
        if let Some(name) = spec.new_branch.as_deref() {
            reject_flag_like("branch name", name)?;
        }
        if let Some(commitish) = spec.commitish.as_deref() {
            reject_flag_like("commit-ish", commitish)?;
        }
        let mut command = self.core.command_in(dir, ["worktree", "add"]);
        if let Some(name) = spec.new_branch.as_deref() {
            command = command.arg("-b").arg(name);
        }
        if spec.no_checkout {
            command = command.arg("--no-checkout");
        }
        command = command.arg(&spec.path);
        if let Some(commitish) = spec.commitish.as_deref() {
            command = command.arg(commitish);
        }
        self.core.unit(command).await
    }

    async fn worktree_remove(&self, dir: &Path, path: &Path, force: bool) -> Result<()> {
        let mut command = self.core.command_in(dir, ["worktree", "remove"]);
        if force {
            command = command.arg("--force");
        }
        command = command.arg(path);
        self.core.unit(command).await
    }

    async fn worktree_move(&self, dir: &Path, from: &Path, to: &Path) -> Result<()> {
        let command = self
            .core
            .command_in(dir, ["worktree", "move"])
            .arg(from)
            .arg(to);
        self.core.unit(command).await
    }

    async fn worktree_prune(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["worktree", "prune"]))
            .await
    }

    async fn clone_repo(&self, url: &str, dest: &Path, spec: CloneSpec) -> Result<()> {
        // A leading-`-` url is a bare positional — `git clone --upload-pack=<cmd>`
        // would run an arbitrary local program. A real URL never leads with `-`,
        // so this guard has no false positives.
        reject_flag_like("url", url)?;
        // No working directory: clone creates `dest` itself, so `dest` should
        // be absolute (a relative path would resolve against this process' cwd).
        let mut command = self.core.command(["clone"]);
        if let Some(branch) = spec.branch.as_deref() {
            command = command.arg("--branch").arg(branch);
        }
        if let Some(depth) = spec.depth {
            command = command.arg("--depth").arg(depth.to_string());
        }
        if spec.bare {
            command = command.arg("--bare");
        }
        let command = command.arg(url).arg(dest).env("GIT_TERMINAL_PROMPT", "0");
        self.core.unit(command).await
    }

    async fn tag_create(&self, dir: &Path, name: &str, rev: Option<String>) -> Result<()> {
        reject_flag_like("tag name", name)?;
        if let Some(rev) = rev.as_deref() {
            reject_flag_like("revision", rev)?;
        }
        let mut args = vec!["tag", name];
        if let Some(rev) = rev.as_deref() {
            args.push(rev);
        }
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn tag_create_annotated(
        &self,
        dir: &Path,
        name: &str,
        message: &str,
        rev: Option<String>,
    ) -> Result<()> {
        reject_flag_like("tag name", name)?;
        if let Some(rev) = rev.as_deref() {
            reject_flag_like("revision", rev)?;
        }
        let mut args = vec!["tag", "-a", name, "-m", message];
        if let Some(rev) = rev.as_deref() {
            args.push(rev);
        }
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn tag_list(&self, dir: &Path) -> Result<Vec<String>> {
        let out = self
            .core
            .text(self.core.command_in(dir, ["tag", "--list"]))
            .await?;
        Ok(out.lines().map(str::to_string).collect())
    }

    async fn tag_delete(&self, dir: &Path, name: &str) -> Result<()> {
        reject_flag_like("tag name", name)?;
        self.core
            .unit(self.core.command_in(dir, ["tag", "-d", name]))
            .await
    }

    async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String> {
        // A leading-`-` rev makes the whole `<rev>:<path>` token start with `-`,
        // so git would parse it as a flag — guard it before building the spec.
        reject_flag_like("revision", rev)?;
        // git rejects backslash separators in the `<rev>:<path>` spec ("exists
        // on disk, but not in <rev>") — normalise for Windows callers.
        let spec = format!("{rev}:{}", path.replace('\\', "/"));
        self.core
            .text(self.core.command_in(dir, ["show", spec.as_str()]))
            .await
    }

    async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>> {
        reject_flag_like("config key", key)?;
        let res = self
            .core
            .capture(self.core.command_in(dir, ["config", "--get", key]))
            .await?;
        match res.code() {
            // Exit 1 = unset (git lumps "no such key/section" in here too).
            Some(1) => Ok(None),
            Some(0) => Ok(Some(res.stdout().trim_end().to_string())),
            _ => {
                res.ensure_success()?;
                Ok(None) // unreachable: a non-zero exit always errors above.
            }
        }
    }

    async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()> {
        reject_flag_like("config key", key)?;
        self.core
            .unit(self.core.command_in(dir, ["config", key, value]))
            .await
    }

    async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        reject_flag_like("remote name", name)?;
        reject_flag_like("url", url)?;
        self.core
            .unit(self.core.command_in(dir, ["remote", "add", name, url]))
            .await
    }

    async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        reject_flag_like("remote name", name)?;
        reject_flag_like("url", url)?;
        self.core
            .unit(self.core.command_in(dir, ["remote", "set-url", name, url]))
            .await
    }

    async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>> {
        let mut args = vec!["blame", "--line-porcelain"];
        if let Some(rev) = rev.as_deref() {
            // A standalone positional rev with a leading `-` would be any blame
            // flag (`-s`, `--reverse`, `-L…`) — guard before the `--`.
            reject_flag_like("revision", rev)?;
            args.push(rev);
        }
        args.push("--");
        args.push(path);
        self.core
            .parse(
                self.core.command_in(dir, args),
                parse::parse_blame_porcelain,
            )
            .await
    }

    async fn cherry_pick(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        // No editor opens non-interactively, but keep the headless backstop.
        self.core
            .unit(no_editor(self.core.command_in(dir, ["cherry-pick", rev])))
            .await
    }

    async fn revert(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        self.core
            .unit(no_editor(
                self.core.command_in(dir, ["revert", "--no-edit", rev]),
            ))
            .await
    }

    async fn rebase_skip(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(no_editor(self.core.command_in(dir, ["rebase", "--skip"])))
            .await
    }
}

// --- Error classification ----------------------------------------------------
//
// git writes load-bearing diagnostics to *either* stream on failure — `CONFLICT
// (content): …` to stdout, `Automatic merge failed …` to stderr — so these probe
// both `stdout` and `stderr` of `Error::Exit`. Consumers call these instead of
// re-implementing the string-scraping themselves.

/// Git's well-known empty-tree object id — a stable stand-in for `HEAD` when
/// diffing the working tree of an unborn (no-commits-yet) repository.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Total attempts for a transient-retried `fetch` (1 try + 2 retries).
const FETCH_ATTEMPTS: u32 = 3;
/// Fixed backoff between fetch retries.
const FETCH_BACKOFF: Duration = Duration::from_millis(500);

/// Lower-case substrings marking a merge that stopped on conflicts.
const CONFLICT_MARKERS: &[&str] = &["conflict (", "automatic merge failed"];
/// Lower-case substrings marking a commit that found nothing to record.
const NOTHING_TO_COMMIT_MARKERS: &[&str] = &["nothing to commit", "nothing added to commit"];
/// Lower-case substrings marking a transient (retryable) network/fetch failure.
const TRANSIENT_FETCH_MARKERS: &[&str] = &[
    "could not resolve host",
    "couldn't resolve host",
    "temporary failure in name resolution",
    "connection timed out",
    "connection refused",
    "operation timed out",
    "timed out",
    "network is unreachable",
    "failed to connect",
    "could not read from remote repository",
    "the remote end hung up",
    "early eof",
    "rpc failed",
];

/// Whether `err` is an `Error::Exit` whose captured output contains any marker.
fn exit_output_matches(err: &Error, markers: &[&str]) -> bool {
    let Error::Exit { stdout, stderr, .. } = err else {
        return false;
    };
    let out = stdout.to_ascii_lowercase();
    let errt = stderr.to_ascii_lowercase();
    markers.iter().any(|m| out.contains(m) || errt.contains(m))
}

/// Whether a failed `merge`/`merge_commit` stopped on a merge conflict.
pub fn is_merge_conflict(err: &Error) -> bool {
    exit_output_matches(err, CONFLICT_MARKERS)
}

/// Whether a failed `commit`/`commit_paths` reported nothing to commit (a clean
/// tree), as opposed to a real error.
pub fn is_nothing_to_commit(err: &Error) -> bool {
    exit_output_matches(err, NOTHING_TO_COMMIT_MARKERS)
}

/// Whether a failed `fetch`/`fetch_remote_branch`/`remote_branch_exists` looks
/// transient (DNS, timeout, dropped connection) and is worth retrying.
pub fn is_transient_fetch_error(err: &Error) -> bool {
    // A processkit-level timeout (a `.timeout()`-bounded run that expired) carries
    // no captured output but is inherently transient; treat it as retryable too.
    matches!(err, Error::Timeout { .. }) || exit_output_matches(err, TRANSIENT_FETCH_MARKERS)
}

/// Point git's editor at a no-op so any command that would open `$EDITOR`
/// (a rebase reword, the message-confirm on `rebase --continue`) succeeds
/// non-interactively instead of hanging a headless caller.
fn no_editor(cmd: processkit::Command) -> processkit::Command {
    cmd.env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
}

/// Injection guard for bare positional argv slots: a caller-supplied value
/// with a leading `-` would be parsed by git as a *flag* (verified: `git
/// checkout -evil` → "unknown switch"), and an empty value silently changes
/// most commands' meaning. Refuse both before anything spawns. Flag-VALUE
/// positions (`-m <msg>`, `--branch <b>`) don't need this — git consumes the
/// next token verbatim there.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('-') {
        return Err(Error::Spawn {
            program: BINARY.to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} would be parsed as a flag (or is empty) — \
                     refusing to pass it as a positional argument"
                ),
            ),
        });
    }
    Ok(())
}

impl<R: ProcessRunner> Git<R> {
    /// Run `git <args>` over string slices — `git.run_args(&["status", "-s"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GitApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    /// Like [`run_args`](Git::run_args) but never errors on a non-zero exit
    /// (mirrors [`GitApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    /// Bind this client to `dir`, returning a [`GitAt`] handle whose methods omit
    /// the `dir` argument: `git.at(dir).status()` runs [`status`](GitApi::status)
    /// against `dir`. The dir-taking [`GitApi`] methods stay on [`Git`] for
    /// driving many directories (e.g. linked worktrees) from one client.
    pub fn at<'a>(&'a self, dir: &'a Path) -> GitAt<'a, R> {
        GitAt { git: self, dir }
    }

    /// Harden this client for driving repositories it didn't create: running
    /// `git` inside an untrusted checkout executes that repository's hooks and
    /// honours its config — arbitrary code execution by default. The profile
    /// (applied to **every** command this client runs):
    ///
    /// - **Disables hooks** — `core.hooksPath=/dev/null` pinned via git's
    ///   env-based config (`GIT_CONFIG_COUNT`/`KEY_n`/`VALUE_n`, git ≥ 2.31;
    ///   verified to suppress hooks on Windows too) — and `core.fsmonitor`
    ///   (a config-driven daemon launch).
    /// - **Removes inherited repo redirectors** so a poisoned parent
    ///   environment can't point commands at another repository: `GIT_DIR`,
    ///   `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`,
    ///   `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_NAMESPACE`,
    ///   `GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_PARAMETERS`,
    ///   `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`.
    /// - **Skips system config** (`GIT_CONFIG_NOSYSTEM=1`) and keeps terminal
    ///   prompts off everywhere (`GIT_TERMINAL_PROMPT=0`).
    ///
    /// What it does NOT do: sandbox the git binary itself, or stop the repo's
    /// *content* from being malicious. In a **colocated jj repo**, git hooks
    /// only run when *git* commands run — harden the `Git` client; `Jj` needs
    /// no equivalent (jj has no repo-local hooks; see the vcs-jj docs).
    ///
    /// Chainable — `Git::with_runner(rec).harden()` works in tests; use
    /// [`Git::hardened()`](Git::hardened) for the common case.
    pub fn harden(self) -> Self {
        let removed = [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CEILING_DIRECTORIES",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ];
        let mut hardened = self;
        for key in removed {
            hardened = hardened.default_env_remove(key);
        }
        hardened
            .default_env("GIT_CONFIG_NOSYSTEM", "1")
            .default_env("GIT_TERMINAL_PROMPT", "0")
            .default_env("GIT_CONFIG_COUNT", "2")
            .default_env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .default_env("GIT_CONFIG_VALUE_0", "/dev/null")
            .default_env("GIT_CONFIG_KEY_1", "core.fsmonitor")
            .default_env("GIT_CONFIG_VALUE_1", "false")
    }

    /// Switch to `branch`, carrying uncommitted changes (tracked *and*
    /// untracked) across via the stash: `stash push -u` → `checkout` →
    /// `stash pop`. A clean tree skips the stash round-trip entirely — a
    /// `stash push` there would save nothing and the later pop would pop an
    /// older, unrelated stash.
    ///
    /// Failure behaviour:
    /// - `checkout` fails (atomic — the working copy stays on the original
    ///   branch): the stash is popped back to restore the original state, and
    ///   the checkout error is returned. If that restoring pop *also* fails,
    ///   the changes stay safe in the stash (`git stash list`).
    /// - `stash pop` on the target branch conflicts: the error is returned with
    ///   the target branch checked out; git keeps the stash entry, so the
    ///   changes can be resolved or re-applied manually.
    ///
    /// Inherent (not on the object-safe trait): a composed operation, not a 1:1
    /// CLI verb — mock the underlying `status`/`stash_*`/`checkout` instead.
    pub async fn switch_with_stash(&self, dir: &Path, branch: &str) -> Result<()> {
        // Untracked-inclusive guard to match `stash push -u`: "dirty" must mean
        // the same thing to the guard and to the stash.
        if self.status(dir).await?.is_empty() {
            return self.checkout(dir, branch).await;
        }
        self.stash_push(dir, true).await?;
        match self.checkout(dir, branch).await {
            Ok(()) => self.stash_pop(dir).await,
            Err(err) => {
                // A failed checkout is atomic — we are still on the original
                // branch, so popping restores the exact pre-call state. If the
                // pop fails too, the stash entry is preserved for the caller.
                let _ = self.stash_pop(dir).await;
                Err(err)
            }
        }
    }

    /// `git_dir` resolved to an absolute path — `rev-parse --git-dir` may report
    /// it relative to `dir` (e.g. `.git`), which the filesystem probes need joined.
    async fn resolved_git_dir(&self, dir: &Path) -> Result<PathBuf> {
        let git_dir = PathBuf::from(
            self.core
                .text(self.core.command_in(dir, ["rev-parse", "--git-dir"]))
                .await?,
        );
        Ok(if git_dir.is_absolute() {
            git_dir
        } else {
            dir.join(git_dir)
        })
    }
}

impl Git {
    /// A hardened real (job-backed) client — `Git::new().harden()`; see
    /// [`harden`](Git::harden) for what the profile does.
    pub fn hardened() -> Self {
        Self::new().harden()
    }
}

/// A [`Git`] client with a working directory bound, so calls drop the leading
/// `dir` argument — `git.at(dir).status()` is `git.status(dir)`. Construct one
/// with [`Git::at`] (or, through the facade, `vcs_core::Repo::git_at`). Cheap to
/// copy: it only borrows the client and the path.
pub struct GitAt<'a, R: ProcessRunner = processkit::JobRunner> {
    git: &'a Git<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: the view only holds two references, so it is
// `Copy` for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy`
// bound that the real default `JobRunner` doesn't satisfy, silently dropping
// `Copy` on the production `Repo::git_at()` handle.
impl<R: ProcessRunner> Clone for GitAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for GitAt<'_, R> {}

/// Generate [`GitAt`] forwarders from a method list: `bare` methods forward
/// verbatim, `dir` methods inject `self.dir` as the first argument.
macro_rules! git_at_forwarders {
    (
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
    ) => {
        impl<'a, R: ProcessRunner> GitAt<'a, R> {
            $(
                #[doc = concat!("Bound form of [`Git`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.git.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`Git`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.git.$dn(self.dir, $($da),*).await
                }
            )*
        }
    };
}

git_at_forwarders! {
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
        fn capabilities() -> Result<GitCapabilities>;
        fn clone_repo(url: &str, dest: &Path, spec: CloneSpec) -> Result<()>;
    }
    dir {
        fn status() -> Result<Vec<StatusEntry>>;
        fn status_text() -> Result<String>;
        fn status_tracked() -> Result<Vec<StatusEntry>>;
        fn conflicted_files() -> Result<Vec<String>>;
        fn current_branch() -> Result<String>;
        fn branches() -> Result<Vec<Branch>>;
        fn log(max: usize) -> Result<Vec<Commit>>;
        fn log_range(range: &str, max: usize) -> Result<Vec<Commit>>;
        fn rev_parse(rev: &str) -> Result<String>;
        fn rev_parse_short(rev: &str) -> Result<String>;
        fn init() -> Result<()>;
        fn add(paths: &[PathBuf]) -> Result<()>;
        fn commit(message: &str) -> Result<()>;
        fn create_branch(name: &str) -> Result<()>;
        fn checkout(reference: &str) -> Result<()>;
        fn checkout_detach(commit: &str) -> Result<()>;
        fn commit_paths(paths: &[PathBuf], message: &str, amend: bool) -> Result<()>;
        fn last_commit_message() -> Result<String>;
        fn is_unborn() -> Result<bool>;
        fn diff_is_empty() -> Result<bool>;
        fn common_dir() -> Result<PathBuf>;
        fn git_dir() -> Result<PathBuf>;
        fn resolve_commit(rev: &str) -> Result<String>;
        fn remote_head_branch() -> Result<Option<String>>;
        fn branch_exists(name: &str) -> Result<bool>;
        fn remote_branch_exists(name: &str) -> Result<bool>;
        fn remote_url(remote: &str) -> Result<String>;
        fn upstream() -> Result<Option<String>>;
        fn remote_branches(remote: &str) -> Result<Vec<String>>;
        fn is_merged(branch: &str, target: &str) -> Result<bool>;
        fn set_upstream(branch: &str, upstream: &str) -> Result<()>;
        fn delete_branch(name: &str, force: bool) -> Result<()>;
        fn rename_branch(old: &str, new: &str) -> Result<()>;
        fn rev_list_count(range: &str) -> Result<usize>;
        fn diff_range_is_empty(range: &str) -> Result<bool>;
        fn diff_stat(range: &str) -> Result<DiffStat>;
        fn diff_text(spec: DiffSpec) -> Result<String>;
        fn diff(spec: DiffSpec) -> Result<Vec<FileDiff>>;
        fn staged_is_empty() -> Result<bool>;
        fn is_rebase_in_progress() -> Result<bool>;
        fn is_merge_in_progress() -> Result<bool>;
        fn fetch() -> Result<()>;
        fn fetch_from(remote: &str) -> Result<()>;
        fn fetch_remote_branch(branch: &str) -> Result<()>;
        fn push(spec: GitPush) -> Result<()>;
        fn merge_squash(branch: &str) -> Result<()>;
        fn merge_commit(branch: &str, no_ff: bool, message: Option<String>) -> Result<()>;
        fn merge_no_commit(branch: &str, squash: bool, no_ff: bool) -> Result<()>;
        fn merge_abort() -> Result<()>;
        fn merge_continue() -> Result<()>;
        fn reset_merge() -> Result<()>;
        fn reset_hard(rev: &str) -> Result<()>;
        fn rebase(onto: &str) -> Result<()>;
        fn rebase_abort() -> Result<()>;
        fn rebase_continue() -> Result<()>;
        fn stash_push(include_untracked: bool) -> Result<()>;
        fn stash_pop() -> Result<()>;
        fn switch_with_stash(branch: &str) -> Result<()>;
        fn worktree_list() -> Result<Vec<Worktree>>;
        fn worktree_add(spec: WorktreeAdd) -> Result<()>;
        fn worktree_remove(path: &Path, force: bool) -> Result<()>;
        fn worktree_move(from: &Path, to: &Path) -> Result<()>;
        fn worktree_prune() -> Result<()>;
        fn tag_create(name: &str, rev: Option<String>) -> Result<()>;
        fn tag_create_annotated(name: &str, message: &str, rev: Option<String>) -> Result<()>;
        fn tag_list() -> Result<Vec<String>>;
        fn tag_delete(name: &str) -> Result<()>;
        fn show_file(rev: &str, path: &str) -> Result<String>;
        fn config_get(key: &str) -> Result<Option<String>>;
        fn config_set(key: &str, value: &str) -> Result<()>;
        fn remote_add(name: &str, url: &str) -> Result<()>;
        fn remote_set_url(name: &str, url: &str) -> Result<()>;
        fn blame(path: &str, rev: Option<String>) -> Result<Vec<BlameLine>>;
        fn cherry_pick(rev: &str) -> Result<()>;
        fn revert(rev: &str) -> Result<()>;
        fn rebase_skip() -> Result<()>;
    }
}

/// Synchronous, best-effort helpers for contexts that cannot `.await` — chiefly
/// a `Drop` guard. They shell out through `std::process` directly (no async, no
/// job-containment), so reserve them for short-lived cleanup.
pub mod blocking {
    use std::path::Path;
    use std::process::Command;

    /// Remove a worktree synchronously (`git worktree remove [--force] <path>`).
    pub fn worktree_remove(dir: &Path, path: &Path, force: bool) -> std::io::Result<()> {
        let mut cmd = Command::new(super::BINARY);
        cmd.current_dir(dir).args(["worktree", "remove"]);
        if force {
            cmd.arg("--force");
        }
        cmd.arg(path);
        let status = cmd.status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`git worktree remove` exited with {status}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_git() {
        assert_eq!(BINARY, "git");
    }

    // Compile-time guard: the bound view must stay `Copy` for the *default*
    // `JobRunner` (the production `Repo::git_at()` handle), not just for the
    // `&RecordingRunner` the other tests use. A derived `Copy` would regress this.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GitAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`git.at(dir)`) must produce byte-identical argv to the
    // dir-taking call (`git.method(dir, …)`) — the forwarder injects `self.dir`
    // in the right place and nothing else changes.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);

        // A method with trailing args (dir injected first).
        git.merge_commit(dir, "feat", true, None).await.unwrap();
        git.at(dir).merge_commit("feat", true, None).await.unwrap();
        // A method taking a path arg after dir.
        git.worktree_remove(dir, Path::new("/wt"), true)
            .await
            .unwrap();
        git.at(dir)
            .worktree_remove(Path::new("/wt"), true)
            .await
            .unwrap();
        // One of the new query methods.
        git.conflicted_files(dir).await.unwrap();
        git.at(dir).conflicted_files().await.unwrap();
        // One of the §4 additions.
        git.tag_delete(dir, "v1").await.unwrap();
        git.at(dir).tag_delete("v1").await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[4].args_str(), calls[5].args_str());
        assert_eq!(calls[6].args_str(), calls[7].args_str());
        // The bound calls also carried the bound dir as their working directory.
        assert_eq!(calls[1].cwd.as_deref(), Some(dir.as_os_str()));
        assert_eq!(calls[3].cwd.as_deref(), Some(dir.as_os_str()));
    }

    // Hermetic: the real status() command-building + porcelain parsing run
    // against a scripted runner — no `git` binary needed, so this runs on CI.
    #[tokio::test]
    async fn status_parses_scripted_output() {
        // `-z` output: NUL-delimited records, raw paths.
        let git =
            Git::with_runner(ScriptedRunner::new().on(["status"], Reply::ok(" M a.rs\0?? b.rs\0")));
        let entries = git.status(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].code, " M");
        assert_eq!(entries[1].path, "b.rs");
    }

    // `status_tracked` is `status` minus untracked files — same parser, extra flag.
    #[tokio::test]
    async fn status_tracked_excludes_untracked_flag() {
        let rec = RecordingRunner::replying(Reply::ok(" M a.rs\0"));
        let git = Git::with_runner(&rec);
        let entries = git.status_tracked(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, " M");
        assert_eq!(
            rec.only_call().args_str(),
            ["status", "--porcelain=v1", "-z", "--untracked-files=no"]
        );
    }

    // `conflicted_files` lists unmerged paths NUL-delimited (no quoting).
    #[tokio::test]
    async fn conflicted_files_builds_args_and_parses_nul_list() {
        let rec = RecordingRunner::replying(Reply::ok("a.rs\0sub/spaced name.rs\0"));
        let git = Git::with_runner(&rec);
        let paths = git
            .conflicted_files(Path::new("."))
            .await
            .expect("conflicted_files");
        assert_eq!(paths, ["a.rs", "sub/spaced name.rs"]);
        assert_eq!(
            rec.only_call().args_str(),
            ["diff", "--name-only", "--diff-filter=U", "-z"]
        );
    }

    #[tokio::test]
    async fn rev_parse_short_builds_short_flag() {
        let rec = RecordingRunner::replying(Reply::ok("a1b2c3d\n"));
        let git = Git::with_runner(&rec);
        let out = git.rev_parse_short(Path::new("/r"), "HEAD").await.unwrap();
        assert_eq!(out, "a1b2c3d");
        assert_eq!(rec.only_call().args_str(), ["rev-parse", "--short", "HEAD"]);
    }

    // A non-zero exit surfaces as a structured `Error::Exit`.
    #[tokio::test]
    async fn nonzero_exit_is_structured_error() {
        let git = Git::with_runner(
            ScriptedRunner::new().on(["status"], Reply::fail(128, "not a git repository")),
        );
        match git.status(Path::new(".")).await.unwrap_err() {
            Error::Exit { code, stderr, .. } => {
                assert_eq!(code, 128);
                assert!(stderr.contains("not a git repository"), "{stderr}");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    // diff_is_empty maps the raw exit code itself: 0 → clean, 1 → dirty, and
    // anything else is a real failure surfaced as Error::Exit.
    #[tokio::test]
    async fn diff_is_empty_maps_exit_codes() {
        let clean = Git::with_runner(ScriptedRunner::new().on(["diff", "--quiet"], Reply::ok("")));
        assert!(clean.diff_is_empty(Path::new(".")).await.unwrap());

        let dirty =
            Git::with_runner(ScriptedRunner::new().on(["diff", "--quiet"], Reply::fail(1, "")));
        assert!(!dirty.diff_is_empty(Path::new(".")).await.unwrap());

        let broken = Git::with_runner(
            ScriptedRunner::new().on(["diff", "--quiet"], Reply::fail(128, "fatal: not a repo")),
        );
        assert!(matches!(
            broken.diff_is_empty(Path::new(".")).await.unwrap_err(),
            Error::Exit { code: 128, .. }
        ));
    }

    // `add` must insert `--` before the pathspecs so a path can never be parsed
    // as an option. No fallback rule: the run only matches if `add --` was built.
    #[tokio::test]
    async fn add_inserts_pathspec_separator() {
        let git = Git::with_runner(ScriptedRunner::new().on(["add", "--"], Reply::ok("")));
        git.add(Path::new("."), &[PathBuf::from("f.rs")])
            .await
            .expect("add should build `add -- <paths>`");
    }

    #[tokio::test]
    async fn worktree_list_parses_porcelain() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["worktree", "list"],
            Reply::ok("worktree /repo\nHEAD abc\nbranch refs/heads/main\n"),
        ));
        let wts = git.worktree_list(Path::new(".")).await.expect("list");
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert_eq!(wts[0].head.as_deref(), Some("abc"));
    }

    // The new-branch worktree must build `worktree add -b <name> <path> <base>`,
    // in that exact order; only the full argv is scripted (no fallback).
    #[tokio::test]
    async fn worktree_add_builds_branch_path_and_base() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_add(
            Path::new("/repo"),
            WorktreeAdd::create_branch("/wt", "feature", "main"),
        )
        .await
        .expect("worktree add");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "add", "-b", "feature", "/wt", "main"]
        );
    }

    #[tokio::test]
    async fn worktree_remove_passes_force_then_path() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_remove(Path::new("/repo"), Path::new("/wt"), true)
            .await
            .expect("remove");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "remove", "--force", "/wt"]
        );
    }

    // `--no-checkout` must land between `-b <name>` and the path.
    #[tokio::test]
    async fn worktree_add_no_checkout_inserts_flag() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_add(
            Path::new("/repo"),
            WorktreeAdd::checkout("/wt", "main").no_checkout(),
        )
        .await
        .expect("worktree add");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "add", "--no-checkout", "/wt", "main"]
        );
    }

    #[tokio::test]
    async fn checkout_detach_builds_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.checkout_detach(Path::new("."), "abc123")
            .await
            .expect("detach");
        assert_eq!(
            rec.only_call().args_str(),
            ["checkout", "--detach", "abc123"]
        );
    }

    // Partial amend commit must build `commit --amend -m <msg> --only -- <paths>`.
    #[tokio::test]
    async fn commit_paths_builds_only_amend_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.commit_paths(
            Path::new("."),
            &[PathBuf::from("a.rs"), PathBuf::from("b.rs")],
            "msg",
            true,
        )
        .await
        .expect("commit_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "commit", "--amend", "-m", "msg", "--only", "--", "a.rs", "b.rs"
            ]
        );
    }

    // is_unborn maps the rev-parse exit code: 0 → has commits (false), 1 →
    // unborn (true), anything else is a structured error.
    #[tokio::test]
    async fn is_unborn_maps_exit_codes() {
        let born = Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::ok("abc\n")));
        assert!(!born.is_unborn(Path::new(".")).await.unwrap());
        let unborn = Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::fail(1, "")));
        assert!(unborn.is_unborn(Path::new(".")).await.unwrap());
        let broken =
            Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::fail(128, "boom")));
        assert!(matches!(
            broken.is_unborn(Path::new(".")).await.unwrap_err(),
            Error::Exit { code: 128, .. }
        ));
    }

    #[tokio::test]
    async fn log_range_builds_range_and_format() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.log_range(Path::new("."), "main..HEAD", 5)
            .await
            .expect("log_range");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "log",
                "main..HEAD",
                "-n5",
                "-z",
                "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s"
            ]
        );
    }

    #[tokio::test]
    async fn stash_push_adds_include_untracked() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.stash_push(Path::new("."), true).await.expect("stash");
        assert_eq!(
            rec.only_call().args_str(),
            ["stash", "push", "--include-untracked"]
        );
    }

    // `diff_text` for the working tree must build `diff HEAD` plus the stable
    // machine-output flags, in order.
    #[tokio::test]
    async fn diff_text_builds_working_tree_args() {
        // The `rev-parse` unborn probe replies exit 0 (HEAD resolves), so the diff
        // targets HEAD. The probe is the first call; the diff is the last.
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(
            rec.calls().last().unwrap().args_str(),
            ["diff", "HEAD", "--no-color", "--no-ext-diff", "-M"]
        );
    }

    // On an unborn repo the working-tree diff targets the empty tree instead of
    // the unresolvable `HEAD`, so it returns additions rather than erroring. The
    // diff rule only matches the empty-tree argv, so a `HEAD` target would miss it.
    #[tokio::test]
    async fn diff_text_working_tree_uses_empty_tree_when_unborn() {
        let git = Git::with_runner(
            ScriptedRunner::new()
                .on(["rev-parse"], Reply::fail(1, "")) // unborn: HEAD doesn't resolve
                .on(["diff", EMPTY_TREE], Reply::ok("EMPTY")),
        );
        let out = git
            .diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(out, "EMPTY");
    }

    // Hermetic: real diff() arg-building (`Rev`) + the ported parser against
    // canned git-format output.
    #[tokio::test]
    async fn diff_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let git = Git::with_runner(ScriptedRunner::new().on(["diff"], Reply::ok(out)));
        let files = git
            .diff(Path::new("."), DiffSpec::Rev("HEAD~1".into()))
            .await
            .expect("diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
    }

    #[tokio::test]
    async fn branch_exists_maps_exit_codes() {
        let yes = Git::with_runner(ScriptedRunner::new().on(["show-ref"], Reply::ok("")));
        assert!(yes.branch_exists(Path::new("."), "main").await.unwrap());
        let no = Git::with_runner(ScriptedRunner::new().on(["show-ref"], Reply::fail(1, "")));
        assert!(!no.branch_exists(Path::new("."), "nope").await.unwrap());
    }

    // The full ref prefix is stripped but a slashed default branch survives; an
    // unset origin/HEAD (non-zero exit) is `None`, not an error.
    #[tokio::test]
    async fn remote_head_branch_strips_prefix_and_keeps_slashes() {
        let simple = Git::with_runner(
            ScriptedRunner::new().on(["symbolic-ref"], Reply::ok("refs/remotes/origin/main\n")),
        );
        assert_eq!(
            simple
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("main")
        );

        let slashed = Git::with_runner(ScriptedRunner::new().on(
            ["symbolic-ref"],
            Reply::ok("refs/remotes/origin/release/v2\n"),
        ));
        assert_eq!(
            slashed
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("release/v2")
        );

        let unset =
            Git::with_runner(ScriptedRunner::new().on(["symbolic-ref"], Reply::fail(1, "")));
        assert!(
            unset
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .is_none()
        );
    }

    // remote_branch_exists must pass `GIT_TERMINAL_PROMPT=0` and treat empty
    // stdout as "absent".
    #[tokio::test]
    async fn remote_branch_exists_sets_env_and_reads_stdout() {
        let rec = RecordingRunner::replying(Reply::ok("abc123\trefs/heads/main\n"));
        let git = Git::with_runner(&rec);
        assert!(
            git.remote_branch_exists(Path::new("/repo"), "main")
                .await
                .unwrap()
        );
        let call = rec.only_call();
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
        // Exact-ref query — a bare `main` would tail-match `bar/main`.
        assert_eq!(call.args_str(), ["ls-remote", "origin", "refs/heads/main"]);

        let empty = Git::with_runner(ScriptedRunner::new().on(["ls-remote"], Reply::ok("")));
        assert!(
            !empty
                .remote_branch_exists(Path::new("."), "x")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn diff_stat_parses_counts() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["diff", "--shortstat"],
            Reply::ok(" 2 files changed, 5 insertions(+), 1 deletion(-)\n"),
        ));
        let stat = git.diff_stat(Path::new("."), "main..HEAD").await.unwrap();
        assert_eq!(
            (stat.files_changed, stat.insertions, stat.deletions),
            (2, 5, 1)
        );
    }

    #[tokio::test]
    async fn status_text_returns_raw_porcelain() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["status", "--porcelain=v1"],
            Reply::ok(" M a.rs\n?? b.rs\n"),
        ));
        let text = git.status_text(Path::new(".")).await.expect("status_text");
        assert!(text.contains(" M a.rs") && text.contains("?? b.rs"));
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let git = Git::with_runner(ScriptedRunner::new().on(["status", "-s"], Reply::ok("ok\n")));
        assert_eq!(git.run_args(&["status", "-s"]).await.unwrap(), "ok");
    }

    // Conflict markers live on stdout (`CONFLICT (…)`) or stderr (`Automatic
    // merge failed`); either should classify. A plain error must not.
    #[test]
    fn classifies_merge_conflict() {
        let on_stdout = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: "CONFLICT (content): Merge conflict in a.rs".into(),
            stderr: String::new(),
        };
        let on_stderr = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: String::new(),
            stderr: "Automatic merge failed; fix conflicts and then commit".into(),
        };
        let unrelated = Error::Exit {
            program: "git".into(),
            code: 128,
            stdout: String::new(),
            stderr: "fatal: not a git repository".into(),
        };
        assert!(is_merge_conflict(&on_stdout));
        assert!(is_merge_conflict(&on_stderr));
        assert!(!is_merge_conflict(&unrelated));
        assert!(!is_nothing_to_commit(&on_stdout));
    }

    #[test]
    fn classifies_nothing_to_commit_and_transient_fetch() {
        let nothing = Error::Exit {
            program: "git".into(),
            code: 1,
            stdout: "nothing to commit, working tree clean".into(),
            stderr: String::new(),
        };
        assert!(is_nothing_to_commit(&nothing));

        let dns = Error::Exit {
            program: "git".into(),
            code: 128,
            stdout: String::new(),
            stderr: "fatal: unable to access 'https://x/': Could not resolve host: x".into(),
        };
        assert!(is_transient_fetch_error(&dns));
        assert!(!is_transient_fetch_error(&nothing));

        // A processkit timeout (no captured output) is transient too.
        let timeout = Error::Timeout {
            program: "git".into(),
            timeout: Duration::from_secs(10),
        };
        assert!(is_transient_fetch_error(&timeout));
    }

    #[tokio::test]
    async fn merge_commit_builds_no_ff_and_message() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.merge_commit(Path::new("/r"), "feature", true, Some("merge it".into()))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["merge", "--no-ff", "-m", "merge it", "feature"]
        );
    }

    // No message → `--no-edit` (default message, non-interactive) instead of `$EDITOR`.
    #[tokio::test]
    async fn merge_commit_without_message_uses_no_edit() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.merge_commit(Path::new("/r"), "feature", false, None)
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["merge", "--no-edit", "feature"]
        );
    }

    // rebase/rebase_continue force a no-op editor so a headless caller never hangs.
    #[tokio::test]
    async fn rebase_suppresses_editor() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.rebase(Path::new("/r"), "main").await.unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["rebase", "main"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_EDITOR")
                && v.as_deref().and_then(|o| o.to_str()) == Some("true")
        }));
    }

    #[tokio::test]
    async fn push_builds_set_upstream_remote_refspec() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.push(
            Path::new("/r"),
            GitPush::refspec("feat", "feature").set_upstream(),
        )
        .await
        .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["push", "-u", "origin", "feat:feature"]
        );
    }

    #[tokio::test]
    async fn upstream_maps_unset_to_none() {
        let set =
            Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::ok("origin/main\n")));
        assert_eq!(
            set.upstream(Path::new(".")).await.unwrap().as_deref(),
            Some("origin/main")
        );
        let unset = Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::fail(128, "")));
        assert!(unset.upstream(Path::new(".")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_upstream_builds_branch_flag() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.set_upstream(Path::new("/r"), "feat", "origin/feature")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["branch", "--set-upstream-to=origin/feature", "feat"]
        );
    }

    #[tokio::test]
    async fn remote_branches_parses_ls_remote() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["ls-remote"],
            Reply::ok("aaa\trefs/heads/main\nbbb\trefs/heads/feat/x\n"),
        ));
        let branches = git.remote_branches(Path::new("."), "origin").await.unwrap();
        assert_eq!(branches, ["main", "feat/x"]);
    }

    #[tokio::test]
    async fn delete_branch_force_uses_capital_d() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.delete_branch(Path::new("/r"), "old", true)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["branch", "-D", "old"]);
    }

    // `branch --merged` marks the current branch with `*` and a branch checked out
    // in another worktree with `+`; both must still match after marker stripping.
    #[tokio::test]
    async fn is_merged_strips_branch_markers() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["branch", "--merged"],
            Reply::ok("  main\n* feature\n+ wt-branch\n"),
        ));
        for name in ["main", "feature", "wt-branch"] {
            assert!(
                git.is_merged(Path::new("."), name, "main").await.unwrap(),
                "{name} should be reported merged"
            );
        }
        assert!(
            !git.is_merged(Path::new("."), "absent", "main")
                .await
                .unwrap()
        );
    }

    // `fetch` must disable the credential prompt so it fails fast (never hangs) on
    // a remote needing auth — matching the other remote ops.
    #[tokio::test]
    async fn fetch_disables_terminal_prompt() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.fetch(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["fetch", "--quiet"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
    }

    // A transient failure (DNS/network) is retried up to FETCH_ATTEMPTS times.
    #[tokio::test]
    async fn fetch_retries_transient_failures() {
        let rec = RecordingRunner::replying(Reply::fail(
            128,
            "fatal: unable to access: Could not resolve host: example.com",
        ));
        let git = Git::with_runner(&rec);
        assert!(git.fetch(Path::new("/r")).await.is_err());
        assert_eq!(rec.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // A non-transient failure fails fast — no retry.
    #[tokio::test]
    async fn fetch_does_not_retry_permanent_failures() {
        let rec = RecordingRunner::replying(Reply::fail(1, "fatal: couldn't find remote ref"));
        let git = Git::with_runner(&rec);
        assert!(git.fetch(Path::new("/r")).await.is_err());
        assert_eq!(rec.calls().len(), 1);
    }

    // The injection guard: a flag-shaped value in any exposed positional slot
    // must be refused BEFORE anything spawns.
    #[tokio::test]
    async fn flag_like_positionals_are_rejected_before_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        let dir = Path::new("/r");

        assert!(git.checkout(dir, "-evil").await.is_err());
        assert!(git.create_branch(dir, "--force").await.is_err());
        assert!(git.delete_branch(dir, "-D", false).await.is_err());
        assert!(git.rename_branch(dir, "ok", "-bad").await.is_err());
        assert!(git.merge_commit(dir, "-evil", false, None).await.is_err());
        assert!(
            git.merge_no_commit(dir, "-evil", false, true)
                .await
                .is_err()
        );
        assert!(git.merge_squash(dir, "-evil").await.is_err());
        assert!(git.rebase(dir, "-i").await.is_err());
        assert!(git.cherry_pick(dir, "-n").await.is_err());
        assert!(git.revert(dir, "-evil").await.is_err());
        assert!(git.tag_create(dir, "-d", None).await.is_err());
        assert!(
            git.tag_create(dir, "ok", Some("-evil".into()))
                .await
                .is_err()
        );
        assert!(git.tag_delete(dir, "-evil").await.is_err());
        assert!(git.remote_add(dir, "-evil", "url").await.is_err());
        assert!(git.remote_set_url(dir, "-evil", "url").await.is_err());
        assert!(git.set_upstream(dir, "-evil", "origin/x").await.is_err());
        assert!(git.log_range(dir, "-evil", 5).await.is_err());
        assert!(git.rev_list_count(dir, "-evil").await.is_err());
        assert!(git.diff_stat(dir, "-evil").await.is_err());
        assert!(git.diff_range_is_empty(dir, "-evil").await.is_err());
        assert!(
            git.diff_text(dir, DiffSpec::Rev("-evil".into()))
                .await
                .is_err()
        );
        assert!(git.rev_parse(dir, "-evil").await.is_err());
        assert!(git.rev_parse_short(dir, "-evil").await.is_err());
        assert!(git.resolve_commit(dir, "-evil").await.is_err());
        assert!(git.reset_hard(dir, "-evil").await.is_err());
        assert!(git.checkout_detach(dir, "-evil").await.is_err());
        assert!(git.config_set(dir, "-evil", "v").await.is_err());
        assert!(
            git.push(dir, GitPush::branch("-evil")).await.is_err(),
            "refspec guard"
        );
        // Embedded-token-prefix and standalone-rev positionals:
        assert!(git.show_file(dir, "-evil", "f.txt").await.is_err());
        assert!(git.blame(dir, "f.txt", Some("-s".into())).await.is_err());
        assert!(git.remote_url(dir, "-evil").await.is_err());
        assert!(git.remote_branches(dir, "-evil").await.is_err());
        assert!(git.fetch_from(dir, "--upload-pack=x").await.is_err());
        // URL positionals (a leading-`-` url is an RCE-class flag injection).
        assert!(
            git.clone_repo("--upload-pack=x", Path::new("/d"), CloneSpec::new())
                .await
                .is_err()
        );
        assert!(git.remote_add(dir, "ok", "--upload-pack=x").await.is_err());
        assert!(git.remote_set_url(dir, "ok", "-evil").await.is_err());
        assert!(git.is_merged(dir, "-evil", "main").await.is_err());
        assert!(git.config_get(dir, "-evil").await.is_err());
        assert!(
            git.worktree_add(
                dir,
                WorktreeAdd::create_branch(Path::new("/wt"), "-evil", "HEAD")
            )
            .await
            .is_err()
        );
        // Empty values are refused too.
        assert!(git.checkout(dir, "").await.is_err());

        assert!(
            rec.calls().is_empty(),
            "nothing may spawn: {:?}",
            rec.calls()
        );

        // …and legitimate values still pass through unchanged.
        git.checkout(dir, "feature/x").await.expect("checkout");
        assert_eq!(rec.only_call().args_str(), ["checkout", "feature/x"]);
    }

    // The hardened profile lands its env pairs/removals on EVERY command, and
    // composes with per-command env like GIT_TERMINAL_PROMPT.
    #[tokio::test]
    async fn harden_applies_env_profile_to_every_command() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec).harden();
        git.status(Path::new("/r")).await.expect("status");
        git.fetch(Path::new("/r")).await.expect("fetch");

        for call in rec.calls() {
            let has = |k: &str, v: &str| {
                call.envs.iter().any(|(key, val)| {
                    key.to_str() == Some(k) && val.as_deref().and_then(|o| o.to_str()) == Some(v)
                })
            };
            let removed = |k: &str| {
                call.envs
                    .iter()
                    .any(|(key, val)| key.to_str() == Some(k) && val.is_none())
            };
            assert!(has("GIT_CONFIG_NOSYSTEM", "1"), "{:?}", call.args_str());
            assert!(has("GIT_CONFIG_KEY_0", "core.hooksPath"));
            assert!(has("GIT_CONFIG_VALUE_0", "/dev/null"));
            assert!(has("GIT_TERMINAL_PROMPT", "0"));
            assert!(removed("GIT_DIR"), "GIT_DIR scrubbed");
            assert!(removed("GIT_CONFIG_GLOBAL"), "global config scrubbed");
        }
    }

    // RefName/RevSpec accept/reject tables.
    #[test]
    fn ref_name_and_rev_spec_validate() {
        for ok in ["main", "feature/x", "v1.2.3", "a-b_c"] {
            assert!(RefName::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-evil", ".hidden", "a..b", "a b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b",
            "a\\b", "end/", "x.lock",
        ] {
            assert!(RefName::new(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(RevSpec::new("HEAD~2").is_ok());
        assert!(RevSpec::new("main..feature").is_ok());
        assert!(RevSpec::new("-evil").is_err());
        assert!(RevSpec::new("").is_err());
    }

    // capabilities parses real-world version shapes (incl. the Windows build
    // trailer) and gates on the major floor only.
    #[tokio::test]
    async fn capabilities_parse_and_gate_versions() {
        let gh = Git::with_runner(
            ScriptedRunner::new().on(["--version"], Reply::ok("git version 2.54.0.windows.1\n")),
        );
        let caps = gh.capabilities().await.expect("capabilities");
        assert_eq!(caps.version.to_string(), "2.54.0");
        assert!(caps.is_supported());
        caps.ensure_supported().expect("supported");

        // Two-part versions parse (patch defaults to 0); an ancient major fails
        // the gate with a clear message.
        let old = Git::with_runner(
            ScriptedRunner::new().on(["--version"], Reply::ok("git version 1.9\n")),
        );
        let caps = old.capabilities().await.expect("capabilities");
        assert_eq!(
            caps.version,
            GitVersion {
                major: 1,
                minor: 9,
                patch: 0
            }
        );
        let err = caps.ensure_supported().expect_err("unsupported");
        // The message must name the floor and the found version.
        let Error::Spawn { source, .. } = &err else {
            panic!("expected Spawn, got {err:?}");
        };
        let message = source.to_string();
        assert!(message.contains(">= 2"), "names the floor: {message}");
        assert!(
            message.contains("1.9.0"),
            "names the found version: {message}"
        );

        // Garbage output is a parse error, not a silent zero version.
        let garbage =
            Git::with_runner(ScriptedRunner::new().on(["--version"], Reply::ok("not a version")));
        assert!(matches!(
            garbage.capabilities().await.unwrap_err(),
            Error::Parse { .. }
        ));
    }

    // clone_repo is dir-less and appends only the requested flags.
    #[tokio::test]
    async fn clone_repo_builds_flags_and_runs_dirless() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.clone_repo(
            "https://example.com/r.git",
            Path::new("/dest"),
            CloneSpec::new().branch("main").depth(1).bare(),
        )
        .await
        .expect("clone");
        let call = rec.only_call();
        assert_eq!(
            call.args_str(),
            [
                "clone",
                "--branch",
                "main",
                "--depth",
                "1",
                "--bare",
                "https://example.com/r.git",
                "/dest"
            ]
        );
        assert_eq!(call.cwd, None, "clone runs without a working directory");

        let bare = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&bare);
        git.clone_repo("u", Path::new("/d"), CloneSpec::new())
            .await
            .expect("clone");
        assert_eq!(bare.only_call().args_str(), ["clone", "u", "/d"]);
    }

    #[tokio::test]
    async fn tag_methods_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.tag_create(Path::new("/r"), "v1", None).await.unwrap();
        git.tag_create(Path::new("/r"), "v1", Some("abc".into()))
            .await
            .unwrap();
        git.tag_create_annotated(Path::new("/r"), "v2", "notes", None)
            .await
            .unwrap();
        git.tag_delete(Path::new("/r"), "v1").await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["tag", "v1"]);
        assert_eq!(calls[1].args_str(), ["tag", "v1", "abc"]);
        assert_eq!(calls[2].args_str(), ["tag", "-a", "v2", "-m", "notes"]);
        assert_eq!(calls[3].args_str(), ["tag", "-d", "v1"]);
    }

    #[tokio::test]
    async fn tag_list_splits_lines() {
        let git =
            Git::with_runner(ScriptedRunner::new().on(["tag", "--list"], Reply::ok("v1\nv2.0\n")));
        assert_eq!(git.tag_list(Path::new(".")).await.unwrap(), ["v1", "v2.0"]);
    }

    // The `<rev>:<path>` spec requires forward slashes — Windows callers may
    // hand in backslashes.
    #[tokio::test]
    async fn show_file_normalises_path_separators() {
        let rec = RecordingRunner::replying(Reply::ok("content\n"));
        let git = Git::with_runner(&rec);
        let out = git
            .show_file(Path::new("/r"), "HEAD", "sub\\dir\\f.txt")
            .await
            .expect("show_file");
        assert_eq!(out, "content");
        assert_eq!(rec.only_call().args_str(), ["show", "HEAD:sub/dir/f.txt"]);
    }

    // config --get: exit 0 → Some(value), exit 1 → None (unset), other → error.
    #[tokio::test]
    async fn config_get_maps_exit_codes() {
        let set =
            Git::with_runner(ScriptedRunner::new().on(["config", "--get"], Reply::ok("Alice\n")));
        assert_eq!(
            set.config_get(Path::new("."), "user.name").await.unwrap(),
            Some("Alice".to_string())
        );
        let unset =
            Git::with_runner(ScriptedRunner::new().on(["config", "--get"], Reply::fail(1, "")));
        assert_eq!(
            unset.config_get(Path::new("."), "user.name").await.unwrap(),
            None
        );
        // A multi-valued key (exit 2) or worse is a real error.
        let multi = Git::with_runner(
            ScriptedRunner::new().on(["config", "--get"], Reply::fail(2, "multiple values")),
        );
        assert!(
            multi
                .config_get(Path::new("."), "remote.all")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn blame_builds_rev_before_pathspec_separator() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.blame(Path::new("/r"), "src/lib.rs", Some("HEAD~1".into()))
            .await
            .unwrap();
        git.blame(Path::new("/r"), "src/lib.rs", None)
            .await
            .unwrap();
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["blame", "--line-porcelain", "HEAD~1", "--", "src/lib.rs"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["blame", "--line-porcelain", "--", "src/lib.rs"]
        );
    }

    // revert must never open an editor: --no-edit plus the env backstop.
    #[tokio::test]
    async fn sequencer_methods_suppress_editors() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.revert(Path::new("/r"), "abc").await.unwrap();
        git.cherry_pick(Path::new("/r"), "abc").await.unwrap();
        git.rebase_skip(Path::new("/r")).await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["revert", "--no-edit", "abc"]);
        assert_eq!(calls[1].args_str(), ["cherry-pick", "abc"]);
        assert_eq!(calls[2].args_str(), ["rebase", "--skip"]);
        for call in &calls {
            assert!(
                call.envs
                    .iter()
                    .any(|(k, _)| k.to_str() == Some("GIT_EDITOR")),
                "editor suppressed on {:?}",
                call.args_str()
            );
        }
    }

    #[tokio::test]
    async fn remote_add_and_set_url_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.remote_add(Path::new("/r"), "up", "https://x/y.git")
            .await
            .unwrap();
        git.remote_set_url(Path::new("/r"), "up", "https://x/z.git")
            .await
            .unwrap();
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["remote", "add", "up", "https://x/y.git"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["remote", "set-url", "up", "https://x/z.git"]
        );
    }

    // Dirty tree: stash -u → checkout → pop, in that order.
    #[tokio::test]
    async fn switch_with_stash_round_trips_dirty_tree() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["status"], Reply::ok(" M a.rs\0"))
                .on(["stash", "push"], Reply::ok(""))
                .on(["checkout"], Reply::ok(""))
                .on(["stash", "pop"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        git.switch_with_stash(Path::new("/r"), "feature")
            .await
            .expect("switch");
        let calls = rec.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[1].args_str(),
            ["stash", "push", "--include-untracked"]
        );
        assert_eq!(calls[2].args_str(), ["checkout", "feature"]);
        assert_eq!(calls[3].args_str(), ["stash", "pop"]);
    }

    // A clean tree skips the stash round-trip — a no-op `stash push` would make
    // the later pop grab an older, unrelated stash.
    #[tokio::test]
    async fn switch_with_stash_skips_stash_on_clean_tree() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["status"], Reply::ok(""))
                .on(["checkout"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        git.switch_with_stash(Path::new("/r"), "feature")
            .await
            .expect("switch");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|c| c.args_str()[0] != "stash"));
    }

    // A failed checkout pops the stash back (we are still on the original
    // branch) and surfaces the checkout error.
    #[tokio::test]
    async fn switch_with_stash_restores_on_checkout_failure() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["status"], Reply::ok(" M a.rs\0"))
                .on(["stash", "push"], Reply::ok(""))
                .on(["checkout"], Reply::fail(1, "error: pathspec 'nope'"))
                .on(["stash", "pop"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        let err = git
            .switch_with_stash(Path::new("/r"), "nope")
            .await
            .expect_err("checkout error must surface");
        assert!(matches!(err, Error::Exit { .. }));
        let calls = rec.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[3].args_str(), ["stash", "pop"], "restoring pop ran");
    }

    // `fetch_from` names the remote, keeps the prompt off, and shares the
    // transient retry.
    #[tokio::test]
    async fn fetch_from_builds_args_and_retries() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.fetch_from(Path::new("/r"), "upstream")
            .await
            .expect("fetch_from");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["fetch", "--quiet", "upstream"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));

        let failing = RecordingRunner::replying(Reply::fail(128, "fatal: Connection timed out"));
        let git = Git::with_runner(&failing);
        assert!(git.fetch_from(Path::new("/r"), "upstream").await.is_err());
        assert_eq!(failing.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // The consumer-facing mock seam: a function depending on `&dyn GitApi` is
    // tested with a generated mock.
    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        async fn on_branch(git: &dyn GitApi, want: &str) -> bool {
            git.current_branch(Path::new(".")).await.unwrap() == want
        }
        let mut mock = MockGitApi::new();
        mock.expect_current_branch()
            .returning(|_| Ok("main".to_string()));
        assert!(on_branch(&mock, "main").await);
    }
}
