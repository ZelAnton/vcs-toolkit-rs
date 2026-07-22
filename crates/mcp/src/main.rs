//! The `vcs-mcp` binary: an MCP server over stdio. An agent harness launches it
//! with a `mcpServers` config entry; it speaks JSON-RPC on stdin/stdout.
//!
//! ```text
//! vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
//!         [--allow-tools <name,…>] [--timeout <seconds>]
//!         [--max-output-bytes <n>] [--log-commands]
//! ```
//!
//! Read tools are always available; `--allow-write` enables every mutating tool,
//! `--allow-tools` enables only the named ones.
//! The forge is auto-detected from the repo's `origin` remote unless `--forge`
//! overrides it. The git client is **hardened** (repo hooks and config disabled)
//! so serving a repository you didn't create can't execute its hooks, and every
//! command carries a `--timeout` so a stalled network call can't hang the server.
//! `--log-commands` wraps the git/jj/forge clients in a command-logging
//! [`ProcessRunner`](vcs_cli_support::logging::LoggingRunner) that reports every
//! spawn (program, redacted argv, working directory, exit code, duration) to
//! **stderr** — the stdout JSON-RPC transport stays a clean transport, and argv
//! values that could carry a secret are redacted.
//! Content-returning tools (`repo_show_file`, `repo_diff`, `forge_pr_diff`) are bounded by an
//! [`OutputBudget`](vcs_core::OutputBudget) so a giant blob or PR diff can't be
//! buffered whole into the server's (and then the JSON response's) memory;
//! `--max-output-bytes` raises/lowers it, `0` removes the cap.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use vcs_cli_support::logging::LoggingRunner;
use vcs_core::OutputBudget;
use vcs_core::Repo;
use vcs_core::processkit::{JobRunner, ProcessRunner};
use vcs_core::vcs_git::Git;
use vcs_core::vcs_jj::Jj;
use vcs_forge::vcs_gitea::Gitea;
use vcs_forge::vcs_github::GitHub;
use vcs_forge::vcs_gitlab::GitLab;
use vcs_forge::{Forge, ForgeKind};
use vcs_mcp::{VcsMcpServer, WriteGate};

/// The runner every git/jj/forge client is built over: a `Box<dyn ProcessRunner>`
/// so the client types are identical whether or not `--log-commands` wrapped a
/// [`LoggingRunner`] around the real [`JobRunner`] — a runtime choice, one type.
type Runner = Box<dyn ProcessRunner>;

/// The stderr tag the command log prefixes each line with.
const LOG_TAG: &str = "vcs-mcp";

/// Default per-command timeout (seconds): a generous ceiling so a stalled fetch
/// or forge call can't hang a request forever, while leaving headroom for a
/// normal network op. Override with `--timeout`; `--timeout 0` disables it.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Default content-output ceiling (bytes): large enough to hold an ordinary file
/// or PR diff, small enough that a pathological blob/diff can't buffer unbounded
/// memory into the server. Override with `--max-output-bytes`; `0` disables it
/// (the pre-T-049 behaviour). Applies to content tools (`repo_show_file`,
/// `repo_diff`, `forge_pr_diff`); exceeding it returns `OutputTooLarge` rather
/// than a silently truncated result.
const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vcs-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
vcs-mcp — a Model Context Protocol server over a git/jj repository.

USAGE:
    vcs-mcp [OPTIONS]

OPTIONS:
    --repo <path>             Repository to serve (default: current directory)
    --forge <github|gitlab|gitea>
                              Force the forge for PR/MR tools (default: detect
                              from the `origin` remote)
    --allow-write             Enable ALL mutating tools (off by default)
    --allow-tools <name,…>    Enable only the named mutating tools (comma-
                              separated; repeatable). Tool names are the method
                              names, e.g. repo_commit,forge_pr_create. Read
                              tools are always available. --allow-write wins
                              when both are given.
    --timeout <seconds>       Per-command timeout (default: 120; 0 disables) — a
                              ceiling so a stalled fetch/forge call can't hang
    --max-output-bytes <n>    Ceiling on content-tool output in bytes (default:
                              10485760 = 10 MiB; 0 disables) — repo_show_file,
                              repo_diff, and forge_pr_diff refuse with an error
                              rather than buffering an oversized blob/diff into
                              memory
    --log-commands            Log every git/jj/forge command (program, redacted
                              argv, working dir, exit code, duration) to STDERR
                              for diagnostics. stdout stays a clean JSON-RPC
                              transport; argv values that could carry a secret
                              are redacted. Off by default.
    -h, --help                Print this help

