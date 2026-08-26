//! The `vcs-mcp` binary: an MCP server over stdio. An agent harness launches it
//! with a `mcpServers` config entry; it speaks JSON-RPC on stdin/stdout.
//!
//! ```text
//! vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
//!         [--allow-tools <name,…>] [--timeout <seconds>]
//!         [--max-output-bytes <n>] [--log-commands]
//!         [--ssh-command <command>] [--trust-repo-ssh-command]
//!         [--gh-account <login> | --gh-token-env <VAR>]
//! ```
//!
//! Read tools are always available; `--allow-write` enables every mutating tool,
//! `--allow-tools` enables only the named ones.
//! The forge is auto-detected from the repo's `origin` remote unless `--forge`
//! overrides it. The git client is **hardened** (repo hooks and `core.fsmonitor`
//! disabled, with the code-execution `GIT_*` variables scrubbed) so serving a
//! repository you didn't create can't execute its hooks, and every command
//! carries a `--timeout` so a stalled network call can't hang the server. The
//! hardened client also **refuses** a network operation when the repository
//! overrides `core.sshCommand` (git would run that value through a shell);
//! `--ssh-command` / `--trust-repo-ssh-command` are the two explicit ways to
//! continue. That refusal is the **git backend's** alone: a valid `.jj` wins
//! backend detection, so on a jj or colocated repo `repo_fetch`/`repo_push` run
//! `jj git fetch`/`jj git push`, which still honour the repository's
//! `core.sshCommand`, and both flags are no-ops there.
//! That profile does *not* disable repo-local config wholesale — see
//! "A hardened git client" in `crates/mcp/docs/mcp.md` for the residual vectors it
//! leaves (`filter.*`, `diff.*.textconv`).
//! `--log-commands` wraps the git/jj/forge clients in a command-logging
//! [`ProcessRunner`](vcs_cli_support::logging::LoggingRunner) that reports every
//! spawn (program, redacted argv, working directory, exit code, duration) to
//! **stderr** — the stdout JSON-RPC transport stays a clean transport, and argv
//! values that could carry a secret are redacted.
//! The forge tools authenticate through the forge CLI's own ambient login unless
//! one of the two **GitHub** identity flags picks another: `--gh-account <login>`
//! runs them as that `gh` account (its token is resolved **once** — lazily, on
//! the first forge call that needs it — with `gh auth token --user`, then cached
//! for the life of the server and injected into each command's environment,
//! leaving the machine's active account untouched; a token rotated or revoked in
//! `gh` afterwards is picked up only when the server restarts) and
//! `--gh-token-env <VAR>` takes the token from that environment variable, which
//! is re-read on every call. They are mutually exclusive, and either one is an
//! error when the forge in play is not GitHub. Neither puts a token in argv:
//! only the login, or the variable's *name*, is ever a command argument.
//! Content-returning tools (`repo_show_file`, `repo_diff`, `forge_pr_diff`, and the
//! two conflict tools' working-copy read) are bounded by an
//! [`OutputBudget`](vcs_core::OutputBudget) so a giant blob or PR diff can't be
//! buffered whole into the server's (and then the JSON response's) memory;
//! `--max-output-bytes` raises/lowers it, `0` removes the cap. The same budget
//! goes to the git/jj/forge clients (which enforce it on their subprocess output)
//! and to the server itself, whose conflict tools read the working copy directly
//! and so have no subprocess to inherit it from.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
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
use vcs_forge::vcs_github::{GhAccountToken, GitHub, GitHubHost};
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
/// `repo_diff`, `forge_pr_diff`, and `repo_conflict_regions` /
/// `repo_resolve_conflict`'s working-copy read); exceeding it returns
/// `OutputTooLarge` — or, for the direct filesystem read, the same refusal naming
/// this ceiling — rather than a silently truncated result.
///
/// The unit is the bytes the wrapped CLI writes to its output pipe, verbatim —
/// see [`OutputBudget::bytes`](vcs_core::OutputBudget::bytes), which is where the
/// per-stream accounting is documented. This ceiling is deliberately left at its
/// pre-processkit-3.0 value: 3.0's raw-pipe-byte switch did not move the
/// raw-stdout ceiling these tools read through (T-130), so re-tuning it would
/// change the server's behaviour for no reason.
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
                              repo_diff, forge_pr_diff, and the conflict tools'
                              working-copy read refuse with an error rather than
                              buffering an oversized blob/diff/file into memory
    --log-commands            Log every git/jj/forge command (program, redacted
                              argv, working dir, exit code, duration) to STDERR
                              for diagnostics. stdout stays a clean JSON-RPC
                              transport; argv values that could carry a secret
                              are redacted. Off by default.
    --ssh-command <command>   Run SSH network operations with this command
                              (delivered as GIT_SSH_COMMAND). Also lifts the
                              refusal below, and outranks whatever the
                              repository set, so its value never runs. Applies to
                              the git backend only (see below).
    --trust-repo-ssh-command  Accept a `core.sshCommand` the REPOSITORY sets, and
                              lift the refusal below. Use it for a repository you
                              own that carries its own ssh identity — it accepts
                              whatever that repository says. --ssh-command wins
                              when both are given (whatever the order). Applies to
                              the git backend only (see below).
    --gh-account <login>      Run the forge tools as this `gh` account instead of
                              the machine's active one. Its token is resolved
                              ONCE — lazily, on the first forge call that needs
                              it — with `gh auth token --user <login>`, then
                              cached for the life of the server and injected into
                              each command's environment; the active account is
                              never switched. A token rotated or revoked in gh
                              after that is picked up only on restart. Only the
                              login is ever an argument, so the token stays out of
                              --log-commands. GITHUB ONLY, and exclusive with
                              --gh-token-env (see the note under both flags).
    --gh-token-env <VAR>      Take the GitHub token from environment variable VAR
                              (for CI). Only the NAME is a flag value; the value
                              is read per operation and injected into the
                              command's environment, never argv. A VAR that is
                              unset, blank, or simply misspelled (a typo is still
                              a valid NAME, so it passes the startup check and
                              reads as unset) falls back to the ambient `gh`
                              login. GITHUB ONLY, and exclusive with
                              --gh-account.

                              Both GitHub identity flags fail loudly rather than
                              being ignored: giving BOTH is a startup error (they
                              name two different identities, and guessing which
                              one you meant is exactly the silent identity swap
                              they exist to prevent), and giving either one when
                              the forge in play is not GitHub — a --forge naming
                              another, or an `origin` that resolves to another or
                              to no forge at all — is a startup error naming the
                              flag and that forge.
    -h, --help                Print this help

