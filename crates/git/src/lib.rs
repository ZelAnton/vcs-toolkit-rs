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
pub use parse::{Branch, Commit, DiffStat, StatusEntry, Worktree};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "git";

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
}

impl WorktreeAdd {
    /// A worktree at `path` checking out an existing `commitish` (e.g. a branch):
    /// `git worktree add <path> <commitish>`.
    pub fn checkout(path: impl Into<PathBuf>, commitish: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            new_branch: None,
            commitish: Some(commitish.into()),
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
        }
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
    /// Current branch name (`git rev-parse --abbrev-ref HEAD`).
    async fn current_branch(&self, dir: &Path) -> Result<String>;
    /// Local branches, current one flagged (`git branch`).
    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>>;
    /// Latest `max` commits, newest first (`git log`).
    async fn log(&self, dir: &Path, max: usize) -> Result<Vec<Commit>>;
    /// Resolve a revision to a full hash (`git rev-parse <rev>`).
    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String>;
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
    /// Whether the working tree has no unstaged changes (`git diff --quiet`).
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
    /// Whether `origin` has `name`, without fetching (`ls-remote --heads origin
    /// <name>`). Runs with `GIT_TERMINAL_PROMPT=0` and a 10s timeout so a missing
    /// credential or a flaky network can't hang the call.
    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
    /// A remote's URL (`remote get-url <remote>`).
    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String>;

    // --- Branches ------------------------------------------------------------

    /// Whether `branch` is fully merged into `target` (`branch --merged <target>`).
    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool>;
    /// Delete a local branch (`branch -d`, or `-D` when `force`).
    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()>;
    /// Rename a local branch (`branch -m <old> <new>`).
    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
    /// Count commits in a range (`rev-list --count <range>`).
    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize>;
    /// Whether a diff range is empty (`diff --quiet <range>`).
    async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool>;
    /// Aggregate change stats for a range (`diff --shortstat <range>`).
    async fn diff_shortstat(&self, dir: &Path, range: &str) -> Result<DiffStat>;

    // --- In-progress state ---------------------------------------------------

    /// Whether the index has no staged changes (`diff --cached --quiet`).
    async fn staged_is_empty(&self, dir: &Path) -> Result<bool>;
    /// Whether a rebase is in progress (a `rebase-merge`/`rebase-apply` dir exists
    /// under the git dir).
    async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool>;
    /// Whether a merge is in progress (a `MERGE_HEAD` exists under the git dir).
    async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool>;

    // --- Mutations -----------------------------------------------------------