The server speaks MCP over stdio; point an agent harness at it via a
`mcpServers` config entry. The git client is hardened (repo hooks and config
disabled), so serving a repository you didn't create can't run its hooks.";

struct Args {
    repo: PathBuf,
    forge: Option<ForgeKind>,
    writes: WriteGate,
    /// Per-command deadline; `None` means no timeout (`--timeout 0`).
    timeout: Option<Duration>,
    /// Content-tool output ceiling in bytes; `None` means unlimited
    /// (`--max-output-bytes 0`).
    max_output_bytes: Option<usize>,
    /// Wrap the clients' runner in a command-logging decorator (`--log-commands`).
    log_commands: bool,
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        // --help was requested; usage already printed.
        return Ok(());
    };

    let budget = output_budget(args.max_output_bytes);
    let repo = open_repo(&args.repo, args.timeout, budget, args.log_commands)?;
    let forge = resolve_forge(&repo, args.forge, args.timeout, budget, args.log_commands).await;
    let server = VcsMcpServer::new(repo, forge, args.writes);

    // Serve MCP over stdio until the client disconnects.
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}

/// Open the repo at `dir` with a **hardened** git client — the hardened profile
/// disables repo hooks and `core.fsmonitor`, scrubs repo-redirecting `GIT_*`
/// variables, and skips system config, so serving a repository the operator
/// didn't create can't execute its hooks (or honour a `core.fsmonitor` program)
/// on a tool call. jj has no repo-local hooks, so its client needs no equivalent.
/// Both carry the per-command `timeout` and the content-output `budget`.
///
/// Delegates the whole discovery walk to `Repo::discover_with`, injecting the
/// hardened/timeout-bound client for whichever backend it detects — the facade
/// owns the `.git`/`.jj` detection and the bare-repository diagnostic, so this
/// binary no longer re-implements the walk, matches `BackendKind` by hand, or
/// carries a wildcard arm for a future backend. A bare repository now surfaces as
/// `vcs_core::Error::BareRepository`, exactly as `Repo::discover` reports it,
/// rather than the old generic "no git or jj repository found …" string.
fn open_repo(
    dir: &Path,
    timeout: Option<Duration>,
    budget: OutputBudget,
    log_commands: bool,
) -> Result<Repo<Runner>, Box<dyn std::error::Error>> {
    let repo = Repo::discover_with(
        dir,
        || hardened_git(timeout, budget, log_commands),
        || jj_client(timeout, budget, log_commands),
    )?;
    Ok(repo)
}

/// The [`ProcessRunner`] the clients drive: the real [`JobRunner`], optionally
/// wrapped in a command-logging [`LoggingRunner`] when `--log-commands` is set.
/// Boxed so both branches share one type. Each client gets its own runner
/// instance (both are cheap to construct).
fn make_runner(log_commands: bool) -> Runner {
    if log_commands {
        Box::new(LoggingRunner::new(JobRunner::new(), LOG_TAG))
    } else {
        Box::new(JobRunner::new())
    }
}

/// The content-tool [`OutputBudget`] for `max_bytes`: [`OutputBudget::unlimited`]
/// when `None` (`--max-output-bytes 0`), else a byte ceiling.
fn output_budget(max_bytes: Option<usize>) -> OutputBudget {
    match max_bytes {
        Some(b) => OutputBudget::bytes(b),
        None => OutputBudget::unlimited(),
    }
}

