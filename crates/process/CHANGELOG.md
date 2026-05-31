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
  dependency-free test double mapping a command to a canned `Output`. New
  `Output::ok`/`Output::fail` constructors and `Exec::program`/`arguments`/
  `working_dir` accessors. With the `mock` feature, `mockall` also generates
  `MockRunner`.
- **Timeouts:** `Exec::timeout`/`maybe_timeout` kill the job when the deadline
  elapses; `Output::timed_out` reports it. Inspired by the .NET sibling.
- **Structured errors:** `CommandError` (and the `Result` alias) carries the
  program, args, exit code, stderr, and timeout — a typed alternative to the old
  stringly `io::Error`. `Exec::run` and the new `Exec::checked_with`/`output_with`
  (run via an injected `Runner`) return it.

### Changed
- **Now async (tokio).** `Exec::run`/`output`/`spawn`, the `Runner` trait, and the
  free `run`/`output` helpers are `async`; processes spawn via `tokio::process`.
  `Child` wraps `tokio::process::Child`. Adds `tokio`, `async-trait`, `thiserror`.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/commits/main
