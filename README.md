# vcs-toolkit-rs

[![CI](https://github.com/ZelAnton/vcs-toolkit-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ZelAnton/vcs-toolkit-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)

A Rust toolkit for automating **Git**, **Jujutsu**, and **GitHub** through CLI
process execution. Rather than reimplementing each tool's protocol, these crates
shell out to the official binaries (`git`, `jj`, `gh`) and capture their output —
thin, predictable wrappers you can compose into automation.

Every command is **async** (tokio) and runs inside an OS **job** (a Windows Job
Object or a Linux cgroup v2) so the whole process tree dies with the parent — no
orphaned subprocesses. That mechanism comes from the external
[`processkit`](https://crates.io/crates/processkit) crate, which also provides
timeouts, the structured `Error`, and the test seams these wrappers build on.

## Why

- **No reinvented protocols.** You get exactly the behaviour of the `git`/`jj`/`gh`
  you already have installed — same config, credentials, and version semantics.
- **No leaked subprocesses.** A crashing, panicking, or `Ctrl-C`'d parent never
  leaves a `git gc` or a hung `gh` behind: the OS job reaps the entire tree on
  close (see the platform table below).
- **Testable by construction.** Consumers depend on an interface trait, not the
  concrete client, and swap in a mock or a scripted runner — no temp repos, no
  network, no installed binaries needed for unit tests.
- **Structured failures.** A non-zero exit, a spawn failure, a timeout, and a
  parse error are distinct `processkit::Error` variants carrying program, exit
  code, and stderr — not a stringly-typed blob.
- **Async with deadlines.** Every call is a future; an optional per-client or
  per-call timeout kills the job (and the whole tree) when it elapses.

## Crates

This is a Cargo workspace, each crate **versioned and published independently**:
three CLI wrappers built on the external
[`processkit`](https://crates.io/crates/processkit) crate, a facade over the
git/jj pair, and a dependency-free test-fixture crate:

| Crate | Drives | crates.io name |
|---|---|---|
| [`crates/git`](crates/git) | the `git` binary | `vcs-git` |
| [`crates/jj`](crates/jj) | the `jj` (Jujutsu) binary | `vcs-jj` |
| [`crates/github`](crates/github) | the `gh` (GitHub CLI) binary | `vcs-github` |
| [`crates/core`](crates/core) | — (facade over `vcs-git`/`vcs-jj`) | `vcs-core` |
| [`crates/testkit`](crates/testkit) | — (test fixtures: git/jj sandboxes, bare remote) | `vcs-testkit` |

Each **CLI wrapper** exposes an **interface trait** (`GitApi`/`JjApi`/`GitHubApi`) and a
real client (`Git`/`Jj`/`GitHub`) with typed, repo-scoped async commands that
return parsed structs and fail with the structured `processkit::Error`. They build
on `processkit` (its `CliClient` core, the `cli_client!` macro, the `ProcessRunner`
seam) and depend on `async-trait`; `vcs-github` additionally adds
`serde`/`serde_json` to deserialize `gh … --json` output.

### Process containment

`processkit` launches every child inside an OS job so kill-on-close holds — the
mechanism is platform-specific and observable at runtime via its `Mechanism`:

| Platform | Mechanism | Kill-on-close |
|---|---|---|
| Windows | [Job Object](https://learn.microsoft.com/windows/win32/procthread/job-objects) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | ✅ whole tree |
| Linux | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) via `cgroup.kill`, with a POSIX **process-group** fallback when no writable cgroup is available | ✅ whole tree (cgroup) / ✅ process group (fallback) |
| macOS, BSD (and other Unix) | POSIX **process group** (`killpg` on drop) — the same backend Linux falls back to | ✅ whole tree (process group) |

v1 guarantees kill-on-close; resource limits are intentionally out of scope.

## Quick start

Add the wrapper(s) you need. Every method is `async`, so call them from a tokio
runtime:

```rust
use processkit::Error;
use std::path::Path;
use std::time::Duration;
use vcs_git::{Git, GitApi};

#[tokio::main]
async fn main() -> Result<(), Error> {
    // A real, job-backed client; give every command a 10s deadline.
    let git = Git::new().default_timeout(Duration::from_secs(10));
    let repo = Path::new(".");

    let branch = git.current_branch(repo).await?; // String
    let status = git.status(repo).await?; // Vec<StatusEntry>
    let log = git.log(repo, 5).await?; // Vec<Commit>, newest first

    println!(
        "on {branch}: {} change(s), HEAD = {}",
        status.len(),
        log[0].short_hash
    );

    // Distinguish failure modes structurally instead of matching on strings.
    match git.checkout(repo, "does-not-exist").await {
        Err(Error::Exit { code, stderr, .. }) => {
            eprintln!("git exited {code}: {stderr}");
        }
        Err(Error::Timeout { .. }) => eprintln!("git timed out"),
        other => {
            other?;
        }
    }
    Ok(())
}
```

`vcs-jj` and `vcs-github` follow the same shape:

```rust
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};
use vcs_jj::{Jj, JjApi};

# async fn demo() -> Result<(), processkit::Error> {
    let jj = Jj::new();
    let head = jj.current_change(Path::new(".")).await?; // Change
    jj.describe(Path::new("."), "wip: refactor").await?;

    let gh = GitHub::new();
    if gh.auth_status().await? {
        // bool, never errors on exit code
        let prs = gh.pr_list(Path::new(".")).await?; // Vec<PullRequest>
        let _ = prs;
    }
# Ok(()) }
```

## What each client exposes

Every client also has a `run(args)` / `run_raw(args)` escape hatch for commands
that aren't modelled yet, plus `version()`.

| `vcs-git` (`GitApi`) | `vcs-jj` (`JjApi`) | `vcs-github` (`GitHubApi`) |
|---|---|---|
| `status` → `Vec<StatusEntry>` | `status` → `String` | `auth_status` → `bool` |
| `current_branch` → `String` | `current_change` → `Change` | `repo_view` → `Repo` |
| `branches` → `Vec<Branch>` | `log` → `Vec<Change>` | `pr_list` → `Vec<PullRequest>` |
| `log` → `Vec<Commit>` | `describe` / `new_change` | `pr_view` → `PullRequest` |
| `rev_parse` → `String` | `bookmarks` → `Vec<Bookmark>` | `issue_list` → `Vec<Issue>` |
| `init` / `add` / `commit` | `bookmark_set` | `pr_create` → URL |
| `create_branch` / `checkout` | `git_fetch` / `git_push` | `api` → raw JSON |
| `diff_is_empty` → `bool` | | `pr_merge` / `pr_ready` / `pr_close` |
| `worktree_list` → `Vec<Worktree>`, `worktree_add`/`_remove`/`_move`/`_prune` | `workspace_list` → `Vec<Workspace>`, `workspace_add`/`_root`/`_forget` | `pr_checks` → `Vec<CheckRun>` |
| `branch_exists`/`remote_branch_exists` → `bool`, `common_dir`/`git_dir`/`remote_head_branch` | `root`/`trunk`/`current_bookmark`, `bookmark_create`/`_move`/`_rename`/`_delete` | `run_list`/`run_view`/`run_watch` → `WorkflowRun` |
| `diff_shortstat` → `DiffStat`, `is_merged`, `rev_list_count` | `diff_summary`/`diff_stat`, `commit_count`, `is_conflicted`, `template_query` | `pr_review` / `pr_comment`, `pr_feedback` → reviews+comments |
| `merge_*` / `rebase_*` / `reset_*` / `fetch` | `rebase`/`edit`/`squash_into`/`new_merge`/`git_import`, `op_head`/`op_restore`/`op_undo` | `issue_create`/`issue_view`, `release_list`/`release_view` |
| `clone_repo`, `tag_*`, `show_file`, `blame` → `Vec<BlameLine>`, `config_get`/`_set`, `cherry_pick`/`revert`/`rebase_skip` | `git_clone`, `absorb`/`split_paths`/`duplicate`, `op_log` → `Vec<Operation>`, `evolog`, `file_annotate`/`file_show` | |

## Recipes

**Stage everything changed and commit (git):**

```rust
use std::path::{Path, PathBuf};
use vcs_git::{Git, GitApi};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let git = Git::new();
    let paths: Vec<PathBuf> = git
        .status(repo)
        .await?
        .into_iter()
        .map(|e| PathBuf::from(e.path))
        .collect();
    if !paths.is_empty() {
        git.add(repo, &paths).await?;
        git.commit(repo, "chore: snapshot").await?;
    }
# Ok(()) }
```

**Describe the working copy and push a bookmark (jj):**

```rust
use std::path::Path;
use vcs_jj::{Jj, JjApi};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let jj = Jj::new();
    jj.describe(repo, "feat: parser").await?;
    jj.git_fetch(repo).await?;
    jj.bookmark_set(repo, "main", "@").await?;
    jj.git_push(repo, Some("main".to_string())).await?;
# Ok(()) }
```

**Open a PR only when authenticated (github):**

```rust
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let gh = GitHub::new();
    if gh.auth_status().await? {
        // head / base optional: `None` head = current branch, `None` base = repo default.
        let url = gh.pr_create(repo, "My change", "Body", None, None).await?;
        println!("opened {url}");
    }
# Ok(()) }
```

**Drop to a raw command (any client) when something isn't modelled yet:**

```rust
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
    // `run` returns trimmed stdout (errors on non-zero); `run_raw` returns the full
    // `processkit::ProcessResult<String>` without erroring on a non-zero exit.
    let sha = git.run(&["rev-parse".into(), "HEAD".into()]).await?;
    let res = git
        .run_raw(&["status".into(), "--porcelain".into()])
        .await?;
    println!("{sha} — exit {:?}", res.code()); // `code()` is `Option<i32>`
# Ok(()) }
```

## Built for testing

Consumers code against the trait and substitute a fake in their tests — two seams,
neither of which needs the real binary, a temp repo, or the network:

```rust
use std::path::Path;
use vcs_git::{Git, GitApi};

// Production code depends on the interface, not the concrete client:
async fn current(git: &dyn GitApi) -> Result<String, processkit::Error> {
    git.current_branch(Path::new(".")).await
}

let git = Git::new(); // real, job-backed git
// current(&git).await ...
```

- **Mock the interface** — enable the `mock` feature; `mockall` generates
  `MockGitApi` for stubbing whole methods (`expect_current_branch().returning(…)`).
  A consumer enables it only under `[dev-dependencies]`, so `mockall` never lands
  in a release build.
- **Inject a runner** — `Git::with_runner(processkit::ScriptedRunner::new()…)`
  feeds canned binary output through the *real* argument-building and parsing, so
  a test exercises the actual command wiring without spawning anything. Wrap it in
  a `processkit::RecordingRunner` to assert the exact command that was built — full
  args, cwd, env, and even that a flag is *absent*:

  ```rust
  use processkit::{Reply, ScriptedRunner};
  use std::path::Path;
  use vcs_git::{Git, GitApi};

  # async fn demo() {
      let git = Git::with_runner(ScriptedRunner::new().on(["status"], Reply::ok(" M src/lib.rs\0")));
      let entries = git.status(Path::new(".")).await.unwrap();
      assert_eq!(entries[0].code, " M");
  # }
  ```

For building integration-test scenarios, the [`vcs-testkit`](crates/testkit)
crate (a dev-dependency) provides throwaway `GitSandbox`/`JjSandbox` repos, a
seeded `BareRemote` to clone/fetch against, and a self-cleaning `TempDir` — the
same fixtures this workspace's own ignored tests run on.

## Untrusted input and repos

Two layers, both on by default or one call away:

- **Injection guards (automatic).** Every exposed positional argument
  (branch/tag/bookmark names, revisions, revsets, endpoints) refuses a
  leading-`-` or empty value *before* anything spawns — a caller-supplied
  string can't smuggle a flag into argv. For pre-validation at your input
  boundary, the `RefName` / `RevSpec` (vcs-git) and `RevsetExpr` (vcs-jj)
  newtypes validate eagerly; method signatures stay `&str`.
- **`Git::hardened()`.** Running `git` inside a repository you didn't create
  executes that repo's hooks and honours its config. The hardened profile
  disables hooks and `core.fsmonitor`, scrubs repo-redirecting `GIT_*`
  variables, skips system config, and keeps prompts off — on every command
  the client runs. jj needs no equivalent (no repo-local hooks); in a
  colocated repo, harden the `Git` client you point at it.

Conflicted files parse into a typed model too: `vcs_git::conflict` /
`vcs_jj::conflict` turn marker soup into structured regions
(ours/base/theirs; jj's `diff` and `snapshot` styles) with a byte-exact
`render` and a `resolve(side)` writer — the primitive for programmatic
conflict resolution.

## Observing commands

Four seams, no extra configuration:

- **Argv observation** — wrap the real runner the same way tests wrap fakes:
  `RecordingRunner::new(JobRunner::new())`, hand `&rec` to `with_runner`, and
  read `rec.calls()` (full argv, cwd, env per invocation).
- **Live output streaming** — `processkit::Command` supports per-line
  callbacks (`.on_stdout_line(|l| …)` / `.on_stderr_line(…)`), so a
  long-running command built directly against processkit can report progress
  while it runs. The typed `Git`/`Jj` methods consume their `Command`
  internally and do **not** surface the hook yet — streaming wrappers (e.g. a
  fetch-with-progress) land once the upstream hardening (callback panic
  isolation, scripted-replay testability) ships in processkit.
- **`tracing` feature** — each crate's `tracing` feature makes processkit emit
  a `debug` event per command run (program, args, exit) for any subscriber.
- **Dry-run harness** — `ScriptedRunner::new().fallback(Reply::ok(""))`
  executes nothing and answers everything, so a whole flow can be exercised
  without touching a repository; add `.on(…)` rules for the calls that need
  realistic replies.

## Build, test

Requires a Rust toolchain with the **2024 edition** (Rust 1.88+; the wrappers use
let-chains). The real-binary tests additionally need `git` / `jj` / `gh` on `PATH`.

```bash
cargo build                         # build all crates
cargo test                          # unit + integration tests (whole workspace)
cargo test -p vcs-git               # one crate
cargo test --workspace --features mock      # exercise the mockall mocks + ScriptedRunner
cargo test -- --ignored             # tests that require the real binaries installed
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

Tests that shell out to the real `git` / `jj` / `gh` binaries are marked
`#[ignore]` so CI stays hermetic; run them locally with `--ignored`. CI
(`.github/workflows/ci.yml`) runs fmt, clippy (with and without `mock`), the test
suite on Linux/Windows/macOS, `cargo-deny`, and a `cargo package` gate.

## Publishing

Releases go through the **`Release` GitHub Action** (`workflow_dispatch`) — you
never type a version. Click *Run workflow* and pick:

- **Crate** — `vcs-git`, `vcs-jj`, `vcs-github`, `vcs-testkit`, `vcs-core`, or
  **`all`** (release every crate in one run).
- **Bump** — `patch` / `minor` / `major`.

For each selected crate it reads the current version from that crate's
`Cargo.toml`, computes the next one (a crate's **first release** — no
`<crate>-v*` tag yet — ships the current version as-is, ignoring the bump),
promotes its `CHANGELOG.md`, **publishes to crates.io before tagging**
`<crate>-v<version>`, and opens a GitHub Release from the curated notes. `all`
does them in a single commit + atomic push.

The CLI wrappers depend only on the already-published
[`processkit`](https://crates.io/crates/processkit) crate, and `vcs-testkit`
depends on nothing at all, so they release independently. The **`vcs-core`
facade is the exception** — it depends on `vcs-git`/`vcs-jj`, so `all`
publishes it **last**, and its `^MAJOR.MINOR` requirement on them must stay in
range when they cross a minor/major boundary.

## Conventions

See [AGENTS.md](AGENTS.md) for code style, dependency management (every
dependency gets a "why" comment; no fixed allow-list), the per-crate changelog
process, and the `jj` version-control workflow. Planned future work lives in
[ROADMAP.md](ROADMAP.md).

## License

Licensed under the [MIT License](LICENSE).
