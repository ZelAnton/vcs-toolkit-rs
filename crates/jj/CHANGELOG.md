# Changelog — vcs-jj

All notable changes to the `vcs-jj` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-jj-v<version>`.

## [Unreleased]

### Added
- `description(dir, revset)` — the full (multiline) description of the commit a
  revset resolves to (`log --limit 1 -T description`); empty for an undescribed
  change, newest commit only (log order) for a multi-commit revset.
- `git_fetch_from(dir, remote)` — fetch from a *named* git remote
  (`git fetch --remote <remote>`), retried on transient failures like
  `git_fetch`.

### Changed
-

### Fixed
-

## [0.4.0] - 2026-06-04

### Added
- `Jj::at(dir)` → `JjAt`, a cwd-bound view whose methods omit the leading `dir`
  argument (`jj.at(dir).status()`); the dir-taking `JjApi` stays for driving many
  workspaces from one client.
- `reachable_bookmarks` — local bookmarks on the nearest commits reachable from
  `@` (`log -r 'heads(::@ & bookmarks())'`), the candidate targets a commit belongs
  to; one entry per name when a commit carries several.
- `resolve_list(revset)` — conflicted paths from `jj resolve --list` (empty when
  there are none, including the no-conflict non-zero exit).
- Revision-scoped variants of the `@`-only ops: `describe_rev(revset, msg)` and
  `rebase_branch(branch, dest)` (`rebase -b … -d …`).
- Remote-tracking bookmarks: `bookmarks_all` (`bookmark list -a`, new `BookmarkRef`
  with name/remote/target/tracked) and `bookmark_track(name, remote)`.
- `FileDiff.raw` — the verbatim per-file diff section.
- Sync `blocking::workspace_forget` and `blocking::workspace_name_for_path`
  (resolve a workspace name by path) for `Drop`-time cleanup that can't `.await`.

### Changed
- `squash_into` and `squash_paths` gained a `use_destination_message: bool`
  (`--use-destination-message`) — *breaking* for these two signatures.
- Bumped `processkit` to 0.6. `git_fetch` / `git_fetch_branch` now retry transient
  failures (3 attempts, 500 ms backoff).

### Fixed
- Every `jj` invocation now forces `--color never`, so a user's
  `ui.color = "always"` config can no longer wrap templated output (and the error
  text classified by `is_transient_fetch_error`) in ANSI escapes and break parsing.
- A change description containing a literal tab is no longer truncated when parsing
  `jj log` template rows (`splitn` keeps the remainder).
- `diff_summary` parenthesises each endpoint of the `<from>..<to>` revset range, so
  a compound revset keeps its meaning instead of rebinding by operator precedence.

## [0.3.1] - 2026-06-03

### Added

- feat(diff): typed diff (raw + parsed) for git and jj
- feat(git,jj): fill Phase 1 API gaps
- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics


### Changed

- deps: bump processkit 0.4 -> 0.5; absorb breaking API changes
- Release: vcs-git v0.3.0, vcs-jj v0.3.0, vcs-github v0.3.0


### Changed

- Release: vcs-git v0.2.1, vcs-jj v0.2.1, vcs-github v0.2.1


### Added

- feat(git,jj): expand clients with worktree/workspace, discovery, diff, merge ops for agent-workspace


### Changed

- Release: vcs-git v0.2.0, vcs-jj v0.2.0, vcs-github v0.2.0


### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.4.0...HEAD
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.1...vcs-jj-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.0...vcs-jj-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.1...vcs-jj-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.0...vcs-jj-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.1.0...vcs-jj-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-jj-v0.1.0
