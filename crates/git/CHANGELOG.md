# Changelog — vcs-git

All notable changes to the `vcs-git` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-git-v<version>`.

## [Unreleased]

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_git::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.

### Changed
- `DiffSpec` is now a re-export of `vcs_diff::DiffSpec` (hoisted to the shared
  crate so `vcs-git`/`vcs-jj` share one definition; `vcs_git::DiffSpec` still
  resolves) and is no longer `#[non_exhaustive]`, so a `match` over it can be
  exhaustive. Requires `vcs-diff` ≥ the version that introduces `DiffSpec`.
- **Renamed `GitApi::fetch_remote_branch` → `fetch_branch` (breaking).** The
  single-branch fetch (and its `at(dir)` bound form) is now `fetch_branch`, so
  git exposes a consistent `fetch`/`fetch_from`/`fetch_branch` family; the emitted
  `git fetch --quiet origin <refspec>` command is unchanged. Update callers of
  `fetch_remote_branch` to `fetch_branch`.

### Fixed
-

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.6.0...HEAD
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.5.0...vcs-git-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.4.0...vcs-git-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.3.1...vcs-git-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.3.0...vcs-git-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.1...vcs-git-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.2.0...vcs-git-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-git-v0.1.0...vcs-git-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-git-v0.1.0
