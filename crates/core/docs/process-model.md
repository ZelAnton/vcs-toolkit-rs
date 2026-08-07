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

```rust,ignore
use std::time::Duration;
use vcs_git::Git;

let git = Git::new().default_timeout(Duration::from_secs(10));
// Every command this client runs gets a 10s deadline.
```

A command that outruns its deadline fails with **`processkit::ErrorReason::Timeout {
program, timeout }`**, and the job — the whole process tree — is eventually killed,
not just the top process. Network commands additionally use the shared
`vcs_cli_support::apply_fetch_completion_policy` policy: a two-second grace window
(`FETCH_TIMEOUT_GRACE`) is retained on every platform, and the processkit hard-kill
fallback remains the final containment boundary.

The soft tier is platform-specific:

- On Unix, processkit sends its graceful terminate signal, waits through the grace
  window, and then sends the hard kill if the tree has not exited.
- On Windows, processkit has no POSIX signal tier. It first tries `WM_CLOSE` for
  top-level windows and sends console `CTRL_BREAK` to direct children opted in by
  `windows_graceful_ctrl_break`; a child that handles the event can flush buffers,
  close its connection, and release locks before the grace expires. A console is
  required for delivery, so a GUI/service caller without one and a child created
  with `create_no_window` or `DETACHED_PROCESS` receive no `CTRL_BREAK`; survivors
  are still terminated by the Job Object.

The helper only configures the completion policy; it does not set a deadline. It
therefore has no effect unless `default_timeout` (or a per-command timeout) is set.
`default_timeout` chains with the other builders, so a hardened, deadlined client
is `Git::hardened().default_timeout(…)`.

The hermetic wrapper tests assert the built grace/soft-trigger shape and drive the
full streamed `events()` → `finish()` lifecycle, including cancellation observed
after the child exits but before the finisher's first exit observation. They do not
claim to prove Windows console delivery: a normal `cargo test` process does not
guarantee a shared console or a child-side `CTRL_BREAK` handler, so this workspace
does not include an ignored probe that would only simulate that boundary.

## The error model

A non-zero exit, a spawn failure, a timeout, and a parse failure are *distinct*
structured failures carrying typed fields — not a stringly-typed blob. Branch on
the failure kind rather than matching substrings of stderr.

**Where the variants live (processkit 3.0).** `processkit::Error` itself is an
opaque, pointer-sized wrapper around a boxed **`processkit::ErrorReason`** — the
variant enum below. That keeps every `Result<T, Error>` on the run path small,
including a facade enum that embeds one (`vcs_core::Error::Vcs`,
`vcs_forge::Error::Forge`). Reading an error needs no unwrapping: `code()`,
`program()`, `stdout()`/`stderr()`/`stdout_bytes()`, `diagnostic()`, `combined()`,
the `is_*()` predicates, `signal()`, `Display`, `Debug` and `source()` all work on
`Error` directly. Only a **variant match** goes through the reason:

- `err.reason() -> &ErrorReason` — borrow, the usual case;
- `err.into_reason() -> ErrorReason` — take ownership, when a captured stream or
  the owned `io::Error` must be moved out;
- `err.kind() -> ErrorKind` — a flat classifier (`not_found`, `permission_denied`,
  `timeout`, `exit`, `signalled`, `cancelled`, `unsupported`, `spawn`, `predicate`,
  `other`, each with a stable `name()`), when a coarse bucket is all you need.

Every wrapper crate re-exports `ErrorReason` and `ErrorKind` next to `Error`
(`vcs_git::ErrorReason`, `vcs_jj::ErrorKind`, …), and `vcs-core`/`vcs-forge`/
`vcs-watch` re-export the whole `processkit` crate — so classifying a failure never
needs a direct `processkit` dependency.

`ErrorReason` is `#[non_exhaustive]`, so keep a catch-all arm. The variants:

- **`Exit { program: String, code: i32, stdout: String, stderr: String }`** — ran
  to completion, exited non-zero. Both streams are captured (each truncated to
  4 KiB) because `git`/`jj` write decisive diagnostics to **stdout** on failure
  (`CONFLICT (content): …`, `nothing to commit, working tree clean`). Raised by
  the `ensure_success` path; a bare non-zero exit is otherwise *not* treated as
  an error (see `run_raw` below).
- **`Timeout { program, timeout, stdout, stderr }`** — exceeded its deadline and
  was killed; carries whatever partial output was captured before the deadline
  (processkit 0.10), so the reason a hung step stalled is available here.
- **`Signalled { program, signal, stdout, stderr }`** — killed by a signal
  (external SIGTERM/SIGKILL), carrying the signal number and partial output
  (processkit 0.9.2/0.10). Terminal — the toolkit never auto-retries it.
- **`NotFound { program, searched }`** — the binary isn't installed / isn't on
  `PATH` (processkit 0.10; `is_not_found()` is true only for this). A setup error,
  surfaced by `vcs_core::Error::is_not_found`.