The server speaks MCP over stdio; point an agent harness at it via a
`mcpServers` config entry. The git client is hardened (repo hooks and
`core.fsmonitor` disabled), so serving a repository you didn't create can't run
its hooks. It also refuses a git network operation (repo_fetch/repo_push) when
the repository overrides `core.sshCommand` — git runs that value through a
shell — naming the value and these two flags; a `core.sshCommand` that is only
in your own global git config is not affected. That refusal covers the GIT
backend only: a valid `.jj` wins backend detection, so on a jj or colocated repo
repo_fetch/repo_push run `jj git fetch`/`jj git push`, which still honour the
repository's `core.sshCommand`, and neither flag above has any effect there.
Repo-local config is otherwise not disabled wholesale — see \"A hardened git
client\" in the vcs-mcp guide for the vectors it leaves (`filter.*`,
`diff.*.textconv`).";

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
    /// The resolved `--ssh-command` / `--trust-repo-ssh-command` choice.
    ssh: SshOptIn,
    /// The resolved `--gh-account` / `--gh-token-env` choice.
    gh: GhAuth,
}

/// What the server does about a `core.sshCommand` the served repository
/// configures — the operator's half of the hardened client's SSH-transport
/// refusal (`Git::harden`).
///
/// The two flags are resolved into one value **at parse time**, so the outcome
/// does not depend on which one appeared last on the command line: `--ssh-command`
/// wins. It is the narrower and safer of the two (it names exactly what will run,
/// and `GIT_SSH_COMMAND` outranks the repository's key so that key never
/// executes), whereas `--trust-repo-ssh-command` accepts whatever the repository
/// says. When an operator asks for both, honouring the specific one can only
/// narrow what runs. This mirrors how `--allow-write` / `--allow-tools` are
/// resolved once, after the parse loop, rather than by flag order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
enum SshOptIn {
    /// No opt-in: a repository-configured `core.sshCommand` refuses the network
    /// operation (naming both flags).
    #[default]
    Refuse,
    /// `--trust-repo-ssh-command`.
    TrustRepo,
    /// `--ssh-command <command>`.
    Command(String),
}

/// Which GitHub identity the forge tools authenticate as — the operator's
/// `--gh-account` / `--gh-token-env` choice, resolved at parse time.
///
/// Unlike [`SshOptIn`], the two flags here are **mutually exclusive rather than
/// ranked**: neither is a narrower form of the other (a `gh` account login and a
/// token in an environment variable are two unrelated identities, potentially on
/// two different GitHub users), so a precedence rule would silently run every
/// forge call as whichever identity the rule happened to favour — the identity
/// swap `--gh-account` exists to prevent. Giving both is therefore a parse error;
/// repeating *one* of them is not (last wins, matching `--repo`/`--ssh-command`).
///
/// Both variants carry an identifier, never a secret: the account **login**, or
/// the **name** of the environment variable. The token itself is resolved inside
/// the client's credential path — once, then cached for the provider's life, for
/// [`GhAuth::Account`]; on every request for [`GhAuth::TokenEnv`], which just
/// reads the variable — and injected into the child's environment, so it reaches
/// neither argv nor the `--log-commands` log.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
enum GhAuth {
    /// No flag: the forge CLI's own ambient login, as before.
    #[default]
    Ambient,
    /// `--gh-account <login>`: the token of that `gh` account.
    Account(String),
    /// `--gh-token-env <VAR>`: the token in that environment variable.
    TokenEnv(String),
}

impl GhAuth {
    /// The flag that selected this identity, for an error message naming it;
    /// `None` for [`GhAuth::Ambient`], which no flag selected.
    fn flag(&self) -> Option<&'static str> {
        match self {
            GhAuth::Ambient => None,
            GhAuth::Account(_) => Some("--gh-account"),
            GhAuth::TokenEnv(_) => Some("--gh-token-env"),
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        // --help was requested; usage already printed.
        return Ok(());
    };

