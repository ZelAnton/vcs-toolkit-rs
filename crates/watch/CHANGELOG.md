# Changelog — vcs-watch

All notable changes to the `vcs-watch` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-watch-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.6.0] - 2026-07-10

### Added
- `WatcherStats::retries`/`recoveries`/`terminal_failures` — a skipped
  (timed-out or transiently failed) re-query now schedules a bounded backoff
  retry on its own, without waiting for a new filesystem event, and these
  counters distinguish a self-scheduled retry, a subsequent recovery, and a
  terminal watch-backend failure.
- `WatchError` — an opaque filesystem-watch backend error with the classifiers
  `is_path_not_found()`, `is_watch_limit()`, `io_error()`, and `paths()`, plus a
  source-chain to its underlying `std::io::Error`. Reachable via the new
  `Error::watch_error()` accessor or by matching `Error::Notify`. Lets a consumer
  classify and source-chain a watch failure through `vcs-watch` alone, with no
  direct dependency on the third-party watch backend. (T-055.)

### Changed
- **Breaking:** `Error::Notify` now wraps the crate's own opaque `WatchError`
  instead of the third-party `notify::Error`. The filesystem-watch backend is now
  a fully private dependency and is **not** part of this crate's stability
  contract, so a backend major bump is an internal, non-breaking change here
  rather than forcing consumers to keep a matching `notify` version. Match
  `Error::Notify(watch_error)` (or call `Error::watch_error()`) and use the
  `WatchError` classifiers in place of matching `notify::ErrorKind`. (T-055.)
- The `tracing` feature now also forwards to `vcs-core/tracing` (which fans out
  to `vcs-git`/`vcs-jj` and `processkit`), so enabling `vcs-watch/tracing` traces
  the underlying git/jj commands each re-query issues — not just the watcher's own
  skip/retry line. `tracing` and `stream` remain off by default and isolated from
  minimal/default builds. (T-055.)
- `RepoEvent::OperationChanged` now also fires for the git cherry-pick, revert,
  and bisect states that `vcs-core`'s `OperationState` gained: starting, ending,
  or moving between any sequencer state (`CherryPick`/`Revert`/`Bisect`, alongside
  `Merge`/`Rebase`/`ApplyMailbox`) is reported, since the snapshot diff is generic
  over `operation`. No API change — the event's `from`/`to` simply carry the new
  variants. (T-044.)
- A permanent filesystem-watch backend failure (e.g. the watched `.git`/`.jj`
  directory was removed and re-created) now closes the watcher's output
  channel: `RepoWatcher::recv`/the `stream` feature observe it as `None`/end
  of stream directly, instead of only being visible via `stats().watch_errors`.

### Fixed
-

## [0.5.2] - 2026-07-06

### Changed

- core: rename Repo::open to Repo::discover; add strict Repo::open
- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- refactor(a7): make data-carrying RepoEvent/Error variants #[non_exhaustive] (field-safe)
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(watch): Error classifiers + processkit_error reach-through


### Changed

- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): bound watch raw-event bridge + deadline the baseline snapshot (R2/R4)
- fix(wave2): a gone upstream reads uncountable, not in-sync (M17, breaking DTO)
- fix(m-cluster-followup): snapshot() detects git am (BLOCKER) + audit status + M17/M19/M20 doc coherence


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(watch): vcs-watch — filesystem-watch repo events (Wave E)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.1] - 2026-07-05

### Changed

- core: rename Repo::open to Repo::discover; add strict Repo::open


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- refactor(a7): make data-carrying RepoEvent/Error variants #[non_exhaustive] (field-safe)
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(watch): Error classifiers + processkit_error reach-through


### Changed

- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): bound watch raw-event bridge + deadline the baseline snapshot (R2/R4)
- fix(wave2): a gone upstream reads uncountable, not in-sync (M17, breaking DTO)
- fix(m-cluster-followup): snapshot() detects git am (BLOCKER) + audit status + M17/M19/M20 doc coherence


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(watch): vcs-watch — filesystem-watch repo events (Wave E)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.0] - 2026-07-05

### Changed

- refactor(a7): make data-carrying RepoEvent/Error variants #[non_exhaustive] (field-safe)
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(watch): Error classifiers + processkit_error reach-through


### Changed

- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): bound watch raw-event bridge + deadline the baseline snapshot (R2/R4)
- fix(wave2): a gone upstream reads uncountable, not in-sync (M17, breaking DTO)
- fix(m-cluster-followup): snapshot() detects git am (BLOCKER) + audit status + M17/M19/M20 doc coherence


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(watch): vcs-watch — filesystem-watch repo events (Wave E)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Added
-

### Changed
- **Every data-carrying `RepoEvent` variant is now individually `#[non_exhaustive]`
  (breaking).** A `match`/`matches!` arm that binds a variant's fields must add `..`
  (e.g. `RepoEvent::BranchCreated { name, .. }`). The enum was already
  `#[non_exhaustive]` (new *variants* were safe); this makes new *fields* on an
  existing event safe too, so an event can gain context (a timestamp, an id) after 1.0
  without a breaking bump. (`docs/audit-2026-07.md` A7.)

### Fixed
-

## [0.3.0] - 2026-07-03

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
- **`WatcherStats::watch_errors`** — a counter of OS-watch errors reported by the
  `notify` backend. A non-zero/climbing count means the underlying watch is failing
  (most often the watched `.git`/`.jj` dir was removed and re-created, e.g. a
  re-clone — which on Windows fails the watch), so the watcher may have gone silently
  deaf; treat it as "rebuild the watcher". (`docs/audit-2026-07.md` R3.)

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` (`vcs_watch::processkit`) 1.x —
  **breaking** for a downstream that pins `processkit` `0.x` directly. No behaviour
  change. processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.

### Fixed
- **A huge `Builder::max_wait` no longer panics (and silently kills) the watcher.**
  The re-query ceiling is clamped before it is added to an `Instant`, so
  `max_wait(Duration::MAX)` (a natural "disable the ceiling" idiom) no longer
  overflows `Instant + Duration` and aborts the background loop — which would drop
  the event channel and leave the watcher permanently, silently deaf.
  (`docs/audit-2026-07.md` P1.)
- **The notify→loop bridge is now bounded, so a back-pressured watcher can't leak
  memory.** It was an *unbounded* channel: if the consumer kept the `RepoWatcher`
  alive but stopped calling `recv()` while the filesystem churned, the raw-event queue
  grew without limit. It is now a **capacity-1** channel the callback fills with
  `try_send` — a burst *coalesces* into one pending "re-check" signal (the loop
  re-queries the full snapshot anyway, so no state is lost). (`docs/audit-2026-07.md` R2.)
- **The startup baseline snapshot now honors `requery_timeout`.** A snapshot that
  wedged (a hung fsmonitor, a network filesystem, a held jj lock) on a `Repo` built
  without its own `default_timeout` would hang `build()` **at startup** — the very
  failure the loop-side deadline prevents. `build()` now bounds the baseline capture
  with the same deadline, returning an `Io` `TimedOut` error instead of hanging.
  (`docs/audit-2026-07.md` R4.)

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.6.0...HEAD
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.5.2...vcs-watch-v0.6.0
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.5.1...vcs-watch-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.5.0...vcs-watch-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.4.0...vcs-watch-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.3.0...vcs-watch-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.2.0...vcs-watch-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-watch-v0.1.0...vcs-watch-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-watch-v0.1.0
