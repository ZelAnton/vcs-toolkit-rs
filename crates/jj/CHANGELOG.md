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
- **`JjApi::root` (and the `root_wc` helper feeding `status`/
  `status_ignoring_working_copy`/`diff_summary`) now decodes `jj root`'s stdout
  byte-losslessly**, via `ManagedClient::parse_bytes` +
  `parse::workspace_root_from_bytes` — the same path `workspace_root` already
  used — instead of `PathBuf::from(self.core.run(..))`, which decoded through
  `String::from_utf8_lossy` and trimmed *all* trailing whitespace
  (`str::trim_end`). A non-UTF-8 workspace root (legal on Unix) no longer
  collapses to `U+FFFD`, and a root that legitimately ends in a space/tab is no
  longer truncated. This also fixes `status`/`status_ignoring_working_copy`/
  `diff_summary`, which resolve their machine query's cwd through `root_wc`/
  `root`: on a repo with such a root they previously risked spawning `jj diff`
  against a corrupted, non-existent path. (T-090.)
- **`bookmark_track`'s `remote` is now validated before spawning, closing a
  silent-no-op gap.** An empty or whitespace-only `remote` previously slipped
  past the existing glob-metacharacter guard into `exact:<name>@`, which jj
  parses as an empty remote name and answers with a warning and a **successful
  no-op** — the call returned `Ok(())` without tracking anything. A `remote`
  containing `@` similarly exploited jj's legacy last-`@`-split parsing of the
  positional `name@remote` form, silently retargeting which segment is the
  remote. Both are now rejected up front with a classifiable
  (`vcs_cli_support::is_invalid_input`) error, matching the metacharacter
  guard's existing behaviour. **Breaking for a caller that passed an empty/
  blank or `@`-containing `remote` and relied on the silent `Ok(())`:** it now
  gets an `Err`. (T-086.)
- **`status`/`diff_summary`/`resolve --list` no longer corrupt or reject a Unix
  filename containing a literal backslash or colon.** `parse::normalize_slashes`
  (used by `parse_diff_summary`/`parse_resolve_list`) and
  `normalize_workspace_path`'s `\`→`/` rewrite — plus the latter's
  second-byte-`:` drive-letter heuristic — are now **Windows-only**
  (`#[cfg(windows)]`), matching `JjFileset::path`'s existing convention. On
  Unix a `\` is a legitimate filename byte and a `:` is legal anywhere in a
  name; both are now preserved verbatim instead of being rewritten or, for a
  leading `a:b.txt`-style name, rejected with `Error::parse`. (T-084.)

## [0.11.0] - 2026-07-19

### Added
- `normalize_workspace_root`/`workspace_root_matches`: the pure path-normalisation
  and jj-workspace-root-matching helpers, factored out of the async
  (`vcs-core`) and blocking (`blocking::workspace_name_for_path`) jj-workspace
  resolvers so both use the same comparison set. `workspace_root_matches` is
  the union of the two resolvers' previously-diverged comparisons — a `root`/
  `path` pair either resolver used to match still matches. (T-080.)

### Changed
- `blocking::workspace_name_for_path` now matches a candidate workspace root
  against the requested path via the shared `workspace_root_matches` instead
  of its own inline `normalize`/comparison logic — same signature and
  behaviour (a superset of matches, see `workspace_root_matches`'s docs), no
  observable change for callers. (T-080.)

### Fixed
- **Docs:** the crate-level `# Safety` rustdoc names `DiffSpec::Rev` (the
  `diff_text`/`diff` target, via `diff_text_budgeted`) as a concrete instance
  of the already-documented "flag-value slots are not guarded" rule — it
  lands verbatim in `-r <revset>`, same as any other flag-value slot. No
  behaviour change. (T-081.)

## [0.10.0] - 2026-07-10

### Added

- feat: add `JjApi::log_paths` — like `log`, but scoped to changes that
  touched the given filesets (`jj log -r <revset> <filesets>`), built with
  `JjFileset::path` (same primitive as `commit_paths`/`squash_paths`) and a
  refusal of an empty fileset list before spawning.
- feat: add `Jj::rollback_to(dir, pre)` — the concurrency-safe op-log rollback
  primitive `transaction` runs, exposed for non-closure / FFI callers. It runs the
  cleanup on a **fresh cancellation context** with its own deadline (so a cancelled
  or timed-out operation no longer disables its own rollback) and **detects a
  concurrent op-log divergence** — refusing to revert (returning
  `Rollback::SkippedDiverged`) when another jj process advanced the operation log,
  rather than silently clobbering that work. Returns the structured `Rollback`
  outcome (`Restored` / `SkippedDiverged` / `Failed` / `NotAttempted`).
