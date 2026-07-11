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
- `Repo::open`, called directly on a directory that is itself a **bare** git
  repository (`git init --bare`: `HEAD`/`config`/`objects`/`refs` with no `.git`
  subdirectory), now returns `Error::BareRepository` instead of the generic
  `Error::NotARepository` — matching what `Repo::discover` already reported for
  the same directory (issue #6, T-004; the `BareRepository` variant itself isn't
  new, only `open`'s use of it). `Error::NotARepository`'s `Display` message no
  longer claims the repository was searched for "at or above" the given path;
  that phrasing was only ever accurate for `discover`'s upward walk and was
  misleading for `open`'s strict, non-walking check of exactly the given
  directory. (T-060.)
- `Repo::try_merge`'s git-side rollback is now **cancellation-safe**, matching the
  jj path: the *whole* cleanup path — both the "is a trial merge still staged?"
  decision (via the new `Git::is_merge_in_progress_detached`) and the `merge --abort`
  that undoes it (via `Git::merge_abort_detached`) — runs on a fresh cancellation
  context with its own bounded deadline, so a `default_cancel_on` token that fires
  during the probe merge no longer cancels the rollback and leaves the trial merge
  staged in the working tree. Previously only the abort command was detached while
  the gating `is_merge_in_progress` probe still inherited the (possibly
  already-fired) client token, so a cancellation that landed on the probe skipped
  the abort and abandoned the half-staged merge — the defect the jj path had already
  closed (T-036/T-051), and which the earlier fix left open on the `try_merge`
  Ok/Err branches. The `try_merge` doc's cancellation caveat is rewritten
  accordingly: the entire rollback (decision plus command) now survives a cancelled
  probe on **both** backends, not just jj. (T-059.)

## [0.8.0] - 2026-07-10

### Added
- `OperationState` now models the remaining git sequencer states: `CherryPick`
  (`CHERRY_PICK_HEAD`), `Revert` (`REVERT_HEAD`), and `Bisect` (`BISECT_LOG`),
  alongside `Merge`/`Rebase`/`ApplyMailbox`/`Conflict`/`Clear`. `in_progress_state`
  and `snapshot().operation` now report these instead of a misleading `Clear`, and
  `abort_in_progress` dispatches the state's OWN git command (`cherry-pick --abort`
  / `revert --abort` / `bisect reset`) rather than silently doing nothing.
  `continue_in_progress` drives `cherry-pick --continue` / `revert --continue`
  (reporting `Conflict` when they stop on the next commit, like a rebase). A
  cherry-pick/revert conflict writes its own head file, **not** `MERGE_HEAD`, so
  these are never confused with a merge. (T-044.)
- `Error::Unsupported(String)` + `Error::is_unsupported()`: an action refused
  because the repository's current in-progress state has no such step — currently
  `continue_in_progress` during a `git bisect` (which advances by marking commits
  good/bad, not `--continue`). Explicit refusal instead of a misleading success.
  Mirrors `vcs_forge::Error::is_unsupported`. (T-044.)
- `Repo::log(revspec_or_revset, max)` / `VcsRepo::log`: backend-agnostic recent
  history, dispatching to `GitApi::log` / `JjApi::log`. Returns the new
  `Commit` DTO (`id`, `description`, and `author`/`date` — the latter two
  `Some` only on git, since jj's typed log doesn't currently surface them).
- `Error::Rollback(vcs_jj::Rollback)`: a new variant raised when the jj backend's
  `Repo::try_merge` trial-merge rollback cannot complete cleanly — the `op restore`
  failed, or a **concurrent** jj process advanced the operation log so reverting
  would have clobbered its work. Carries the structured `vcs_jj::Rollback` so the
  caller can tell a failed restore from a divergence-refused one.

### Changed

