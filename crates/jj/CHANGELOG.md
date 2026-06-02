# Changelog — vcs-jj

All notable changes to the `vcs-jj` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-jj-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.3.0] - 2026-06-02

### Added
- Typed diff: `diff_text(dir, DiffSpec)` returns the raw git-format unified diff
  (`diff -r <spec> --git`), and `diff(dir, DiffSpec)` returns a parsed
  `Vec<FileDiff>` (change kind, path, rename old-path, and `@@` hunks with
  per-line `DiffLine`s). The pure parser `parse::parse_diff` is public for
  parsing externally-obtained diff text. `DiffSpec::WorkingTree` diffs `@`;
  `DiffSpec::Rev(_)` diffs a revset.
- Partial-change ops with a safe `JjFileset` newtype (escapes `\`/`"`, renders
  `file:"…"`): `commit_paths`, `squash_paths`, and `sparse_set` (`sparse set
  --clear --add …`). `WorkspaceAdd` gains a `sparse(SparseMode)` builder
  (`workspace add --sparse-patterns copy|full|empty`).
- `status_text` — the raw `jj status` text (the previous `status` return), and
  `is_transient_fetch_error` classifier mirroring `vcs_git`.
- Inherent `Jj::run_args` / `run_raw_args` taking `&[&str]`, so callers needn't
  allocate a `Vec<String>` for the `run` escape hatch.

### Changed
- `status` now returns parsed `Vec<ChangedPath>` (backed by `diff -r @ --summary`)
  instead of the raw `jj status` string, mirroring `vcs_git::GitApi::status`. The
  raw text moved to the new `status_text`.
- Bumped `processkit` to 0.5. No change to the rest of this crate's public API.

### Fixed
-

## [0.2.1] - 2026-06-01

### Added
-

### Changed
- Bumped `processkit` to 0.4 — macOS/BSD process trees are now contained via a
  POSIX process group (`killpg` on drop) instead of an uncontained spawn.

### Fixed
-

## [0.2.0] - 2026-06-01

### Added
- **Workspace management:** `workspace_list` (new `Workspace` struct),
  `workspace_root`, `workspace_add` (`WorkspaceAdd` options), `workspace_forget`.
- **Discovery:** `root`, `current_bookmark`, `trunk`.
- **Bookmarks:** `bookmark_create`, `bookmark_rename`, `bookmark_delete`,
  `bookmark_move`.
- **Diff / query / state:** `diff_summary` (new `ChangedPath` struct), `diff_stat`
  (new `DiffStat` struct), `commit_count`, `is_conflicted`,
  `has_workingcopy_conflict`, and `template_query` (a typed `jj log -T` escape hatch).
- **Mutations:** `rebase`, `edit`, `squash_into`, `new_merge`, `abandon`,
  `git_fetch_branch`, `git_import`.
- **Operation log:** `op_head`, `op_restore`, `op_undo`.

## [0.1.0] - 2026-06-01

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.0...HEAD
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.1...vcs-jj-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.0...vcs-jj-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.1.0...vcs-jj-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-jj-v0.1.0