/// A hardened git client carrying the optional per-command `timeout` and the
/// content-output `budget`, driving the (optionally command-logging) runner.
/// `Git::with_runner(...).harden()` is `Git::hardened()` with the injected runner.
fn hardened_git(
    timeout: Option<Duration>,
    budget: OutputBudget,
    log_commands: bool,
) -> Git<Runner> {
    let git = Git::with_runner(make_runner(log_commands)).harden();
    let git = match timeout {
        Some(t) => git.default_timeout(t),
        None => git,
    };
    git.default_output_budget(budget)
}

/// A jj client carrying the optional per-command `timeout` and the content-output
/// `budget`, driving the (optionally command-logging) runner. jj has no
/// repo-local hooks, so (unlike git) it needs no hardening profile.
fn jj_client(timeout: Option<Duration>, budget: OutputBudget, log_commands: bool) -> Jj<Runner> {
    let jj = match timeout {
        Some(t) => Jj::with_runner(make_runner(log_commands)).default_timeout(t),
        None => Jj::with_runner(make_runner(log_commands)),
    };
    jj.default_output_budget(budget)
}

/// Parse argv. Returns `Ok(None)` when `--help` was printed (caller should exit
/// successfully); `Err` on an unknown flag or a bad value.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut repo = PathBuf::from(".");
    let mut forge = None;
    let mut allow_write = false;
    let mut allow_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut timeout = Some(Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    let mut max_output_bytes = Some(DEFAULT_MAX_OUTPUT_BYTES);
    let mut log_commands = false;

    let mut it = args;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--allow-write" => allow_write = true,
            "--log-commands" => log_commands = true,
            "--allow-tools" => {
                let value = it
                    .next()
                    .ok_or("--allow-tools needs a comma-separated list of tool names")?;
                let names: Vec<&str> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                if names.is_empty() {
                    return Err(format!(
                        "--allow-tools {value:?} names no tools (expected e.g. \
                         repo_commit,forge_pr_create)"
                    ));
                }
                // Validate against the canonical write-tool set so a typo is
                // rejected up front rather than silently producing an inert
                // allowlist entry (a misspelled name never matches a real tool, so
                // the intended write would stay disabled with no warning).
                if let Some(unknown) = names.iter().find(|n| !vcs_mcp::WRITE_TOOLS.contains(n)) {
                    return Err(format!(
                        "--allow-tools: unknown tool {unknown:?}; valid write tools are: {}",
                        vcs_mcp::WRITE_TOOLS.join(", ")
                    ));
                }
                // Repeatable: each occurrence extends the allowlist.
                allow_tools.extend(names.into_iter().map(String::from));
            }
            "--repo" => {
                repo = it.next().ok_or("--repo needs a path argument")?.into();
            }
            "--forge" => {
                let value = it.next().ok_or("--forge needs a value")?;
                forge = Some(parse_forge(&value)?);
            }
            "--timeout" => {
                let value = it.next().ok_or("--timeout needs a value (whole seconds)")?;
                let secs: u64 = value.parse().map_err(|_| {
                    format!("invalid --timeout {value:?} (expected a whole number of seconds)")
                })?;
                // 0 disables the deadline; any positive value sets it.
                timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            "--max-output-bytes" => {
                let value = it
                    .next()
                    .ok_or("--max-output-bytes needs a value (whole bytes)")?;
                let bytes: usize = value.parse().map_err(|_| {
                    format!(
                        "invalid --max-output-bytes {value:?} (expected a whole number of bytes)"
                    )
                })?;
                // 0 disables the ceiling; any positive value sets it.
                max_output_bytes = (bytes > 0).then_some(bytes);
            }
            other => return Err(format!("unknown argument: {other} (try --help)")),
        }
    }
    // --allow-write is the superset, so it wins over a (redundant) allowlist.
    let writes = if allow_write {
        WriteGate::All
    } else if !allow_tools.is_empty() {
        WriteGate::Set(allow_tools)
    } else {
        WriteGate::None
    };
    Ok(Some(Args {
        repo,
        forge,
        writes,
        timeout,
        max_output_bytes,
        log_commands,
    }))
}

