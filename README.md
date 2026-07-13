# vcs-toolkit-rs

[![CI](https://github.com/ZelAnton/vcs-toolkit-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ZelAnton/vcs-toolkit-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)

**Automate Git, Jujutsu, and the GitHub / GitLab / Gitea forges from Rust.**
vcs-toolkit drives the command-line tools you already have — `git`, `jj`, `gh`,
`glab`, `tea` — and hands you their output as typed Rust values. No `libgit2`, no
FFI, no reimplemented Git: you get the real tools' exact behavior, config, and
credentials, behind a clean **async** API. There's even a [Model Context
Protocol](https://modelcontextprotocol.io) (MCP) server, so an AI agent can drive
your repositories safely.

![Cover](https://raw.githubusercontent.com/ZelAnton/vcs-toolkit-rs/main/cover.png)

Every command is **async** (tokio) and runs inside an OS **job** (a Windows Job
Object or a Linux cgroup v2) so the whole process tree dies with the parent — no
orphaned subprocesses. That mechanism comes from the external
[`processkit`](https://crates.io/crates/processkit) crate, which also provides
timeouts, the structured `Error`, and the test seams these wrappers build on.

## What you can do

vcs-toolkit covers the everyday automation surface for git, Jujutsu, and the
three forges — in plain terms:

- **Drive git or Jujutsu directly** — status, branches/bookmarks, commit, diff,
  log, merge & rebase, worktrees, tags, blame, clone, and more, through one typed
  async API per tool (`vcs-git`, `vcs-jj`).
- **Treat "the repository" as one thing** — a single `Repo` that auto-detects
  whether a directory is git or jj and runs whatever both support, so your code
  doesn't have to care which it is (`vcs-core`).
- **Automate code forges** — list, open, review, and merge pull/merge requests, and
  manage issues and releases, on GitHub, GitLab, and Gitea — one API across all three
  (`vcs-forge`); read CI status on the forges that expose it (GitHub, GitLab).
- **React to repository changes** — stream typed events (HEAD moved, branch
  switched, a conflict appeared, the working copy changed…) as they happen
  (`vcs-watch`).
- **Supply credentials per operation** — by default every backend uses the CLI's own
  ambient auth; opt in to a `CredentialProvider` (CI token, vault, per-account) and
  the secret is injected per call (`GH_TOKEN`/`GITLAB_TOKEN` for forges, an inline
  git credential helper for HTTPS) — kept out of `argv` and never persisted.
- **Give an AI agent safe repo access** — a ready-made MCP server exposes every
  operation as an agent-callable tool, with writes gated off by default
  (`vcs-mcp`).
- **Test it all without the real tools** — point your code at a mock or canned
  command output; no installed binary, temp repo, or network needed
  (`vcs-testkit`).

## Start here

Pick the crate that matches your task — each links to its guide:

- **Control a repo, git *or* jj** → [`vcs-core`](crates/core/docs/core.md) (the
  unified `Repo` — the usual starting point).
- **Just one VCS** → [`vcs-git`](crates/git/docs/git.md) or
  [`vcs-jj`](crates/jj/docs/jj.md).
- **A code forge** → [`vcs-forge`](crates/forge/docs/forge.md) for all three, or
  [`vcs-github`](crates/github/docs/github.md) /
  [`vcs-gitlab`](crates/gitlab/docs/gitlab.md) /
  [`vcs-gitea`](crates/gitea/docs/gitea.md) directly.
- **Watch a repo for changes** → [`vcs-watch`](crates/watch/docs/watch.md).
- **Serve operations to an AI agent** → [`vcs-mcp`](crates/mcp/docs/mcp.md).
- **Write tests against a repo** → [`vcs-testkit`](crates/testkit/docs/testkit.md).

## Why

- **No reinvented protocols.** You get exactly the behaviour of the
  `git`/`jj`/`gh`/`glab`/`tea` you already have installed — same config,
  credentials, and version semantics.
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

## When to reach for it (and when not)

vcs-toolkit **shells out to the installed CLIs**, so you get the user's *exact*
`git`/`jj`/`gh` behavior, config, and credentials — plus **Jujutsu and the forges,
which the in-process git libraries don't cover at all**. The trade is a process
spawn per command. For high-throughput object-database work (blame over a huge
history, walking millions of commits), reach for an in-process library instead.

| | **vcs-toolkit** | [`gitoxide`](https://crates.io/crates/gix) | [`git2`](https://crates.io/crates/git2) (`libgit2`) | [`octocrab`](https://crates.io/crates/octocrab) |
|---|---|---|---|---|
| Model | subprocess to the CLIs | in-process, pure Rust | in-process, C bindings | in-process HTTP client |
| Honors user config / credentials / hooks | **yes — it *is* their binary** | no | no | token only |
| Covers **Jujutsu (`jj`)** | **yes** | no | no | no |
| Covers **GitHub / GitLab / Gitea** | **yes — one API** | no | no | GitHub only |
| Per-operation cost | a process spawn | none — in-process | none — in-process | one HTTP call |
| Fidelity to the tool | exact CLI behavior | reimplementation | reimplementation | REST/GraphQL |

They compose: use an in-process library on the **hot path** (bulk object reads),
and vcs-toolkit for **workflow automation** and **anything `jj` or a forge
touches**. Full breakdown: the [positioning guide](crates/core/docs/positioning.md).

## Crates

This is a Cargo workspace, each crate **versioned and published independently**:
five CLI wrappers built on the external
[`processkit`](https://crates.io/crates/processkit) crate, two facades (over the
git/jj pair and over the three forges), a filesystem-watch crate emitting typed
repo events, an MCP server exposing the facades to agent harnesses, two
foundational crates the wrappers share, and a dependency-free test-fixture crate:

| Crate | Drives | crates.io name |
|---|---|---|
| [`crates/git`](crates/git) | the `git` binary | `vcs-git` |
| [`crates/jj`](crates/jj) | the `jj` (Jujutsu) binary | `vcs-jj` |
| [`crates/github`](crates/github) | the `gh` (GitHub CLI) binary | `vcs-github` |
| [`crates/gitlab`](crates/gitlab) | the `glab` (GitLab CLI) binary | `vcs-gitlab` |
| [`crates/gitea`](crates/gitea) | the `tea` (Gitea CLI) binary | `vcs-gitea` |
| [`crates/forge`](crates/forge) | — (facade over `vcs-github`/`vcs-gitlab`/`vcs-gitea`) | `vcs-forge` |
| [`crates/core`](crates/core) | — (facade over `vcs-git`/`vcs-jj`) | `vcs-core` |
| [`crates/watch`](crates/watch) | — (filesystem-watch repo events, on `vcs-core`) | `vcs-watch` |
| [`crates/mcp`](crates/mcp) | — (MCP server over `vcs-core`/`vcs-forge`, on `rmcp` + tokio) | `vcs-mcp` |
| [`crates/diff`](crates/diff) | — (shared std-only diff model + parser, `Version`) | `vcs-diff` |
| [`crates/cli-support`](crates/cli-support) | — (shared argv guard, fetch policy, error classifiers) | `vcs-cli-support` |
| [`crates/testkit`](crates/testkit) | — (test fixtures: git/jj sandboxes, bare remote) | `vcs-testkit` |

`vcs-diff` and `vcs-cli-support` are foundational: `vcs-git`/`vcs-jj`/`vcs-github`/
`vcs-core` depend on one or both and re-export their types (so `vcs_git::FileDiff`,
`vcs_git::is_merge_conflict`, … still resolve — `vcs-github` uses only
`vcs-cli-support`, as the `gh` wrapper does no diff parsing), since `git diff` and
`jj diff --git` are byte-identical for ASCII paths (they differ only in
non-ASCII filename rendering — git octal-C-quotes, jj writes raw UTF-8 — both of
which the shared parser decodes), so the wrappers share one parser/guard.

Each **CLI wrapper** exposes an **interface trait** (`GitApi`/`JjApi`/`GitHubApi`/
`GitLabApi`/`GiteaApi`) and a real client (`Git`/`Jj`/`GitHub`/`GitLab`/`Gitea`)
with typed, repo-scoped async commands that return parsed structs and fail with
the structured `processkit::Error`. They build on `processkit` (its `CliClient`
core, the `cli_client!` macro, the `ProcessRunner` seam) and depend on
`async-trait`; the forge wrappers add `serde`/`serde_json` to deserialize the
CLIs' `--json` output. **Two facades** unify a family behind one handle:
`vcs-core` over git/jj (a `Repo`), and `vcs-forge` over the three forges (a
`Forge`).

### Process containment

`processkit` launches every child inside an OS job so kill-on-close holds — the
mechanism is platform-specific and observable at runtime via its `Mechanism`:

| Platform | Mechanism | Kill-on-close |
|---|---|---|
| Windows | [Job Object](https://learn.microsoft.com/windows/win32/procthread/job-objects) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | ✅ whole tree |
| Linux | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) via `cgroup.kill`, with a POSIX **process-group** fallback when no writable cgroup is available | ✅ whole tree (cgroup) / ✅ process group (fallback) |
| macOS, BSD (and other Unix) | POSIX **process group** (`killpg` on drop) — the same backend Linux falls back to | ✅ whole tree (process group) |

v1 guarantees kill-on-close; resource limits are intentionally out of scope.

## Documentation

This README is the overview. For depth — every command grouped by theme, the
parsed result types, the builder and validating-newtype APIs, and worked
examples — see the **[guide set in `docs/`](docs/README.md)**:

- Per-crate references: [vcs-git](crates/git/docs/git.md) · [vcs-jj](crates/jj/docs/jj.md) ·
  [vcs-github](crates/github/docs/github.md) · [vcs-gitlab](crates/gitlab/docs/gitlab.md) ·
  [vcs-gitea](crates/gitea/docs/gitea.md) · [vcs-core](crates/core/docs/core.md) (the git/jj facade) ·
  [vcs-forge](crates/forge/docs/forge.md) (the forge facade) ·
  [vcs-watch](crates/watch/docs/watch.md) (repo-event stream) ·
  [vcs-mcp](crates/mcp/docs/mcp.md) (the MCP server) ·
  [vcs-testkit](crates/testkit/docs/testkit.md) (fixtures).
- Cross-cutting topics: [Conflict resolution](crates/git/docs/conflicts.md) ·
  [Testing & mocking](crates/testkit/docs/testing.md) · [Security & hardening](crates/git/docs/security.md) ·
  [Process model, errors & observability](crates/core/docs/process-model.md).

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

    let branch = git.current_branch(repo).await?; // Option<String> (None if detached)
    let status = git.status(repo).await?; // Vec<StatusEntry>
    let log = git.log(repo, "HEAD", 5).await?; // Vec<Commit>, newest first

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

Each client is a typed async API over its binary; its guide lists every method
with the parsed return type. In brief:

- **`vcs-git` (`GitApi`)** — status, branches, commit/checkout, diff & log,
  merge/rebase/reset, worktrees, tags, blame, clone, config, cherry-pick/revert,
  conflict parsing/resolution, and a hardened (hooks-off) mode for untrusted repos.
  → [full reference](crates/git/docs/git.md)
- **`vcs-jj` (`JjApi`)** — changes & descriptions, bookmarks, the operation log
  (restore/undo), workspaces, squash/split/absorb/duplicate/abandon, diff & template
  queries, git sync (fetch/push/clone/import), conflict parsing/resolution, and
  op-log-rollback transactions.
  → [full reference](crates/jj/docs/jj.md)
- **`vcs-github` (`GitHubApi`)** — auth, repo view, the full PR lifecycle
  (list/view/create/merge/ready/close, review/comment, checks, feedback), issues,
  releases, and GitHub Actions runs (list/view/watch).
  → [full reference](crates/github/docs/github.md)
- **`vcs-gitlab` (`GitLabApi`)** / **`vcs-gitea` (`GiteaApi`)** — the lean MR/PR
  lifecycle plus issues and releases, via `glab` / `tea`.
  → [gitlab](crates/gitlab/docs/gitlab.md) · [gitea](crates/gitea/docs/gitea.md)

Every client also has a `run(args)` / `run_raw(args)` escape hatch — drop to a raw
command when something isn't modelled yet — plus `version()`.

## Recipes

A few inline snippets below; the full collection — a prompt line in one call,
open-a-PR-and-watch-CI, stash-safe switch, programmatic conflict resolution,
backend dispatch — is in the **[Cookbook](crates/core/docs/cookbook.md)**.

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
use vcs_github::{GitHub, GitHubApi, PrCreate};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let gh = GitHub::new();
    if gh.auth_status().await? {
        // .head()/.base() optional: omitted = current branch / repo default.
        let url = gh.pr_create(repo, PrCreate::new("My change", "Body")).await?;
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
async fn current(git: &dyn GitApi) -> Result<Option<String>, processkit::Error> {
    git.current_branch(Path::new(".")).await // None on a detached HEAD
}

let git = Git::new(); // real, job-backed git
// current(&git).await ...
```

- **Mock the interface** — enable the `mock` feature; `mockall` generates
  `MockGitApi` for stubbing whole methods (`expect_current_branch().returning(…)`).
  A consumer enables it only under `[dev-dependencies]`, so `mockall` never lands
  in a release build.
- **Inject a runner** — `Git::with_runner(processkit::testing::ScriptedRunner::new()…)`
  feeds canned binary output through the *real* argument-building and parsing, so
  a test exercises the actual command wiring without spawning anything. Wrap it in
  a `processkit::testing::RecordingRunner` to assert the exact command that was built — full
  args, cwd, env, and even that a flag is *absent*:

  ```rust
  use processkit::testing::{Reply, ScriptedRunner};
  use std::path::Path;
  use vcs_git::{Git, GitApi};

  # async fn demo() {
      let git = Git::with_runner(ScriptedRunner::new().on(["git", "status"], Reply::ok(" M src/lib.rs\0")));
      let entries = git.status(Path::new(".")).await.unwrap();
      assert_eq!(entries[0].code, " M");
  # }
  ```

For building integration-test scenarios, the [`vcs-testkit`](crates/testkit)
crate (a dev-dependency) provides throwaway `GitSandbox`/`JjSandbox` repos, a
seeded `BareRemote` to clone/fetch against, and a self-cleaning `TempDir` — the
same fixtures this workspace's own ignored tests run on.

→ Full guide: **[Testing & mocking](crates/testkit/docs/testing.md)** and the
**[vcs-testkit fixtures](crates/testkit/docs/testkit.md)**.

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
  disables hooks, `core.fsmonitor`, and a repo-local `core.sshCommand`, scrubs
  repo-redirecting `GIT_*` variables *and* the env-based command hooks that make
  git spawn an arbitrary program (`GIT_SSH_COMMAND`/`GIT_ASKPASS`/
  `GIT_EXTERNAL_DIFF`/…), skips system config, and keeps prompts off — on every
  command the client runs. (It is hardening, not a sandbox: repo-local
  `filter`/`textconv` keys survive, so don't checkout/diff a *fully* untrusted
  repo without an OS sandbox — see the security guide.) jj needs no equivalent
  (no repo-local hooks); in a colocated repo, harden the `Git` client you point at it.

A related opt-in is **supplying** a secret rather than guarding against one:
`Git`/`GitHub`/`GitLab` accept a `CredentialProvider` via `with_credentials(...)`,
injecting a per-operation token (a git HTTPS `credential.helper`, or
`GH_TOKEN`/`GITLAB_TOKEN`) that is wrapped in a self-redacting `Secret`, kept out of
`argv`, and never persisted to a credential store. Off by default → ambient CLI
auth, unchanged.

Conflicted files parse into a typed model too: `vcs_git::conflict` /
`vcs_jj::conflict` turn marker soup into structured regions
(ours/base/theirs; jj's `diff` and `snapshot` styles) with a byte-exact
`render` and a `resolve(side)` writer — the primitive for programmatic
conflict resolution.

→ Full guides: **[Security & hardening](crates/git/docs/security.md)** and
**[Conflict resolution](crates/git/docs/conflicts.md)**.

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

→ Full guide: **[Process model, errors & observability](crates/core/docs/process-model.md)**.

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
`#[ignore]` so CI stays hermetic; run them locally with `--ignored`. The pure
parsers (status/diff/blame, the operation and conflict models) are additionally
**property-tested** with `proptest` for panic-freedom on arbitrary input and a
byte-exact `render(parse(x)) == x` conflict roundtrip — these run in the normal
`cargo test` gate. CI (`.github/workflows/ci.yml`) runs fmt, clippy (with and
without `mock`), the test suite on Linux/Windows/macOS, `cargo-deny`, a
`cargo package` gate, and an `integration` job that installs several **jj
versions** (oldest supported … latest) plus an older-git runner image and runs
the `--ignored` suites against each, so CLI/template drift in the parsers is
caught before users hit it. A separate weekly, non-gating
**[scheduled drift lane](.github/workflows/scheduled-cli-drift.yml)** re-runs the
`--ignored` suites against the *actual latest* jj/glab/tea, and stands up a
one-shot **live Gitea** to exercise the real create → merge PR lifecycle (plus
issues/releases) end-to-end through `vcs-forge`/`vcs-gitea` — the forge coverage
hermetic fixtures can't give — reporting any drift as a tracking issue rather
than failing a PR.

## Publishing

Releases go through the **`Release` GitHub Action** (`workflow_dispatch`) — you
never type a version. Click *Run workflow* and pick:

- **Crate** — `vcs-diff`, `vcs-cli-support`, `vcs-git`, `vcs-jj`, `vcs-github`,
  `vcs-gitlab`, `vcs-gitea`, `vcs-forge`, `vcs-testkit`, `vcs-core`, `vcs-watch`,
  `vcs-mcp`, or **`all`** (release every crate in one run).
- **Bump** — `patch` / `minor` / `major`.

For each selected crate it reads the current version from that crate's
`Cargo.toml`, computes the next one (a crate's **first release** — no
`<crate>-v*` tag yet — ships the current version as-is, ignoring the bump),
promotes its `CHANGELOG.md`, **publishes to crates.io before tagging**
`<crate>-v<version>`, and opens a GitHub Release from the curated notes. `all`
does them in a single commit + atomic push.

The dependency layers drive the publish order. The two foundational crates
(`vcs-diff` — std-only — and `vcs-cli-support`, which depends only on the
already-published [`processkit`](https://crates.io/crates/processkit)) publish
**first**; the CLI wrappers depend on them (plus `processkit`), so they publish
next; the **facades** come after — `vcs-forge` (depends on the github/gitlab/gitea
wrappers) and `vcs-core` (depends on `vcs-git`/`vcs-jj`) — then `vcs-watch` (on
`vcs-core`) and `vcs-mcp` (on both `vcs-core` and `vcs-forge`) **last**.
`vcs-testkit` depends on nothing and can go anywhere. So `all` releases in that
order, and each `^MAJOR.MINOR` requirement on an in-workspace dependency must stay
in range when that dependency crosses a minor/major boundary.

## Conventions

See [CONTRIBUTING.md](CONTRIBUTING.md) for building, testing, the dependency policy
(every dependency gets a "why" comment), the per-crate changelog process, and the
release workflow.

## License

Licensed under the [MIT License](LICENSE).
