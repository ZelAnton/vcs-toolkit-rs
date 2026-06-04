# Changelog — vcs-core

All notable changes to the `vcs-core` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-core-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.2.0] - 2026-06-04

### Added
- `Repo::git_at()` / `Repo::jj_at()` — the backend client bound to the handle's
  `cwd` (`GitAt`/`JjAt`), so tool-specific calls drop the `dir` argument:
  `repo.git_at()?.merge_continue().await?`. For another worktree, bind the
  re-anchored handle first (`let wt = repo.at(path); wt.git_at()…`).
- Wider common surface: `checkout`, `rebase`, `fetch_remote_branch`, and
  `in_progress_state` → `OperationState` (a backend-agnostic merge/rebase/conflict
  state), so consumers stop re-implementing git-vs-jj dispatch for them.
- `VcsRepo` trait over the common surface, so a consumer can hold a
  `Box<dyn VcsRepo>` / `&dyn VcsRepo` instead of threading the runner generic.
- `Error::is_conflict()` / `is_nothing_to_commit()` / `is_transient_fetch()` —
  classify a failure without matching on `processkit::Error` internals.
- `Repo::cleanup_worktree_blocking(path)` — synchronous, best-effort worktree
  removal for a `Drop` guard that can't `.await` (git: `worktree remove --force`;
  jj: resolve the workspace name by path, delete the dir, `workspace forget`).

### Changed
- `trunk()` now falls back to a local `main`, then `master`, when the backend has
  no native trunk (git `origin/HEAD` unset / jj `trunk()` unresolved).
- Requires `vcs-git` / `vcs-jj` **0.4** (for the `blocking` helpers it dispatches
  to). See AGENTS.md "Releasing" for the two-phase release coordination.
- Bumped `processkit` to 0.6 (no code change).

### Fixed
-

## [0.1.0] - 2026-06-03

### Added
- Initial release: a unified facade over `vcs-git` and `vcs-jj`.
- `detect(dir) -> Option<Located>` — walk up to find a `.git`/`.jj` repository
  (jj wins when colocated), returning `BackendKind` + root.
- `Repo` — a cwd-bound handle (`Repo::open`, `Repo::at`) dispatching the common
  surface to whichever backend is present: `current_branch`, `trunk`,
  `changed_files`, `diff_stat`, `commit_paths`, `fetch`, `list_worktrees`,
  `create_worktree`, `remove_worktree`, plus `local_branches`, `branch_exists`,
  `has_uncommitted_changes`, `delete_branch`, `rename_branch` — with `git()` /
  `jj()` escape hatches for tool-specific operations.
- Backend-agnostic, `#[non_exhaustive]` DTOs: `BackendKind`, `ChangeKind`,
  `FileChange`, `DiffStat`, `WorktreeInfo`, `CreateOutcome`.
- Generic over the `processkit::ProcessRunner` so tests can inject a fake runner
  via `Repo::from_git` / `Repo::from_jj`.
- Re-exports `vcs_git` and `vcs_jj` so a consumer depending only on `vcs-core`
  can reach the raw clients and their types without a separate dependency.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.2.0...HEAD
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.1.0...vcs-core-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-core-v0.1.0