- feat: add the `Rollback` enum and `TransactionError` struct describing a
  transaction's rollback outcome and preserving the closure's cause.

### Changed

- **Breaking:** path-carrying results are now lossless for non-UTF-8 names.
  `ChangedPath.path` / `ChangedPath.old_path` are `PathBuf` / `Option<PathBuf>` (were
  `String` / `Option<String>`), and `JjApi::resolve_list` returns `Vec<PathBuf>` (was
  `Vec<String>`). `status` / `diff_summary` / `resolve_list` now parse `jj diff
  --summary` / `resolve --list` output from **raw bytes** (`parse_diff_summary` /
  `parse_resolve_list` consume `&[u8]`) via `ManagedClient::parse_bytes`, so a
  non-UTF-8 filename (legal on Unix) survives byte-for-byte instead of being flattened
  by `String::from_utf8_lossy`. `workspace_root` / `workspace_roots` likewise now build
  their returned `PathBuf` from raw `jj workspace root` stdout (via `parse_bytes` /
  `processkit::output_all_bytes`), so a workspace root that is not valid UTF-8 survives
  losslessly into the facade's `WorktreeInfo.path` — and only the trailing line
  terminator is stripped now, so a root path ending in a space/tab is preserved rather
  than trimmed. Text-only templated output (change/commit ids, bookmark/workspace
  names, descriptions) still decodes as `String`. (Note: jj's fileset language is text,
  so committing a non-UTF-8 path via `commit_paths` remains bounded by jj itself; the
  byte-faithful round trip is exercised on git.) (T-050.)

### Fixed

- A locally-deleted bookmark that a remote still tracks (a **tombstone**) no
  longer masquerades as a live local bookmark: `bookmarks()` — and through it the
  facade's `local_branches`/`branch_exists` — now filters it out. `bookmark list`
  renders such a bookmark as a `present=0` local row plus a `present=1`
  remote-tracking row; both are dropped, while a *conflicted* bookmark
  (`present=1`, no single target) is correctly kept. (T-041.)

### Changed

- deps: bump `mockall` to 0.15 (unified workspace dependency, was 0.13 per-crate).
- Machine templates now follow one **framing/escaping contract**: free-text
  fields (descriptions, bookmark/workspace names, the op-log user) are rendered
  with jj's `.escape_json()` and decoded on parse, and per-commit/-workspace
  bookmark lists are space-joined escaped names instead of comma/space-joined raw
  ones. Names/descriptions carrying spaces, commas, tabs, quotes, or newlines now
  round-trip unambiguously instead of mangling the row (e.g. a git-imported
  `co,mma` bookmark, or a workspace name with a tab). (T-041.)
