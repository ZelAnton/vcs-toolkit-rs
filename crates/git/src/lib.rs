//! `vcs-git` — automate Git from Rust through CLI process execution.
//!
//! Async, mockable, and structured-error: consumers depend on the [`GitApi`]
//! trait and substitute a mock for the real [`Git`] client in tests. Commands
//! run inside an OS job (via [`vcs_process`]) so a `git` subprocess is never
//! orphaned, and honour an optional [timeout](Git::default_timeout).
//!
//! ```no_run
//! use vcs_git::{Git, GitApi};
//! use std::path::Path;
//!
//! # async fn run(git: &dyn GitApi) -> Result<(), vcs_process::CommandError> {
//! let branch = git.current_branch(Path::new(".")).await?;
//! # let _ = branch; Ok(()) }
//! ```
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockGitApi`, or inject a fake runner with
//! `Git::with_runner(`[`ScriptedRunner`](vcs_process::ScriptedRunner)`)`.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use vcs_process::{CommandError, Exec, JobRunner, Output, Result, Runner};

mod parse;
pub use parse::{Branch, Commit, StatusEntry};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "git";

/// The Git operations this crate exposes — the interface consumers code against
/// and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitApi: Send + Sync {
    /// Run `git <args>` in the current directory, returning trimmed stdout
    /// (throws on a non-zero exit). A raw escape hatch for unmodelled commands.
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitApi::run`] but never errors on exit code — returns the [`Output`].
    async fn run_raw(&self, args: &[String]) -> io::Result<Output>;
    /// Installed Git version (`git --version`).
    async fn version(&self) -> Result<String>;
    /// Working-tree status (`git status --porcelain`).
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
}

/// The real Git client. Generic over the [`Runner`] so tests can inject a fake
/// process executor; `Git::new()` uses the real job-backed runner.
pub struct Git<R: Runner = JobRunner> {
    runner: R,
    timeout: Option<Duration>,
}

impl Git<JobRunner> {
    /// A client backed by the real `git` binary.
    pub fn new() -> Self {
        Git {
            runner: JobRunner,
            timeout: None,
        }
    }
}

impl Default for Git<JobRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Runner> Git<R> {
    /// A client that runs commands through `runner` — pass a fake in tests.
    pub fn with_runner(runner: R) -> Self {
        Git {
            runner,
            timeout: None,
        }
    }

    /// Kill any command that runs longer than `timeout` (applies to all commands;
    /// override per-call via the raw [`Exec`] API).
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn exec(&self, args: &[&str]) -> Exec {
        Exec::new(BINARY).maybe_timeout(self.timeout).args(args)
    }

    fn exec_in(&self, dir: &Path, args: &[&str]) -> Exec {
        Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .current_dir(dir)
            .args(args)
    }
}