    let budget = output_budget(args.max_output_bytes);
    let repo = open_repo(
        &args.repo,
        args.timeout,
        budget,
        args.log_commands,
        &args.ssh,
    )?;
    let forge = resolve_forge(
        &repo,
        args.forge,
        args.timeout,
        budget,
        args.log_commands,
        &args.gh,
    )
    .await?;
    // The same ceiling goes to the server itself: the conflict tools read the
    // working copy directly (markers exist nowhere else), so they have no
    // subprocess whose OutputBudget they could inherit — without this the
    // operator's `--max-output-bytes` would not reach the one content path that
    // is callable with no write gate at all.
    let server = VcsMcpServer::new(repo, forge, args.writes).with_output_budget(budget);

    // Serve MCP over stdio until the client disconnects.
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}

/// Open the repo at `dir` with a **hardened** git client — the hardened profile
/// disables repo hooks and `core.fsmonitor`, scrubs repo-redirecting `GIT_*`
/// variables, and skips system config, so serving a repository the operator
/// didn't create can't execute its hooks (or honour a `core.fsmonitor` program)
/// on a tool call. jj has no repo-local hooks, so its client needs no equivalent
/// for *that* vector — but the hardened client's `core.sshCommand` refusal is
/// git-only in the same way: a colocated repo is driven by jj, whose
/// `git fetch`/`push` still honour the repository's value (see `hardened_git`).
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
    ssh: &SshOptIn,
) -> Result<Repo<Runner>, Box<dyn std::error::Error>> {
    let repo = Repo::discover_with(
        dir,
        || hardened_git(timeout, budget, log_commands, ssh),
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

/// A hardened git client carrying the optional per-command `timeout`, the
/// content-output `budget`, and the operator's SSH opt-in, driving the (optionally
/// command-logging) runner. `Git::with_runner(...).harden()` is `Git::hardened()`
/// with the injected runner.
///
/// The `ssh` opt-in is applied to the **client**, which is what the repo tools end
/// up running through **when the repository is git-backed**: `Repo::discover_with`
/// takes this very client and dispatches `repo_fetch`/`repo_push` to it, so the
/// setting reaches the actual network calls without any facade-level plumbing. It
/// reaches nothing on a **jj-backed** repo: a valid `.jj` wins detection (colocated
/// included), so this closure is never called, `repo_fetch`/`repo_push` go to
/// `jj git fetch`/`push`, and the repository's `core.sshCommand` runs unchecked.
fn hardened_git(
    timeout: Option<Duration>,
    budget: OutputBudget,
    log_commands: bool,
    ssh: &SshOptIn,
) -> Git<Runner> {
    let git = Git::with_runner(make_runner(log_commands)).harden();
    let git = match timeout {
        Some(t) => git.default_timeout(t),
        None => git,
    };
    apply_ssh_opt_in(git, ssh).default_output_budget(budget)
}

/// Map the resolved [`SshOptIn`] onto the git client's builders. Split out (and
/// generic over the runner) so a hermetic test can drive it with a recording
/// runner and check the choice really lands on the network command — `hardened_git`
/// itself always builds the real `JobRunner`.
fn apply_ssh_opt_in<R: ProcessRunner>(git: Git<R>, ssh: &SshOptIn) -> Git<R> {
    match ssh {
        SshOptIn::Refuse => git,
        SshOptIn::TrustRepo => git.trust_repo_ssh_command(),
        SshOptIn::Command(command) => git.with_ssh_command(command),
    }
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
    let mut ssh_command: Option<String> = None;
    let mut trust_repo_ssh_command = false;
    let mut gh_account: Option<String> = None;
    let mut gh_token_env: Option<String> = None;

    let mut it = args;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--allow-write" => allow_write = true,
            "--log-commands" => log_commands = true,
            "--trust-repo-ssh-command" => trust_repo_ssh_command = true,
            "--ssh-command" => {
                let value = it
                    .next()
                    .ok_or("--ssh-command needs a command (e.g. \"ssh -i /path/to/key\")")?;
                // An empty value is rejected here rather than at the first network
                // call: git treats `GIT_SSH_COMMAND` as *set* regardless of its
                // value, so an empty one makes every SSH operation die with
                // `cannot spawn` — a startup error is far clearer than that.
                if value.trim().is_empty() {
                    return Err(
                        "--ssh-command needs a non-empty command (e.g. \"ssh -i /path/to/key\"); \
                         omit the flag to use git's built-in ssh"
                            .to_string(),
                    );
                }
                // Repeated occurrences: last wins, matching --repo/--timeout.
                ssh_command = Some(value);
            }
            "--gh-account" => {
                let value = it
                    .next()
                    .ok_or("--gh-account needs a `gh` account login (e.g. \"octocat\")")?;
                // Trim here so the login the error messages, the `gh auth token
                // --user` argument, and the token cache key all name the same
                // account; a blank value would otherwise reach `gh` as an empty
                // `--user`, which is not an identity at all.
                let value = value.trim().to_string();
                if value.is_empty() {
                    return Err(
                        "--gh-account needs a non-empty `gh` account login (e.g. \"octocat\"); \
                         omit the flag to use gh's active account"
                            .to_string(),
                    );
                }
                // Repeated occurrences: last wins, matching --repo/--ssh-command.
                gh_account = Some(value);
            }
            "--gh-token-env" => {
                let value = it.next().ok_or(
                    "--gh-token-env needs the NAME of an environment variable (e.g. \"GH_TOKEN\"), \
                     not a token",
                )?;
                let value = value.trim().to_string();
                // No error on this flag echoes its value: an operator who pastes
                // the *token* where the variable NAME belongs would otherwise have
                // the secret printed to stderr by the very diagnostic meant to
                // help them.
                if value.is_empty() {
                    return Err(
                        "--gh-token-env needs a non-empty environment variable NAME (e.g. \
                         \"GH_TOKEN\"); omit the flag to use gh's ambient login"
                            .to_string(),
                    );
                }
                // A name no process could hold is rejected here rather than at the
                // first forge call: `std::env::var` reports an unusable name as
                // simply *not present*, and this provider treats "not present" as
                // "fall back to the ambient login" — so a typo would silently run
                // every forge call as the wrong identity. Only the two characters
                // that can never appear in an environment variable name are
                // refused (`=` separates name from value; whitespace survives no
                // shell), which leaves the platform-specific oddities (Windows'
                // `ProgramFiles(x86)`) usable.
                if value.contains('=') || value.chars().any(char::is_whitespace) {
                    return Err(
                        "--gh-token-env takes the NAME of an environment variable (e.g. \
                         \"GH_TOKEN\"); the value given contains `=` or whitespace, which no \
                         environment variable name can hold"
                            .to_string(),
                    );
                }
                // Repeated occurrences: last wins, matching --repo/--ssh-command.
                gh_token_env = Some(value);
            }
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
    // --ssh-command wins over --trust-repo-ssh-command, in either order: it names
    // exactly what will run and (via GIT_SSH_COMMAND) keeps the repository's own
    // value from executing, so it can only narrow what the broader flag allows.
    // See `SshOptIn`.
    let ssh = match (ssh_command, trust_repo_ssh_command) {
        (Some(command), _) => SshOptIn::Command(command),
        (None, true) => SshOptIn::TrustRepo,
        (None, false) => SshOptIn::Refuse,
    };
    // The GitHub identity flags are exclusive, not ranked: see `GhAuth`. Neither
    // value is echoed — `--gh-token-env`'s could be a mispasted token.
    let gh = match (gh_account, gh_token_env) {
        (Some(login), None) => GhAuth::Account(login),
        (None, Some(var)) => GhAuth::TokenEnv(var),
        (None, None) => GhAuth::Ambient,
        (Some(_), Some(_)) => {
            return Err(
                "--gh-account and --gh-token-env both choose a GitHub identity, and they name \
                 different ones; pass exactly one (--gh-account <login> for a `gh` account on \
                 this machine, --gh-token-env <VAR> for a token in the environment)"
                    .to_string(),
            );
        }
    };
    Ok(Some(Args {
        repo,
        forge,
        writes,
        timeout,
        max_output_bytes,
        log_commands,
        ssh,
        gh,
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
///
/// Fails — rather than returning a forge — when a GitHub identity flag was given
/// for a forge that isn't GitHub; see [`check_gh_auth_forge`]. That check runs
/// **after** the detection above precisely so it covers the auto-detected case,
/// not just an explicit `--forge`.
async fn resolve_forge(
    repo: &Repo<Runner>,
    forced: Option<ForgeKind>,
    timeout: Option<Duration>,
    budget: OutputBudget,
    log_commands: bool,
    gh: &GhAuth,
) -> Result<Option<Forge<Runner>>, String> {
    let cwd = repo.root().to_path_buf();
    let github_host = repo
        .remotes()
        .await
        .ok()
        .and_then(|remotes| remotes.into_iter().find(|remote| remote.name == "origin"))
        .and_then(|remote| GitHubHost::from_remote_url(&remote.url).ok());
    let kind = match forced {
        Some(k) => Some(k),
        None => detect_forge_kind(repo).await,
    };
    check_gh_auth_forge(gh, kind, forced.is_some())?;
    // Each forge CLI client exposes the same `with_runner`/`default_timeout`/
    // `default_output_budget` builders, but they are distinct types with no
    // shared trait — so apply them inline per arm.
    Ok(kind.and_then(|k| match k {
        ForgeKind::GitHub => {
            let c = GitHub::with_runner(make_runner(log_commands));
            let c = match timeout {
                Some(t) => c.default_timeout(t),
                None => c,
            };
            let c = c.default_output_budget(budget);
            // The identity opt-in is applied last, so it composes with (rather
            // than replaces) the timeout/budget the other flags set. `check_gh_auth_forge`
            // above has already refused a non-GitHub forge, so this is the only
            // arm that can carry one.
            let c = apply_gh_auth(c, gh, || make_runner(log_commands), timeout)
                .default_env_remove("GH_REPO");
            let c = match github_host {
                Some(host) => c.with_host(host),
                None => c,
            };
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
    }))
}

/// Refuse a GitHub identity flag when the forge actually in play is not GitHub.
///
/// `--gh-account` / `--gh-token-env` reach exactly one client — the `gh` one — so
/// on a GitLab/Gitea (or no) forge they would otherwise be **silently inert**: the
/// server would keep running every forge call under the ambient login the operator
/// just tried to override, with nothing said. An operator who names an identity is
/// stating which account the calls must run as, so the honest outcome is a startup
/// error naming the flag and the forge that displaced it. (Attaching the credential
/// to `glab`/`tea` instead is not an alternative: a GitHub token is not their
/// credential, and handing it over would ship the secret to the wrong service.)
///
/// `forced` distinguishes the two ways the forge was picked, so the message points
/// at the thing to change: the `--forge` value, or the repository's `origin`.
fn check_gh_auth_forge(gh: &GhAuth, kind: Option<ForgeKind>, forced: bool) -> Result<(), String> {
    let Some(flag) = gh.flag() else {
        return Ok(());
    };
    let source = if forced {
        "named by --forge"
    } else {
        "detected from the repository's `origin` remote"
    };
    match kind {
        Some(ForgeKind::GitHub) => Ok(()),
        // `Unknown` means "a remote that classifies as no known forge", which is
        // the same dead end as no forge at all: `resolve_forge` builds no client
        // for it, so the flag would reach nothing.
        Some(ForgeKind::Unknown) | None => Err(format!(
            "{flag} selects a GitHub identity, but this server has no GitHub forge: none was \
             {source}. Pass `--forge github` if the repository is on GitHub (a self-hosted or \
             otherwise unrecognised host is never guessed), or drop {flag}."
        )),
        Some(other) => Err(format!(
            "{flag} selects a GitHub identity, but the forge is {} ({source}). The flag reaches \
             the `gh` client only, so it would change nothing here — drop it, or serve a GitHub \
             repository.",
            other.as_str()
        )),
    }
}

/// Attach the resolved [`GhAuth`] to the GitHub client. Split out (and generic
/// over both runners) so a hermetic test can drive it with recording runners and
/// check the choice really reaches the spawned `gh` command — `resolve_forge`
/// itself always builds the real [`JobRunner`].
///
/// `probe` is called **only** for [`GhAuth::Account`]: that is the one variant
/// that resolves its token by running `gh auth token --user <login>`, and it runs
/// it on its own client (never the caller's, which would recurse back into this
/// provider). `timeout` bounds that probe, because a client's `default_timeout`
/// bounds the commands the client spawns, not credential resolution — without it
/// `--timeout`'s promise that no single stalled call can hang a request would
/// have a hole exactly the size of this new flag.
///
/// Where the provider is attached **is** the service boundary: this is the only
/// call site, and it holds the `gh` client alone. `GhAccountToken` additionally
/// refuses a request from another service (and a git request for another host) on
/// its own, but `EnvToken` — what `with_env_token` installs — answers whatever it
/// is asked, so keeping it off the git client and the `glab`/`tea` ones is what
/// stops a GitHub token from reaching a foreign service. `check_gh_auth_forge`
/// guarantees the non-GitHub arms are never reached with a flag set.
fn apply_gh_auth<R: ProcessRunner, P: ProcessRunner + 'static>(
    client: GitHub<R>,
    gh: &GhAuth,
    probe: impl FnOnce() -> P,
    timeout: Option<Duration>,
) -> GitHub<R> {
    match gh {
        GhAuth::Ambient => client,
        GhAuth::TokenEnv(var) => client.with_env_token(var.as_str()),
        GhAuth::Account(login) => {
            let provider = GhAccountToken::with_runner(probe(), login.as_str());
            let provider = match timeout {
                Some(t) => provider.default_timeout(t),
                None => provider,
            };
            client.with_credentials(Arc::new(provider))
        }
    }
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
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};
    use vcs_core::vcs_jj::Jj;
    use vcs_forge::vcs_github::GitHubApi;

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
        assert_eq!(
            args.ssh,
            SshOptIn::Refuse,
            "a repository-configured core.sshCommand is refused unless opted in"
        );
        assert_eq!(
            args.gh,
            GhAuth::Ambient,
            "the forge tools use the CLI's ambient login unless a flag picks one"
        );
    }

    // `--ssh-command` carries its value through to the client builder; a missing
    // or empty value is a startup error, not a network-time `cannot spawn`.
    #[test]
    fn ssh_command_flag_takes_a_value() {
        let args = parse(&["--ssh-command", "ssh -i /keys/id_ed25519"])
            .unwrap()
            .unwrap();
        assert_eq!(
            args.ssh,
            SshOptIn::Command("ssh -i /keys/id_ed25519".to_string())
        );

        assert!(parse(&["--ssh-command"]).is_err(), "value is required");
        let err = parse_err(&["--ssh-command", "   "]);
        assert!(err.contains("non-empty"), "got: {err}");

        // Repeated: last wins, like --repo/--timeout.
        let args = parse(&["--ssh-command", "ssh -1", "--ssh-command", "ssh -2"])
            .unwrap()
            .unwrap();
        assert_eq!(args.ssh, SshOptIn::Command("ssh -2".to_string()));
    }

    #[test]
    fn trust_repo_ssh_command_flag_is_a_switch() {
        let args = parse(&["--trust-repo-ssh-command"]).unwrap().unwrap();
        assert_eq!(args.ssh, SshOptIn::TrustRepo);
    }

    // The documented conflict resolution: the specific command wins over "trust
    // whatever the repository set", whichever order the two flags arrive in.
    #[test]
    fn ssh_command_wins_over_trust_repo_ssh_command() {
        for argv in [
            ["--trust-repo-ssh-command", "--ssh-command", "ssh -i k"],
            ["--ssh-command", "ssh -i k", "--trust-repo-ssh-command"],
        ] {
            let args = parse(&argv).unwrap().unwrap();
            assert_eq!(
                args.ssh,
                SshOptIn::Command("ssh -i k".to_string()),
                "for {argv:?}"
            );
        }
    }

    // The two flags coexist with the other options rather than shadowing them.
    #[test]
    fn ssh_flags_compose_with_the_write_gate_and_timeout() {
        let args = parse(&[
            "--allow-write",
            "--timeout",
            "9",
            "--trust-repo-ssh-command",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(args.writes, WriteGate::All);
        assert_eq!(args.timeout, Some(Duration::from_secs(9)));
        assert_eq!(args.ssh, SshOptIn::TrustRepo);
    }

    // The `--help` text must name both flags: it is the only place an operator
    // hitting the refusal learns how to continue (the error names them too).
    #[test]
    fn usage_documents_the_ssh_opt_ins() {
        for flag in ["--ssh-command", "--trust-repo-ssh-command"] {
            assert!(USAGE.contains(flag), "USAGE must document {flag}");
        }
        assert!(
            USAGE.contains("core.sshCommand"),
            "USAGE must name the key the hardened client refuses"
        );
    }

    // `--gh-account` carries its login through to the client builder; a missing or
    // empty value is a startup error, not an empty `gh auth token --user`.
    #[test]
    fn gh_account_flag_takes_a_value() {
        let args = parse(&["--gh-account", "octocat"]).unwrap().unwrap();
        assert_eq!(args.gh, GhAuth::Account("octocat".to_string()));

        assert!(parse(&["--gh-account"]).is_err(), "value is required");
        let err = parse_err(&["--gh-account", "   "]);
        assert!(err.contains("non-empty"), "got: {err}");

        // Surrounding whitespace is trimmed, so the login in the error text, in
        // the `gh --user` argument, and in the token cache key all agree.
        let args = parse(&["--gh-account", "  octocat "]).unwrap().unwrap();
        assert_eq!(args.gh, GhAuth::Account("octocat".to_string()));

        // Repeated: last wins, like --repo/--ssh-command.
        let args = parse(&["--gh-account", "one", "--gh-account", "two"])
            .unwrap()
            .unwrap();
        assert_eq!(args.gh, GhAuth::Account("two".to_string()));
    }

    // `--gh-token-env` takes the variable's NAME. A value that cannot be one is
    // rejected at startup rather than silently resolving to "unset" (which the
    // provider treats as "use the ambient login" — a silent identity swap), and no
    // error echoes the value, which may be a mispasted token.
    #[test]
    fn gh_token_env_flag_takes_a_variable_name() {
        let args = parse(&["--gh-token-env", "CI_GH_TOKEN"]).unwrap().unwrap();
        assert_eq!(args.gh, GhAuth::TokenEnv("CI_GH_TOKEN".to_string()));

        assert!(parse(&["--gh-token-env"]).is_err(), "value is required");
        let err = parse_err(&["--gh-token-env", "   "]);
        assert!(err.contains("non-empty"), "got: {err}");

        for bad in ["GH TOKEN", "GH_TOKEN=ghp_secret"] {
            let err = parse_err(&["--gh-token-env", bad]);
            assert!(err.contains("NAME"), "got: {err}");
            assert!(
                !err.contains(bad),
                "the rejected value must not be echoed (it may be a token): {err}"
            );
        }

        // Repeated: last wins, like --repo/--ssh-command.
        let args = parse(&["--gh-token-env", "A", "--gh-token-env", "B"])
            .unwrap()
            .unwrap();
        assert_eq!(args.gh, GhAuth::TokenEnv("B".to_string()));
    }

    // The documented conflict resolution, and the one place it differs from the
    // SSH pair: two identities can't be ranked, so both flags together are an
    // error rather than a silent precedence — in either order.
    #[test]
    fn gh_identity_flags_are_mutually_exclusive() {
        for argv in [
            ["--gh-account", "octocat", "--gh-token-env", "CI_GH_TOKEN"],
            ["--gh-token-env", "CI_GH_TOKEN", "--gh-account", "octocat"],
        ] {
            let err = parse_err(&argv);
            assert!(err.contains("--gh-account"), "for {argv:?}: {err}");
            assert!(err.contains("--gh-token-env"), "for {argv:?}: {err}");
        }
    }

    // Neither flag is set by default: the forge tools keep the ambient CLI login.
    #[test]
    fn gh_identity_is_ambient_by_default() {
        assert_eq!(parse(&[]).unwrap().unwrap().gh, GhAuth::Ambient);
        // And they compose with the other options rather than shadowing them.
        let args = parse(&["--allow-write", "--timeout", "9", "--gh-account", "octocat"])
            .unwrap()
            .unwrap();
        assert_eq!(args.writes, WriteGate::All);
        assert_eq!(args.timeout, Some(Duration::from_secs(9)));
        assert_eq!(args.gh, GhAuth::Account("octocat".to_string()));
    }

    // Either identity flag reaches the `gh` client only, so anything but a GitHub
    // forge must be a startup error naming the flag and the forge — never a
    // silently ignored flag that leaves every call on the ambient login.
    #[test]
    fn gh_identity_flags_require_a_github_forge() {
        let account = GhAuth::Account("octocat".to_string());
        let token_env = GhAuth::TokenEnv("CI_GH_TOKEN".to_string());

        // GitHub, however it was picked: fine.
        for forced in [true, false] {
            check_gh_auth_forge(&account, Some(ForgeKind::GitHub), forced).expect("github");
            check_gh_auth_forge(&token_env, Some(ForgeKind::GitHub), forced).expect("github");
        }

        // Another forge: named, along with the flag and how the forge was picked.
        let err = check_gh_auth_forge(&account, Some(ForgeKind::GitLab), true).unwrap_err();
        assert!(err.contains("--gh-account"), "got: {err}");
        assert!(err.contains("gitlab"), "names the actual forge: {err}");
        assert!(err.contains("--forge"), "points at what chose it: {err}");

        let err = check_gh_auth_forge(&token_env, Some(ForgeKind::Gitea), false).unwrap_err();
        assert!(err.contains("--gh-token-env"), "got: {err}");
        assert!(err.contains("gitea"), "names the actual forge: {err}");
        assert!(err.contains("origin"), "points at what chose it: {err}");

        // No forge at all (and the unclassified remote, which builds no client
        // either): also an error, with the fix spelled out.
        for kind in [None, Some(ForgeKind::Unknown)] {
            let err = check_gh_auth_forge(&account, kind, false).unwrap_err();
            assert!(err.contains("--gh-account"), "for {kind:?}: {err}");
            assert!(err.contains("--forge github"), "for {kind:?}: {err}");
        }

        // Without a flag there is nothing to refuse, on any forge.
        for kind in [None, Some(ForgeKind::GitLab), Some(ForgeKind::GitHub)] {
            check_gh_auth_forge(&GhAuth::Ambient, kind, false).expect("no flag, no refusal");
        }
    }

    // The `--help` text must name both flags: it is where an operator serving a
    // machine with several `gh` logins learns the server can pick one.
    #[test]
    fn usage_documents_the_gh_identity_flags() {
        for flag in ["--gh-account", "--gh-token-env"] {
            assert!(USAGE.contains(flag), "USAGE must document {flag}");
        }
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

    // The SSH opt-in is a *client* setting, and the repo tools run through the
    // very client `open_repo` hands to `Repo::discover_with` — so a flag set here
    // must be observable on the actual network command the facade spawns, not just
    // on the `Git` value this binary built. Both opt-ins are checked end to end
    // through `Repo::fetch` (what `repo_fetch` calls):
    //
    //   * `--ssh-command` must put its exact value in `GIT_SSH_COMMAND` on the
    //     fetch (git gives that precedence over any repo `core.sshCommand`), and
    //   * `--trust-repo-ssh-command` must let the fetch run against a repository
    //     whose `core.sshCommand` differs from the global one — the case a
    //     hardened client refuses without an opt-in.
    //
    // Both also prove the opt-in costs no config probe: exactly one command runs.
    #[tokio::test]
    async fn ssh_opt_in_reaches_the_network_call_through_the_facade() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = apply_ssh_opt_in(
            Git::with_runner(&rec).harden(),
            &SshOptIn::Command("ssh -i /keys/id_ed25519".to_string()),
        );
        Repo::from_git("/r", "/r", git)
            .fetch()
            .await
            .expect("fetch with a pinned ssh command");

        let calls = rec.calls();
        assert_eq!(calls.len(), 1, "opt-in must not spawn a config probe");
        let call = &calls[0];
        assert!(
            call.args_str().contains(&"fetch".to_string()),
            "expected the fetch, got {:?}",
            call.args_str()
        );
        // `env_is` reads the EFFECTIVE override (last write wins), which is what
        // the child will see: the hardened profile's scrub of this same variable
        // is still in the list, ahead of the per-command pin that supersedes it.
        assert!(
            call.env_is("GIT_SSH_COMMAND", "ssh -i /keys/id_ed25519"),
            "the flag's value must reach git as GIT_SSH_COMMAND, got {:?}",
            call.env("GIT_SSH_COMMAND")
        );
    }

    #[tokio::test]
    async fn trust_repo_ssh_command_lets_a_repo_configured_value_through() {
        // A repository that overrides the (absent) global `core.sshCommand`: the
        // shape a hardened client refuses when nothing is opted in.
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(
                    ["git", "config", "--get", "core.sshCommand"],
                    Reply::ok("/tmp/evil"),
                )
                .fallback(Reply::ok("")),
        );
        let git = apply_ssh_opt_in(Git::with_runner(&rec).harden(), &SshOptIn::TrustRepo);
        Repo::from_git("/r", "/r", git)
            .fetch()
            .await
            .expect("--trust-repo-ssh-command must lift the refusal");

        let calls = rec.calls();
        assert_eq!(calls.len(), 1, "trusting must not spawn a config probe");
        assert!(
            calls[0].args_str().contains(&"fetch".to_string()),
            "expected the fetch, got {:?}",
            calls[0].args_str()
        );
        // The hardened profile still *scrubs* an inherited GIT_SSH_COMMAND (a
        // removal, which `has_env` reports as absent); what must not happen is a
        // pinned value — trusting the repository means running ITS command.
        assert!(
            !calls[0].has_env("GIT_SSH_COMMAND"),
            "trusting the repository must not also pin a command: {:?}",
            calls[0].env("GIT_SSH_COMMAND")
        );
    }

    // The token `--gh-account` resolves must reach `gh` the way every other
    // credential in this workspace does — in the environment, never in argv, so
    // `--log-commands` (which logs argv and deliberately never the environment)
    // cannot print it. The provider is also the only new thing that *spawns*, so
    // this pins what its own command line carries: the login, and nothing else.
    #[tokio::test]
    async fn gh_account_authenticates_gh_without_the_token_reaching_argv() {
        const TOKEN: &str = "gho_t180_account_token";
        // `Arc` the probe recorder: attaching the provider coerces it to
        // `Arc<dyn CredentialProvider>`, which is `'static`, so the probe runner
        // must be owned — and shared, so the test can still read its calls.
        let probe = Arc::new(RecordingRunner::replying(Reply::ok(format!("{TOKEN}\n"))));
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let client = apply_gh_auth(
            GitHub::with_runner(&rec),
            &GhAuth::Account("octocat".to_string()),
            || Arc::clone(&probe),
            Some(Duration::from_secs(9)),
        );

        client.pr_list(Path::new("/r")).await.expect("pr list");

        let call = rec.only_call();
        assert!(
            call.env_is("GH_TOKEN", TOKEN),
            "the account's token must reach gh in the environment, got {:?}",
            call.env("GH_TOKEN")
        );
        assert!(
            !call.args_str().iter().any(|a| a.contains(TOKEN)),
            "the token must never reach argv (that is what --log-commands prints)"
        );
        // The one command the provider itself runs names the login only.
        assert_eq!(
            probe.only_call().args_str(),
            ["auth", "token", "--user", "octocat"]
        );
    }

    // What the flag's documentation claims about *when* the token is resolved:
    // once, on the first call that needs it, and cached from then on — only the
    // injection is per operation. Two forge calls, one `gh auth token` spawn.
    // This is the visible half of the trade-off the docs spell out: a token
    // rotated in `gh` after the first call is not picked up until restart, since
    // this binary builds the provider once (`resolve_forge`) and the server holds
    // that forge for its whole life.
    #[tokio::test]
    async fn gh_account_resolves_its_token_once_and_then_reuses_it() {
        const TOKEN: &str = "gho_t180_cached_token";
        let probe = Arc::new(RecordingRunner::replying(Reply::ok(format!("{TOKEN}\n"))));
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let client = apply_gh_auth(
            GitHub::with_runner(&rec),
            &GhAuth::Account("octocat".to_string()),
            || Arc::clone(&probe),
            None,
        );

        client
            .pr_list(Path::new("/r"))
            .await
            .expect("first pr list");
        client
            .pr_list(Path::new("/r"))
            .await
            .expect("second pr list");

        let calls = rec.calls();
        assert_eq!(calls.len(), 2, "both forge calls ran");
        assert!(
            calls.iter().all(|c| c.env_is("GH_TOKEN", TOKEN)),
            "every command carries the token in its environment (injection is \
             what happens per operation)"
        );
        assert_eq!(
            probe.calls().len(),
            1,
            "the token is resolved once and cached, not re-resolved per operation"
        );
    }

    // The env-token path, proved without mutating the process environment (which
    // `std::env::set_var` makes unsafe precisely because it races every other
    // test in this binary): point the flag at a variable the test process already
    // has. What the flag carries is the variable's NAME; the value is read inside
    // the credential path and injected into the child's environment, so — like the
    // account path — it is absent from argv.
    #[tokio::test]
    async fn gh_token_env_authenticates_gh_from_the_named_variable() {
        const VAR: &str = "PATH";
        let expected = std::env::var(VAR).expect("PATH is set for a test process");
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let client = apply_gh_auth(
            GitHub::with_runner(&rec),
            &GhAuth::TokenEnv(VAR.to_string()),
            // Never called for this variant: nothing is spawned to resolve an
            // environment variable. (The return type is named only because a
            // diverging closure has none of its own.)
            || -> JobRunner { unreachable!("--gh-token-env must not spawn a credential probe") },
            None,
        );

        client.pr_list(Path::new("/r")).await.expect("pr list");

        let call = rec.only_call();
        assert!(
            call.env_is("GH_TOKEN", &expected),
            "the named variable's value must reach gh in the environment"
        );
        assert!(
            !call.args_str().iter().any(|a| a.contains(&expected)),
            "the value must never reach argv"
        );
    }

    // No flag: the client is handed to the forge exactly as before, with no
    // credential attached, so the forge tools keep using gh's ambient login.
    #[tokio::test]
    async fn ambient_gh_auth_injects_no_token() {
        let rec = RecordingRunner::replying(Reply::ok("[]"));
        let client = apply_gh_auth(
            GitHub::with_runner(&rec),
            &GhAuth::Ambient,
            || -> JobRunner { unreachable!("ambient auth must not spawn a credential probe") },
            None,
        );

        client.pr_list(Path::new("/r")).await.expect("pr list");

        assert!(
            !rec.only_call().has_env("GH_TOKEN"),
            "ambient auth must leave gh's own login in charge"
        );
    }

    // The forge gate on the *detected* path — the case an explicit `--forge` check
    // would miss. The refusal also precedes every client build, so no `gh` probe
    // (and no GitHub client) is created for a repository that isn't on GitHub.
    #[tokio::test]
    async fn resolve_forge_refuses_a_gh_identity_flag_on_a_detected_non_github_forge() {
        let rec = Arc::new(RecordingRunner::replying(Reply::ok(
            "origin https://gitlab.com/example/repo.git\n",
        )));
        let runner: Runner = Box::new(Arc::clone(&rec));
        let repo = Repo::from_jj("/r", "/r", Jj::with_runner(runner));

        let err = resolve_forge(
            &repo,
            None,
            None,
            OutputBudget::unlimited(),
            false,
            &GhAuth::Account("octocat".to_string()),
        )
        .await
        .expect_err("a detected GitLab forge must refuse --gh-account");
        assert!(err.contains("--gh-account"), "got: {err}");
        assert!(err.contains("gitlab"), "names the detected forge: {err}");
        // Only the remote query ran — the refusal happens before any forge client.
        assert_eq!(rec.calls().len(), 1);

        // The same repository without the flag still resolves its GitLab forge.
        assert!(
            resolve_forge(
                &repo,
                None,
                None,
                OutputBudget::unlimited(),
                false,
                &GhAuth::Ambient
            )
            .await
            .expect("no flag, no refusal")
            .is_some()
        );
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