fn parse_forge(value: &str) -> Result<ForgeKind, String> {
    match value {
        "github" => Ok(ForgeKind::GitHub),
        "gitlab" => Ok(ForgeKind::GitLab),
        "gitea" => Ok(ForgeKind::Gitea),
        other => Err(format!(
            "unknown forge {other:?} (expected github, gitlab, or gitea)"
        )),
    }
}

/// Pick the forge: the explicit `--forge`, else the `origin` remote's host, else
/// none (forge tools then report "no forge configured"). The forge CLI clients
/// carry the same per-command `timeout` and content-output `budget` as the repo
/// client, so `forge_pr_diff` is bounded the same way `repo_show_file` is.
async fn resolve_forge(
    repo: &Repo<Runner>,
    forced: Option<ForgeKind>,
    timeout: Option<Duration>,
    budget: OutputBudget,
    log_commands: bool,
) -> Option<Forge<Runner>> {
    let cwd = repo.root().to_path_buf();
    let kind = match forced {
        Some(k) => Some(k),
        None => detect_forge_kind(repo).await,
    };
    // Each forge CLI client exposes the same `with_runner`/`default_timeout`/
    // `default_output_budget` builders, but they are distinct types with no
    // shared trait — so apply them inline per arm.
    kind.and_then(|k| match k {
        ForgeKind::GitHub => {
            let c = GitHub::with_runner(make_runner(log_commands));
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            let c = c.default_output_budget(budget);
            Some(Forge::from_github(&cwd, c))
        }
        ForgeKind::GitLab => {
            let c = GitLab::with_runner(make_runner(log_commands));
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            let c = c.default_output_budget(budget);
            Some(Forge::from_gitlab(&cwd, c))
        }
        ForgeKind::Gitea => {
            let c = Gitea::with_runner(make_runner(log_commands));
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            let c = c.default_output_budget(budget);
            Some(Forge::from_gitea(&cwd, c))
        }
        // `ForgeKind` is `#[non_exhaustive]`; a future kind has no constructor here.
        _ => None,
    })
}

