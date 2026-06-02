# Changelog — vcs-git

All notable changes to the `vcs-git` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-git-v<version>`.

## [Unreleased]

### Added
- Typed diff: `diff_text(dir, DiffSpec)` returns the raw git-format unified diff
  (`diff <spec> --no-color --no-ext-diff -M`), and `diff(dir, DiffSpec)` returns
  a parsed `Vec<FileDiff>` (change kind, path, rename old-path, and `@@` hunks
  with per-line `DiffLine`s). The pure parser `parse::parse_diff` is public for
  parsing externally-obtained diff text. `DiffSpec::WorkingTree` diffs the working
  tree vs `HEAD`; `DiffSpec::Rev(_)` diffs a revision/range.
- API gaps consumers previously hand-rolled via `run()`: `checkout_detach`,
  `commit_paths` (partial `commit --only`, with optional `--amend`),
  `last_commit_message`, `is_unborn`, `log_range`, and `stash_push`/`stash_pop`.
  `WorktreeAdd` gains a `no_checkout()` builder (`worktree add --no-checkout`).

### Changed
- Bumped `processkit` to 0.5 and absorbed its breaking changes: exit-code probes
  now read `ProcessResult::code() -> Option<i32>` (the removed `exit_code() -> i32`
  with its `-1` timeout sentinel is gone), and synthetic `Error::Exit` values carry
  the new `stdout` field. No change to this crate's public API.

### Fixed
- `remote_head_branch` now keeps a slashed default-branch name intact (e.g.
  `release/v2`) instead of returning only its last path segment.

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
- **Worktree management:** `worktree_list` (new `Worktree` struct),
  `worktree_add` (`WorktreeAdd` options), `worktree_remove`, `worktree_move`,
  `worktree_prune`.
- **Discovery:** `common_dir`, `git_dir`, `resolve_commit`, `remote_head_branch`,
  `branch_exists`, `remote_branch_exists` (no credential prompt, 10s timeout),
  `remote_url`.
- **Branches & diff:** `is_merged`, `delete_branch`, `rename_branch`,
  `rev_list_count`, `diff_range_is_empty`, `diff_shortstat` (new `DiffStat` struct).
- **In-progress state:** `staged_is_empty`, `is_rebase_in_progress`,
  `is_merge_in_progress`.
- **Mutations:** `fetch`, `fetch_remote_branch`, `merge_squash`, `merge_commit`,
  `merge_no_commit`, `merge_abort`, `merge_continue`, `reset_merge`, `reset_hard`,
  `rebase`, `rebase_abort`, `rebase_continue`.

## [0.1.0] - 2026-06-01

### Added
- `GitApi` trait + `Git` client with typed, repo-scoped commands returning parsed
  structs: `status` (`StatusEntry`), `log`/`current_branch`/`branches`/`rev_parse`,
  `init`/`add`/`commit`, `diff_is_empty`. New `Commit`/`Branch`/`StatusEntry` types.
- **Mockable by design:** consumers code against `GitApi`; `Git::with_runner`
  injects a fake process runner (e.g. `processkit::ScriptedRunner`), and the
  `mock` feature generates `MockGitApi` (via `mockall`) for stubbing whole methods.
- `create_branch`, `checkout`, and raw `run`/`run_raw` escape hatches on `GitApi`.
- `Commit` gained `short_hash` and `date` (ISO-8601 `%aI`).
- `Git::default_timeout` kills any command exceeding the deadline.

### Changed
- The API is now the `Git` client + `GitApi` trait — the original free functions
  (`run`/`version`/`status`/…) are gone. Commands launch `git` inside an OS job
  (Windows Job Object / Linux cgroup v2) via `processkit`, killed on close.
- **Now async (tokio):** every `GitApi` method is `async`. Errors are the typed
  `processkit::Error` (exit code, stderr, …) instead of `io::Error`.
  Adds `async-trait`.
- `status` now runs `git status --porcelain=v1 -z` (NUL-delimited records, raw
  unescaped paths — robust to spaces and special characters) and `log` uses `-z`
  record separation (robust to multi-line fields). `StatusEntry` gained
  `orig_path`, the source path for a rename/copy (`R`/`C`).
- Built on the external **`processkit`** crate (the `CliClient` core, the
  `cli_client!` macro, the `ProcessRunner` seam, and the structured `Error`) —
  replacing the prototype internal `vcs-process` crate. No public API change
  beyond `run_raw` now returning `processkit::ProcessResult<String>`.
- `StatusEntry`/`Commit`/`Branch` are now `#[non_exhaustive]` — future fields
  won't be breaking changes.
- Optional `tracing` feature (forwards to `processkit/tracing`): a `debug` event
  per `git` command.

### Fixed
- `status`/`branches` parsing no longer corrupts the first entry: output is parsed
  raw instead of being trimmed, which had stripped leading `--porcelain` status
  spaces and `branch` markers.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.1...HEAD
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.0...vcs-git-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.1.0...vcs-git-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-git-v0.1.0