    /// Fetch from the default remote (`fetch --quiet`).
    async fn fetch(&self, dir: &Path) -> Result<()>;
    /// Fetch a single branch from `origin` into its remote-tracking ref
    /// (`fetch --quiet origin refs/heads/<b>:refs/remotes/origin/<b>`), with
    /// `GIT_TERMINAL_PROMPT=0`.
    async fn fetch_remote_branch(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Stage a branch's changes without committing (`merge --squash <branch>`).
    async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Merge a branch (`merge [--no-ff] [-m <msg>] <branch>`).
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
    /// Rebase the current branch onto `onto` (`rebase <onto>`).
    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
    /// Abort an in-progress rebase (`rebase --abort`).
    async fn rebase_abort(&self, dir: &Path) -> Result<()>;
    /// Continue a rebase after resolving conflicts (`rebase --continue`).
    async fn rebase_continue(&self, dir: &Path) -> Result<()>;

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

    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        self.core
            .text(self.core.command_in(dir, ["rev-parse", rev]))
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

    async fn diff_is_empty(&self, dir: &Path) -> Result<bool> {
        // `git diff --quiet` is an exit-code answer: 0 = clean, 1 = dirty.
        // `code` still surfaces spawn/timeout/signal failures for us.
        match self
            .core
            .code(self.core.command_in(dir, ["diff", "--quiet"]))
            .await?
        {
            0 => Ok(true),
            1 => Ok(false),
            other => Err(Error::Exit {
                program: BINARY.to_string(),
                code: other,
                stderr: String::new(),
            }),
        }
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
        if res.exit_code() == 0 {
            // e.g. "refs/remotes/origin/main" → "main".
            Ok(res.stdout().trim().rsplit('/').next().map(str::to_string))
        } else {
            Ok(None)
        }
    }

    async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        let refname = format!("refs/heads/{name}");
        match self
            .core
            .code(
                self.core
                    .command_in(dir, ["show-ref", "--verify", "--quiet", refname.as_str()]),
            )
            .await?
        {
            0 => Ok(true),
            1 => Ok(false),
            other => Err(Error::Exit {
                program: BINARY.to_string(),
                code: other,
                stderr: String::new(),
            }),
        }
    }

    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        // No credential prompt, bounded wait: a missing helper or a flaky network
        // must not hang the call. `capture` reports a timeout as a flagged result
        // (non-zero exit) rather than erroring, so an unreachable remote reads as
        // "absent" (`false`) — the best-effort answer a probe wants. A genuine
        // spawn failure (no `git`) still surfaces as an error.
        let cmd = self
            .core
            .command_in(dir, ["ls-remote", "--heads", "origin", name])
            .env("GIT_TERMINAL_PROMPT", "0")
            .timeout(Duration::from_secs(10));
        let res = self.core.capture(cmd).await?;
        Ok(res.exit_code() == 0 && !res.stdout().trim().is_empty())
    }

    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String> {
        self.core
            .text(self.core.command_in(dir, ["remote", "get-url", remote]))
            .await
    }

    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool> {
        let out = self
            .core
            .text(self.core.command_in(dir, ["branch", "--merged", target]))
            .await?;
        // Each line is `  name` / `* name` (current) / `+ name` (checked out in
        // another worktree); strip the marker before comparing.
        Ok(out
            .lines()
            .map(|line| line.trim_start_matches(['*', '+', ' ']))
            .any(|b| b == branch))
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
        match self
            .core
            .code(self.core.command_in(dir, ["diff", "--quiet", range]))
            .await?
        {
            0 => Ok(true),
            1 => Ok(false),
            other => Err(Error::Exit {
                program: BINARY.to_string(),
                code: other,
                stderr: String::new(),
            }),
        }
    }

    async fn diff_shortstat(&self, dir: &Path, range: &str) -> Result<DiffStat> {
        self.core
            .parse(
                self.core.command_in(dir, ["diff", "--shortstat", range]),
                parse::parse_shortstat,
            )
            .await
    }

    async fn staged_is_empty(&self, dir: &Path) -> Result<bool> {
        match self
            .core
            .code(self.core.command_in(dir, ["diff", "--cached", "--quiet"]))
            .await?
        {
            0 => Ok(true),
            1 => Ok(false),
            other => Err(Error::Exit {
                program: BINARY.to_string(),
                code: other,
                stderr: String::new(),
            }),
        }
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
        self.core
            .unit(self.core.command_in(dir, ["fetch", "--quiet"]))
            .await
    }

    async fn fetch_remote_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        let cmd = self
            .core
            .command_in(dir, ["fetch", "--quiet", "origin", refspec.as_str()])
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
        if squash {
            args.push("--squash");
        }
        if no_ff {
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
        self.core
            .unit(self.core.command_in(dir, ["commit", "--no-edit"]))
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
        self.core
            .unit(self.core.command_in(dir, ["rebase", onto]))
            .await
    }

    async fn rebase_abort(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["rebase", "--abort"]))
            .await
    }

    async fn rebase_continue(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["rebase", "--continue"]))
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
}

impl<R: ProcessRunner> Git<R> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_git() {
        assert_eq!(BINARY, "git");
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

    #[tokio::test]
    async fn branch_exists_maps_exit_codes() {
        let yes = Git::with_runner(ScriptedRunner::new().on(["show-ref"], Reply::ok("")));
        assert!(yes.branch_exists(Path::new("."), "main").await.unwrap());
        let no = Git::with_runner(ScriptedRunner::new().on(["show-ref"], Reply::fail(1, "")));
        assert!(!no.branch_exists(Path::new("."), "nope").await.unwrap());
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
        assert!(rec.only_call().envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));

        let empty = Git::with_runner(ScriptedRunner::new().on(["ls-remote"], Reply::ok("")));
        assert!(
            !empty
                .remote_branch_exists(Path::new("."), "x")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn diff_shortstat_parses_counts() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["diff", "--shortstat"],
            Reply::ok(" 2 files changed, 5 insertions(+), 1 deletion(-)\n"),
        ));
        let stat = git
            .diff_shortstat(Path::new("."), "main..HEAD")
            .await
            .unwrap();
        assert_eq!(
            (stat.files_changed, stat.insertions, stat.deletions),
            (2, 5, 1)
        );
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