- **Breaking:** the path-carrying facade surface is now lossless for non-UTF-8 names.
  `FileChange.path` / `FileChange.old_path` are `PathBuf` / `Option<PathBuf>` (were
  `String` / `Option<String>`); `Repo::conflicted_files` returns `Vec<PathBuf>` (was
  `Vec<String>`); `MergeProbe::Conflicts` carries `Vec<PathBuf>` (was `Vec<String>`);
  and `Repo::commit_paths` / `VcsRepo::commit_paths` take `&[PathBuf]` (was `&[String]`).
  A path obtained from `changed_files` / `conflicted_files` now round-trips **losslessly**
  into `commit_paths` — on git a filename whose bytes are not valid UTF-8 (legal on Unix)
  reaches the commit unchanged and addresses the SAME file, where a `String::from_utf8_lossy`
  decode would have substituted `U+FFFD` and retargeted it. `WorktreeInfo.path` (from
  `worktrees`) is lossless the same way on both backends — the git worktree listing and
  the jj workspace-root listing that feed it now parse from raw bytes, so a worktree /
  workspace whose directory name is not valid UTF-8 no longer collapses to `U+FFFD`.
  (`FileChange`/`MergeProbe`
  still `Serialize`: a `PathBuf` renders as a JSON string for a UTF-8 path and, per the
  fail-closed policy, a non-UTF-8 path is a serialization **error**, never a silent
  `U+FFFD`.) The `FileChange` builder (`FileChange::new` / `.old_path`) now takes
  `impl Into<PathBuf>`. (T-050.)
- `WorktreeInfo.commit` is now the checked-out commit's **full** object id on
  both backends (the jj side previously reported a short prefix), the same
  identity `RepoSnapshot.head` carries — so the two can be compared directly to
  tell whether a worktree sits on the snapshotted commit, without a short-prefix
  collision. Documented as such on both fields. (T-041.)
