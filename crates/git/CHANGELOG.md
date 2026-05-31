# Changelog — vcs-git

All notable changes to the `vcs-git` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-git-v<version>`.

## [Unreleased]

### Added
- `GitApi` trait + `Git` client with typed, repo-scoped commands returning parsed
  structs: `status` (`StatusEntry`), `log`/`current_branch`/`branches`/`rev_parse`,
  `init`/`add`/`commit`, `diff_is_empty`. New `Commit`/`Branch`/`StatusEntry` types.
- **Mockable by design:** consumers code against `GitApi`; `Git::with_runner`
  injects a fake process runner (e.g. `vcs_process::ScriptedRunner`), and the
  `mock` feature generates `MockGitApi` (via `mockall`) for stubbing whole methods.
- `create_branch`, `checkout`, and raw `run`/`run_raw` escape hatches on `GitApi`.
- `Commit` gained `short_hash` and `date` (ISO-8601 `%aI`).
- `Git::default_timeout` kills any command exceeding the deadline.

### Changed
- The API is now the `Git` client + `GitApi` trait — the original free functions
  (`run`/`version`/`status`/…) are gone. Commands launch `git` inside an OS job
  (Windows Job Object / Linux cgroup v2) via `vcs-process`, killed on close.
- **Now async (tokio):** every `GitApi` method is `async`. Errors are the typed
  `vcs_process::CommandError` (exit code, stderr, …) instead of `io::Error`.
  Adds `async-trait`.
- `status` now runs `git status --porcelain=v1 -z` (NUL-delimited records, raw
  unescaped paths — robust to spaces and special characters) and `log` uses `-z`
  record separation (robust to multi-line fields). `StatusEntry` gained
  `orig_path`, the source path for a rename/copy (`R`/`C`).
- Builds on `vcs_process::CliClient`, the shared client core (internal refactor;
  no API change beyond `StatusEntry`).

### Fixed
- `status`/`branches` parsing no longer corrupts the first entry: output is parsed
  raw instead of being trimmed, which had stripped leading `--porcelain` status
  spaces and `branch` markers.

## [0.1.0] - 2026-05-29

### Added
- Initial skeleton: `run` CLI-execution helper and `version()` over the `git` binary.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-git-v0.1.0
