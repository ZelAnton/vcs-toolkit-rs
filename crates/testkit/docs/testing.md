# Testing & mocking guide

These wrappers are testable by construction: consumers depend on an interface
trait, not the concrete client, and most tests never touch a real binary, a temp
repo, or the network. There are **three seams**, each answering a different
question — pick the cheapest one that proves what you need.

| Seam | Substitutes | Proves |
|---|---|---|
| [Depend on the trait](#1-depend-on-the-trait) | a hand-written fake / mock | *your* code's logic around the client |
| [The `mock` feature](#2-the-mock-feature) | whole methods (`mockall`) | logic given a method's return, ignoring argv |
| [Inject a runner](#3-inject-a-runner) | the process, not the method | the real argv-building + output parsing |

A fourth path — real binaries through [vcs-testkit](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/) fixtures — is for
the handful of end-to-end checks the seams above can't cover; see
[below](#integration-tests-with-real-binaries).

---

## 1. Depend on the trait

Production code takes `&dyn GitApi` / `&dyn JjApi` / `&dyn GitHubApi` (or
`&dyn VcsRepo` for the backend-agnostic facade), never the concrete `Git` /
`Jj` / `GitHub` / `Repo`. The real client implements the trait; a test swaps in
whatever stand-in is convenient. This is the seam that makes the other two
possible.

```rust,ignore
use std::path::Path;
use vcs_git::{Git, GitApi};

// Production code depends on the interface, not the concrete client:
async fn current(git: &dyn GitApi) -> Result<String, processkit::Error> {
    git.current_branch(Path::new(".")).await
}

let git = Git::new();   // real, job-backed git — passes as `&dyn GitApi`
// current(&git).await ...
```

`vcs-core`'s `VcsRepo` works the same, so a consumer can hold a
`Box<dyn VcsRepo>` / `&dyn VcsRepo` and code against the common git/jj surface
without naming the `ProcessRunner` generic. Note the facade traits (`VcsRepo`
and `vcs-forge`'s `ForgeApi`) deliberately have **no** `mock` feature — unlike
the wrapper traits in §2 — so test them with the runner seam (§3): build a
`Repo`/`Forge` over a fake runner via `Repo::from_git` / `Forge::for_github`
rather than mocking the trait.

---

## 2. The `mock` feature

Enable the `mock` feature and `mockall` generates `MockGitApi` / `MockJjApi` /
`MockGitHubApi`, letting you stub *whole methods* — `expect_<method>()` then
`.returning(…)`. Reach for this when the test cares about a return value, not the
command that produced it.

The mock is gated so it never reaches production: each crate's `Cargo.toml`
declares `mockall` as an *optional* dependency, and the `mock` feature turns it
on (plus processkit's own mocks). A consumer enables the feature only under
`[dev-dependencies]`, so `mockall` is stripped from release builds.

```toml
# A consumer's Cargo.toml — mock lives strictly in dev-deps.
[dependencies]
vcs-git = "0.4"

[dev-dependencies]
vcs-git = { version = "0.4", features = ["mock"] }
```

```toml
# vcs-git's own Cargo.toml — how the feature is wired:
[dependencies]
mockall = { version = "0.13", optional = true }

[features]
# Expose the `mockall`-generated `MockGitApi` (and pull in processkit's mocks).
mock = ["dep:mockall", "processkit/mock"]
```

The trait is annotated `#[cfg_attr(feature = "mock", mockall::automock)]`, so the
mock only exists under the feature. Stub a method and drive code that depends on
the trait:

```rust,ignore
use std::path::Path;
use vcs_git::{GitApi, MockGitApi};

#[cfg(feature = "mock")]
#[tokio::test]
async fn consumer_mocks_the_interface() {
    async fn on_branch(git: &dyn GitApi, want: &str) -> bool {
        git.current_branch(Path::new(".")).await.unwrap() == want
    }
    let mut mock = MockGitApi::new();
    mock.expect_current_branch()
        .returning(|_| Ok("main".to_string()));
    assert!(on_branch(&mock, "main").await);
}
```

`MockGitHubApi` is identical in shape — `expect_auth_status().returning(|| Ok(true))`
and so on. The mock substitutes the *method*, so it proves nothing about the argv
the real client would build; for that, use the runner seam.

---

## 3. Inject a runner

`Git::with_runner(…)` (and `Jj::`/`GitHub::with_runner`) takes a
`processkit::ProcessRunner` and feeds its canned output through the **real**
argument-building and parsing — so a test exercises the actual command wiring
without spawning anything. `ScriptedRunner::new().on([program, args…], reply)`
matches a call when the argv contains that prefix — which now **leads with the
program name** (e.g. `["git", "status"]`) — and replies with a `Reply`.

`Reply` constructors seen in this repo:

- `Reply::ok(stdout)` — exit 0 with this stdout.
- `Reply::fail(code, stderr)` — non-zero exit with this stderr.
- `Reply::timeout()` — surfaces as `processkit::Error::Timeout`.
- `Reply::fail(code, stderr).with_stdout(json)` — a non-zero exit that *also*
  carried stdout (e.g. `gh pr checks` reporting failures as JSON on a failing
  exit).

### git

```rust,ignore
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_git::{Git, GitApi};

# async fn demo() {
    // Real status() command-building + porcelain parsing against canned `-z` output.
    let git = Git::with_runner(
        ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0?? b.rs\0")),
    );
    let entries = git.status(Path::new(".")).await.unwrap();
    assert_eq!(entries[0].code, " M");
    assert_eq!(entries[1].path, "b.rs");
# }
```

### jj

```rust,ignore
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_jj::{Jj, JjApi};

# async fn demo() {
    // The tab-separated log template parses into a `Change`.
    let jj = Jj::with_runner(
        ScriptedRunner::new().on(["jj", "log"], Reply::ok("kztuxlro\t38e00654\tfalse\thello\n")),
    );
    assert_eq!(
        jj.current_change(Path::new(".")).await.unwrap().description,
        "hello",
    );
# }
```

### github

```rust,ignore
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};

# async fn demo() {
    let json = r#"[{"number":7,"title":"Add X","state":"OPEN"}]"#;
    let gh = GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)),
    );
    assert_eq!(gh.pr_list(Path::new(".")).await.unwrap()[0].number, 7);
# }
```

### Asserting the exact command with `RecordingRunner`

Wrap the runner in a `RecordingRunner` to capture every invocation, then assert
the exact argv, cwd, and env that was built — including that a flag is **absent**.
`RecordingRunner::replying(reply)` answers everything with one `Reply` while
recording; `RecordingRunner::new(inner)` records in front of another runner (e.g.
a `ScriptedRunner` with per-call rules). Read `rec.calls()` (a slice of
invocations) or `rec.only_call()` when exactly one is expected; each invocation
exposes `args_str()`, `cwd`, and `envs`.

```rust,ignore
use processkit::testing::{RecordingRunner, Reply};
use std::path::Path;
use vcs_github::{GitHub, GitHubApi, PrCreate};

# async fn demo() {
    let rec = RecordingRunner::replying(Reply::ok("https://gh/pr/9\n"));
    let gh = GitHub::with_runner(&rec);

    // Without a base, pr_create must omit `--base` entirely.
    gh.pr_create(Path::new("/repo"), PrCreate::new("T", "B")).await.unwrap();

    assert_eq!(
        rec.only_call().args_str(),
        ["pr", "create", "--title", "T", "--body", "B"], // no `--base`, no `--head`
    );
# }
```

The same applies to env and cwd. A git example asserting a flag's effect, the
exact ref, and an injected environment variable:

```rust,ignore
use processkit::testing::{RecordingRunner, Reply};
use std::path::Path;
use vcs_git::{Git, GitApi};

# async fn demo() {
    let rec = RecordingRunner::replying(Reply::ok("abc123\trefs/heads/main\n"));
    let git = Git::with_runner(&rec);
    assert!(git.remote_branch_exists(Path::new("/repo"), "main").await.unwrap());

    let call = rec.only_call();
    // Exact-ref query — a bare `main` would tail-match `bar/main`.
    assert_eq!(call.args_str(), ["ls-remote", "origin", "refs/heads/main"]);
    // The non-interactive guard was injected into the environment.
    assert!(call.envs.iter().any(|(k, v)| {
        k.to_str() == Some("GIT_TERMINAL_PROMPT")
            && v.as_deref().and_then(|o| o.to_str()) == Some("0")
    }));
# }
```

`vcs-core`'s `Repo` is generic over `ProcessRunner` too: build one from an
explicit client with `Repo::from_git("/repo", "/repo", Git::with_runner(runner))`
/ `Repo::from_jj(…)` to test the facade's dispatch hermetically, exactly as the
underlying crates do.

### Cancellation, hermetically

`Reply::pending()` (always available) parks a matched call until the
command's token fires (a per-command `cancel_on` or a client `default_cancel_on`),
then resolves `Err(Error::Cancelled)`. That tests the cancellation *behaviour* — the
call really parks, then really unwinds — with no real binary. Run it on a paused
clock (`#[tokio::test(start_paused = true)]`): a long `time::timeout` elapses
instantly while the call is parked, proving it doesn't resolve early.

```rust,ignore
use processkit::CancellationToken;
use processkit::testing::{Reply, ScriptedRunner};
use vcs_github::{GitHub, GitHubApi};

#[tokio::test(start_paused = true)]
async fn run_watch_cancels() {
    let token = CancellationToken::new();
    let gh = GitHub::with_runner(ScriptedRunner::new().on(["gh", "run", "watch"], Reply::pending()))
        .default_cancel_on(token.clone());
    let call = gh.run_watch(std::path::Path::new("."), 42);
    tokio::pin!(call);
    // Parked: a 1 h timeout elapses (paused clock) without the call resolving.
    assert!(tokio::time::timeout(std::time::Duration::from_secs(3600), &mut call).await.is_err());
    token.cancel();
    assert!(matches!(call.await.unwrap_err(), processkit::Error::Cancelled { .. }));
}
```

A pending reply for a command with **no** token parks forever (a hung child nobody
can cancel) — always pair it with a token or a test timeout. `Cancelled` is
terminal, so a retried op (e.g. `fetch`) records exactly one spawn, not a replay.

---

## Dry-run harness

To exercise a *whole flow* without a repository, give the scripted runner a
catch-all: `ScriptedRunner::new().fallback(Reply::ok(""))` answers every call
with empty success and executes nothing. Add `.on(…)` rules for the specific
calls that need a realistic reply (a branch name, a JSON blob, a non-zero exit);
everything else falls through to the fallback.

```rust,ignore
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_git::{Git, GitApi};

# async fn demo() {
    let git = Git::with_runner(
        ScriptedRunner::new()
            .fallback(Reply::ok(""))                          // answers everything…
            .on(["git", "rev-parse"], Reply::ok("feature\n")), // …except the calls that matter
    );
    // The whole sequence runs; only the branch query gets a meaningful answer.
    assert_eq!(git.current_branch(Path::new(".")).await.unwrap(), "feature");
    git.add(Path::new("."), &[]).await.unwrap();           // satisfied by the fallback
    git.commit(Path::new("."), "snapshot").await.unwrap(); // satisfied by the fallback
# }
```

Pair it with a `RecordingRunner` (`RecordingRunner::new(scripted)`) when you also
want to assert *which* commands the flow would have run.

> **What about processkit's `record` cassettes?** processkit 0.7 ships a
> `RecordReplayRunner` (behind the off-by-default `record` feature) that records
> real runs to JSON and replays them without subprocesses. This workspace
> deliberately doesn't use it: the replay key includes the **cwd**, so
> throwaway-temp-repo scenarios never match a recorded cassette; and a cassette
> freezes the CLI's output at record time — masking exactly the CLI drift the
> `#[ignore]` real-binary suites below exist to catch. Consumers with stable
> working directories (e.g. `gh api` flows) may find cassettes a good fit.

---

## Integration tests with real binaries

When a check genuinely needs the real `git` / `jj` / `gh` — output formats,
version-specific behaviour, a true push/fetch round-trip — build the scenario
with [vcs-testkit](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/) fixtures and gate it behind `#[ignore]` so a
hermetic CI without the binaries stays green. Run them locally (or in the
binary-equipped CI lane) with `cargo test -- --ignored`.

```rust,ignore
use vcs_testkit::{BareRemote, GitSandbox};

#[test]
#[ignore = "requires the git binary"]
fn fetch_brings_back_the_seed() {
    let repo = GitSandbox::init("e2e");
    repo.commit_file("a.txt", "one\n", "first");

    let remote = BareRemote::seeded("remote");
    repo.git(&["remote", "add", "origin", remote.url().as_str()]);
    repo.git(&["fetch", "-q", "origin"]);

    assert_eq!(repo.rev_parse("origin/main").len(), 40);
}
```

CI runs the `--ignored` suites against a **matrix of jj versions** (oldest
supported … latest) plus an older-git image, so CLI/template drift is caught in
the parsers before users hit it — see the [workspace README](https://github.com/ZelAnton/vcs-toolkit-rs#readme).

---

See also: [vcs-testkit fixtures](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/),
[Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/), and the per-crate guides
[git](https://docs.rs/vcs-git/latest/vcs_git/guide/) / [jj](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) / [github](https://docs.rs/vcs-github/latest/vcs_github/guide/).