- The facade keeps its ergonomic `&str`-taking `Repo` API but now converts each
  ref-name / revision input into the backend's validated newtype
  (`vcs_git::RefName`/`RevSpec` / `vcs_jj::BookmarkName`/`RevsetExpr`) **at the
  boundary**, so an invalid or flag-like value from a caller (CLI/MCP/UI) is
  rejected with a classifiable `Error::is_invalid_input` **before** any child
  process spawns, on both backends. Behavioural change: a flag-like branch passed
  to `Repo::push` is now refused pre-spawn on the **jj** backend too (previously it
  rode jj's `-b` flag-value slot verbatim), matching the git backend — the
  conversion is uniform. `Repo::checkout("-")` maps to git's "previous branch"
  (`CheckoutTarget::Previous`) at the boundary.
- Internal only (no public API change): the git backend now drives `vcs-git`'s
  spec-typed `delete_branch(BranchDelete)` / `worktree_remove(WorktreeRemove)` and
  `blocking::worktree_remove(WorktreeRemove)` instead of the removed positional
  `bool` flags. `Repo::delete_branch` / `remove_worktree` /
  `cleanup_worktree_blocking` keep their existing signatures.

### Fixed

- fix: jj worktree cleanup no longer swallows partial-failure state. The rollback
  after a failed `bookmark create` in `create_worktree` still spares a pre-existing
  directory, but now **reports** a secondary cleanup failure (a directory it couldn't
  remove, or a workspace it couldn't `forget`) instead of discarding it with
  `let _ = …`; a clean rollback still surfaces the original bookmark-step cause
  unchanged. `remove_worktree` and `cleanup_worktree_blocking` likewise surface a
  `remove_dir_all` failure and name what is still registered so the cleanup is
  diagnosable and safely repeatable (the blocking path no longer swallows the removal
  error, and skips the `forget` on a failed removal to avoid orphaning the directory).
  Behavioural change: when a jj worktree path matches none of the *resolvable*
  workspaces but some registered workspace couldn't be resolved via `workspace root
  --name`, the lookup now returns a distinct diagnosable error (not a clean
  `WorktreeNotFound` — `is_resource_not_found` stays `false`) that names the
  unresolved workspaces, since the path's absence can't be proven. Pairs with the
  `vcs-jj` `blocking::workspace_name_for_path` signature change
  (`io::Result<Option<String>>`).
- fix: `Repo::try_merge` on the jj backend now rolls its trial merge back through
  the shared concurrency-safe protocol (`Jj::rollback_to`) instead of a bare
  `op_restore`, so the two rollback paths (`try_merge` and `Jj::transaction`) share
  one mechanism. The rollback survives a cancelled operation and, if a concurrent jj
  process advanced the operation log during the trial merge, is **refused** —
  surfacing `Error::Rollback` rather than reporting a stale, untrustworthy
  `MergeProbe::Clean`/`Conflicts` while the probe change lingers.

## [0.7.2] - 2026-07-06

### Changed

- Minimal Debug impl for vcs_core::Repo (#7)
- core: distinguish bare git repositories from not-a-repository
- core: rename Repo::open to Repo::discover; add strict Repo::open
- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Fixed

- fix(core): rustfmt the discover ancestor-walk test (CI fmt check)


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(m10): update vcs-core test mocks for jj workspace-root --ignore-working-copy
- fix(git): rev_parse_short --verify + diff verbs terminate revisions with -- (pathspec-collision hardening, C2/M13 class)


### Added

- feat(a4): public builder constructors for the core return DTOs (external impls/test doubles)


### Changed

- refactor(a5): create_worktree takes a WorktreeCreate spec (branch/base not transposable)
- review(0.4.0): whole-solution followups — MergeCheckPartial rename, is_merged test, mcp/core changelogs
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wave1.5a): is_invalid_input + is_resource_not_found classifiers (A2/A3)


### Changed

- refactor(core): use vcs_testkit::TempDir in tests (drop duplicate fixture)
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(wave1.5b): Repo::remove_worktree takes a WorktreeRemove spec, not a bare force bool (A1)
- refactor(wave1.5b): Repo::delete_branch takes a BranchDelete spec, not a bare force bool (A1)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(review): branch-listing color-safety (git); try_merge always rolls back (core)
- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave2): has_uncommitted honors jj conflict; detect requires .jj/repo (M18/M19)
- fix(wave2): a gone upstream reads uncountable, not in-sync (M17, breaking DTO)
- fix(wave2): detect cherry-pick/revert/bisect/am state; don't rebase-abort a git am (M20)
- fix(m-cluster-followup): snapshot() detects git am (BLOCKER) + audit status + M17/M19/M20 doc coherence
- fix(wave2): switch_with_stash pops only its own stash, with --index (M12)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(core): re-export processkit + is_transient helper on Error (fewer direct deps for downstream)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- refactor(api): git current_branch -> Option; gitlab mr id -> number (pre-1.0 consistency)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(core): atomic jj worktree creation — clean up on bookmark-step failure (R1)
- fix(core): jj worktree-rollback & forget-error safety, snapshot arity-guard; bookmarks() via template
- fix(git): current_branch handles unborn repos via symbolic-ref
- fix(core): gitlink-aware detect, conflicted-empty snapshot dirty, worktree zip guard
- fix(core): resolve relative jj worktree paths against the repo dir, not process cwd
- fix(core): unify jj current_branch on the nearest reachable bookmark (#3)
- fix(core): deterministic tie-break for jj current_branch among equally-near bookmarks (#4)


### Added

- feat: typed description/fetch_from/conflicted_files/status_tracked + facade surface
- feat: orchestration primitives — jj transaction, try_merge, abort/continue, switch_with_stash
- feat: vcs-testkit crate, version capabilities, observation docs
- feat(core): batched Repo::snapshot + maturity docs (Wave C)
- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- refactor(core+forge): macro-mirror VcsRepo/ForgeApi trait decl + delegating impl (Wave S)
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts


### Added

- feat: optimize toolkit for consumers — non-interactive git, blocking cleanup, API gaps, FileDiff.raw (0.4)
- feat: cwd-bound handles, wider facade, new ops, VcsRepo trait


### Changed

- review: fix cross-cutting issues — CI packages vcs-core, doc consistency, facade tracing feature + crates.io README links
- deps: processkit 0.6 — probe() predicates + transient fetch-retry
- review: fix stale README exit_code() example + clean vcs-core changelog maintainer-note
- review(jj): force --color never; fix tab-truncation, revset range, git merge flags
- Release: vcs-git v0.4.0, vcs-jj v0.4.0, vcs-github v0.4.0, vcs-core v0.2.0


### Fixed

- fix: jj rename paths, Windows separators, unborn-repo diff


### Added

- feat(vcs-core): Phase 2 unified VCS facade crate
- feat(vcs-core): extend common surface for agent-workspace migration + re-export underlying crates


### Changed

- build(vcs-core): wire facade crate into the release pipeline
- Release: vcs-git v0.3.1, vcs-jj v0.3.1, vcs-github v0.3.1, vcs-core v0.1.0

## [0.7.1] - 2026-07-05

### Added
- **`Repo`/`Backend` now implement `Debug`.** The impl is hand-written rather
  than derived, for two reasons: it avoids forcing an `R: Debug` bound onto the
  generic runner type parameter (`R: ProcessRunner`), which callers would
  otherwise have to satisfy even though `R` itself is never printed; and it
  never formats the inner `Git`/`Jj` client — `Backend` prints only its
  discriminant (`Git(..)`/`Jj(..)`) via `finish_non_exhaustive`, so a
  credential token set via `with_token` can't leak through `{:?}`.
- **`Error::BareRepository(PathBuf)`** — `Repo::discover` (see the breaking
  rename below) now returns this instead of the generic
  `Error::NotARepository` when the directory walk instead reaches a **bare**
  git repository (`git init --bare`: `HEAD`/`config`/
  `objects`/`refs` directly in the directory, no `.git` subdirectory, no
  working tree). A bare repository *is* a valid git repository — just one this
  facade doesn't drive, since there's no working tree for the CLI wrappers to
  operate against — so it deserves its own, more precise error rather than
  being folded into "not a repository". Opening a normal (non-bare) git
  repository, a jj repository, or a genuinely non-repository directory is
  unaffected. Fixes #6.
- **New `Repo::open(dir)`** — opens the repository at **exactly** `dir`, with
  no ancestor walk: `dir` itself must hold the `.jj`/`.git` marker, or this
  errors with `Error::NotARepository(dir)` even if a repository exists
  somewhere above `dir`. Mirrors the `discover`-vs-`open` split in gitoxide
  (`gix::discover`/`gix::open`) and libgit2
  (`git_repository_discover`/`git_repository_open`). See below for the
  breaking rename that freed up the `open` name. Fixes #8.

### Changed
- **Breaking: `Repo::open` → `Repo::discover`; `detect` → `discover`.** The
  project is pre-1.0 with no external users yet, so this
  ships without a deprecation shim. What `Repo::open`/`detect` used to do —
  walk up from a directory to the filesystem root looking for a `.jj`/`.git`
  marker — is what gitoxide and libgit2 call **discovery**
  (`gix::discover`/`git_repository_discover`), not "open": both of those
  libraries reserve `open` for a strict check of exactly one directory, with
  no ancestor walk. The old names conflated the two, and gave no way to ask
  "is this *exact* directory a repository root?". `Repo::discover` and the
  top-level `discover` function behave exactly like the old `Repo::open`/
  `detect` (including the new `Error::BareRepository` classification above);
  the `open` name is now free for the new, stricter method described above.
  Fixes #8.

### Fixed
-

## [0.7.0] - 2026-07-05

### Added
-

### Changed
-

### Fixed
- **Docs:** `Repo::rebase`'s contract now documents the git-vs-jj **divergence** as
  an explicit least-common-denominator. git (`rebase <onto>` =
  `merge-base(HEAD,onto)..HEAD`) moves only `HEAD`'s ancestor line; jj (`rebase -d`
  = default `-b @` = `(onto..@)::`) also moves everything stacked on `@` and any
  sibling off an *intermediate* commit — strictly more on a stacked/intermediate-fork
  layout. (An earlier note implied parity; they agree only on a linear line. A
  sibling sharing only the fork point is moved by neither.) No behavior change.
  (`docs/audit-2026-07.md` M6.)

## [0.6.0] - 2026-07-03

### Added
- **Public builder constructors for the return DTOs** `RepoSnapshot`, `WorktreeInfo`,
  `FileChange`, and `UpstreamTracking` (e.g. `RepoSnapshot::new().head(id).dirty(n)`,
  `FileChange::new(path, kind).old_path(old)`, `WorktreeInfo::new(path).branch(b)`,
  `UpstreamTracking::new("origin/main").ahead(2)`). These are `#[non_exhaustive]`, so a
  consumer writing a custom `VcsRepo` backend or a test double previously **could not
  build one to return**; the builders make them constructible outside `vcs-core`
  (`RepoSnapshot` also gains a `Default`). Additive. (`docs/audit-2026-07.md` A4.)
- `WorktreeCreate` (+ its partial builder `WorktreeCreatePartial`) — the spec that
  `Repo::create_worktree` now takes.

### Changed
- **`Repo::create_worktree` (and the `VcsRepo` trait method) takes a `WorktreeCreate`
  spec, not `(path, branch, base)` positional args (breaking).** The new-branch name and
  the fork-point `base` were two
  adjacent plain strings that compiled when transposed — `create_worktree(p, "main",
  "feature")` silently created a branch *named* `main` off `feature`. It's now
  `create_worktree(WorktreeCreate::new(path, "feature").base("main"))`: `path`+`branch`
  go in `new` (distinct types), and `base` is a separate named step, so the two names
  can't be swapped. `base` stays explicit (no default — the "current" sentinel is git
  `HEAD` vs jj `@`). (`docs/audit-2026-07.md` A5.)

### Fixed
-

## [0.5.0] - 2026-07-03

### Added
- **`Error::is_invalid_input()`** and **`Error::is_resource_not_found()`** classifiers,
  completing the `is_*` family. `is_invalid_input` recognizes a caller bug — a value
  the facade refused before spawning (a flag-like/empty guarded positional, an empty
  file set, removing the main workspace) — distinct from a real IO/backend failure.
  `is_resource_not_found` covers a worktree/workspace lookup that matched nothing
  (`WorktreeNotFound`), distinct from the `git`/`jj` *binary* being missing
  (`is_not_found`). A binding maps them to `ValueError` / `NotFoundError`.
  (`docs/audit-2026-07.md` A2, A3.)

### Changed
- **`Repo::remove_worktree` takes a `WorktreeRemove` spec, not a bare `force` bool
  (breaking).** `remove_worktree(path, true)` didn't say what `true` meant; it's now
  `remove_worktree(WorktreeRemove::new(path).force())` — self-documenting at the call
  site, and `#[non_exhaustive]` so future options don't re-break the signature. Behavior
  is unchanged (the main-workspace refusal and the dirty-guard still apply). First of
  the pre-1.0 bare-bool→spec conversions. (`docs/audit-2026-07.md` A1.)
- **`Repo::delete_branch` takes a `BranchDelete` spec, not a bare `force` bool
  (breaking).** `delete_branch(name, true)` → `delete_branch(BranchDelete::new(name).force())`.
  `force` is git-only (`branch -D` vs `-d`); jj has no force and ignores it. Behavior
  unchanged. (`docs/audit-2026-07.md` A1.)
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
- **A git-backed `Repo` now scrubs the inherited repo-redirector env vars**
  (`GIT_DIR`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, …), transitively via `vcs-git`'s
  default client. So a `Repo::open`ed inside a git hook (which exports `GIT_DIR`)
  now targets the *discovered* repository rather than the hook's — commands can no
  longer be silently redirected at a different repo. (`docs/audit-2026-07.md` H4.)
- **Docs:** `Repo::checkout` now warns prominently that it diverges by *consequence*,
  not just verb: on jj it maps to `jj edit`, so a following `commit_paths` **rewrites
  the checked-out commit in place** (a silent amend of a possibly-pushed commit),
  whereas git appends on top. Backend-agnostic "start work on top of `main`" code
  should start a new child change explicitly (`jj new <ref>`). No behavior change.
  (`docs/audit-2026-07.md` H11.)

### Fixed
- **`abort_in_progress` no longer aborts an in-progress `git am` with `rebase --abort`.**
  A `git am` and an apply-backend rebase share git's `rebase-apply/` dir, so the state
  probe mistook an am for a rebase and ran the wrong abort. A new
  `OperationState::ApplyMailbox` variant now reports a `git am` distinctly (via
  `is_am_in_progress`), and `abort_in_progress` routes it to `am --abort`. (Detecting
  cherry-pick/revert/bisect is a separate, deferred follow-up; they read `Clear` and
  `abort_in_progress` safely no-ops on them.) `OperationState` is `#[non_exhaustive]`,
  so the new variant is additive. (`docs/audit-2026-07.md` M20.)
- **A gone upstream is no longer reported as "in sync" (breaking: `UpstreamTracking`
  `ahead`/`behind` are now `Option<usize>`).** git's porcelain omits the ahead/behind
  line when the upstream is set but doesn't resolve (deleted on the remote, or not yet
  fetched); the old `unwrap_or(0)` turned that into a fabricated `↑0↓0` (falsely
  in-sync). `ahead`/`behind` are now `Option<usize>` — `None` means "tracking
  configured but uncountable", distinct from `Some(0)` (genuinely in sync). Update
  consumers to handle `None` (e.g. render "gone"/"?" rather than "up to date").
  (`docs/audit-2026-07.md` M17.)
