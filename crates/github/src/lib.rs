//! `vcs-github` — automate GitHub from Rust through the `gh` CLI.
//!
//! Async, mockable, and structured-error: consumers depend on the [`GitHubApi`]
//! trait and substitute a mock for the real [`GitHub`] client in tests. Commands
//! run inside an OS job (via [`processkit`]) so a `gh` subprocess is never
//! orphaned, and honour an optional [timeout](GitHub::default_timeout).
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockGitHubApi`, or inject a fake runner with
//! `GitHub::with_runner(`[`ScriptedRunner`](processkit::ScriptedRunner)`)`.

use std::path::Path;

use processkit::ProcessRunner;
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};

mod parse;
pub use parse::{Issue, PullRequest, Repo};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "gh";

const PR_FIELDS: &str = "number,title,state,headRefName,baseRefName,url";
const REPO_FIELDS: &str = "name,owner,description,url,isPrivate,defaultBranchRef";

/// The GitHub operations this crate exposes — the interface consumers code
/// against and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitHubApi: Send + Sync {
    /// Run `gh <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitHubApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed GitHub CLI version (`gh --version`).
    async fn version(&self) -> Result<String>;
    /// Whether the user is authenticated (`gh auth status` exits zero).
    async fn auth_status(&self) -> Result<bool>;
    /// The repository for `dir` (`gh repo view --json …`).
    async fn repo_view(&self, dir: &Path) -> Result<Repo>;
    /// Pull requests for `dir` (`gh pr list --json …`).
    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>>;
    /// A single pull request by number (`gh pr view <n> --json …`).
    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest>;
    /// Issues for `dir` (`gh issue list --json …`).
    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>>;
    /// Open a pull request, returning its URL (`gh pr create`). `base` is owned
    /// (`Option<String>`) to keep the trait `mockall`-friendly.
    async fn pr_create(
        &self,
        dir: &Path,
        title: &str,
        body: &str,
        base: Option<String>,
    ) -> Result<String>;
    /// Raw GitHub REST/GraphQL response body (`gh api <endpoint>`).
    async fn api(&self, endpoint: &str) -> Result<String>;
}

processkit::cli_client!(
    /// The real GitHub client. Generic over the [`ProcessRunner`] so tests can
    /// inject a fake process executor; `GitHub::new()` uses the real job-backed
    /// runner.
    pub struct GitHub => BINARY
);

#[async_trait::async_trait]
impl<R: ProcessRunner> GitHubApi for GitHub<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.text(self.core.command(["--version"])).await
    }

    async fn auth_status(&self) -> Result<bool> {
        // `gh auth status` exits 0 when authenticated, non-zero when not — an
        // exit-code answer. `code` reports the bool but still errors on a spawn
        // failure or timeout (processkit surfaces a timeout as `Error::Timeout`),
        // rather than silently reporting "not authenticated".
        Ok(self
            .core
            .code(self.core.command(["auth", "status"]))
            .await?
            == 0)
    }

    async fn repo_view(&self, dir: &Path) -> Result<Repo> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["repo", "view", "--json", REPO_FIELDS]),
                parse::parse_repo,
            )
            .await
    }

    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["pr", "list", "--json", PR_FIELDS]),
                parse::from_json,
            )
            .await
    }

    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
        let n = number.to_string();
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["pr", "view", n.as_str(), "--json", PR_FIELDS]),
                parse::from_json,
            )
            .await
    }

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        self.core
            .try_parse(
                self.core
                    .command_in(dir, ["issue", "list", "--json", "number,title,state"]),
                parse::from_json,
            )
            .await
    }

    async fn pr_create(
        &self,
        dir: &Path,
        title: &str,
        body: &str,
        base: Option<String>,
    ) -> Result<String> {
        let mut args = vec!["pr", "create", "--title", title, "--body", body];
        if let Some(base) = base.as_deref() {
            args.push("--base");
            args.push(base);
        }
        self.core.text(self.core.command_in(dir, args)).await
    }

    async fn api(&self, endpoint: &str) -> Result<String> {
        self.core.text(self.core.command(["api", endpoint])).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_gh() {
        assert_eq!(BINARY, "gh");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `gh` binary or network needed, so this runs on CI.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"number":7,"title":"Add X","state":"OPEN","headRefName":"feat/x","baseRefName":"main","url":"u"}]"#;
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
        let prs = gh.pr_list(Path::new(".")).await.expect("pr_list");
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].base_ref_name, "main");
    }

    // Hermetic: auth_status reflects the exit code without erroring.
    #[tokio::test]
    async fn auth_status_reads_exit_code() {
        let yes = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Reply::ok("")));
        assert!(yes.auth_status().await.unwrap());
        let no = GitHub::with_runner(
            ScriptedRunner::new().on(["auth"], Reply::fail(1, "not logged in")),
        );
        assert!(!no.auth_status().await.unwrap());
    }

    // Regression guard for the timeout fix: a timed-out auth check must error,
    // not silently report "not authenticated" (the old hand-rolled mapping bug).
    // Relies on processkit surfacing a timed-out run as `Error::Timeout`.
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Reply::timeout()));
        assert!(matches!(
            gh.auth_status().await.unwrap_err(),
            Error::Timeout { .. }
        ));
    }

    // pr_create appends `--base <branch>` when given one, and returns the trimmed
    // PR URL. The exact command (incl. --base) is the only scripted rule.
    #[tokio::test]
    async fn pr_create_appends_base_and_returns_url() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(
            [
                "pr", "create", "--title", "T", "--body", "B", "--base", "main",
            ],
            Reply::ok("https://gh/pr/1\n"),
        ));
        let url = gh
            .pr_create(Path::new("."), "T", "B", Some("main".to_string()))
            .await
            .expect("should build `pr create … --base main`");
        assert_eq!(url, "https://gh/pr/1");
    }

    // Without a base, `pr_create` must omit `--base` entirely. RecordingRunner
    // captures the exact invocation (and `&rec` plumbs through CliClient), so we
    // can assert flag *absence* and the cwd — which prefix matching can't.
    #[tokio::test]
    async fn pr_create_omits_base_when_none() {
        use processkit::RecordingRunner;
        use std::ffi::OsStr;
        let rec = RecordingRunner::replying(Reply::ok("https://gh/pr/2\n"));
        let gh = GitHub::with_runner(&rec);
        let url = gh
            .pr_create(Path::new("/repo"), "T", "B", None)
            .await
            .expect("pr_create");
        assert_eq!(url, "https://gh/pr/2");

        let call = rec.only_call();
        assert_eq!(call.cwd.as_deref(), Some(OsStr::new("/repo")));
        assert_eq!(
            call.args_str(),
            ["pr", "create", "--title", "T", "--body", "B"]
        );
        assert!(!call.has_flag("--base"), "no base was given");
    }

    // repo_view builds the --json request and flattens gh's nested owner/branch
    // objects into the public Repo.
    #[tokio::test]
    async fn repo_view_parses_scripted_json() {
        let json = r#"{"name":"r","owner":{"login":"o"},"description":"d","url":"u","isPrivate":false,"defaultBranchRef":{"name":"main"}}"#;
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["repo", "view"], Reply::ok(json)));
        let repo = gh.repo_view(Path::new(".")).await.expect("repo_view");
        assert_eq!(repo.owner, "o");
        assert_eq!(repo.default_branch, "main");
        assert!(!repo.is_private);
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockGitHubApi::new();
        mock.expect_auth_status().returning(|| Ok(true));
        assert!(mock.auth_status().await.unwrap());
    }
}
