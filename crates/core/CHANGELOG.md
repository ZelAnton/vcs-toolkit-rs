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

## [0.3.0] - 2026-06-08

### Added
- `Repo::snapshot() -> RepoSnapshot` (also on `VcsRepo`) — a batched query for a
  prompt/status-bar/TUI: branch, upstream, ahead/behind, HEAD, dirtiness, change
  count, and operation state in **one or two** spawns instead of N. git uses one
  `status --porcelain=v2 --branch` + the in-progress probe; jj uses one
  `log -r @` template + a change count only when dirty. `upstream`/`ahead`/
  `behind` are always `None` on jj. `RepoSnapshot` is re-exported.
- `Repo::conflicted_files()` (also on `VcsRepo`) — paths with unresolved merge
  conflicts in the working copy (git `diff --diff-filter=U` / jj
  `resolve --list -r @`).
- `Repo::has_tracked_changes()` (also on `VcsRepo`) — uncommitted changes to
  *tracked* files only. git ignores untracked files
  (`status --untracked-files=no`); jj auto-tracks new files, so this equals
  `has_uncommitted_changes` there.
- `Repo::fetch_from(remote)` (also on `VcsRepo`) — fetch from a *named* remote
  (git `fetch <remote>` / jj `git fetch --remote <remote>`), transient failures
  retried by the underlying client.
- `Repo::push(branch)` (also on `VcsRepo`) — push an **existing** local
  branch/bookmark to `origin`: git `push -u origin <branch>` (`-u` records the
  upstream; idempotent on repeat pushes), jj `git push -b <branch>`. The docs
  spell out the backend asymmetry (git pushes the ref; jj pushes the bookmark's
  *state*, including a remote deletion for a locally-deleted bookmark). Renamed
  refspecs / non-`origin` remotes stay on the `vcs_git::GitPush` escape hatch.
- `Repo::try_merge(source)` (also on `VcsRepo`) returning the new `MergeProbe`
  (`Clean` / `Conflicts(paths)`) — probe whether a merge would conflict, with
  guaranteed rollback before returning (git: `merge --no-commit --no-ff` +
  `merge --abort`; jj: a probe merge undone via `op restore`). A failing
  rollback propagates as an error instead of misreporting the tree state.
- `Repo::abort_in_progress()` / `Repo::continue_in_progress()` (also on
  `VcsRepo`) — drive a paused git merge/rebase to ground and return the fresh
  post-call `OperationState`. On git, `continue_in_progress` reports `Conflict`
  while unresolved paths block continuing (unlike `in_progress_state`, which
  still never returns `Conflict` for git). On jj both are reporting no-ops —
  nothing is ever paused; roll back via `Jj::transaction` / `op_restore`.
- Optional `serde` feature: derives `serde::Serialize` on the public DTOs
  (`RepoSnapshot`, `FileChange`, `WorktreeInfo`, `OperationState`, `BackendKind`,
  `MergeProbe`, `CreateOutcome`) and enables `vcs-diff/serde` for the re-exported
  `ChangeKind`/`DiffStat`, so a consumer (e.g. `vcs-mcp`) can emit them as JSON.
  **Off by default.**

### Changed
- Bumped `processkit` to **0.8** — `Error::Vcs` wraps the `#[non_exhaustive]`
  `processkit::Error`; `Error::Exit` Display gained a stderr-tail suffix. Breaking
  for consumers matching the wrapped error exhaustively, or bumping their own
  direct `processkit` separately (caret `"0.7"` does not span 0.8).
- New off-by-default **`cancellation`** feature, forwarding to `vcs-git`/`vcs-jj`:
  build a cancellable `Git`/`Jj` (via `default_cancel_on`) and hand it to
  `Repo::from_git`/`from_jj`. No new API.
- Internal: `Repo::list_worktrees` (jj) resolves workspace roots in one bounded
  fan-out via the new `Jj::workspace_roots` (processkit 0.8 `output_all`) instead
  of a per-workspace `await` loop. No behaviour change.
- **Renamed the `Error` classifiers** for one name per concept across the
  workspace: `Error::is_conflict` → `is_merge_conflict` and
  `Error::is_transient_fetch` → `is_transient_fetch_error` (matching the wrapper
  classifiers); `is_nothing_to_commit` is unchanged.
- Internal: `ChangeKind`/`DiffStat` are now the shared `vcs-diff` types
  (re-exported, so `vcs_core::ChangeKind` still resolves), eliminating the third
  copy and the per-backend `DiffStat` remap; the classifiers delegate to
  `vcs-cli-support`.

### Fixed
- `commit_paths` refuses an empty path set up front: the backends would diverge
  dangerously — git errors out, while jj's `commit` with no filesets would
  silently commit the **entire** working copy under the given message.
- `FileChange.old_path` doc corrected: the rename's original path is populated
  by **both** backends (jj's `{old => new}` summary form included), not git-only.

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.3.0...HEAD
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.2.0...vcs-core-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.1.0...vcs-core-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-core-v0.1.0