- Identity/cross-reference commit ids are now the **full** id, not a short prefix:
  `Bookmark::target`, `BookmarkRef::target`, and `Workspace::commit` (and thus the
  facade's `WorktreeInfo.commit`) carry the full commit id so they can be matched
  against a git oid / `RepoSnapshot.head` without a short-prefix collision. The
  history-display `Change` (change/commit id) stays short by design. (T-041.)
- **Breaking:** `blocking::workspace_name_for_path` now returns
  `io::Result<Option<String>>` instead of `Option<String>`, so a `Drop`-guard caller
  can tell a genuine "no such workspace" (`Ok(None)`) from a probe that could not
  answer (`Err`: `jj` missing / failed to spawn, `workspace list` exited non-zero, or
  a registered workspace did not resolve via `workspace root --name`). The old
  `Option` folded every failure into `None`, silently skipping cleanup that a real
  error should have surfaced.
- **Breaking:** the raw escape hatches on the bound view (`JjAt::run`/`run_raw`/
  `run_args`/`run_raw_args`) now run **in the bound `dir`** instead of the process's
  current directory. Previously they sat in the `bare` forwarder group, so
  `jj.at(dir).run(…)` silently ran in the process cwd — a bound handle whose raw call
  could target a *different* repository than the one it was bound to. New dir-taking
  client methods `Jj::run_in`/`run_raw_in`/`run_args_in`/`run_raw_args_in` back the
  bound forwarders (argv forwarded verbatim — like the process-cwd `run`, they do
  **not** inject `--color never`; only the cwd is bound). The **process-cwd** escape
  hatch is unchanged and still reached by calling `run`/`run_raw`/… on `Jj` itself
  (`jj.run(…)`) — migrate a caller that relied on `jj.at(dir).run(…)` running in the
  process cwd to `jj.run(…)`. (T-035.)
- **Breaking:** bookmark names and revsets are now taken as the validated newtypes
  `BookmarkName` (new — jj's equivalent of a branch) and `RevsetExpr` (previously
  constructible but accepted by no method). Every `JjApi` op that names a bookmark
  to create/move/rename/delete/track/fetch/push now takes `&BookmarkName`; every op
  that resolves a revset takes `&RevsetExpr`; the option structs follow
  (`BookmarkMove::new(BookmarkName, RevsetExpr)`, `SquashInto::new(RevsetExpr)`,
  `SquashPaths::new(RevsetExpr, RevsetExpr)`, `WorkspaceAdd::new(name, RevsetExpr,
  path)`, `git_push(Option<BookmarkName>)`, `new_merge(msg, Vec<RevsetExpr>)`,
  `file_annotate(path, Option<RevsetExpr>)`, `absorb(Option<RevsetExpr>, …)`). A
  flag-like or malformed value is now rejected at construction, before it can reach
  an argv slot, as a classifiable `Error::is_invalid_input`. Migrate by wrapping
  the string: `jj.edit(dir, "@-")` → `jj.edit(dir, &RevsetExpr::new("@-")?)`,
  `jj.bookmark_create(dir, "feat", "@")` →
  `jj.bookmark_create(dir, &BookmarkName::new("feat")?, &RevsetExpr::new("@")?)`.
  Remaining bare-positional `&str` inputs that are not bookmarks/revsets (remote
  names, operation ids, workspace names) keep their internal guard.
- **Breaking:** replace the trailing positional `bool` on three `JjApi` methods
  with named specs, so the flag reads at the call site: `bookmark_move(dir, name,
  to, allow_backwards)` → `bookmark_move(dir, BookmarkMove::new(name,
  to)[.allow_backwards()])`, `squash_into(dir, into, use_destination_message)` →
  `squash_into(dir, SquashInto::new(into)[.use_destination_message()])`, and
  `git_clone(url, dest, colocate)` → `git_clone(url, dest,
  GitClone::colocated()|GitClone::separate())` (the colocation choice is still
  always explicit — there is deliberately no default). The `JjAt` bound view moves
  to the same specs.
- **Breaking:** `Jj::transaction` / `JjAt::transaction` now return
  `Result<T, TransactionError>` instead of `Result<T>`. On the closure's `Err` the
  `TransactionError` preserves that error as `cause` **and** reports what the
  rollback did in `rollback` — so a failed or refused rollback is visible instead of
  swallowed. Migrate a caller that only wants the old closure error with
  `.map_err(TransactionError::into_cause)`.

### Fixed

- fix: make the `transaction` op-log rollback safe under cancellation and
  concurrency. Previously a *fired* cancellation of the closure (on a client with
  `default_cancel_on`) also cancelled the `op_restore`, leaving the repo
  mid-transaction; the rollback now runs on a fresh cancellation context with its
  own deadline. A concurrent jj process's operation landing between the savepoint
  capture and the restore was silently reverted; the rollback now detects the
  op-log divergence and refuses (`Rollback::SkippedDiverged`). A failing
  `op_restore` was discarded (`let _ = …`); it is now surfaced as
  `Rollback::Failed`. See `Jj::rollback_to`.

## [0.9.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(jj): commit_paths refuses empty fileset (M7); backslash rewrite is Windows-only (M4)
- fix(jj): workspace root-path probes are read-only (--ignore-working-copy), no snapshot on Drop-cleanup (M10)
- fix(m2): JjFileset uses root-relative root-file: so filesets target the right file when dir != workspace root
- fix(m28): jj git fetch forces LC_ALL=C so transient network markers classify on a non-English locale
- fix(m29): git support gate enforces the real (2, 31) floor the crate's argv needs, not major-only


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Fixed

- fix(a10): jj op-log timestamp uses RFC-3339 offset (%:z), matching git's %aI


### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(review): C-locale on git diff_stat; --color never on jj blocking workspace probe
- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): content verbs preserve trailing bytes (H7)
- fix(wave2): clean a partial dest after a failed clone so a retry isn't blocked (R7)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(retry): lock-contention classifier + opt-in jittered RetryPolicy on git/jj mutations
- feat(credentials): git remote (HTTPS) credential injection via credential.helper (Phase 2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- security(git): harden() pins core.sshCommand + honestly scope the guarantee
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(core): jj worktree-rollback & forget-error safety, snapshot arity-guard; bookmarks() via template
- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard
- fix(cli-support+jj): tighten transient marker, resolve_list match, conflict end-marker
- fix(diff): unquote git-quoted paths so non-ASCII filenames aren't dropped
- fix(jj): parser robustness — drop empty-name bookmark rows; annotate CRLF; conflict to:-marker length


### Added

- feat: typed description/fetch_from/conflicted_files/status_tracked + facade surface
- feat: orchestration primitives — jj transaction, try_merge, abort/continue, switch_with_stash
- feat: client coverage — git clone/tags/show/blame/config, jj clone/absorb/split/op_log/evolog/annotate
- feat: vcs-testkit crate, version capabilities, observation docs
- feat: injection guards + validating newtypes, Git::hardened, typed conflict model
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests


### Added

- feat: optimize toolkit for consumers — non-interactive git, blocking cleanup, API gaps, FileDiff.raw (0.4)
- feat: cwd-bound handles, wider facade, new ops, VcsRepo trait


### Changed

- review: harden whole solution — non-interactive git fetch, fix stale root-README pr_create example
- deps: processkit 0.6 — probe() predicates + transient fetch-retry
- review: fix stale README exit_code() example + clean vcs-core changelog maintainer-note
- review(jj): force --color never; fix tab-truncation, revset range, git merge flags
- Release: vcs-git v0.4.0, vcs-jj v0.4.0, vcs-github v0.4.0, vcs-core v0.2.0


### Fixed

- fix: jj rename paths, Windows separators, unborn-repo diff


### Changed

- Release: vcs-git v0.3.1, vcs-jj v0.3.1, vcs-github v0.3.1, vcs-core v0.1.0


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

## [0.9.1] - 2026-07-05

### Added
- **`Jj<R>` now implements `Debug`**, via the shared `vcs_cli_support::managed_client!`
  macro (no code change here). No `R: Debug` bound.

### Changed
-

### Fixed
-

## [0.9.0] - 2026-07-05

### Added
-

### Changed
-

### Fixed
- **Docs:** `rebase`'s contract is corrected to document the jj-vs-git
  **divergence** honestly. jj's default `-b @` moves `(onto..@)::` — the
  fork-point-to-`@` line *and its whole descendant closure* (`@`, anything stacked
  on `@`, and any sibling off an *intermediate* commit) — which is **strictly more**
  than git's `rebase <onto>` (`merge-base(@,onto)..@`, `@`'s ancestor line only) on
  a stacked or intermediate-fork layout. (An earlier note overclaimed "matching
  git"; the two agree only on a linear `@`. A sibling off the fork point itself is
  moved by neither — verified on jj 0.42.) No behavior change.
  (`docs/audit-2026-07.md` M6.)
