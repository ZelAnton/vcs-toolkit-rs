//! `vcs-jj` — automate Jujutsu (`jj`) from Rust through CLI process execution.
//!
//! Async, mockable, and structured-error: consumers depend on the [`JjApi`]
//! trait and substitute a mock for the real [`Jj`] client in tests. Commands run
//! inside an OS job (via [`vcs_process`]) so a `jj` subprocess is never orphaned,
//! and honour an optional [timeout](Jj::default_timeout).
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockJjApi`, or inject a fake runner with
//! `Jj::with_runner(`[`ScriptedRunner`](vcs_process::ScriptedRunner)`)`.

use std::io;
use std::path::Path;
use std::time::Duration;

use vcs_process::{Exec, JobRunner, Output, Result, Runner};

mod parse;
pub use parse::{Bookmark, Change};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "jj";

/// The jj operations this crate exposes — the interface consumers code against
/// and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait JjApi: Send + Sync {
    /// Run `jj <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`JjApi::run`] but never errors on exit code — returns the [`Output`].
    async fn run_raw(&self, args: &[String]) -> io::Result<Output>;
    /// Installed Jujutsu version (`jj --version`).
    async fn version(&self) -> Result<String>;
    /// Working-copy status (`jj status`).
    async fn status(&self, dir: &Path) -> Result<String>;
    /// Changes matching `revset`, newest first, up to `max` (`jj log`).
    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>>;
    /// The working-copy change (`jj log -r @`).
    async fn current_change(&self, dir: &Path) -> Result<Change>;
    /// Set the working-copy change's description (`jj describe -m`).
    async fn describe(&self, dir: &Path, message: &str) -> Result<()>;
    /// Start a new change on top of the working copy (`jj new -m`).
    async fn new_change(&self, dir: &Path, message: &str) -> Result<()>;
    /// Local bookmarks (`jj bookmark list`).
    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Point a bookmark at `revision` (`jj bookmark set <name> -r <revision>`).
    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
    /// Fetch from the git remote (`jj git fetch`).
    async fn git_fetch(&self, dir: &Path) -> Result<()>;
    /// Push to the git remote (`jj git push`, optionally `-b <bookmark>`). The
    /// bookmark is owned (`Option<String>`) to keep the trait `mockall`-friendly.
    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()>;
}

/// The real jj client. Generic over the [`Runner`] so tests can inject a fake
/// process executor; `Jj::new()` uses the real job-backed runner.
pub struct Jj<R: Runner = JobRunner> {
    runner: R,
    timeout: Option<Duration>,
}

impl Jj<JobRunner> {
    /// A client backed by the real `jj` binary.
    pub fn new() -> Self {
        Jj {
            runner: JobRunner,
            timeout: None,
        }
    }
}

impl Default for Jj<JobRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Runner> Jj<R> {
    /// A client that runs commands through `runner` — pass a fake in tests.
    pub fn with_runner(runner: R) -> Self {
        Jj {
            runner,
            timeout: None,
        }
    }

    /// Kill any command that runs longer than `timeout`.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn exec_in(&self, dir: &Path, args: &[&str]) -> Exec {
        Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .current_dir(dir)
            .args(args)
    }
}

#[async_trait::async_trait]
impl<R: Runner> JjApi for Jj<R> {
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
        Ok(Exec::new(BINARY)
            .maybe_timeout(self.timeout)
            .args(["--version"])
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn status(&self, dir: &Path) -> Result<String> {
        Ok(self
            .exec_in(dir, &["status"])
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>> {
        let n = format!("-n{max}");
        let out = self
            .exec_in(
                dir,
                &[
                    "log",
                    "-r",
                    revset,
                    &n,
                    "--no-graph",
                    "-T",
                    parse::CHANGE_TEMPLATE,
                ],
            )
            .checked_with(&self.runner)
            .await?;
        Ok(parse::parse_changes(&out.stdout))
    }

    async fn current_change(&self, dir: &Path) -> Result<Change> {
        let mut changes = self.log(dir, "@", 1).await?;
        changes
            .pop()
            .ok_or_else(|| vcs_process::CommandError::Parse {
                program: BINARY.to_string(),
                message: "no working-copy change found".to_string(),
            })
    }

    async fn describe(&self, dir: &Path, message: &str) -> Result<()> {
        self.exec_in(dir, &["describe", "-m", message])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn new_change(&self, dir: &Path, message: &str) -> Result<()> {
        self.exec_in(dir, &["new", "-m", message])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        let out = self
            .exec_in(dir, &["bookmark", "list"])
            .checked_with(&self.runner)
            .await?;
        Ok(parse::parse_bookmarks(&out.stdout))
    }

    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()> {
        self.exec_in(dir, &["bookmark", "set", name, "-r", revision])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn git_fetch(&self, dir: &Path) -> Result<()> {
        self.exec_in(dir, &["git", "fetch"])
            .checked_with(&self.runner)
            .await
            .map(drop)
    }

    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()> {
        let mut args = vec!["git", "push"];
        if let Some(name) = bookmark.as_deref() {
            args.push("-b");
            args.push(name);
        }
        self.exec_in(dir, &args)
            .checked_with(&self.runner)
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs_process::ScriptedRunner;

    #[test]
    fn binary_name_is_jj() {
        assert_eq!(BINARY, "jj");
    }

    // Hermetic: real log() arg-building + template parsing against canned output.
    #[tokio::test]
    async fn current_change_parses_scripted_output() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["log"], Output::ok("kztuxlro\t38e00654\tfalse\thello jj\n")),
        );
        let change = jj
            .current_change(Path::new("."))
            .await
            .expect("current_change");
        assert_eq!(change.change_id, "kztuxlro");
        assert!(!change.empty);
        assert_eq!(change.description, "hello jj");
    }

    // With a bookmark, the run must build `git push -b <name>`. Only that 4-token
    // command is scripted (no fallback), so a regression that dropped the flag
    // would match no rule and error.
    #[tokio::test]
    async fn git_push_appends_bookmark_flag() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["git", "push", "-b", "feature"], Output::ok("")),
        );
        jj.git_push(Path::new("."), Some("feature".to_string()))
            .await
            .expect("should build `git push -b feature`");
    }

    // Without a bookmark, the run is a bare `git push`.
    #[tokio::test]
    async fn git_push_without_bookmark_is_bare() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["git", "push"], Output::ok("")));
        jj.git_push(Path::new("."), None).await.expect("bare push");
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockJjApi::new();
        mock.expect_describe().returning(|_, _| Ok(()));
        assert!(mock.describe(Path::new("."), "msg").await.is_ok());
    }
}
