# Changelog — vcs-git

All notable changes to the `vcs-git` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-git-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
- security: `GitApi::config_set` now passes its argv as `git config -- <key> <value>`,
  pinning `key` and `value` behind the `--` option terminator. A `value` shaped like
  a flag (`--global`, `--file=<path>`, `--worktree`, …) is stored literally instead of
  being reparsed by git as an option that could redirect the write to an arbitrary
  config file. A legitimate `-`-leading value (e.g. `-1`) is still accepted — the flag
  *parse* is blocked, not the leading dash. (T-083.)

## [0.11.0] - 2026-07-19

### Added

- feat: add `GitApi::am_continue` — `git am --continue` to resume an interrupted
  mailbox apply after resolving a patch's conflict, completing the `git am` driver
  pair alongside `am_abort`. Like the other sequencer `--continue`s it suppresses
  the editor (`GIT_EDITOR=true`) so a headless caller never hangs on the
  message-confirm, and runs under the C locale so a next-patch re-conflict still
  feeds `is_merge_conflict`. Mirrored on the `GitAt` cwd-bound view. Used by
  `vcs-core`'s `Repo::continue_in_progress`, which previously left an in-progress
  `am` as a silent no-op. (T-065.)
- feat: add `Git::merge_abort_detached` and `Git::is_merge_in_progress_detached` —
  the `merge --abort` rollback **cleanup** and the "is a trial merge still staged?"
  decision that gates it, each run on a fresh cancellation token (not the client's
  `default_cancel_on`) and under its own bounded deadline, mirroring jj's
  `Jj::rollback_to`. A cancelled or timed-out probe merge can then still be undone
  rather than left staged, because **neither** the decision nor the cleanup inherits
  the fired token — previously only the abort command was detached, so a cancelled
  `is_merge_in_progress` probe could still skip the abort. Same `merge --abort` /
  `rev-parse --git-dir` argv as `GitApi::merge_abort` / `GitApi::is_merge_in_progress`;
  used by `vcs-core`'s `Repo::try_merge` cleanup branches. (T-059.)

### Changed
-

### Fixed
- **Docs:** the crate-level `# Safety` rustdoc and `docs/security.md` claimed
  *every* revision/range input goes through the validated `RevSpec` newtype;
  `DiffSpec::Rev` (`diff_text`/`diff`, `diff_text_within`/`diff_within`) never
  did — it's a bare `String` from the shared `vcs-diff` crate, guarded
  per-call by an inline `reject_flag_like` (plus a trailing `--`) inside
  `diff_text_budgeted` instead. Docs now name it as the one exception; no
  behaviour change. (T-081.)

## [0.10.0] - 2026-07-10

### Added

- feat: model the remaining paused-sequencer states on `GitApi`. New detection
  probes `is_cherry_pick_in_progress` / `is_revert_in_progress` /
  `is_bisect_in_progress` (keyed off `CHERRY_PICK_HEAD` / `REVERT_HEAD` /
  `BISECT_LOG` under the git dir), and the matching drivers `cherry_pick_abort` /
  `cherry_pick_continue` / `revert_abort` / `revert_continue` / `bisect_reset`.
  The two `--continue` commits suppress the editor (`GIT_EDITOR=true`) so a
  headless caller never hangs, and run under the C locale so a re-conflict still
  feeds `is_merge_conflict`. A cherry-pick/revert conflict writes its own head
  file, **not** `MERGE_HEAD`, so these stay distinct from a merge. (T-044.)
- feat: host-keyed credential resolution for remote ops. When the operation's
  target host is known — `clone` derives it from the URL — it is now passed as the
  `CredentialRequest`'s host (alongside the existing `credential.helper` host gate),
  so a **host-keyed** `CredentialProvider` hands each op only that host's secret. One
  `Git` client can safely drive several hosts (each clone draws its own token, never
  a neighbour's); a provider `Err` is fail-closed (the op aborts before git spawns),
  while `Ok(None)` defers to ambient auth. (T-045.)
- feat: add `Git::empty_tree_oid` — the empty-tree object id for a repository's
  **active object format**, computed via `git hash-object -t tree --stdin` (an
  empty tree is empty content; `--stdin`, not `-w`, only computes the id). This is
  the format-correct stand-in for `HEAD` when diffing/stat-ing an unborn
  (no-commits-yet) working tree, and unlike the old constant it is also correct in
  a SHA-256 repo, where the SHA-1 empty-tree id does not exist. (T-043.)
- feat: add `GitApi::log_paths` — like `log`, but scoped to commits that
  touched the given paths (`git log <revspec> -n <max> -- <paths>`), with the
  same `--` pathspec separator as `add`/`commit_paths` and a refusal of an
  empty path list before spawning.
- test: lock in the `remote_branch_exists` bounded wait — a hung `ls-remote`
  resolves via its per-command 10 s timeout rather than hanging (pins the
  processkit 2.1 guarantee that a scripted pending reply honors `Command::timeout`
  on bulk verbs).

### Changed

- **Breaking:** path-carrying results are now lossless for non-UTF-8 names.
  `StatusEntry.path` / `StatusEntry.old_path` are `PathBuf` / `Option<PathBuf>`
  (were `String` / `Option<String>`), and `GitApi::conflicted_files` returns
  `Vec<PathBuf>` (was `Vec<String>`). `status` / `status_tracked` / `conflicted_files`
  now parse the `-z` output from **raw bytes** (`parse_porcelain` / `parse_nul_paths`
  consume `&[u8]`) via the new `ManagedClient::parse_bytes`, so a filename whose bytes
  are not valid UTF-8 (legal on Unix) survives byte-for-byte and can be fed straight
  back into `add` / `commit_paths` (which already take `PathBuf` through the NUL-safe
  pathspec transport) to address the SAME file. `worktree_list` (`Worktree.path`,
  already a `PathBuf`) now parses `worktree list --porcelain` from raw bytes too, so a
  worktree whose directory name is not valid UTF-8 survives losslessly into the
  facade's `WorktreeInfo.path` instead of collapsing to `U+FFFD` (no `-z`: that
  porcelain variant needs git ≥ 2.36, above the crate's 2.31 support floor — the
  newline framing already covers the non-UTF-8 case). Text-only machine output (branch
  names, commit metadata, hashes) still decodes as `String`, where lossy decoding is
  acceptable. `FileDiff.path` / `old_path` are `PathBuf` too (via `vcs-diff`). (T-050.)
