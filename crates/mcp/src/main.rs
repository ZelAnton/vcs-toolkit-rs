//! The `vcs-mcp` binary: an MCP server over stdio. An agent harness launches it
//! with a `mcpServers` config entry; it speaks JSON-RPC on stdin/stdout.
//!
//! ```text
//! vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
//!         [--allow-tools <name,…>] [--timeout <seconds>]
//! ```
//!
//! Read tools are always available; `--allow-write` enables every mutating tool,
//! `--allow-tools` enables only the named ones.
//! The forge is auto-detected from the repo's `origin` remote unless `--forge`
//! overrides it. The git client is **hardened** (repo hooks and config disabled)
//! so serving a repository you didn't create can't execute its hooks, and every
//! command carries a `--timeout` so a stalled network call can't hang the server.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use vcs_core::vcs_git::{Git, GitApi};
use vcs_core::vcs_jj::Jj;
use vcs_core::{BackendKind, Repo, detect};
use vcs_forge::vcs_gitea::Gitea;
use vcs_forge::vcs_github::GitHub;
use vcs_forge::vcs_gitlab::GitLab;
use vcs_forge::{Forge, ForgeKind};
use vcs_mcp::{VcsMcpServer, WriteGate};

/// Default per-command timeout (seconds): a generous ceiling so a stalled fetch
/// or forge call can't hang a request forever, while leaving headroom for a
/// normal network op. Override with `--timeout`; `--timeout 0` disables it.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

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
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        // --help was requested; usage already printed.
        return Ok(());
    };

    let repo = open_repo(&args.repo, args.timeout)?;
    let forge = resolve_forge(&repo, args.forge, args.timeout).await;
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
/// Both carry the per-command `timeout`. This mirrors [`Repo::open`]'s detection
/// but injects the hardened/timeout-bound client instead of the plain default.
fn open_repo(dir: &Path, timeout: Option<Duration>) -> Result<Repo, Box<dyn std::error::Error>> {
    let dir = std::path::absolute(dir)?;
    let located = detect(&dir).ok_or_else(|| {
        format!(
            "no git or jj repository found at or above {}",
            dir.display()
        )
    })?;
    let repo = match located.kind {
        BackendKind::Git => Repo::from_git(located.root, dir, hardened_git(timeout)),
        BackendKind::Jj => {
            let jj = match timeout {
                Some(t) => Jj::new().default_timeout(t),
                None => Jj::new(),
            };
            Repo::from_jj(located.root, dir, jj)
        }
        // `BackendKind` is `#[non_exhaustive]`; a future backend has no client here.
        _ => return Err("unsupported repository backend".into()),
    };
    Ok(repo)
}

/// A hardened git client carrying the optional per-command `timeout`.
fn hardened_git(timeout: Option<Duration>) -> Git {
    match timeout {
        Some(t) => Git::hardened().default_timeout(t),
        None => Git::hardened(),
    }
}

/// Parse argv. Returns `Ok(None)` when `--help` was printed (caller should exit
/// successfully); `Err` on an unknown flag or a bad value.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut repo = PathBuf::from(".");
    let mut forge = None;
    let mut allow_write = false;
    let mut allow_tools: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut timeout = Some(Duration::from_secs(DEFAULT_TIMEOUT_SECS));

    let mut it = args;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--allow-write" => allow_write = true,
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
/// carry the same per-command `timeout` as the repo client.
async fn resolve_forge(
    repo: &Repo,
    forced: Option<ForgeKind>,
    timeout: Option<Duration>,
) -> Option<Forge> {
    let cwd = repo.root().to_path_buf();
    let kind = match forced {
        Some(k) => Some(k),
        None => detect_forge_kind(repo.root(), timeout).await,
    };
    // Each forge CLI client exposes the same `default_timeout` builder, but they
    // are distinct types with no shared trait — so apply it inline per arm.
    kind.and_then(|k| match k {
        ForgeKind::GitHub => {
            let c = GitHub::new();
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            Some(Forge::for_github(&cwd, c))
        }
        ForgeKind::GitLab => {
            let c = GitLab::new();
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            Some(Forge::for_gitlab(&cwd, c))
        }
        ForgeKind::Gitea => {
            let c = Gitea::new();
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            Some(Forge::for_gitea(&cwd, c))
        }
        // `ForgeKind` is `#[non_exhaustive]`; a future kind has no constructor here.
        _ => None,
    })
}

/// Best-effort: read the `origin` remote URL (works on a colocated jj repo too)
/// and classify its host. `None` when there's no git remote or the host is
/// unrecognised. Uses the hardened, timeout-bound client.
async fn detect_forge_kind(root: &Path, timeout: Option<Duration>) -> Option<ForgeKind> {
    let url = hardened_git(timeout)
        .remote_url(root, "origin")
        .await
        .ok()?;
    ForgeKind::from_remote_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ])
        .unwrap()
        .unwrap();
        assert_eq!(args.repo, PathBuf::from("X"));
        assert_eq!(args.forge, Some(ForgeKind::Gitea));
        assert_eq!(args.writes, WriteGate::All);
        assert_eq!(args.timeout, Some(Duration::from_secs(7)));
    }
}