- **Docs:** `Jj::transaction` now documents two rollback caveats it left implicit — it
  is **single-actor** (the `op_restore <pre>` rollback restores the *whole* repo view,
  so a change another jj process landed between the capture and the restore is reverted
  too), and a **cancelled `f` also cancels the rollback** (a fired `default_cancel_on`
  token skips the restore, leaving the repo mid-transaction — run the restore yourself
  on a token-free client if you need it to survive cancellation). No behavior change.
  (`docs/audit-2026-07.md` M8.)
- **`git_fetch`/`git_fetch_from`/`git_fetch_branch` run under `LC_ALL=C`, so a transient
  network failure is retried on a non-English locale.** jj's `git fetch` surfaces
  libc/gai/curl errors ("Temporary failure in name resolution"); a localized
  environment translated them, so `is_transient_fetch_error` didn't match and the fetch
  wasn't retried. Pinning the C locale (mirroring `vcs-git`) keeps the retry markers
  stable. (`docs/audit-2026-07.md` M28.)
- **`JjFileset` now targets the intended file when a command runs below the workspace
  root.** It emitted jj's **cwd-relative** `file:"<path>"`, but the path is documented
  workspace-root-relative — so with `dir` ≠ the workspace root (a command run from a
  subdirectory), a fileset silently hit a *same-named file under `dir`*, or nothing.
  Every fileset consumer (`file_show`, `commit_paths`, `squash_paths`, `split_paths`,
  `absorb`) was affected. It now emits the **root-relative** `root-file:"<path>"`, so
  the path resolves from the workspace root regardless of the working directory. No
  change when `dir` already is the root. (`docs/audit-2026-07.md` M2.)
