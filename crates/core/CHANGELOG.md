# Changelog — vcs-core

All notable changes to the `vcs-core` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-core-v<version>`.

## [Unreleased]

### Added
- Initial release: a unified facade over `vcs-git` and `vcs-jj`.
  - `detect(dir) -> Option<Located>` — walk up to find a `.git`/`.jj` repository
    (jj wins when colocated), returning `BackendKind` + root.
  - `Repo` — a cwd-bound handle (`Repo::open`, `Repo::at`) dispatching the common
    surface to whichever backend is present: `current_branch`, `trunk`,
    `changed_files`, `diff_stat`, `commit_paths`, `fetch`, `list_worktrees`,
    `create_worktree`, `remove_worktree`, with `git()`/`jj()` escape hatches for
    tool-specific operations.
  - Backend-agnostic DTOs: `BackendKind`, `ChangeKind`, `FileChange`, `DiffStat`,
    `WorktreeInfo`, `CreateOutcome`.
  - Generic over the `processkit::ProcessRunner` so tests can inject a fake
    runner via `Repo::from_git` / `Repo::from_jj`.