/// Best-effort: read the `origin` remote URL through the backend-agnostic repo
/// facade and classify its host. This works for both colocated and non-colocated
/// jj repositories. `None` when there is no `origin`, the remote query fails, or
/// the host is unrecognised.
async fn detect_forge_kind<R: vcs_core::processkit::ProcessRunner>(
    repo: &Repo<R>,
) -> Option<ForgeKind> {
    let origin = repo
        .remotes()
        .await
        .ok()?
        .into_iter()
        .find(|remote| remote.name == "origin")?;
    ForgeKind::from_remote_url(&origin.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply};
    use vcs_core::vcs_jj::Jj;

    /// Run `parse_args` over a borrowed slice of `&str` args, as if they were argv.
    fn parse(args: &[&str]) -> Result<Option<Args>, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    /// The error message from a parse expected to fail (`Args` has no `Debug`, so
    /// we can't lean on `unwrap_err`).
    fn parse_err(args: &[&str]) -> String {
        match parse(args) {
            Err(e) => e,
            Ok(_) => panic!("expected parse error for {args:?}"),
        }
    }

    #[test]
    fn defaults_with_no_args() {
        let args = parse(&[]).unwrap().expect("no --help, so Some(Args)");
        assert_eq!(args.repo, PathBuf::from("."));
        assert_eq!(args.forge, None);
        assert_eq!(args.writes, WriteGate::None);
        assert_eq!(
            args.timeout,
            Some(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        );
        assert_eq!(args.max_output_bytes, Some(DEFAULT_MAX_OUTPUT_BYTES));
        assert!(!args.log_commands, "command logging is off by default");
    }

    #[test]
    fn log_commands_flag_enables_it() {
        let args = parse(&["--log-commands"]).unwrap().unwrap();
        assert!(args.log_commands);
        // Absent by default (guards against a flipped default).
        assert!(!parse(&[]).unwrap().unwrap().log_commands);
    }

    // --allow-tools builds a Set gate; the list splits on commas, trims, and is
    // repeatable (occurrences accumulate). An effectively-empty list errors.
    #[test]
    fn allow_tools_builds_a_set_gate() {
        let args = parse(&["--allow-tools", "repo_commit, forge_pr_create"])
            .unwrap()
            .unwrap();
        let WriteGate::Set(tools) = &args.writes else {
            panic!("expected Set gate, got {:?}", args.writes);
        };
        assert!(tools.contains("repo_commit"));
        assert!(tools.contains("forge_pr_create"));
        assert_eq!(tools.len(), 2);

        let args = parse(&["--allow-tools", "repo_push", "--allow-tools", "repo_fetch"])
            .unwrap()
            .unwrap();
        let WriteGate::Set(tools) = &args.writes else {
            panic!("expected Set gate");
        };
        assert_eq!(tools.len(), 2);

        assert!(parse(&["--allow-tools"]).is_err());
        let err = parse_err(&["--allow-tools", " , "]);
        assert!(err.contains("names no tools"), "got: {err}");
    }

    // A misspelled tool name is rejected up front (it would otherwise be a silently
    // inert allowlist entry — never matching a real tool, so the write stays off).
    #[test]
    fn allow_tools_rejects_unknown_tool_name() {
        let err = parse_err(&["--allow-tools", "repo_comit"]); // typo
        assert!(err.contains("unknown tool"), "got: {err}");
        assert!(err.contains("repo_comit"), "names the offender: {err}");
        // A read-tool name is also not a valid *write* allowlist entry.
        let err = parse_err(&["--allow-tools", "repo_commit,repo_status"]);
        assert!(err.contains("repo_status"), "got: {err}");
    }

    // --allow-write is the superset and wins over a redundant allowlist.
    #[test]
    fn allow_write_wins_over_allow_tools() {
        let args = parse(&["--allow-tools", "repo_commit", "--allow-write"])
            .unwrap()
            .unwrap();
        assert_eq!(args.writes, WriteGate::All);
    }

    #[test]
    fn help_short_circuits() {
        assert!(parse(&["--help"]).unwrap().is_none());
        assert!(parse(&["-h"]).unwrap().is_none());
    }

    #[test]
    fn unknown_flag_errors() {
        let err = parse_err(&["--bogus"]);
        assert!(err.contains("unknown argument"), "got: {err}");
    }

    #[test]
    fn missing_values_error() {
        assert!(parse(&["--repo"]).is_err());
        assert!(parse(&["--forge"]).is_err());
        assert!(parse(&["--timeout"]).is_err());
        assert!(parse(&["--max-output-bytes"]).is_err());
    }

    #[test]
    fn timeout_zero_disables() {
        let args = parse(&["--timeout", "0"]).unwrap().unwrap();
        assert_eq!(args.timeout, None);
    }

    #[test]
    fn timeout_positive_sets_duration() {
        let args = parse(&["--timeout", "45"]).unwrap().unwrap();
        assert_eq!(args.timeout, Some(Duration::from_secs(45)));
    }

    #[test]
    fn timeout_junk_errors() {
        let err = parse_err(&["--timeout", "junk"]);
        assert!(err.contains("invalid --timeout"), "got: {err}");
        // A negative value isn't a valid `u64` either.
        assert!(parse(&["--timeout", "-5"]).is_err());
    }

    #[test]
    fn max_output_bytes_zero_disables() {
        let args = parse(&["--max-output-bytes", "0"]).unwrap().unwrap();
        assert_eq!(args.max_output_bytes, None);
    }

    #[test]
    fn max_output_bytes_positive_sets_ceiling() {
        let args = parse(&["--max-output-bytes", "4096"]).unwrap().unwrap();
        assert_eq!(args.max_output_bytes, Some(4096));
    }

    #[test]
    fn max_output_bytes_junk_errors() {
        let err = parse_err(&["--max-output-bytes", "junk"]);
        assert!(err.contains("invalid --max-output-bytes"), "got: {err}");
        // A negative value isn't a valid `usize` either.
        assert!(parse(&["--max-output-bytes", "-5"]).is_err());
    }

    #[test]
    fn forge_parsing() {
        assert_eq!(
            parse(&["--forge", "github"]).unwrap().unwrap().forge,
            Some(ForgeKind::GitHub)
        );
        assert_eq!(
            parse(&["--forge", "gitlab"]).unwrap().unwrap().forge,
            Some(ForgeKind::GitLab)
        );
        assert_eq!(
            parse(&["--forge", "gitea"]).unwrap().unwrap().forge,
            Some(ForgeKind::Gitea)
        );
        let err = parse_err(&["--forge", "bitbucket"]);
        assert!(err.contains("unknown forge"), "got: {err}");
    }

    #[test]
    fn combined_flags() {
        let args = parse(&[
            "--repo",
            "X",
            "--forge",
            "gitea",
            "--allow-write",
            "--timeout",
            "7",
            "--max-output-bytes",
            "8192",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(args.repo, PathBuf::from("X"));
        assert_eq!(args.forge, Some(ForgeKind::Gitea));
        assert_eq!(args.writes, WriteGate::All);
        assert_eq!(args.timeout, Some(Duration::from_secs(7)));
        assert_eq!(args.max_output_bytes, Some(8192));
    }

    #[test]
    fn output_budget_conversion() {
        assert_eq!(output_budget(None), OutputBudget::unlimited());
        assert_eq!(output_budget(Some(4096)), OutputBudget::bytes(4096));
    }

    // A `Repo` backed by jj has no need for a colocated `.git`: its remote list
    // goes through `jj git remote list`. The same facade is selected for a
    // colocated jj checkout, so this hermetic test pins both code paths without
    // requiring a real jj binary.
    #[tokio::test]
    async fn detect_forge_kind_uses_jj_remotes_without_a_colocated_git_dir() {
        let rec = RecordingRunner::replying(Reply::ok(
            "upstream https://gitlab.com/example/ignored.git\norigin git@github.com:example/repo.git\n",
        ));
        let repo = Repo::from_jj(
            "/non-colocated-jj",
            "/non-colocated-jj",
            Jj::with_runner(&rec),
        );

        assert_eq!(detect_forge_kind(&repo).await, Some(ForgeKind::GitHub));
        assert_eq!(rec.calls().len(), 1);
        assert_eq!(
            rec.calls()[0].args_str(),
            [
                "git",
                "remote",
                "list",
                "--color",
                "never",
                "--ignore-working-copy"
            ],
            "jj remote discovery must override ui.color=always and avoid a working-copy snapshot"
        );
    }

    // Exercise both jj layouts against the real CLI. The non-colocated case also
    // sets the user configuration that used to inject ANSI escapes into the
    // parsed remote-list output; `Jj::cmd_in_wc` must override it with
    // `--color never`.
    #[tokio::test]
    #[ignore = "requires the jj binary"]
    async fn detect_forge_kind_handles_colocated_and_non_colocated_jj() {
        let colocated = vcs_testkit::JjSandbox::colocated("mcp-forge-colocated");
        colocated.jj(&[
            "git",
            "remote",
            "add",
            "origin",
            "https://github.com/example/colocated.git",
        ]);
        assert!(colocated.path().join(".git").is_dir());
        let colocated_repo = Repo::discover(colocated.path()).expect("discover colocated jj");
        assert_eq!(
            detect_forge_kind(&colocated_repo).await,
            Some(ForgeKind::GitHub)
        );

        let non_colocated = vcs_testkit::JjSandbox::init_non_colocated("mcp-forge-non-colocated");
        non_colocated.jj(&["config", "set", "--repo", "ui.color", "always"]);
        non_colocated.jj(&[
            "git",
            "remote",
            "add",
            "origin",
            "https://gitlab.com/example/non-colocated.git",
        ]);
        assert!(!non_colocated.path().join(".git").exists());
        let non_colocated_repo =
            Repo::discover(non_colocated.path()).expect("discover non-colocated jj");
        assert_eq!(
            detect_forge_kind(&non_colocated_repo).await,
            Some(ForgeKind::GitLab)
        );
    }
}
