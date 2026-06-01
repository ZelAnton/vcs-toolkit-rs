# Changelog — vcs-jj

All notable changes to the `vcs-jj` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-jj-v<version>`.

## [Unreleased]

### Added
- `JjApi` trait + `Jj` client with typed, repo-scoped commands returning parsed
  structs: `log`/`current_change` (`Change`), `describe`/`new_change`, `status`,
  `bookmarks` (`Bookmark`).
- **Mockable by design:** consumers code against `JjApi`; `Jj::with_runner`
  injects a fake process runner, and the `mock` feature generates `MockJjApi`
  (via `mockall`).
- `bookmark_set`, `git_fetch`, `git_push`, and raw `run`/`run_raw` on `JjApi`.
- `Change` gained the `empty` flag (no file modifications).
- `Jj::default_timeout` kills any command exceeding the deadline.

### Changed
- The API is now the `Jj` client + `JjApi` trait — the original free functions
  are gone. Commands launch `jj` inside an OS job (Windows Job Object / Linux
  cgroup v2) via `processkit`, killed on close.
- **Now async (tokio):** every `JjApi` method is `async`; errors are the typed
  `processkit::Error`. Adds `async-trait`.
- Built on the external **`processkit`** crate (the `CliClient` core, the
  `cli_client!` macro, the `ProcessRunner` seam, and the structured `Error`) —
  replacing the prototype internal `vcs-process` crate. `run_raw` now returns
  `processkit::ProcessResult<String>`.
- `Change`/`Bookmark` are now `#[non_exhaustive]` — future fields won't be
  breaking changes.
- Optional `tracing` feature (forwards to `processkit/tracing`): a `debug` event
  per `jj` command.

## [0.1.0] - 2026-05-29

### Added
- Initial skeleton: `run` CLI-execution helper and `version()` over the `jj` binary.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-jj-v0.1.0
