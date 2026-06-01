//! `vcs-jj` — automate Jujutsu (`jj`) from Rust through CLI process execution.
//!
//! Async, mockable, and structured-error: consumers depend on the [`JjApi`]
//! trait and substitute a mock for the real [`Jj`] client in tests. Commands run
//! inside an OS job (via [`processkit`]) so a `jj` subprocess is never orphaned,
//! and honour an optional [timeout](Jj::default_timeout).
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockJjApi`, or inject a fake runner with
//! `Jj::with_runner(`[`ScriptedRunner`](processkit::ScriptedRunner)`)`.

use std::path::Path;

use processkit::ProcessRunner;
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};

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
    /// Like [`JjApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
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

processkit::cli_client!(
    /// The real jj client. Generic over the [`ProcessRunner`] so tests can inject
    /// a fake process executor; `Jj::new()` uses the real job-backed runner.
    pub struct Jj => BINARY
);

#[async_trait::async_trait]
impl<R: ProcessRunner> JjApi for Jj<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.text(self.core.command(["--version"])).await
    }

    async fn status(&self, dir: &Path) -> Result<String> {
        self.core.text(self.core.command_in(dir, ["status"])).await
    }

    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>> {
        let n = format!("-n{max}");
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    [
                        "log",
                        "-r",
                        revset,
                        n.as_str(),
                        "--no-graph",
                        "-T",
                        parse::CHANGE_TEMPLATE,
                    ],
                ),
                parse::parse_changes,
            )
            .await
    }

    async fn current_change(&self, dir: &Path) -> Result<Change> {
        let mut changes = self.log(dir, "@", 1).await?;
        changes.pop().ok_or_else(|| Error::Parse {
            program: BINARY.to_string(),
            message: "no working-copy change found".to_string(),
        })
    }

    async fn describe(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["describe", "-m", message]))
            .await
    }

    async fn new_change(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["new", "-m", message]))
            .await
    }

    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.core.command_in(dir, ["bookmark", "list"]),
                parse::parse_bookmarks,
            )
            .await
    }

    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()> {
        self.core
            .unit(
                self.core
                    .command_in(dir, ["bookmark", "set", name, "-r", revision]),
            )
            .await
    }

    async fn git_fetch(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["git", "fetch"]))
            .await
    }

    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()> {
        let mut args = vec!["git", "push"];
        if let Some(name) = bookmark.as_deref() {
            args.push("-b");
            args.push(name);
        }
        self.core.unit(self.core.command_in(dir, args)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_jj() {
        assert_eq!(BINARY, "jj");
    }

    // Hermetic: real log() arg-building + template parsing against canned output.
    #[tokio::test]
    async fn current_change_parses_scripted_output() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["log"], Reply::ok("kztuxlro\t38e00654\tfalse\thello jj\n")),
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
            ScriptedRunner::new().on(["git", "push", "-b", "feature"], Reply::ok("")),
        );
        jj.git_push(Path::new("."), Some("feature".to_string()))
            .await
            .expect("should build `git push -b feature`");
    }

    // Without a bookmark, the run is a bare `git push`.
    #[tokio::test]
    async fn git_push_without_bookmark_is_bare() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["git", "push"], Reply::ok("")));
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
