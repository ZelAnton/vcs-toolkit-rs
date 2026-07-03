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
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` (`vcs_core::processkit`, incl.
  `Error`/`ProcessResult`) 1.x — **breaking** for a downstream that pins `processkit`
  `0.x` directly. No behaviour change. processkit is semver-stable from 1.0, so future
  1.x updates are non-breaking.
- **Renamed `Repo::fetch_remote_branch` → `fetch_branch` (breaking).** The unified
  single-branch/bookmark fetch (and the `VcsRepo` facade-trait method) is now
  `fetch_branch`, aligning with `vcs-git`'s renamed `fetch_branch`; backend
  dispatch and behaviour are unchanged. Update callers of `fetch_remote_branch`
  to `fetch_branch`.

### Fixed
- **`remove_worktree` no longer risks wiping the repository or silently losing
  edits on jj.** It now (a) refuses to remove the repository's **main** workspace —
  whose directory is the main working copy, so deleting it destroyed the whole
  checkout (reachable as `remove_worktree(".", …)`); the guard checks both the
  `default` name and the store-owning `.jj/repo` directory, so a `jj workspace
  rename` can't bypass it — and (b) honors `force`: with `force = false` a workspace
  with uncommitted changes is refused (the changes are first snapshotted into jj's
  op log, so they're recoverable), instead of being deleted unconditionally as
  before. Pass `force = true` to remove a *dirty* worktree anyway (the main-workspace
  refusal holds regardless of `force`). The same main-workspace guard was added to the
  blocking Drop-path `cleanup_worktree_blocking`. (`docs/audit-2026-07.md` C1.)

## [0.4.0] - 2026-06-27

### Added
- `UpstreamTracking { branch, ahead, behind }` — the upstream ref and ahead/behind
  counts as one value, carried by `RepoSnapshot::tracking`.
- Re-export of `processkit` itself (`vcs_core::processkit`) so a `vcs-core`-only
  consumer can match the wrapped `Error::Vcs(processkit::Error::…)` (and reach
  `Outcome`/`CancellationToken`/…) without taking a direct `processkit` dependency.
- `Error::is_transient()` (transient io/spawn failure — interrupted/would-block/
  busy) and `Error::is_not_found()` (the `git`/`jj` binary isn't installed) —
  completing the `is_*` classifier family so callers branch on intent without
  reaching into `processkit::Error`.

### Changed
- **`Repo::current_branch` and `RepoSnapshot::branch` on jj now report the
  nearest reachable bookmark** (revset `heads(::@ & bookmarks())`) instead of
  only a bookmark strictly on `@`. After a `jj describe`/`jj new`/`jj commit` the
  bookmark is left on the described parent while the new working-copy change
  carries none, so the strict rule returned `None` right after a commit. Both
  surfaces now derive from one source (`jj_backend::current_branch`), matching
  `git_backend`'s structure, and stay non-empty across a commit like git's "still
  on my branch" reporting. The strict "does `@` carry a bookmark" probe remains
  on `vcs_jj::JjApi::current_bookmark`. As a side effect the jj `snapshot` gains
  one spawn (a `reachable_bookmarks` query for `branch`); git is unchanged.
- **`RepoSnapshot` tracking shape (breaking).** The three coupled `Option` fields
  `upstream` / `ahead` / `behind` are replaced by a single
  `tracking: Option<UpstreamTracking>` — `Some` only when an upstream is set,
  `None` otherwise (always `None` on jj). A half-populated state (e.g. an upstream
  with no counts) is now unrepresentable; serde nests it under `tracking`.
- Bumped `processkit` to **0.11.0** (via `vcs-git`/`vcs-jj`). Re-exported
  `processkit::Error` changed (partial `stdout`/`stderr` on `Timeout`/`Signalled`;
  new `Signalled`/`NotFound`/`CassetteMiss` variants; `Invocation::cwd: Option<PathBuf>`)
  — breaking for downstream.

### Removed
- The **`cancellation`** feature (which forwarded to `vcs-git`/`vcs-jj`) —
  cancellation is now core in processkit 0.10; `default_cancel_on` is always
  available without a feature.

### Fixed
- **jj worktree ops resolve a relative `path` against the repo dir, not the process
  cwd.** `create_worktree`/`remove_worktree` (and `cleanup_worktree_blocking`) ran
  their own `exists()` / `remove_dir_all` / canonicalization on the raw caller
  `path`, which a relative path resolves against the **process cwd** — while
  `jj workspace add` runs with cwd = the repo `dir` and resolves the path against
  *that*. When the two differ (e.g. a `Repo` opened via `vcs-mcp --repo /elsewhere`
  with a relative worktree path), the facade probed/deleted the wrong location:
  leaking the half-made worktree on a rollback, or a spurious `WorktreeNotFound` /
  orphaned dir on removal. The path is now resolved against `dir` for every
  filesystem op. git is unaffected (`git worktree` resolves the path itself).
- **jj worktree safety.** `create_worktree`'s rollback (run when the bookmark
  anchor fails after `workspace add` already created the workspace) no longer
  deletes the destination directory unless `workspace add` itself created it — a
  pre-existing directory the caller already had is left intact instead of being
  wiped on an unrelated failure.
- **jj `remove_worktree` no longer hides a `workspace forget` failure.** The dir is
  still deleted first (an orphan dir is worse than a dangling registration), but a
  failing `forget` now surfaces as an `Err` (name resolution already proved the
  workspace is registered) instead of being swallowed — the caller can retry.
- **jj `snapshot` parses defensively.** The `@`-template row is now read field-by-
  position with a debug-assert on its arity, so a truncated/garbled row yields a
  *coherent* snapshot (clean, unconflicted) rather than one whose `dirty` flag flips
  to a contradictory "dirty with 0 changes."
- **A conflicted jj `@` is now reported `dirty`** even when jj marks the change
  `empty` (a conflict with no net content change): the conflict is uncommitted state
  needing resolution, so `dirty`/`change_count` reflect it — mirroring git, where
  conflict markers are unstaged changes — instead of the surprising
  `conflicted: true` next to `dirty: false`.
- **`detect` validates a `.git` *file*** is a real gitlink (content starts with
  `gitdir:`), not just any file named `.git`. A stray/garbage `.git` file no longer
  registers as a repository (or shadows a real repo higher up the tree), making the
  `.git` probe as strict as the `.jj` `is_dir()` one.
- **jj worktree listing is guarded against silent truncation.** `list_worktrees` /
  the worktree-name lookup `debug_assert` that the batched `workspace_roots` fan-out
  returns one result per workspace, so a future contract drift can't silently drop a
  worktree from the listing (or wrongly report one as not-found).

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.4.0...HEAD
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.3.0...vcs-core-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.2.0...vcs-core-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.1.0...vcs-core-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-core-v0.1.0