#[async_trait::async_trait]
impl<R: Runner> GitApi for Git<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        Ok(Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .args(args)
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn run_raw(&self, args: &[String]) -> io::Result<Output> {
        Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .args(args)
            .output_with(&self.runner)
            .await
    }

    async fn version(&self) -> Result<String> {
        Ok(self
            .exec(&["--version"])
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>> {
        let out = self
            .exec_in(dir, &["status", "--porcelain"])
            .checked_with(&self.runner)
            .await?;
        Ok(parse::parse_porcelain(&out.stdout))
    }

    async fn current_branch(&self, dir: &Path) -> Result<String> {
        Ok(self
            .exec_in(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>> {
        let out = self
            .exec_in(dir, &["branch"])
            .checked_with(&self.runner)
            .await?;
        Ok(parse::parse_branches(&out.stdout))
    }

    async fn log(&self, dir: &Path, max: usize) -> Result<Vec<Commit>> {
        let n = format!("-n{max}");
        let out = self
            .exec_in(dir, &["log", &n, "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s"])
            .checked_with(&self.runner)
            .await?;
        Ok(parse::parse_log(&out.stdout))
    }

    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        Ok(self
            .exec_in(dir, &["rev-parse", rev])
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn init(&self, dir: &Path) -> Result<()> {
        self.exec_in(dir, &["init"])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()> {
        let mut exec = Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .current_dir(dir)
            .arg("add")
            .arg("--");
        for path in paths {
            exec = exec.arg(path);
        }
        exec.checked_with(&self.runner).await.map(drop)
    }

    async fn commit(&self, dir: &Path, message: &str) -> Result<()> {
        self.exec_in(dir, &["commit", "-m", message])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn create_branch(&self, dir: &Path, name: &str) -> Result<()> {
        self.exec_in(dir, &["branch", name])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()> {
        self.exec_in(dir, &["checkout", reference])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn diff_is_empty(&self, dir: &Path) -> Result<bool> {
        let out = self
            .exec_in(dir, &["diff", "--quiet"])
            .output_with(&self.runner)
            .await
            .map_err(|source| CommandError::Spawn {
                program: BINARY.to_string(),
                source,
            })?;
        if out.timed_out {
            return Err(CommandError::Timeout {
                program: BINARY.to_string(),
                args: "diff --quiet".to_string(),
                timeout: self.timeout.unwrap_or_default(),
            });
        }
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            other => Err(CommandError::Exit {
                program: BINARY.to_string(),
                args: "diff --quiet".to_string(),
                code: other.unwrap_or(-1),
                stderr: out.stderr.trim().to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs_process::ScriptedRunner;

    #[test]
    fn binary_name_is_git() {
        assert_eq!(BINARY, "git");
    }

    // Hermetic: the real status() command-building + porcelain parsing run
    // against a scripted runner — no `git` binary needed, so this runs on CI.
    #[tokio::test]
    async fn status_parses_scripted_output() {
        let git = Git::with_runner(
            ScriptedRunner::new().on(["status", "--porcelain"], Output::ok(" M a.rs\n?? b.rs\n")),
        );
        let entries = git.status(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].code, " M");
        assert_eq!(entries[1].path, "b.rs");
    }

    // A non-zero exit surfaces as a structured `CommandError::Exit`.
    #[tokio::test]
    async fn nonzero_exit_is_structured_error() {
        let git = Git::with_runner(
            ScriptedRunner::new().on(["status"], Output::fail(128, "not a git repository")),
        );
        match git.status(Path::new(".")).await.unwrap_err() {
            CommandError::Exit { code, stderr, .. } => {
                assert_eq!(code, 128);
                assert!(stderr.contains("not a git repository"), "{stderr}");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    // diff_is_empty maps the raw exit code itself: 0 → clean, 1 → dirty, and
    // anything else is a real failure surfaced as CommandError::Exit.
    #[tokio::test]
    async fn diff_is_empty_maps_exit_codes() {
        let clean = Git::with_runner(ScriptedRunner::new().on(["diff", "--quiet"], Output::ok("")));
        assert!(clean.diff_is_empty(Path::new(".")).await.unwrap());

        let dirty =
            Git::with_runner(ScriptedRunner::new().on(["diff", "--quiet"], Output::fail(1, "")));
        assert!(!dirty.diff_is_empty(Path::new(".")).await.unwrap());

        let broken = Git::with_runner(
            ScriptedRunner::new().on(["diff", "--quiet"], Output::fail(128, "fatal: not a repo")),
        );
        assert!(matches!(
            broken.diff_is_empty(Path::new(".")).await.unwrap_err(),
            CommandError::Exit { code: 128, .. }
        ));
    }

    // `add` must insert `--` before the pathspecs so a path can never be parsed
    // as an option. No fallback rule: the run only matches if `add --` was built.
    #[tokio::test]
    async fn add_inserts_pathspec_separator() {
        let git = Git::with_runner(ScriptedRunner::new().on(["add", "--"], Output::ok("")));
        git.add(Path::new("."), &[PathBuf::from("f.rs")])
            .await
            .expect("add should build `add -- <paths>`");
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
