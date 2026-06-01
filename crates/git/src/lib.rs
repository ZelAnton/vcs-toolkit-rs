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

use processkit::ProcessRunner;
// Re-export the processkit types that appear in this crate's public API, so
// consumers needn't depend on processkit directly. (`Error`/`Result`/`ProcessResult`
// are in scope here too via this `pub use`.)
pub use processkit::{Error, ProcessResult, Result};

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{Reply, ScriptedRunner};

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
