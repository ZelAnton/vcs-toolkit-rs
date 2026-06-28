# Changelog — vcs-watch

All notable changes to the `vcs-watch` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-watch-v<version>`.

## [Unreleased]

### Added
- `Error::is_transient()` and `Error::is_not_found()` classifiers — delegate through
  the wrapped `vcs-core` error so a caller can branch on a transient io/spawn hiccup
  or a missing `git`/`jj` binary without hand-walking the nesting. Mirrors the
  corresponding classifiers on `vcs_core::Error` / `vcs_forge::Error` (which expose a
  superset; `is_transient_fetch_error` is intentionally omitted here — the watcher
  never fetches, so it would always be `false`).
- `Error::processkit_error() -> Option<&processkit::Error>` — flattens the two-level
  `Vcs(vcs_core::Error::Vcs(_))` nesting to the structured underlying process error
  (`program`/`code`/`stdout`/`stderr`), so a consumer (or the planned `vcs-toolkit-py`
  binding) can read it uniformly. `None` for `Notify`/`Io` and non-subprocess
  `vcs-core` errors.
- Re-export of `processkit` (`vcs_watch::processkit`) so a `vcs-watch`-only consumer
  can name the `processkit_error()` return type without a direct dependency (mirrors
  `vcs_core::processkit` / `vcs_forge::processkit`).

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` (`vcs_watch::processkit`) 1.x —
  **breaking** for a downstream that pins `processkit` `0.x` directly. No behaviour
  change. processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.

### Fixed
-

## [0.2.0] - 2026-06-27

### Added
-

### Changed
- Bumped `processkit` to **0.11.0**. Test doubles moved to `processkit::testing`;
  cancellation is now core (no feature flag).

### Fixed
- Corrected the `stats()` doc: the wedged-repo signal is a climbing
  [`skipped`](WatcherStats::skipped) with **flat `changes`**, not flat
  `requeries` — a skipped re-query bumps `requeries` too, so it is never flat
  while skips climb. (Matches the module-level and config docs, which were
  already correct.)

## [0.1.0] - 2026-06-08

### Added
- Initial release: `RepoWatcher` filesystem-watches a git/jj repository and
  streams typed `RepoEvent`s. On each filesystem change it debounces the burst,
  re-queries `vcs-core`'s batched `Repo::snapshot()` (+ `local_branches()`), and
  diffs against the previous state — so raw-event noise (ref temp-renames,
  `index.lock`, reflog churn) coalesces into one re-check instead of spurious
  events.
- `RepoEvent` (`#[non_exhaustive]`): `HeadMoved`, `BranchSwitched`,
  `BranchCreated`/`BranchDeleted`, `WorkingCopyChanged`, `UpstreamChanged`,
  `AheadBehindChanged`, `OperationChanged`, `ConflictChanged`. Each settled change
  arrives as a `RepoChange { snapshot, events }` — the new full `RepoSnapshot`
  (re-exported from `vcs-core`) plus the deltas; `recv()` / `current()` consume it.
- Builder: `working_tree(bool)` (default off — state-dir-only watching; opt in to
  also watch the working tree for bare unstaged edits), `debounce(Duration)`
  (default 250 ms), `max_wait(Duration)` (default 1 s). Backend + watch dir come
  from `vcs-core`'s pure `detect` (`.jj` wins when colocated; worktree gitlinks
  resolved). Dropping the `RepoWatcher` stops the watch and the background task.
- The pure snapshot-`diff` is hermetically unit-tested; the notify → debounce →
  re-query → emit pipeline is covered by `#[ignore]` real-repo integration tests
  (git + jj).
- `Builder::requery_timeout(Option<Duration>)` (default **30 s**,
  `DEFAULT_REQUERY_TIMEOUT`): a deadline on each re-query, so a wedged command
  (a held `index.lock` on a client with no timeout configured) is killed
  (kill-on-drop) and skipped as transient instead of stalling the watch loop
  forever. Orthogonal to `max_wait` (that bounds how long signals *defer* a
  re-query; this bounds how long one re-query *runs*).
- `RepoWatcher::stats() -> WatcherStats` — lock-free health counters
  (re-queries run / changes emitted / skips, plus what the last skip failed
  on), so a long-running consumer can notice a silently wedged repository
  instead of inferring health from event silence.
- `stream` feature: `impl futures_core::Stream for RepoWatcher`, so the watcher
  drops straight into `select!`/stream combinators. `recv()` and the stream
  share one channel (an item goes to whichever is polled first) and both
  advance `current()`. Off by default; pulls in only the `futures-core` trait
  crate.

### Notes
- This is the workspace's **first runtime tokio dependency** (everything else
  hides tokio behind `processkit`) and **first streaming API** — build/await the
  watcher inside a tokio runtime. Transient mid-operation re-query failures are
  skipped and retried on the next event (settled-state semantics).

### Changed
- The `max_wait` ceiling is now **exact**: a dedicated timer arm fires the
  re-query at the deadline even when the signal stream pauses right after it —
  previously the ceiling was only observed when the *next* signal arrived.
- The debounce → ceiling → re-query pipeline is now **hermetically tested**:
  `watch_loop` runs against a fake signal channel, a `ScriptedRunner`-backed
  repo, and a paused tokio clock (9 tests pinning coalescing, the `max_wait`
  ceiling, transient skip + recovery, the re-query deadline, teardown,
  backpressure, and the stream adapter) — no real filesystem or process
  involved.

### Fixed
- A watcher on a **linked git worktree** now also watches the shared `.git`
  directory (resolved via the worktree gitdir's `commondir` file), where
  `refs/heads/*` and `packed-refs` actually live — previously only the private
  per-worktree gitdir was watched, so `BranchCreated`/`BranchDeleted` never
  fired for a watched worktree.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.2.0...HEAD
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.1.0...vcs-watch-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-watch-v0.1.0
