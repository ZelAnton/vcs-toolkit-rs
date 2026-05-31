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
orphaned subprocesses. That shared mechanism lives in `vcs-process`, which also
provides timeouts and the structured `CommandError`.

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
  parse error are distinct `CommandError` variants carrying program, args, exit
  code, and stderr — not a stringly-typed blob.
- **Async with deadlines.** Every call is a future; an optional per-client or
  per-call timeout kills the job (and the whole tree) when it elapses.

## Crates

This is a Cargo workspace of four crates, each **versioned and published
independently**:

| Crate | Drives | crates.io name |
|---|---|---|
| [`crates/process`](crates/process) | the job-backed process launcher (shared) | `vcs-process` |
| [`crates/git`](crates/git) | the `git` binary | `vcs-git` |
| [`crates/jj`](crates/jj) | the `jj` (Jujutsu) binary | `vcs-jj` |
| [`crates/github`](crates/github) | the `gh` (GitHub CLI) binary | `vcs-github` |

Each wrapper exposes an **interface trait** (`GitApi`/`JjApi`/`GitHubApi`) and a
real client (`Git`/`Jj`/`GitHub`) with typed, repo-scoped async commands that
return parsed structs and fail with the structured `CommandError`. They delegate
process launching to `vcs-process` and depend on `async-trait`; `vcs-github`
additionally adds `serde`/`serde_json` to deserialize `gh … --json` output.

### Process containment

`vcs-process` launches every child inside an OS job so kill-on-close holds — the
mechanism is platform-specific and observable at runtime via `Job::mechanism()`:

| Platform | Mechanism | Kill-on-close |
|---|---|---|
| Windows | [Job Object](https://learn.microsoft.com/windows/win32/procthread/job-objects) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | ✅ whole tree |
| Linux | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) via `cgroup.kill`, with a POSIX **process-group** fallback when no writable cgroup is available | ✅ whole tree (cgroup) / ✅ process group (fallback) |
| other (macOS, BSD) | plain spawn, no containment | ⚠️ best-effort — direct child only (`kill_on_drop`) |

v1 guarantees kill-on-close; resource limits are intentionally out of scope.

## Quick start

Add the wrapper(s) you need. Every method is `async`, so call them from a tokio
runtime:

```rust
use std::path::Path;
use std::time::Duration;
use vcs_git::{Git, GitApi};
use vcs_process::CommandError;

#[tokio::main]
async fn main() -> Result<(), CommandError> {
    // A real, job-backed client; give every command a 10s deadline.
    let git = Git::new().default_timeout(Duration::from_secs(10));
    let repo = Path::new(".");

    let branch = git.current_branch(repo).await?;        // String
    let status = git.status(repo).await?;                // Vec<StatusEntry>
    let log    = git.log(repo, 5).await?;                // Vec<Commit>, newest first

    println!("on {branch}: {} change(s), HEAD = {}", status.len(), log[0].short_hash);

    // Distinguish failure modes structurally instead of matching on strings.
    match git.checkout(repo, "does-not-exist").await {
        Err(CommandError::Exit { code, stderr, .. }) => {
            eprintln!("git exited {code}: {stderr}");
        }
        Err(CommandError::Timeout { .. }) => eprintln!("git timed out"),
        other => { other?; }
    }
    Ok(())
}
```

`vcs-jj` and `vcs-github` follow the same shape:

```rust
use std::path::Path;
use vcs_jj::{Jj, JjApi};
use vcs_github::{GitHub, GitHubApi};

# async fn demo() -> Result<(), vcs_process::CommandError> {
let jj = Jj::new();
let head = jj.current_change(Path::new(".")).await?;      // Change
jj.describe(Path::new("."), "wip: refactor").await?;

let gh = GitHub::new();
if gh.auth_status().await? {                              // bool, never errors on exit code
    let prs = gh.pr_list(Path::new(".")).await?;          // Vec<PullRequest>
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
| `diff_is_empty` → `bool` | | |

## Built for testing

Consumers code against the trait and substitute a fake in their tests — two seams,
neither of which needs the real binary, a temp repo, or the network:

```rust
use vcs_git::{Git, GitApi};
use std::path::Path;

// Production code depends on the interface, not the concrete client:
async fn current(git: &dyn GitApi) -> Result<String, vcs_process::CommandError> {
    git.current_branch(Path::new(".")).await
}

let git = Git::new();              // real, job-backed git
// current(&git).await ...
```

- **Mock the interface** — enable the `mock` feature; `mockall` generates
  `MockGitApi` for stubbing whole methods (`expect_current_branch().returning(…)`).
  A consumer enables it only under `[dev-dependencies]`, so `mockall` never lands
  in a release build.
- **Inject a runner** — `Git::with_runner(vcs_process::ScriptedRunner::new()…)`
  feeds canned binary output through the *real* argument-building and parsing, so
  a test exercises the actual command wiring without spawning anything. Wrap it in
  a `RecordingRunner` to assert the exact command that was built — full args, cwd,
  env, and even that a flag is *absent*:

  ```rust
  use vcs_git::{Git, GitApi};
  use vcs_process::{Output, ScriptedRunner};
  use std::path::Path;

  # async fn demo() {
  let git = Git::with_runner(
      ScriptedRunner::new().on(["status", "--porcelain"], Output::ok(" M src/lib.rs\n")),
  );
  let entries = git.status(Path::new(".")).await.unwrap();
  assert_eq!(entries[0].code, " M");
  # }
  ```

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

Each crate releases on its own cadence. Bump the `version` in that crate's
`Cargo.toml` (the single source of truth), update its `CHANGELOG.md`, tag as
`<crate>-v<version>` (e.g. `vcs-git-v0.2.0`), then `cargo publish -p <crate>`.
The `Release` GitHub Action (`workflow_dispatch`) automates the bump, changelog
promotion, tag, and publish for a chosen crate.

**Publish order:** `vcs-process` must be on crates.io *before* the wrappers,
since `vcs-git`/`vcs-jj`/`vcs-github` depend on it by version. Release
`vcs-process` first whenever its version changed.

## Conventions

See [AGENTS.md](AGENTS.md) for code style, dependency management (every
dependency gets a "why" comment; no fixed allow-list), the per-crate changelog
process, and the `jj` version-control workflow.

## License

Licensed under the [MIT License](LICENSE).