- **Workspace root-path probes (`workspace_root`, `workspace_roots`, the Drop-cleanup
  `workspace_name_for_path`) run `--ignore-working-copy`, so a read no longer snapshots
  the working copy.** All resolve *static* workspace root paths, but ran plain `jj
  workspace list`/`root`, which take the working-copy lock and write a snapshot op —
  mutating the very repo a `remove_worktree`/Drop cleanup is tearing down, and failing
  (→ leaked workspace) under lock contention. They're now read-only. (The audit's paired
  suggestion to *surface* a probe error rather than collapse to `None` is obviated: a
  read-only probe no longer takes the working-copy lock, so the contention path that
  drove the silent leak is gone.) (`docs/audit-2026-07.md` M10.)
- **`commit_paths` refuses an empty fileset slice instead of committing the whole
  working copy.** `commit_paths(dir, &[], msg)` degraded to a bare `jj commit -m msg`,
  which finalises *every* pending change — the opposite of its "exactly these filesets"
  contract. It now returns `Error::Spawn`/`InvalidInput` before spawning, mirroring
  `split_paths`. (`docs/audit-2026-07.md` M7.)
- **`JjFileset::path` no longer corrupts a Unix filename containing a backslash.** The
  `\`→`/` separator rewrite is now **Windows-only** (`#[cfg(windows)]`); on Unix a `\`
  is a legitimate filename byte and is preserved verbatim, matching `vcs-git`'s twin.
  (`docs/audit-2026-07.md` M4.)

## [0.8.0] - 2026-07-03

### Added
-

### Changed
-

### Fixed
- **`Operation::time` (the `op_log` timestamp) is now valid RFC 3339.** The op-log
  template formatted the offset with jj's `%z` (`+0200`) — which a **strict** RFC-3339
  parser rejects (it requires the colon, `+02:00`). It now uses `%:z`, matching
  `vcs-git`'s `%aI` dates, so both backends' timestamps parse uniformly. The string
  shape changes (`+0200` → `+02:00`); a consumer doing an exact-string compare on the
  raw timestamp should re-check, but any date parser is unaffected or fixed.
  (`docs/audit-2026-07.md` A10.)

## [0.7.0] - 2026-07-03

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_jj::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` types (`Error`/`ProcessResult`/…)
  1.x — **breaking** for a downstream that pins `processkit` `0.x` directly. No
  behaviour change (processkit's text-capture verb is now `output_string`, used
  internally). processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.
- `DiffSpec` is now a re-export of `vcs_diff::DiffSpec` (hoisted to the shared
  crate so `vcs-git`/`vcs-jj` share one definition; `vcs_jj::DiffSpec` still
  resolves) and is no longer `#[non_exhaustive]`, so a `match` over it can be
  exhaustive. Requires `vcs-diff` ≥ the version that introduces `DiffSpec`.
- **Docs:** `Jj::transaction` now documents the **non-closure path** for FFI /
  language bindings (which can't drive the borrowed-`JjAt` closure form across
  the boundary): capture `op_head` before the mutations and call `op_restore`
  back to it on failure — the public `JjApi` primitives that `transaction` wraps
  internally. No API change; the imperative path already existed.

### Fixed
- **Glob-scoped bookmark/remote operations no longer fan out.** `bookmark_delete`,
  `bookmark_move`, `bookmark_track`, `git_push` (`-b`), `git_fetch_from`
  (`--remote`), and `git_fetch_branch` (`-b`) now wrap the caller's name as jj's
  `exact:` string pattern. Previously a name containing a glob metacharacter — or a
  hostile `"*"` from a UI/bot — was treated as a **pattern**, so e.g.
  `bookmark_delete("*")` deleted *every* bookmark and `git_push(Some("*"))` pushed
  them all. Now each mutates exactly the named ref. (`docs/audit-2026-07.md` H1.)