- **Docs:** `local_branches` / `branch_exists` now document the jj *tombstone*
  divergence — a bookmark deleted locally but still tracked on a remote lingers in the
  list until the deletion is pushed, so a just-deleted tracked bookmark can still read
  as existing (unlike git). Not filtered, because jj renders a tombstone and a
  *conflicted* bookmark identically. (`docs/audit-2026-07.md` M21.)
- **`detect` no longer lets a stray `.jj` directory shadow a healthy `.git` repo.** It
  checked only that `.jj` `is_dir()`, so an empty/leftover `mkdir .jj` beat a real git
  repository in the same or a parent directory. It now requires a real jj marker (a
  `.jj/repo` store — a directory in a main workspace/colocated repo, a file pointer in
  a secondary workspace), symmetric with the validated `.git` probe. (`docs/audit-2026-07.md` M19.)
- **`has_uncommitted_changes` now agrees with `snapshot().dirty` on a conflicted jj
  change.** A jj change that is `empty` but **conflicted** is uncommitted state (it
  needs resolution); `snapshot().dirty` already treated `conflict ⇒ dirty`, but the
  boolean `has_uncommitted_changes` read only the `empty` flag and reported `false`.
  It now also returns `true` when `@` is conflicted (probed only when `@` is empty, so
  the common case stays one query). (`docs/audit-2026-07.md` M18.)
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
  to). See CONTRIBUTING.md "Releasing" for the two-phase release coordination.
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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.8.0...HEAD
[0.8.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.7.2...vcs-core-v0.8.0
[0.7.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.7.1...vcs-core-v0.7.2
[0.7.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.7.0...vcs-core-v0.7.1
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.6.0...vcs-core-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.5.0...vcs-core-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.4.0...vcs-core-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.3.0...vcs-core-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.2.0...vcs-core-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-core-v0.1.0...vcs-core-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-core-v0.1.0