- deps: bump `mockall` to 0.15 (unified workspace dependency, was 0.13 per-crate).
- **Breaking:** the public empty-tree constant `EMPTY_TREE` is renamed
  `EMPTY_TREE_SHA1` and documented as **SHA-1 only**. It presented git's SHA-1
  empty tree as a universal id, but that value does not exist in a repo created
  with `extensions.objectFormat=sha256` — so `diff_text(DiffSpec::WorkingTree)`
  and the facade `diff_stat` hard-failed on an *unborn* SHA-256 repo (they diffed
  the working tree against a non-existent object). Both now resolve the id from
  git via the new `Git::empty_tree_oid`, so the unborn working-tree diff/stat
  works under either object format; behaviour after the first commit (the `HEAD`
  path) is unchanged. Migrate `vcs_git::EMPTY_TREE` to `vcs_git::EMPTY_TREE_SHA1`
  for the SHA-1-only literal, or to `git.empty_tree_oid(dir).await?` for the
  object-format-correct id. (T-043; `docs/audit-2026-07.md` L6.)
- **Breaking:** the raw escape hatches on the bound view (`GitAt::run`/`run_raw`/
  `run_args`/`run_raw_args`) now run **in the bound `dir`** instead of the process's
  current directory. Previously they sat in the `bare` forwarder group, so
  `git.at(dir).run(…)` silently ran in the process cwd — a bound handle whose raw
  call could target a *different* repository than the one it was bound to. New
  dir-taking client methods `Git::run_in`/`run_raw_in`/`run_args_in`/`run_raw_args_in`
  back the bound forwarders (argv forwarded verbatim; only the cwd is bound). The
  **process-cwd** escape hatch is unchanged and still reached by calling
  `run`/`run_raw`/… on `Git` itself (`git.run(…)`) — migrate a caller that relied on
  `git.at(dir).run(…)` running in the process cwd to `git.run(…)`. (Supersedes the
  M15 docs-only note below; T-035.)
- fix: distinguish an attached branch with no configured upstream from Git
  errors in `GitApi::upstream`; detached HEAD and directories outside a Git
  repository now return `Err` instead of `Ok(None)`.
- **Breaking:** reference names and revision expressions are now taken as the
  validated newtypes `RefName` / `RevSpec` (previously constructible but accepted
  by no method — a false safety promise). Every `GitApi` op that names a branch,
  tag, or ref to create/delete/rename/look-up now takes `&RefName`; every op that
  resolves a commit-ish or range takes `&RevSpec`; the option structs follow
  (`BranchDelete::new(RefName)`, `MergeCheck::branch(RefName).into_base(RevSpec)`,
  `MergeCommit`/`MergeNoCommit::branch(RevSpec)`, `AnnotatedTag::new(RefName,…)
  [.rev(RevSpec)]`, `GitPush::branch(RefName)` / `refspec(&RefName, &RefName)`,
  `WorktreeAdd::create_branch(path, RefName, RevSpec)` / `checkout(path, RevSpec)`,
  `tag_create(&RefName, Option<RevSpec>)`, `blame(path, Option<RevSpec>)`). A
  flag-like or malformed value is now rejected at newtype construction, before it
  can reach an argv slot, as a classifiable `Error::is_invalid_input`. Migrate a
  call by wrapping the string: `git.checkout(dir, "main")` →
  `git.checkout(dir, &CheckoutTarget::Ref(RevSpec::new("main")?))`,
  `git.create_branch(dir, "feat")` → `git.create_branch(dir, &RefName::new("feat")?)`.
- **Breaking:** `checkout` now takes a `CheckoutTarget` enum instead of `&str`, so
  git's `-` "previous branch" shortcut is modelled explicitly as
  `CheckoutTarget::Previous` (a safe fixed literal) rather than being rejected by
  the flag guard. `switch_with_stash` takes a `CheckoutTarget` for the same reason.
  Remaining bare-positional `&str` inputs that are not refs/revisions (remote
  names, URLs, config keys) keep their internal `reject_flag_like` guard.
- **Breaking:** replace the trailing positional `bool` on three `GitApi` methods
  with named `#[non_exhaustive]` specs, so the flag reads at the call site and can
  grow without a signature break: `delete_branch(dir, name, force)` →
  `delete_branch(dir, BranchDelete::new(name)[.force()])`, `stash_push(dir,
  include_untracked)` → `stash_push(dir, StashPush::new()[.include_untracked()])`,
  and `worktree_remove(dir, path, force)` → `worktree_remove(dir,
  WorktreeRemove::new(path)[.force()])`. The `GitAt` bound view and the sync
  `blocking::worktree_remove(dir, WorktreeRemove)` helper move to the same specs.

