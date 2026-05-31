//! `vcs-github` — automate GitHub from Rust through the `gh` CLI.
//!
//! Async, mockable, and structured-error: consumers depend on the [`GitHubApi`]
//! trait and substitute a mock for the real [`GitHub`] client in tests. Commands
//! run inside an OS job (via [`vcs_process`]) so a `gh` subprocess is never
//! orphaned, and honour an optional [timeout](GitHub::default_timeout).
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockGitHubApi`, or inject a fake runner with
//! `GitHub::with_runner(`[`ScriptedRunner`](vcs_process::ScriptedRunner)`)`.

use std::io;
use std::path::Path;
use std::time::Duration;

use vcs_process::{CliClient, JobRunner, Output, Result, Runner};

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
    /// Like [`GitHubApi::run`] but never errors on exit code — returns [`Output`].
    async fn run_raw(&self, args: &[String]) -> io::Result<Output>;
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

/// The real GitHub client. Generic over the [`Runner`] so tests can inject a
/// fake process executor; `GitHub::new()` uses the real job-backed runner.
pub struct GitHub<R: Runner = JobRunner> {
    core: CliClient<R>,
}

impl GitHub<JobRunner> {
    /// A client backed by the real `gh` binary.
    pub fn new() -> Self {
        GitHub {
            core: CliClient::new(BINARY),
        }
    }
}

impl Default for GitHub<JobRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Runner> GitHub<R> {
    /// A client that runs commands through `runner` — pass a fake in tests.
    pub fn with_runner(runner: R) -> Self {
        GitHub {
            core: CliClient::with_runner(BINARY, runner),
        }
    }

    /// Kill any command that runs longer than `timeout`.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.core = self.core.default_timeout(timeout);
        self
    }
}

#[async_trait::async_trait]
impl<R: Runner> GitHubApi for GitHub<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run_text(self.core.exec(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> io::Result<Output> {
        self.core.run_raw(self.core.exec(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run_text(self.core.exec(["--version"])).await
    }

    async fn auth_status(&self) -> Result<bool> {
        // `gh auth status` exits 0 when authenticated, non-zero when not — an
        // exit-code answer. `code_with` reports the bool but, unlike the old
        // hand-rolled mapping, still errors on a spawn failure or timeout rather
        // than silently reporting "not authenticated".
        Ok(self
            .core
            .exec(["auth", "status"])
            .code_with(self.core.runner())
            .await?
            == 0)
    }

    async fn repo_view(&self, dir: &Path) -> Result<Repo> {
        self.core
            .parsed_try(
                self.core
                    .exec_in(dir, ["repo", "view", "--json", REPO_FIELDS]),
                parse::parse_repo,
            )
            .await
    }

    async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>> {
        self.core
            .parsed_try(
                self.core.exec_in(dir, ["pr", "list", "--json", PR_FIELDS]),
                parse::from_json,
            )
            .await
    }

    async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest> {
        let n = number.to_string();
        self.core
            .parsed_try(
                self.core
                    .exec_in(dir, ["pr", "view", n.as_str(), "--json", PR_FIELDS]),
                parse::from_json,
            )
            .await
    }

    async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>> {
        self.core
            .parsed_try(
                self.core
                    .exec_in(dir, ["issue", "list", "--json", "number,title,state"]),
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
        self.core.run_text(self.core.exec_in(dir, args)).await
    }

    async fn api(&self, endpoint: &str) -> Result<String> {
        self.core.run_text(self.core.exec(["api", endpoint])).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs_process::ScriptedRunner;

    #[test]
    fn binary_name_is_gh() {
        assert_eq!(BINARY, "gh");
    }

    // Hermetic: real pr_list() arg-building + JSON deserialization against canned
    // output — no `gh` binary or network needed, so this runs on CI.
    #[tokio::test]
    async fn pr_list_parses_scripted_json() {
        let json = r#"[{"number":7,"title":"Add X","state":"OPEN","headRefName":"feat/x","baseRefName":"main","url":"u"}]"#;
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "list"], Output::ok(json)));
        let prs = gh.pr_list(Path::new(".")).await.expect("pr_list");
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].base_ref_name, "main");
    }

    // Hermetic: auth_status reflects the exit code without erroring.
    #[tokio::test]
    async fn auth_status_reads_exit_code() {
        let yes = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Output::ok("")));
        assert!(yes.auth_status().await.unwrap());
        let no = GitHub::with_runner(
            ScriptedRunner::new().on(["auth"], Output::fail(1, "not logged in")),
        );
        assert!(!no.auth_status().await.unwrap());
    }

    // Regression guard for the timeout fix: a timed-out auth check must error,
    // not silently report "not authenticated" (the old hand-rolled mapping bug).
    #[tokio::test]
    async fn auth_status_errors_on_timeout() {
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["auth"], Output::timeout()));
        assert!(matches!(
            gh.auth_status().await.unwrap_err(),
            vcs_process::CommandError::Timeout { .. }
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
            Output::ok("https://gh/pr/1\n"),
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
        use vcs_process::RecordingRunner;
        let rec = RecordingRunner::replying(Output::ok("https://gh/pr/2\n"));
        let gh = GitHub::with_runner(&rec);
        let url = gh
            .pr_create(Path::new("/repo"), "T", "B", None)
            .await
            .expect("pr_create");
        assert_eq!(url, "https://gh/pr/2");

        let call = rec.only_call();
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/repo")));
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
        let gh = GitHub::with_runner(ScriptedRunner::new().on(["repo", "view"], Output::ok(json)));
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
