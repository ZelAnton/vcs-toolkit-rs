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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.7.0...HEAD
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.6.0...vcs-jj-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.5.0...vcs-jj-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.4.0...vcs-jj-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.1...vcs-jj-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.3.0...vcs-jj-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.1...vcs-jj-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.2.0...vcs-jj-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-jj-v0.1.0...vcs-jj-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-jj-v0.1.0