- **`Spawn { program, source }`** — the child could not be started for another
  reason (e.g. permission denied) — *and* the variant the [injection
  guards](https://docs.rs/vcs-git/latest/vcs_git/guide/security/) raise for a flag-shaped
  positional argument, before any spawn.
- **`Parse { program, message }`** — the process succeeded but its output didn't
  match the expected shape (e.g. an unrecognisable `--version`, malformed
  `--json`).
- **`Io(std::io::Error)`** — an IO error while driving the process (a pipe, a
  stdin write, waiting for exit).
- **`NotReady { program, timeout }`** / **`Unsupported { operation }`** — added
  in processkit 0.7 (readiness probes; platform-unsupported operations). The
  wrappers never raise them today, but they can reach you when you drive
  processkit directly. More variants exist behind processkit features
  (`ResourceLimit` under `limits`; `Cancelled` is always available) — one more
  reason the catch-all arm is mandatory. The toolkit's error classifiers treat
  every unfamiliar variant as "no" (not a conflict, not transient).

> A child killed by a signal surfaces as the dedicated **`Signalled`** variant
> (processkit 0.9.2+), carrying the signal number and any partial output — not
> folded into the exit path.

```rust,ignore
use processkit::{Error, ErrorReason};
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &std::path::Path) -> Result<(), Error> {
if let Err(err) = git.checkout(repo, "does-not-exist").await {
    // `into_reason()` takes ownership, so the fallthrough can hand the failure back
    // (`From<ErrorReason> for Error`); `reason()` would borrow instead.
    match err.into_reason() {
        ErrorReason::Exit { code, stderr, .. } => eprintln!("git exited {code}: {stderr}"),
        ErrorReason::Timeout { .. }           => eprintln!("git timed out"),
        ErrorReason::Spawn { .. }             => eprintln!("could not start git (or a guarded arg)"),
        other => return Err(other.into()), // `#[non_exhaustive]` — keep a fallthrough
    }
}
# Ok(()) }
```

Or, when the bucket is enough, skip the variants entirely:

```rust,ignore
use processkit::ErrorKind;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &std::path::Path) {
if let Err(err) = git.checkout(repo, "does-not-exist").await {
    match err.kind() {
        ErrorKind::Exit => eprintln!("git exited {:?}", err.code()),
        ErrorKind::Timeout => eprintln!("git timed out"),
        ErrorKind::NotFound => eprintln!("git is not installed"),
        _ => eprintln!("{err}"),
    }
}
# }
```

**Exit code as data.** When a non-zero exit is an *answer*, not a failure (e.g.
`gh pr checks` signalling pending via exit 8), reach for `run_raw`: it returns a
`processkit::ProcessResult<String>` and does **not** error on a non-zero exit.
Read the code with its `code()` accessor (`Option<i32>`); `program()`
(processkit 0.7+) names the binary the result came from — handy where one
facade runs both git and jj:

```rust,ignore
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

```rust,ignore
use processkit::JobRunner;
use processkit::testing::RecordingRunner;
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

**(b) Live output streaming.** Processkit 3.1's hardened lifecycle stream is
available through the typed network methods:

- `GitApi::{fetch,push,clone_repo}_with_progress`;
- `JjApi::{git_fetch,git_push,git_clone}_with_progress`;
- portable `Repo::{fetch,push,clone}_with_progress`.

The object-safe callback receives `ProcessEvent::Started`, interleaved
stdout/stderr line events, then terminal `ProcessEvent::Exited`. Internally the
event consumer and `RunningProcess::finish()` are polled concurrently — draining
events first would wait forever for the finisher to publish `Exited`. A rejected
exit is reconstructed as the same structured processkit error as the captured
path, including the streamed stdout/stderr, so the normal classifiers still
work.

Each streaming method intentionally observes one process lifecycle and does not
apply the captured fetch path's automatic transient retry. That keeps `Exited`
truthfully terminal; callers can choose an explicit replay policy after seeing
the event and returned error. A callback panic is caught and disables further
callback delivery for that run; the child is still drained and reaped, so UI
code cannot strand it. `ScriptedRunner` implements `start()`/`events()` as well,
so the complete lifecycle is hermetically replayable in tests.

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

```rust,ignore
use processkit::testing::{Reply, ScriptedRunner};
use vcs_git::Git;

# fn demo() {
let runner = ScriptedRunner::new()
    .on(["git", "status"], Reply::ok(" M src/lib.rs\0")) // realistic where it matters
    .fallback(Reply::ok(""));                     // everything else: answer, run nothing
let git = Git::with_runner(runner);
let _ = git;
# }
```

## See also

- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the runner seams in full (trait, `mock`
  feature, scripted/recording runners) and the real-binary fixtures.
- [Security & hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/) — the injection guards behind `ErrorReason::Spawn`
  and the untrusted-repo profile.
- Per-crate guides: [git](https://docs.rs/vcs-git/latest/vcs_git/guide/), [jj](https://docs.rs/vcs-jj/latest/vcs_jj/guide/), [github](https://docs.rs/vcs-github/latest/vcs_github/guide/), [core](https://docs.rs/vcs-core/latest/vcs_core/guide/).