### Fixed
-

## [0.9.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(m29): git support gate enforces the real (2, 31) floor the crate's argv needs, not major-only
- fix(git): rev_parse_short --verify + diff verbs terminate revisions with -- (pathspec-collision hardening, C2/M13 class)


### Changed

- refactor(a5): is_merged takes a MergeCheck spec so the two refs can't be transposed (A5)
- review(0.4.0): whole-solution followups — MergeCheckPartial rename, is_merged test, mcp/core changelogs
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(review): branch-listing color-safety (git); try_merge always rolls back (core)
- fix(review): C-locale on git diff_stat; --color never on jj blocking workspace probe
- fix(git): force C locale on merge_squash so is_merge_conflict survives a non-English git
- fix(git): force C locale on stash_pop so a stash-pop conflict is classified under a non-English git
- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): content verbs preserve trailing bytes (H7)
- fix(wave2): don't retry a fetch timeout (avoids 3x deadline amplification) (R6)
- fix(wave2): clean a partial dest after a failed clone so a retry isn't blocked (R7)
- fix(wave2): porcelain worktree-rename parse + rev_parse --verify (M11/M13)
- fix(wave2): harden scrubs GIT_PROXY_COMMAND/EXEC_PATH/TEMPLATE_DIR/PATHSPECS + reject push refspec metachars (M14/M16)
- fix(wave2): detect cherry-pick/revert/bisect/am state; don't rebase-abort a git am (M20)
- fix(wave2): switch_with_stash pops only its own stash, with --index (M12)


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
- refactor(api): git current_branch -> Option; gitlab mr id -> number (pre-1.0 consistency)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(cli-support): tighten lock-retry markers, credential robustness, flag-guard hardening
- fix(git): harden() scrubs env command-hooks; config_get preserves trailing whitespace
- fix(git): current_branch handles unborn repos via symbolic-ref
- fix(git): blame on SHA-256 repos; remote_head_branch/upstream surface timeouts


### Added

- feat: typed description/fetch_from/conflicted_files/status_tracked + facade surface
- feat: orchestration primitives — jj transaction, try_merge, abort/continue, switch_with_stash
- feat: client coverage — git clone/tags/show/blame/config, jj clone/absorb/split/op_log/evolog/annotate
- feat: vcs-testkit crate, version capabilities, observation docs
- feat: injection guards + validating newtypes, Git::hardened, typed conflict model
- feat(core): batched Repo::snapshot + maturity docs (Wave C)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts
- fix(docs+ci): text-fence two guide signature shapes; scope integration job to --tests (stop --ignored doctest sweep)


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

- review: fix potential issues across vcs-git/vcs-jj expansion
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
- **`Git<R>` now implements `Debug`**, via the shared `vcs_cli_support::managed_client!`
  macro (no code change here). No `R: Debug` bound; the wrapped client's
  credentials are never printed, only whether one is configured.

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
- **`rev_parse_short` passes `--verify`, matching `rev_parse`/`resolve_commit`.**
  It pins the single-object contract explicitly: `rev` must name exactly one
  object or the call errors. (`--short` already rejects a plain path with `Needed
  a single revision` — unlike the bare `rev-parse` that echoed a filename as a
  fake id, M13 — so this is consistency / defense-in-depth, not an
  observable-behavior fix.) (`docs/audit-2026-07.md` M13.)
