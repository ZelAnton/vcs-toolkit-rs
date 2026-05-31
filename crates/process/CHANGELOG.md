# Changelog — vcs-process

All notable changes to the `vcs-process` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-process-v<version>`.

## [Unreleased]

### Added
- Initial release: `Job` (Windows Job Object / Linux cgroup v2 with a POSIX
  process-group fallback), `Child`, the `Mechanism` reporter, and the one-shot
  `run` helper. Child processes are launched with kill-on-close so the whole
  tree dies with the parent — no orphaned `git`/`jj`/`gh` subprocesses.
- `Exec` builder for richer runs: working directory, env vars, and stdin input,
  with `run()` (error on non-zero exit) and `output()` (capture the status). New
  `Output` type (`success`/`combined`) and free `output()` helper.
  `Child::try_wait` for non-blocking liveness checks.
- `Runner` trait — the execution boundary, so consumers can inject a fake in
  tests. `JobRunner` is the real (default) runner; `ScriptedRunner` is a
  dependency-free test double mapping a command to a canned `Output` by argument
  prefix (`on`) or predicate (`when`). `RecordingRunner` wraps any runner and
  captures each run as an `Invocation` (program, full args, cwd, env, stdin) for
  exact assertions — including that a flag is *absent*. `Runner` is also
  implemented for `&R`, so a test can keep its recorder. New `Output::ok`/`fail`/
  `timeout` constructors and `Exec::program`/`arguments`/`working_dir`/`env_vars`/
  `stdin_bytes` accessors. With the `mock` feature, `mockall` generates `MockRunner`.
- **Streaming I/O:** `Exec::stream` returns a `Streaming` that reads stdout *as it
  is produced* (`next_line`/`stdout`) and writes stdin incrementally (`stdin`,
  enabled by `Exec::pipe_stdin`), instead of buffering until exit. stderr is
  drained in a background task — so a stdout-only reader never deadlocks — and
  returned by `finish` alongside the exit status. New typed `Child::stdout`/
  `stderr`/`stdin` pipe accessors; the `ChildStdin`/`ChildStdout`/`ChildStderr`
  types are re-exported.
- `CliClient<R>` — the shared client core the wrappers build on: binary name +
  runner + default timeout, the `exec`/`exec_in` builders, and the
  `run_text`/`run_raw`/`run_unit`/`parsed`/`parsed_try` terminals. Each wrapper is
  a thin typed facade over it, so a new wrapper is just a `const BINARY`, a `core`
  field, three constructors, and its typed methods.
- **Timeouts:** `Exec::timeout`/`maybe_timeout` kill the job when the deadline
  elapses; `Output::timed_out()` reports it. Inspired by the .NET sibling.
- **Structured errors:** `CommandError` (and the `Result` alias) carries the
  program, args, exit code, stderr, and timeout — a typed alternative to the old
  stringly `io::Error`. `Exec::run`, `checked_with`/`output_with`, and
  `code_with` (returns the exit code for commands where the code is the answer,
  e.g. `git diff --quiet`, while still erroring on spawn/timeout/signal) return it.

### Changed
- **Now async (tokio).** `Exec::run`/`output`/`spawn`, the `Runner` trait, and the
  free `run`/`output` helpers are `async`; processes spawn via `tokio::process`.
  `Child` wraps `tokio::process::Child`. Adds `tokio`, `async-trait`, `thiserror`.
- `Output` models termination portably via the `Termination` enum
  (`Exited`/`Signaled`/`TimedOut`), replacing the stored `ExitStatus` and the
  separate `timed_out` bool. `timed_out()` is now a method; `code()` is new. This
  drops the synthetic wait-status bit-shifting and its `#[cfg]` gates.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/commits/main