- **Conflict resolution honors a missing terminating newline.**
  `conflict::{sides, base, resolve}` reconstruct a side's/base's bytes correctly
  when a side lacks a final newline (jj's explicit trailing-newline representation),
  including CRLF files — previously the phantom trailing/context line became a
  spurious extra blank line (or a stray `\r`), silently corrupting the written-back
  content. (`docs/audit-2026-07.md` C3.)
- **`conflict::parse_conflicts` no longer misreads marker-like content as a
  git-style file.** A line that starts with a `<<<<<<<` run but isn't a
  `conflict N of M` header is kept as text (jj lengthens its own markers past such
  content), so a real jj conflict alongside a marker-like content line parses
  instead of erroring. A genuinely git-style file (the `<<<`/`===`/`>>>` triad with
  no jj header) is still redirected to `vcs_git::conflict`. (`docs/audit-2026-07.md` H6.)
- **Corrected the `is_lock_contention` markers for jj.** They matched strings jj
  never emits (`"failed to lock the working copy"` / `"failed to lock op heads"`);
  they now match jj's actual `"Failed to lock working copy"` /
  `"Failed to lock operation heads store"`. `Jj::with_retry`'s doc also notes that
  modern jj generally *blocks* on these locks rather than failing, so the retry
  catches only residual lock errors. (`docs/audit-2026-07.md` H2.)
- **`file_show`, `diff_text`, and `template_query` no longer strip trailing bytes.**
  They return jj's output **verbatim** (via `run_untrimmed`), so a file's trailing
  newline(s) survive a read-modify-write round-trip, a diff's last hunk stays in sync
  with its `@@` count, and a template that ends in `\n\n`/spaces is preserved.
  `description` trims explicitly to keep its scalar contract. (Behavior change: a
  caller that relied on the old trimming should trim itself.) (`docs/audit-2026-07.md` H7.)
- **A `git_fetch` that times out is no longer retried** (inherited from cli-support's
  `is_transient_fetch_error` change). A timeout already spent the per-client deadline,
  so the old 3× fetch-retry tripled the wall-clock against a black-holed remote; a
  timeout now surfaces immediately. Fast transient failures (DNS, dropped connection)
  still retry. (`docs/audit-2026-07.md` R6.)
- **A failed `git_clone` cleans up its partial `dest`.** A clone that fails midway
  (timeout, network, auth) left a partial, non-empty `dest` that blocked a retry with
  "destination already exists". It now removes a `dest` it could have created (absent,
  or an empty directory) on failure, but **never** a non-empty pre-existing directory
  (the caller's data, which jj/git refuse to clone into). Mirrors `vcs-git`.
  (`docs/audit-2026-07.md` R7.)

## [0.6.0] - 2026-06-27

### Added
- `Jj::with_retry(RetryPolicy)` — opt-in retry of jj **working-copy lock**
  contention, with exponential, jittered backoff. Off by default; safe even for
  mutating commands (a lock-acquisition failure is pre-execution). Re-exports
  `RetryPolicy`. (jj's operation log already auto-resolves most concurrency, so
  hard lock failures are rarer than with git.) Internally `Jj` now wraps a
  `ManagedClient` — no change to existing methods.

### Changed
- Documented that **jj remote authentication is ambient**: unlike `vcs-git`'s new
  per-operation `with_credentials` token provider, `jj`'s in-process git backend
  offers no per-invocation credential override, so `jj git fetch`/`push`
  authenticate from the ambient git credential helpers / SSH agent. (The shared
  `vcs-cli-support` credentials seam documents this; `jj` adds no injection.)
- Bumped `processkit` to **0.11.0** (from 0.9.1), a major breaking release ahead
  of processkit's 1.0 freeze. Breaking for downstream via the re-exported
  `processkit::Error`: `Error::Timeout`/`Signalled` now carry partial
  `stdout`/`stderr`, `Error::Signalled`/`NotFound`/`CassetteMiss` are first-class
  variants, the blanket `From<io::Error>` is gone, and `Invocation::cwd` is now
  `Option<PathBuf>`.
- `bookmarks()` now reads `jj bookmark list` through an explicit `-T` template
  (`name\t<commit>`) instead of scraping jj's human-readable default output (which
  interleaves the change id, description, and indented remote-tracking lines). Same
  `Vec<Bookmark>` result, but robust against jj display-format drift — matching how
  `bookmarks_all`/`reachable_bookmarks` already parse templated rows.

### Removed
- The **`cancellation`** feature — cancellation is always available now
  (processkit 0.10 made it core), so the `cli_client!`-generated
  `default_cancel_on(token)` and the re-exported `CancellationToken` no longer sit
  behind a feature. Downstream that enabled `vcs-jj/cancellation` should drop it.

### Fixed
- `git_push` and `git_clone` now apply the same `timeout_grace` window as
  `git_fetch`: on a per-client timeout the process tree is terminated gracefully
  (then hard-killed after the grace window) so a timed-out push doesn't
  half-update the remote ref and a timed-out clone can clean up its partial
  destination. A no-op when no `default_timeout` is set.
- `parse_diff_summary` no longer reports a self-rename: a malformed `R`/`C` path
  with no `{old => new}` brace form (jj always renders renames with it) expanded to
  `old == new` and set `old_path == path`; it now sets `old_path` to `None`, so
  `old_path != path` stays a reliable "is this a real rename?" test for consumers.
- `resolve_list`'s "no conflicts" detection (the benign non-zero exit) matches the
  stable core phrase **case-insensitively**, absorbing a jj capitalization/wording
  change rather than surfacing a conflict-free revision as an error. (jj output is
  English-only, so the risk is version wording, not locale.)
- The jj conflict parser's region terminator now requires the structural
  `conflict N of M ends` form only — the loose `ends_with("ends")` fallback was
  removed, so a content line that is a run of exactly the marker length followed by
  a word ending in "ends" can't be mistaken for the end marker.
- `bookmarks_all` (`jj bookmark list --all`) now drops a row whose name field is
  empty instead of yielding a phantom `BookmarkRef { name: "" }`, matching how
  `bookmarks`/`workspaces`/`reachable_bookmarks` already reject empty-name rows.
- `file_annotate` keeps a CRLF source line's trailing `\r` in the annotation
  content — parsing now splits on `\n` instead of `str::lines()`, which silently
  stripped the `\r`. Line numbering is unchanged (the trailing-newline artifact
  carries no tab and is dropped).
- The jj conflict parser now validates the `\\\ to:` line's marker-run length
  against the region's marker length (mirroring the `%%%%%%%` gate); a `to:` line
  with a mismatched run is rejected as malformed rather than silently accepted.

## [0.5.0] - 2026-06-08

### Added
- `description(dir, revset)` — the full (multiline) description of the commit a
  revset resolves to (`log --limit 1 -T description`); empty for an undescribed
  change, newest commit only (log order) for a multi-commit revset.
- `git_fetch_from(dir, remote)` — fetch from a *named* git remote
  (`git fetch --remote <remote>`), retried on transient failures like
  `git_fetch`.
- `Jj::transaction(dir, |tx| …)` (also on `JjAt`) — run a mutation sequence with
  op-log rollback: captures `op_head`, hands the closure a bound `JjAt`, and
  restores the captured operation when the closure returns `Err`. Inherent (not
  on the trait — generic closures aren't mockable); rollback runs on `Err` only,
  not on panic/cancellation.
- `git_clone(url, dest, colocate)` — `jj git clone` without a working
  directory (pass an absolute `dest`). The colocate flag is always passed
  explicitly (`--colocate`/`--no-colocate`): jj's default flipped across
  versions and is overridable via `git.colocate` config.
- `absorb(dir, from, filesets)` — fold working-copy edits into the mutable
  ancestors that introduced the touched lines; empty `filesets` absorbs
  everything.
- `split_paths(dir, filesets, message)` — carve named filesets out of `@` into
  their own described commit (the `-m` keeps it non-interactive). Empty
  `filesets` are refused before spawning — a fileset-less `jj split` opens the
  interactive diff editor, a headless hang.
- `duplicate(dir, revset)`.
- `op_log(dir, limit)` → `Vec<Operation>` (id/user/start-time/description) —
  the listing counterpart of `op_head`.
- `evolog(dir, revset, max)` → `Vec<Change>` — how a change evolved, newest
  snapshot first. (Evolog templates render in a *commit* context, so this uses
  a `commit.`-method-form template, unlike `log`.)
- `file_annotate(dir, path, rev)` → `Vec<AnnotationLine>` (change id + 1-based
  line + content) and `file_show(dir, revset, path)` — file content at a
  revision (lossy for binary). `file_show` wraps the path as an exact-path
  fileset (`file:"…"`) so fileset metacharacters stay literal; `file_annotate`
  deliberately doesn't — `jj file annotate` takes a plain path and rejects the
  quoted form.
- `capabilities()` → `JjCapabilities { version: JjVersion }` — the installed
  binary's parsed version (tolerates `-dev`/build-hash suffixes), with
  `is_supported()` / `ensure_supported()` gating **precisely** on jj ≥ 0.38,
  the empirically validated floor (jj's CLI moves fast; every parser and flag
  in this crate was verified against that release). A value type: probe once
  and keep it.
- Injection guards on the exposed positional arguments (bookmark names,
  positional revsets, `new_merge` parents, operation ids, the `git_clone`
  url, the `bookmark_track` `name@remote` token): a leading-`-` or empty
  value is refused **before** anything spawns; `file_annotate`'s path goes
  after a `--` separator. Flag-value positions (`-r`, `-m`) need no guard —
  jj's CLI rejects dash-values there itself.
- `RevsetExpr` validating newtype — optional up-front validation for
  untrusted input (non-empty, no leading `-`; the full revset grammar is
  deliberately not modelled). Method signatures stay `&str`.
- `conflict` module — a typed model of jj's **materialized** conflicts
  (native `diff` and `snapshot` marker styles, `conflict N of M` counters,
  marker-length matching): `parse_conflicts` → segments, byte-exact
  `render`, `resolve(…, JjResolution::{Side(n),Base})` — for `diff` style
  the side content is reconstructed by applying the recorded diff. Files
  materialized with the `git` marker style parse via `vcs_git::conflict`
  (documented asymmetry).
- Doc note: there is deliberately no `Jj::hardened()` — jj has no repo-local
  hooks; in a colocated repo the risk lives on the git side, so harden the
  `Git` client instead.
- `Jj::workspace_roots(dir, names)` — resolve several workspaces' roots in one
  **bounded fan-out** (processkit 0.8 `output_all`, ≤ 8 concurrent `workspace
  root --name <n>` calls) instead of awaiting them one by one; per-name `Ok`/`Err`
  mirrors `workspace_root`, results in input order. Inherent (throughput shape
  over the trait method). The facade's worktree enumeration (`Repo::list_worktrees`)
  uses it.

### Changed
- `squash_paths(dir, from, into, filesets, use_destination_message)` now takes a
  single `SquashPaths` spec — `squash_paths(dir, SquashPaths::new(from, into)
  .filesets(…).use_destination_message())` — mirroring `WorkspaceAdd`. *Breaking*
  for the `squash_paths` signature; argv is byte-identical.
- Bumped `processkit` to **0.8** — the re-exported `Error`/`ProcessResult` carry
  through 0.8 (`Error` still `#[non_exhaustive]` with `NotReady`/`Unsupported` and
  feature-gated `Cancelled`/`ResourceLimit`; `Error::Exit` Display gained a
  stderr-tail suffix; `Command` is `#[must_use]`). **Breaking** for consumers that
  match the re-exported types exhaustively, or that bump their own direct
  `processkit` separately — caret `"0.7"` does not span 0.8, so bump together.
- Internal: the `CliClient` verbs the wrapper bodies call were renamed to one
  shared vocabulary (`text`→`run`, `capture`→`output`, `unit`→`run_unit`,
  `code`→`exit_code`); no public-API or built-argv change.
- New off-by-default **`cancellation`** feature: pulls in processkit's
  `cancellation`, so `cli_client!` emits `default_cancel_on(token)` on the client —
  build a cancellable client (every command it runs dies when the token fires) and
  pass it through the facade. No new vcs-* API; `CancellationToken` is re-exported
  from `processkit`.
- Internal: the diff model + parser (`ChangeKind`/`DiffLine`/`Hunk`/`FileDiff`/
  `DiffStat`/`parse_diff`) and the version type now come from the shared
  `vcs-diff` crate, and the transient-fetch classifier + the argv injection guard
  from `vcs-cli-support` — both re-exported, so the public API is unchanged
  (`vcs_jj::FileDiff`, `vcs_jj::is_transient_fetch_error`, … still resolve;
  `JjVersion` is now an alias of `vcs_diff::Version`). Removes the byte-identical
  duplication with `vcs-git`. `parse_diff` is now part of the public surface.

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.11.0...HEAD
[0.11.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.10.0...vcs-jj-v0.11.0
[0.10.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.9.2...vcs-jj-v0.10.0
[0.9.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.9.1...vcs-jj-v0.9.2
[0.9.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.9.0...vcs-jj-v0.9.1
[0.9.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.8.0...vcs-jj-v0.9.0
[0.8.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.7.0...vcs-jj-v0.8.0
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.6.0...vcs-jj-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.5.0...vcs-jj-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.4.0...vcs-jj-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.1...vcs-jj-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.0...vcs-jj-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.1...vcs-jj-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.0...vcs-jj-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.1.0...vcs-jj-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-jj-v0.1.0
