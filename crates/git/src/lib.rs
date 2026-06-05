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

mod parse;
pub use parse::{
    BlameLine, Branch, ChangeKind, Commit, DiffLine, DiffStat, FileDiff, Hunk, StatusEntry,
    Worktree,
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

/// The Git operations this crate exposes — the interface consumers code against
/// and mock in tests.
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
        self.core
            .text(self.core.command_in(dir, ["rev-parse", rev]))
            .await
    }

    async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String> {
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
        self.core
            .unit(self.core.command_in(dir, ["branch", name]))
            .await
    }

    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["checkout", reference]))
            .await
    }

    async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()> {
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
        // `GIT_TERMINAL_PROMPT=0`: a remote needing credentials must fail fast,
        // never block on an interactive auth prompt.
        let cmd = self
            .core
            .command_in(dir, ["ls-remote", "--heads", remote])
            .env("GIT_TERMINAL_PROMPT", "0");
        self.core.parse(cmd, parse::parse_ls_remote_heads).await
    }

    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool> {
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
        let flag = format!("--set-upstream-to={upstream}");
        self.core
            .unit(self.core.command_in(dir, ["branch", flag.as_str(), branch]))
            .await
    }

    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()> {
        let flag = if force { "-D" } else { "-d" };
        self.core
            .unit(self.core.command_in(dir, ["branch", flag, name]))
            .await
    }

    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["branch", "-m", old, new]))
            .await
    }

    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize> {
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
        // `diff --quiet <range>`: 0 = empty range, 1 = has changes.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--quiet", range]))
            .await
    }

    async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat> {
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
            DiffSpec::Rev(rev) => rev,
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
        self.core
            .unit(self.core.command_in(dir, ["reset", "--hard", rev]))
            .await
    }

    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()> {
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
        self.core
            .unit(self.core.command_in(dir, ["tag", "-d", name]))
            .await
    }

    async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String> {
        // git rejects backslash separators in the `<rev>:<path>` spec ("exists
        // on disk, but not in <rev>") — normalise for Windows callers.
        let spec = format!("{rev}:{}", path.replace('\\', "/"));
        self.core
            .text(self.core.command_in(dir, ["show", spec.as_str()]))
            .await
    }

    async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>> {
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
        self.core
            .unit(self.core.command_in(dir, ["config", key, value]))
            .await
    }

    async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["remote", "add", name, url]))
            .await
    }

    async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["remote", "set-url", name, url]))
            .await
    }

    async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>> {
        let mut args = vec!["blame", "--line-porcelain"];
        if let Some(rev) = rev.as_deref() {
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
        // No editor opens non-interactively, but keep the headless backstop.
        self.core
            .unit(no_editor(self.core.command_in(dir, ["cherry-pick", rev])))
            .await
    }

    async fn revert(&self, dir: &Path, rev: &str) -> Result<()> {
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