- **The range/rev-taking diff verbs terminate their argv with `--`.**
  `diff_range_is_empty`, `diff_stat`, and `diff_text` passed the caller's
  `range`/revision as a bare positional, so a value that named a tracked path fell
  into git's *pathspec* mode — `diff_range_is_empty("Makefile")` reported the
  working-tree state of that file instead of erroring, and `diff_text`/`diff` for
  such a `Rev` diffed the working tree rather than the commit. The trailing `--`
  forces a revision reading; an unresolvable one now errors honestly. (Same
  pathspec-collision class as C2's `checkout` fix.)
- **`GitCapabilities::ensure_supported`/`is_supported` now enforce the real `2.31`
  floor (major.minor), not just the major.** The gate is documented to turn a too-old
  git into a clear "needs git ≥ X" error instead of a cryptic argv failure — but it
  only checked `major >= 2`, so git 2.7 *passed* and then broke on `branch_status`/
  `snapshot` (`status --porcelain=v2`, 2.11), `switch_with_stash` (`stash push`, 2.13),
  and `harden()` (`GIT_CONFIG_COUNT`, 2.31). It now gates on `(major, minor) >= (2, 31)`
  — the highest version the crate's own argv requires. (`docs/audit-2026-07.md` M29.)

## [0.8.0] - 2026-07-03

### Added
- `MergeCheck` (+ its partial builder `MergeCheckPartial`) — the spec that `is_merged`
  now takes.

### Changed
- **`GitApi::is_merged` takes a `MergeCheck` spec, not two bare `&str` refs
  (breaking).** `is_merged(dir, branch, target)` had two adjacent same-typed refs that
  compiled when transposed and **inverted** the answer (asking "is `target` merged into
  `branch`"). It's now `is_merged(dir, MergeCheck::branch("feature").into_base("main"))`
  — the branch and the base are named across two builder steps, so a swap can't compile
  silently. Emitted `git branch --merged <base>` is unchanged. (`docs/audit-2026-07.md`
  A5.)

### Fixed
-

## [0.7.0] - 2026-07-03

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_git::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.
- **`is_am_in_progress`** (a `git am` mailbox-apply is paused — `rebase-apply/applying`)
  and **`am_abort`** (`git am --abort`). A `git am` shares the `rebase-apply/` dir with
  an apply-backend rebase but marks it `applying`; these let a caller detect and abort
  it distinctly. (`docs/audit-2026-07.md` M20.)

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` types (`Error`/`ProcessResult`/…)
  1.x — **breaking** for a downstream that pins `processkit` `0.x` directly. No
  behaviour change (processkit's text-capture verb is now `output_string`, used
  internally). processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.
- `DiffSpec` is now a re-export of `vcs_diff::DiffSpec` (hoisted to the shared
  crate so `vcs-git`/`vcs-jj` share one definition; `vcs_git::DiffSpec` still
  resolves) and is no longer `#[non_exhaustive]`, so a `match` over it can be
  exhaustive. Requires `vcs-diff` ≥ the version that introduces `DiffSpec`.
- **Renamed `GitApi::fetch_remote_branch` → `fetch_branch` (breaking).** The
  single-branch fetch (and its `at(dir)` bound form) is now `fetch_branch`, so
  git exposes a consistent `fetch`/`fetch_from`/`fetch_branch` family; the emitted
  `git fetch --quiet origin <refspec>` command is unchanged. Update callers of
  `fetch_remote_branch` to `fetch_branch`.
- **Every git client now scrubs the inherited repo-redirector env vars**
  (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`,
  `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_NAMESPACE`), not
  just [`harden()`](Git::harden). A `GIT_DIR` leaking from the parent process (e.g.
  running inside a git hook) can no longer silently retarget commands at a
  *different* repository than the bound `dir`. (`docs/audit-2026-07.md` H4.)
- **`harden()` now documents its git ≥ 2.31 requirement prominently.** On older git
  the hook/`fsmonitor`/`sshCommand` config-pins silently no-op (they ride
  `GIT_CONFIG_COUNT`, added in 2.31); the doc now says so and tells you to check
  `capabilities().version` (major/minor) yourself, since there is no built-in 2.31
  gate yet. (`docs/audit-2026-07.md` H3.)

### Fixed
- **`checkout` can no longer silently discard unstaged edits.** It now passes a
  trailing `--`, so a `reference` that doesn't resolve as a ref but names a tracked
  path errors instead of falling into git's *pathspec* mode and restoring that path
  from the index (which reverted unstaged edits and returned `Ok`).
  (`docs/audit-2026-07.md` C2.)
- **`conflict::parse_conflicts` no longer rejects marker-like content.** A
  `=======`/`>>>>>>>` run *outside* a conflict region (a Markdown/RST setext
  underline, a divider banner, a quoted email) is kept as text instead of erroring,
  so programmatic conflict resolution works on files that merely contain
  marker-like lines. Only a genuinely broken region (an opener with no
  separator/terminator) still errors. (`docs/audit-2026-07.md` H6.)
- **`with_retry` lock-contention retry now fires on a non-English runner.** The git
  `is_lock_contention` marker is the locale-stable `index.lock` path fragment (not
  the translated `': File exists'` suffix), with a `refs/` guard that still excludes
  per-ref locks (unsafe to retry mid-way through a multi-ref push/fetch).
  (`docs/audit-2026-07.md` H2.)
- **`show_file` and `diff_text` no longer strip trailing bytes.** They return git's
  output **verbatim** (via `run_untrimmed`) instead of `trim_end`, so a blob's
  trailing newline(s) survive a read-modify-write round-trip and a diff's last hunk
  stays in sync with its `@@` line count. (Behavior change: a caller that relied on
  the old trimming should trim itself.) (`docs/audit-2026-07.md` H7.)
- **A `fetch` that times out is no longer retried** (inherited from cli-support's
  `is_transient_fetch_error` change). A timeout already spent the per-client deadline,
  so the old 3× fetch-retry blocked ≈ 3× the configured ceiling against a black-holed
  remote; a timeout now surfaces immediately. Fast transient failures (DNS, dropped
  connection) still retry. (`docs/audit-2026-07.md` R6.)
- **A failed `clone_repo` cleans up its partial `dest`.** A clone that fails midway
  (timeout, network, auth) left a partial, non-empty `dest` that blocked a retry with
  "destination path already exists and is not empty" (`timeout_grace` can't prevent it
  — Windows' job-kill is atomic, the Unix grace too short for a large partial). It now
  removes a `dest` it could have created (absent, or an empty directory) on failure,
  but **never** a non-empty pre-existing directory (git refuses to clone into one, so
  the caller's data is untouched). (`docs/audit-2026-07.md` R7.)
- **`switch_with_stash` no longer pops an unrelated stash or flattens the index.** If
  `stash push` exited 0 having saved **nothing** (e.g. a submodule-only change that
  `status` still reports as dirty), the following bare `stash pop` splatted an older,
  unrelated pre-existing stash — data loss. It now checks the stash-list depth around
  the push and pops only when the push actually saved, and pops with **`--index`** so
  the staged/unstaged split is restored faithfully (a bare `pop` returned everything
  unstaged). Documents the single-actor contract. (`docs/audit-2026-07.md` M12.)
- **`status` no longer emits a phantom entry for a worktree rename.** `parse_porcelain`
  only consumed a rename/copy's source record when `R`/`C` sat in the **index** column
  (`R `); git also emits it in the **worktree** column (` R`, ` C`), whose source path
  then leaked out as a bogus `StatusEntry` with a garbage code/path. Both columns are
  now checked. (`docs/audit-2026-07.md` M11.)
- **`rev_parse` now passes `--verify`, so a non-revision errors instead of resolving
  to a fake id.** `git rev-parse Makefile` (a filename, not a rev) exited 0 echoing
  `"Makefile"` back; `rev_parse` now requires `rev` to name exactly one object (a valid
  revision still resolves to the same full hash). Matches `rev_parse_short` /
  `resolve_commit`, which already verify. (`docs/audit-2026-07.md` M13.)
- **Docs:** `GitApi::run` now documents that it (and the `run*`/`run_args` escape
  hatches) execute in the **process's current directory** — the `at(dir)` bound view
  does *not* re-bind them, unlike every modelled `GitAt` method. Pass `-C <dir>` to
  target the bound repo. (`docs/audit-2026-07.md` M15.)
- **`is_rebase_in_progress` no longer reports a `git am` as a rebase.** `git am` uses
  the same `rebase-apply/` dir as an apply-backend rebase but adds an `applying` marker;
  `is_rebase_in_progress` now excludes that case (it's an am — see `is_am_in_progress`),
  so a facade won't abort an in-progress `git am` with `rebase --abort`.
  (`docs/audit-2026-07.md` M20.)

### Security
- **Per-operation credentials are scoped to the clone URL's host.** With a
  credential provider set, `clone_repo` now binds the inline `credential.helper` to
  the target URL's host, so an HTTP redirect or a submodule fetch to a *different*
  host during the clone can't extract the token. Other remote ops (fetch/push)
  remain host-ungated for now (they target a configured remote). (`docs/audit-2026-07.md` H5.)
- **`harden()` scrubs more env code-execution vectors.** Added `GIT_PROXY_COMMAND`
  (runs an arbitrary program for a `git://` connection), `GIT_EXEC_PATH` (relocates
  where git finds its own sub-commands), and `GIT_TEMPLATE_DIR` (seeds hooks/config
  into a repo on `init`/`clone`) to the scrub list, plus the pathspec-mode vars
  (`GIT_LITERAL_PATHSPECS` / `GIT_GLOB_PATHSPECS` / `GIT_NOGLOB_PATHSPECS` /
  `GIT_ICASE_PATHSPECS`), which silently change which paths a command matches.
  (`docs/audit-2026-07.md` M14.)
- **`push` refuses a force (`+`) or multi-ref (`:`) metacharacter smuggled into a
  branch name.** `GitPush::branch("+main")` (or `"a:b:c"`) previously rode through the
  argv guard and became a **force-push** / a push to an unexpected ref. `push` now
  rejects a leading `+` and more than one `:` before spawning; a legitimate
  `local:remote` refspec still works, and a real force-push must be explicit via
  `run(["push", "--force", …])`. (`docs/audit-2026-07.md` M16.)

## [0.6.0] - 2026-06-27

### Added
- **Per-operation HTTPS credentials (opt-in).** `Git::with_credentials(provider)`
  accepts a `CredentialProvider` (re-exported from `vcs-cli-support`, with
  `Credential`/`Secret`/`StaticCredential`/`EnvToken`/`provider_fn`), plus the
  convenience `Git::with_token(token)` / `with_env_token(var)` for the common cases.
  When the provider yields a credential, every remote op (`fetch`/`fetch_from`/
  `fetch_remote_branch`/`push`/`clone_repo`/`remote_branch_exists`/
  `remote_branches`) runs with a leading inline `credential.helper` that feeds the
  secret from an environment variable — so the token never appears in `argv`.
  Default is no provider → ambient git credential helpers / SSH agent, unchanged.
- `Git::with_retry(RetryPolicy)` — opt-in retry of **whole-repo lock-contention**
  failures (another process holds the repo's `index.lock`), with exponential,
  jittered backoff. Off by default; safe even for mutating commands because that
  lock is acquired pre-write (the command never ran). Per-ref lock failures are
  *not* retried (a multi-ref op can fail a ref lock mid-way). Re-exports `RetryPolicy`.
  (Internally `Git` now wraps a `ManagedClient` instead of a bare `CliClient` —
  no change to existing methods.)

### Changed
- **`GitApi::log` unified (breaking).** `log(dir, max)` + `log_range(dir, range, max)`
  collapse into one `log(dir, revspec, max)` — pass `"HEAD"` for the current branch
  or a range like `"main..HEAD"`. Mirrors `JjApi::log`'s revset argument so
  cross-backend code shares one signature; the `revspec` is guarded against being
  parsed as a flag.
- **`StatusEntry::orig_path` renamed to `old_path` (breaking)** — matches
  `vcs_jj::ChangedPath::old_path`, so the rename source reads the same on both wrappers.
- **`GitApi::current_branch` now returns `Result<Option<String>>` (breaking)** —
  `None` on a detached HEAD instead of the literal string `"HEAD"`. Mirrors
  `JjApi::current_bookmark`'s `Option` shape, so cross-backend code treats "no named
  branch/bookmark" identically (and the `vcs-core` facade forwards it directly
  instead of remapping `"HEAD"` → `None`). Now backed by
  `git symbolic-ref --quiet --short HEAD` (exit 0 → branch, exit 1 → detached →
  `None`), which **also returns the branch name on an unborn repo** — a fresh
  `init`/`clone` before the first commit, where the previous
  `rev-parse --abbrev-ref HEAD` instead errored with exit 128.
- **`harden()` also scrubs the env-based command hooks** — `GIT_SSH_COMMAND`/
  `GIT_SSH`, `GIT_ASKPASS`, `GIT_EXTERNAL_DIFF`, `GIT_PAGER`, and
  `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR` — closing a second arbitrary-code-execution
  path (a poisoned environment making git spawn a helper) alongside the existing
  repo-redirector and config scrubbing. The opt-in `with_credentials` auth seam is
  unaffected (it injects a `credential.helper` / token env, not these variables); an
  operator who relies on an ambient `GIT_SSH_COMMAND`/`GIT_ASKPASS` for a hardened
  run should inject it per-call rather than inherit it.
- **`harden()` also pins `core.sshCommand` empty** — the *config-key* twin of the
  scrubbed `GIT_SSH_COMMAND` env var, so a poisoned **repo-local** `.git/config`
  can't run an arbitrary program for the SSH transport (env-config overrides
  repo-local config; empty falls back to the default `ssh`). The hardening docs now
  also scope the guarantee honestly: repo-local `.gitattributes`-driven
  `filter.*` smudge/clean and `diff.*.textconv` keys are *not* neutralized, so a
  fully untrusted repo still needs an OS sandbox for checkout/diff — `harden()` is
  hardening, not a sandbox.
- Bumped `processkit` to **0.11.0** (from 0.9.1), a major breaking release ahead
  of processkit's 1.0 freeze. Breaking for downstream via the re-exported
  `processkit::Error`: `Error::Timeout`/`Signalled` now carry partial
  `stdout`/`stderr`, `Error::Signalled`/`NotFound`/`CassetteMiss` are first-class
  variants, the blanket `From<io::Error>` is gone, and `Invocation::cwd` is now
  `Option<PathBuf>`.

### Removed
- The **`cancellation`** feature — cancellation is always available now
  (processkit 0.10 made it core), so the `cli_client!`-generated
  `default_cancel_on(token)` and the re-exported `CancellationToken` no longer sit
  behind a feature. Downstream that enabled `vcs-git/cancellation` should drop it.

### Fixed
- `push` and `clone_repo` now apply the same `timeout_grace` window as `fetch`:
  on a per-client timeout, the process tree is terminated gracefully (then
  hard-killed after the grace window) so a timed-out push releases its lock /
  doesn't half-update the remote ref, and a timed-out clone can clean up its
  partial destination. A no-op when no `default_timeout` is set.
- `config_get` strips only git's trailing line terminator (`\n`/`\r\n`) instead of
  all trailing whitespace, so a config value that legitimately ends in spaces or a
  tab is returned intact.
- **`blame` works on SHA-256 repositories.** The blame-porcelain header parser only
  recognised a **40-hex** (SHA-1) commit id, so on a SHA-256 repo (64-hex object ids)
  no header matched and `blame` silently returned an **empty `Vec`**. It now accepts
  both 40- and 64-hex ids.
- **`remote_head_branch` and `upstream` surface a timeout/signal instead of reporting
  it as "absent".** Both mapped *any* non-success outcome to `None`, so a timed-out or
  signal-killed run read as "no default branch"/"no upstream" rather than an error.
  `remote_head_branch` now maps exit 0 → the branch, exit 1 (the `--quiet` "unset"
  signal) → `None`, and anything else (a real failure / no exit code) errors via
  `ensure_success`; `upstream` keeps a non-zero **exit** as `None` (git uses exit 128
  for both "no upstream" and a real failure, indistinguishable by code) but surfaces a
  no-exit-code timeout/signal — matching `config_get`/`current_branch`.

## [0.5.0] - 2026-06-08

### Added
- `branch_status(dir) -> BranchStatus` — a combined branch + working-tree
  snapshot in **one** spawn (`status --porcelain=v2 --branch -z`): HEAD, branch,
  upstream, ahead/behind, and tracked/untracked/conflict counts. The cheap
  primitive behind the facade's `Repo::snapshot`. `BranchStatus` is re-exported.
- `fetch_from(dir, remote)` — fetch from a *named* remote (`fetch --quiet
  <remote>`), with the same terminal-prompt-off and transient-retry behaviour as
  `fetch`.
- `conflicted_files(dir)` — paths with unresolved merge conflicts
  (`diff --name-only --diff-filter=U -z`); empty when there are none.
- `status_tracked(dir)` — `status` minus untracked files
  (`--untracked-files=no`): "is the *tracked* tree dirty", staged or not.
- `Git::switch_with_stash(dir, branch)` (also on `GitAt`) — switch branches
  carrying uncommitted changes across via `stash push -u` → `checkout` →
  `stash pop`; a clean tree skips the stash round-trip, and a failed checkout
  pops the stash back where it was. Inherent (a composed operation, not a 1:1
  CLI verb).
- `clone_repo(url, dest, CloneSpec)` — `git clone` with a `CloneSpec` builder
  (`.branch()`, `.depth()`, `.bare()`). Runs without a working directory; pass
  an absolute `dest`. Note: git silently ignores `--depth` for a plain
  local-path source.
- Tag operations: `tag_create` (lightweight, optional rev),
  `tag_create_annotated` (`-a -m`), `tag_list`, `tag_delete`.
- `show_file(dir, rev, path)` — file content at a revision
  (`git show <rev>:<path>`); backslash separators are normalised to `/` (git
  requires it), binary content decodes lossily rather than erroring.
- `config_get(dir, key)` → `Option<String>` (`config --get`; exit 1 → `None` —
  git lumps "unset" and "no such section" together) and
  `config_set(dir, key, value)`.
- `remote_add(dir, name, url)` and `remote_set_url(dir, name, url)`.
- `blame(dir, path, rev)` → `Vec<BlameLine>` (`blame --line-porcelain`):
  per-line commit, author, epoch timestamp + tz, and content.
- Sequencer: `cherry_pick(dir, rev)`, `revert(dir, rev)` (`--no-edit` +
  headless editor backstop), and `rebase_skip(dir)` (`rebase --skip`) — mainly
  for the `apply` backend's "nothing to commit" stop; the default `merge`
  backend auto-drops emptied patches on `--continue`.
- `capabilities()` → `GitCapabilities { version: GitVersion }` — the installed
  binary's parsed version (tolerates `2.54.0.windows.1`/`-rc` shapes), with
  `is_supported()` / `ensure_supported()` gating on the major floor only
  (validated on 2.54; expected ≥ 2.30 — an untested minor is not hard-gated).
  A value type: probe once and keep it.
- Injection guards on every exposed positional argument — names, revisions,
  ranges, remotes, and **URLs** (`clone_repo`/`remote_*`: a leading-`-` url
  like `--upload-pack=<cmd>` is an RCE-class flag, refused). A caller-supplied
  value with a leading `-` (or an empty one) is rejected **before** anything
  spawns — git would parse it as a flag (`git checkout -evil` → "unknown
  switch", verified). Flag-value positions (`-m <msg>`) are unaffected.
- `RefName` and `RevSpec` validating newtypes — optional up-front validation
  for untrusted input (`check-ref-format`-shaped rules / minimal flag-shape
  rejection). Method signatures stay `&str`; the internal guards make the
  smuggling impossible either way.
- `Git::harden()` / `Git::hardened()` — an untrusted-repo execution profile
  applied to every command: hooks disabled (`core.hooksPath=/dev/null` via
  git's env-based config; verified to suppress hooks on Windows),
  `core.fsmonitor=false`, repo-redirecting `GIT_*` env scrubbed
  (`GIT_DIR`/`GIT_WORK_TREE`/config overrides/…), system config skipped,
  terminal prompts off.
- `conflict` module — a typed model of conflict markers: `parse_conflicts`
  → `Text`/`Conflict` segments (`merge`/`diff3`/`zdiff3` styles, variable
  marker size, CRLF preserved), byte-exact `render`, and
  `resolve(…, ResolutionSide::{Ours,Base,Theirs})`. Pure functions; also
  parses files materialized by jj's `git` conflict-marker style.

### Changed
- **Breaking:** four multi-option `GitApi` methods now take a spec/builder
  argument instead of positional flags, mirroring `push(GitPush)` /
  `clone_repo(.., CloneSpec)`:
  - `commit_paths(dir, paths, message, amend)` → `commit_paths(dir, CommitPaths)`
    (`CommitPaths::new(paths, message).amend()`).
  - `merge_commit(dir, branch, no_ff, message)` → `merge_commit(dir, MergeCommit)`
    (`MergeCommit::branch(name).no_ff().message(m)`).
  - `merge_no_commit(dir, branch, squash, no_ff)` →
    `merge_no_commit(dir, MergeNoCommit)`
    (`MergeNoCommit::branch(name).squash().no_ff()`).
  - `tag_create_annotated(dir, name, message, rev)` →
    `tag_create_annotated(dir, AnnotatedTag)` (`AnnotatedTag::new(name, message).rev(r)`).

  The built argv and behaviour are unchanged — only the call shape moves to the
  builder style. New types `CommitPaths`, `MergeCommit`, `MergeNoCommit`, and
  `AnnotatedTag` are exported (each `#[non_exhaustive]`).
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
  `vcs-diff` crate, and the error classifiers (`is_merge_conflict`/
  `is_nothing_to_commit`/`is_transient_fetch_error`) + the argv injection guard
  from `vcs-cli-support` — both re-exported, so the public API is unchanged
  (`vcs_git::FileDiff`, `vcs_git::is_merge_conflict`, … still resolve; `GitVersion`
  is now an alias of `vcs_diff::Version`). Removes the byte-identical duplication
  with `vcs-jj`. `parse_diff` is now part of the public surface.

### Fixed
- `diff`/`diff_text` pin the `a/`…`b/` diff prefixes (`--src-prefix`/`--dst-prefix`),
  so a user's global `diff.noprefix` / `diff.mnemonicPrefix` config can no longer
  make every parsed file silently vanish from the result.
- `branches`/`is_merged`/`tag_list` pass `--no-column`, so a user's
  `column.ui = always` (which columnates output even when piped) can no longer
  corrupt the line parsing or yield a false "not merged".
- Commands whose failure output feeds the error classifiers (the `commit`,
  `merge`, `rebase`, `cherry-pick`/`revert`, and `fetch` families) force
  `LC_ALL=C`, so a non-English locale can no longer defeat
  `is_merge_conflict`/`is_nothing_to_commit` or the transient-fetch retry.
- `show_file` normalises `\` → `/` only on Windows — on Unix a backslash is a
  legal filename byte, and the unconditional rewrite made such paths unresolvable.
- `branch_status` runs with `GIT_OPTIONAL_LOCKS=0`, so the snapshot/poll
  primitive no longer opportunistically rewrites `.git/index` — a filesystem
  watcher re-querying through it (vcs-watch) had its own query re-trigger the
  watch for a couple of extra rounds per change burst.
- `conflict::parse_conflicts`: a repeated `|`-run line inside a diff3 region is
  base **content**, not a replacement base marker — the overwrite dropped a
  line on `render`, breaking the byte-exact roundtrip (found by the roundtrip
  proptest; its seed is now committed under `proptest-regressions/`).

## [0.4.0] - 2026-06-04

### Added
- `Git::at(dir)` → `GitAt`, a cwd-bound view whose methods omit the leading `dir`
  argument (`git.at(dir).status()`), so a caller needn't thread `dir` through every
  call. The dir-taking `GitApi` stays for driving many directories from one client.
- `rev_parse_short` (`rev-parse --short <rev>`) — e.g. to label a detached HEAD.
- `push(dir, GitPush)` (git had no push): a `GitPush` builder — `branch(name)` /
  `refspec(local, remote_branch)`, `.remote(_)`, `.set_upstream()`.
- `upstream` (`@{u}`, `None` when unset), `set_upstream`, and `remote_branches`
  (`ls-remote --heads`) — the remote-tracking surface vcs-flow hand-rolled.
- `FileDiff.raw` — the verbatim per-file diff section, so a consumer can show the
  raw text without re-parsing.
- Sync `blocking::worktree_remove` for `Drop`-time cleanup that can't `.await`.

### Changed
- `merge_commit` with no message now passes `--no-edit`, and `rebase` /
  `rebase_continue` force a no-op editor (`GIT_EDITOR`/`GIT_SEQUENCE_EDITOR`), so
  a headless caller never hangs on `$EDITOR`.
- `remote_branch_exists` now queries the fully-qualified `refs/heads/<name>` — a
  bare `foo` could tail-match `bar/foo`.
- `fetch` now runs with `GIT_TERMINAL_PROMPT=0`, matching the other remote ops, so
  a credentials-needing remote fails fast instead of blocking on a prompt.
- Bumped `processkit` to 0.6. `fetch` / `fetch_remote_branch` now retry transient
  failures (3 attempts, 500 ms backoff) — the retry that consumers hand-rolled.
- The exit-code predicates (`diff_is_empty`, `diff_range_is_empty`,
  `staged_is_empty`, `branch_exists`, `is_unborn`) use processkit's `probe()` — no
  API change, but an unexpected exit code now carries the real captured output.

### Fixed
- `merge_no_commit` no longer builds the mutually-exclusive `--squash --no-ff`
  pair (which git rejects); `squash` takes precedence (it never fast-forwards).

## [0.3.1] - 2026-06-03

### Added

- feat(diff): typed diff (raw + parsed) for git and jj
- feat(git,jj): fill Phase 1 API gaps
- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics


### Changed

- review: fix potential issues across vcs-git/vcs-jj expansion
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
  (`diff <spec> --no-color --no-ext-diff -M`), and `diff(dir, DiffSpec)` returns
  a parsed `Vec<FileDiff>` (change kind, path, rename old-path, and `@@` hunks
  with per-line `DiffLine`s). The pure parser `parse::parse_diff` is public for
  parsing externally-obtained diff text. `DiffSpec::WorkingTree` diffs the working
  tree vs `HEAD`; `DiffSpec::Rev(_)` diffs a revision/range.
- API gaps consumers previously hand-rolled via `run()`: `checkout_detach`,
  `commit_paths` (partial `commit --only`, with optional `--amend`),
  `last_commit_message`, `is_unborn`, `log_range`, and `stash_push`/`stash_pop`.
  `WorktreeAdd` gains a `no_checkout()` builder (`worktree add --no-checkout`).
- Error classifiers `is_merge_conflict`, `is_nothing_to_commit`, and
  `is_transient_fetch_error` — inspect both captured streams of an `Error::Exit`
  (git writes `CONFLICT (…)` to stdout, `Automatic merge failed` to stderr) so
  callers stop string-scraping. Enabled by processkit 0.5's `Error::Exit.stdout`.
- `status_text` — raw `git status --porcelain=v1` text, the unparsed counterpart
  of `status`, mirroring `vcs_jj`.
- Inherent `Git::run_args` / `run_raw_args` taking `&[&str]`, so callers needn't
  allocate a `Vec<String>` for the `run` escape hatch.

### Changed
- Renamed `diff_shortstat` → `diff_stat` to match `vcs_jj::JjApi::diff_stat`
  (both return `DiffStat`).
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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.11.0...HEAD
[0.11.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.10.0...vcs-git-v0.11.0
[0.10.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.9.2...vcs-git-v0.10.0
[0.9.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.9.1...vcs-git-v0.9.2
[0.9.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.9.0...vcs-git-v0.9.1
[0.9.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.8.0...vcs-git-v0.9.0
[0.8.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.7.0...vcs-git-v0.8.0
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.6.0...vcs-git-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.5.0...vcs-git-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.4.0...vcs-git-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.3.1...vcs-git-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.3.0...vcs-git-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.1...vcs-git-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.0...vcs-git-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.1.0...vcs-git-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-git-v0.1.0
