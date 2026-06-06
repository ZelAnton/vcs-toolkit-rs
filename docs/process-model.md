# Process model, errors & observability

Every command these wrappers run is an async child process launched through the
external [`processkit`](https://crates.io/crates/processkit) crate. That gives
three things the wrappers lean on and re-export: **OS-job containment** (no leaked
subprocesses), **deadlines** (a timeout kills the whole tree), and a **structured
`Error`** you branch on instead of grepping stderr. This page is the model behind
all three, plus the seams for watching commands go by.

---

## OS-job containment

`processkit` launches every child inside an OS **job** so kill-on-close holds —
when the parent goes away (crash, panic, `Ctrl-C`, a dropped future), the OS
reaps the entire process tree. No orphaned `git gc`, no hung `gh`. The mechanism
is platform-specific:

| Platform | Mechanism | Kill-on-close |
|---|---|---|
| Windows | [Job Object](https://learn.microsoft.com/windows/win32/procthread/job-objects) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` | whole tree |
| Linux | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) via `cgroup.kill`, with a POSIX **process-group** fallback when no writable cgroup is available | whole tree (cgroup) / process group (fallback) |
| macOS, BSD (and other Unix) | POSIX **process group** (`killpg` on drop) — the same backend Linux falls back to | whole tree (process group) |

v1 guarantees **kill-on-close**; resource limits (CPU, memory) are intentionally
out of scope. The mechanism in force is observable at runtime via processkit's
`Mechanism` — the choice is not silent.

## Timeouts

Set a per-client deadline with `default_timeout(Duration)`; every command the
client runs inherits it.

```rust
use std::time::Duration;
use vcs_git::Git;

let git = Git::new().default_timeout(Duration::from_secs(10));
// Every command this client runs gets a 10s deadline.
```

A command that outruns its deadline fails with **`processkit::Error::Timeout {
program, timeout }`**, and the job — the whole process tree — is killed, not just
the top process. `default_timeout` chains with the other builders, so a hardened,
deadlined client is `Git::hardened().default_timeout(…)`.

## The error model

A non-zero exit, a spawn failure, a timeout, and a parse failure are *distinct*
`processkit::Error` variants carrying structured fields — not a stringly-typed
blob. Branch on the variant rather than matching substrings of stderr. The enum
is `#[non_exhaustive]`, so keep a catch-all arm. The variants:

- **`Exit { program: String, code: i32, stdout: String, stderr: String }`** — ran
  to completion, exited non-zero. Both streams are captured (each truncated to
  4 KiB) because `git`/`jj` write decisive diagnostics to **stdout** on failure
  (`CONFLICT (content): …`, `nothing to commit, working tree clean`). Raised by
  the `ensure_success` path; a bare non-zero exit is otherwise *not* treated as
  an error (see `run_raw` below).
- **`Timeout { program, timeout }`** — exceeded its deadline and was killed.
- **`Spawn { program, source }`** — the child could not be started (binary not
  found, permission denied) — *and* the variant the [injection
  guards](security.md#injection-guards-automatic) raise for a flag-shaped
  positional argument, before any spawn.
- **`Parse { program, message }`** — the process succeeded but its output didn't
  match the expected shape (e.g. an unrecognisable `--version`, malformed
  `--json`).
- **`Io(std::io::Error)`** — an IO error while driving the process (a pipe, a
  stdin write, waiting for exit).

> There is **no** dedicated signal variant: a child killed by a signal surfaces
> through the exit path / containment, not a separate enum arm.

```rust
use processkit::Error;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &std::path::Path) -> Result<(), Error> {
match git.checkout(repo, "does-not-exist").await {
    Ok(()) => {}
    Err(Error::Exit { code, stderr, .. }) => eprintln!("git exited {code}: {stderr}"),
    Err(Error::Timeout { .. })           => eprintln!("git timed out"),
    Err(Error::Spawn { .. })             => eprintln!("could not start git (or a guarded arg)"),
    other => { other?; } // `#[non_exhaustive]` — keep a fallthrough
}
# Ok(()) }
```

**Exit code as data.** When a non-zero exit is an *answer*, not a failure (e.g.
`gh pr checks` signalling pending via exit 8), reach for `run_raw`: it returns a
`processkit::ProcessResult<String>` and does **not** error on a non-zero exit.
Read the code with its `code()` accessor (`Option<i32>`):

```rust
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
let res = git.run_raw(&["status".into(), "--porcelain".into()]).await?;
println!("exit {:?}", res.code()); // Option<i32> — not flattened to an error
# Ok(()) }
```

## Observing commands

Four seams, no extra configuration:

**(a) Argv observation.** Wrap the *real* runner the same way tests wrap fakes:
`RecordingRunner::new(JobRunner::new())`, hand `&rec` to `with_runner`, and read
`rec.calls()` — the full argv, cwd, and env of every invocation, after the fact.

```rust
use processkit::{JobRunner, RecordingRunner};
use vcs_git::{Git, GitApi};

# async fn demo(repo: &std::path::Path) -> Result<(), processkit::Error> {
let rec = RecordingRunner::new(JobRunner::new()); // records *and* really runs
let git = Git::with_runner(&rec);
git.current_branch(repo).await?;
for call in rec.calls() {
    // full argv, cwd, env per invocation
    let _ = call;
}
# Ok(()) }
```

**(b) Live output streaming — caveat.** `processkit::Command` supports per-line
callbacks (`.on_stdout_line(|l| …)` / `.on_stderr_line(…)`), so a long-running
command built **directly against processkit** can report progress while it runs.
The typed `Git` / `Jj` methods consume their `Command` internally and do **not**
surface the hook yet — streaming wrappers (e.g. a fetch-with-progress) land once
the upstream hardening (callback panic isolation, scripted-replay testability)
ships in processkit.

**(c) The `tracing` feature.** Each crate's `tracing` feature (forwarding to
`processkit/tracing`) makes processkit emit a `debug` event per command run —
program, args, exit — for any `tracing` subscriber. Pure observability; no API
change.

```toml
# Cargo.toml
vcs-git = { version = "…", features = ["tracing"] }
```

**(d) Dry-run harness.** `ScriptedRunner::new().fallback(Reply::ok(""))` executes
nothing and answers everything, so a whole flow can be exercised without touching
a repository; add `.on(…)` rules for the calls that need realistic replies.

```rust
use processkit::{Reply, ScriptedRunner};
use vcs_git::Git;

# fn demo() {
let runner = ScriptedRunner::new()
    .on(["status"], Reply::ok(" M src/lib.rs\0")) // realistic where it matters
    .fallback(Reply::ok(""));                     // everything else: answer, run nothing
let git = Git::with_runner(runner);
let _ = git;
# }
```

## See also

- [Testing & mocking](testing.md) — the runner seams in full (trait, `mock`
  feature, scripted/recording runners) and the real-binary fixtures.
- [Security & hardening](security.md) — the injection guards behind `Error::Spawn`
  and the untrusted-repo profile.
- Per-crate guides: [git](git.md), [jj](jj.md), [github](github.md), [core](core.md).
