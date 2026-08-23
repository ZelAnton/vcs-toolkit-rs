# Changelog — vcs-cli-support

All notable changes to the `vcs-cli-support` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-cli-support-v<version>`.

## [Unreleased]

### Added
- `managed_client!` accepts an optional trailing `extra = { field: Type, … }`
  clause, adding private fields to the generated newtype for wrapper state that is
  not a `ManagedClient` concern (`vcs-git` keeps its hardened-profile SSH policy
  and the cached trusted `core.sshCommand` there). Each type must implement
  `Default` — how the generated `new`/`with_runner` initialize it — and `Debug`,
  which the generated `Debug` renders, so a field holding sensitive text needs a
  redacting `Debug` of its own, as `ManagedClient`'s has. Purely additive: every
  existing invocation expands exactly as before.

### Changed
-

### Fixed
- Recognize the HTTPS URL scheme ASCII case-insensitively when binding an inline
  Git credential helper to its verbatim `host[:port]`, so uppercase and
  mixed-case HTTPS URLs retain the same cross-host credential gate.

## [0.8.0] - 2026-08-11

- Add `ManagedClient::default_inactivity_timeout` for streamed runs. It is
  disabled by default and preserves captured-command timeout, retry, credential,
  and cleanup behavior while retaining the separate inactivity-timeout result.
- Apply the shared `FETCH_TIMEOUT_GRACE` to both timeout and cancellation for
  network fetch/push/clone commands. Unix keeps the graceful signal ladder;
  Windows opts console children into `CTRL_BREAK` where delivery is possible,
  with the existing hard-kill fallback and unchanged `ErrorReason::Cancelled`.
- Centralize the network-command completion policy in
  `apply_fetch_completion_policy`: it keeps `FETCH_TIMEOUT_GRACE` on Unix and
  opts Windows console children into processkit's graceful `CTRL_BREAK` tier,
  with the documented console/detached-child limitations and hard-kill fallback.
- Add `ProcessEvent`/`ProgressCallback` and the shared
  `run_with_progress`/`ManagedClient::run_with_progress` lifecycle driver. It
  polls processkit 3.1 events beside the finisher, preserves streamed diagnostics
  in structured exit errors, and is replayable through `ScriptedRunner`.

### Added
- `logging` module — a command-logging `ProcessRunner` decorator:
  `LoggingRunner` wraps any real runner and reports every spawn (program, argv,
  working directory, exit code, duration) to a `CommandObserver`; the default
  `StderrObserver` writes a one-line summary to **stderr** (never stdout, so a
  stdout JSON-RPC transport stays clean). Because it sits on the single
  `ProcessRunner` seam every wrapper spawns through, coverage is complete by
  construction. Argv is redacted before it reaches an observer (`redact_args`,
  also public): a value after a sensitive flag (`--token`/`--password`/…) or the
  value of a `--flag=value` form is masked, a secret-shaped token
  (`ghp_`/`github_pat_`/`glpat-`/… prefix, `x-access-token:`) embedded anywhere in
  free text is masked, a URL's
  embedded credentials are masked (host/path kept), and long free text (a PR/issue
  body, a commit message) is truncated — a fail-closed policy that upholds the
  workspace's "token never in argv" contract as defence in depth (the environment,
  which carries the token, is never logged). Also exports `CommandObserver`,
  `CommandRecord`, `CommandStatus`, `StderrObserver`, and the reusable
  single-value `redact_value` editor. (T-117, T-166.)
- `run_with_progress_within` / `ManagedClient::run_with_progress_within` — an
  `OutputBudget` ceiling on the stdout/stderr a **streamed** run retains, closing
  the memory-bound gap between a streaming `clone`/`fetch --progress` and its
  captured twin. A command's `OutputBufferPolicy` does not bound an *event*
  stream, so `budget_diagnostics` never reached the copy `run_with_progress` keeps
  in order to promote a rejected exit into a structured error: on a large
  repository that copy grew with the transfer. The ceiling is **drop-oldest, never
  fail-loud** (the `diagnostic_policy` half of the contract): it truncates what is
  retained without turning a run into `OutputTooLarge`, keeps the **tail** — where
  a CLI's fatal line sits, so `is_transient_fetch_error`/`is_lock_contention` still
  classify it — and leaves delivery untouched (the progress callback still sees
  every line). Each stream carries the budget independently, as on a captured verb.
  A line longer than the cap keeps its own tail (cut on a char boundary) rather
  than being dropped whole, since carriage-return `--progress` output is one
  ever-growing line under processkit's default `\n` framing.
  `ManagedClient::run_with_progress` now applies the client's
  `default_output_budget` this way — **unlimited by default**, so a client that
  never sets one streams exactly as before, and the free `run_with_progress` is
  unchanged. The byte ceiling counts the bytes actually retained (decoded line
  content plus the one `\n` joining each retained pair) — neither processkit's
  raw-pipe-byte fail-loud unit nor its content-only drop-mode unit; the boundary is
  pinned by an exact test. (T-148.)

### Changed
- Reject carriage-return and line-feed characters in Git credential usernames and
  secrets before constructing the inline `credential.helper` protocol output.
  The direct helper validates both fields, while static, environment-backed,
  closure-provider, and common-resolution paths validate both fields before
  applying the empty/whitespace-only secret fallback, so malformed values return
  the crate's deterministic `InvalidInput` error while valid credentials, the
  default username, and host scoping remain unchanged.
- **Bumped `processkit` to the 3.0 line** (workspace requirement `"2.1"` → `"3.0"`).
  Breaking for a downstream that pattern-matches a `processkit::Error` this crate's
  classifiers accept: `Error` is no longer an enum but an opaque, pointer-sized
  wrapper around a boxed `ErrorReason` (the former enum, every variant and field
  unchanged). Read accessors (`code`/`program`/`stdout`/`stderr`/`diagnostic`/the
  `is_*` predicates/…), `Display`, `Debug`, and `source` are untouched — only a direct
  variant match moves to `err.reason()` (borrow) / `err.into_reason()` (own), with the
  new flat `err.kind() -> ErrorKind` for coarse classification. Every signature in this
  crate is unchanged (`is_merge_conflict`, `is_nothing_to_commit`,
  `is_transient_fetch_error`, `is_lock_contention`, `is_invalid_input` still take
  `&processkit::Error`), and their behaviour is preserved exactly — the internal
  variant matches were rewritten against `reason()`, deliberately *not* against the
  `stdout()`/`stderr()` accessors, which would also have admitted a `Timeout`/
  `Signalled` run's partial output. Requires a coordinated release of the whole
  processkit-facing set (see `crates/core/docs/stability.md`); pre-1.0, so the minimum
  necessary bump here is a **minor** one (0.7.0 → 0.8.0). (T-129.)
- `logging::CommandStatus::Failed`'s category string now comes from processkit's flat
  `ErrorKind` classifier instead of an eight-arm variant match. A permission-denied
  spawn/IO failure reads as `permission denied` (was `spawn failed`/`io error`), and a
  signal death (`signalled`) and a predicate rejection (`predicate rejected`) gain their
  own categories instead of falling into the generic `error`. `program not found`,
  `spawn failed`, `timed out`, `cancelled`, `unsupported`, `non-zero exit` and
  `output too large` are unchanged (the last recovered through the dedicated
  `Error::output_overflow` accessor, which `ErrorKind` folds into its catch-all); a
  plain non-permission `Io` failure now reads `error` rather than `io error`. The
  strings are diagnostic text on a stderr log line, not a parsed contract. (T-129.)
- `OutputBudget::bytes`' documented unit follows processkit 3.0: the
  `OutputBufferPolicy::max_bytes` ceiling counts **raw bytes read from the output
  pipe** (line terminators and invalid-UTF-8 bytes included) rather than decoded
  line-content bytes. The projections themselves
  (`content_policy`/`diagnostic_policy`) are unchanged. (T-129; the unit's effect on
  this crate's own ceilings is described by the T-130 audit entry below, which
  supersedes this entry's original "identical for plain ASCII/UTF-8 LF output"
  wording.)
- **Byte-ceiling accounting audited against processkit 3.0 and its documentation
  corrected.** The effect of the new unit splits by *stream*, and `OutputBudget`'s
  docs now say so rather than claiming the change is inert on plain LF output — it
  is not, because an LF terminator is now charged just like a CRLF's extra `\r`:
  - The **raw stdout** every content verb reads (`ManagedClient::run_untrimmed` →
    `output_bytes`) was counted in raw pipe bytes before 3.0 as well, so **no
    content ceiling moved**: a cap on `diff_text`/`show_file`/`pr_diff` refuses
    exactly the reads it refused on the 2.x line.
  - The **line-pumped stderr** the same fail-loud policy also rides now charges each
    line terminator, so a command that floods stderr can raise `OutputTooLarge` one
    byte per line sooner than it did on 2.x. That is a real behavioural change
    downstream inherits from the bump.
  - The drop-oldest `diagnostic_policy` (a discard verb's `clone`/`fetch`
    diagnostics) is unaffected either way: drop-mode retention is still bounded by
    decoded line-content bytes, so it keeps the same tail.

  `OutputBudget::bytes` documents the per-stream unit, the type doc records that
  the ceiling rides each captured stream independently (so one call's worst case is
  ~2x the cap), `diagnostic_policy` records the retention asymmetry, and both
  boundaries are pinned by exact tests (`content_budget_counts_raw_stdout_bytes_verbatim`,
  `content_budget_charges_stderr_line_terminators`) instead of only by
  far-past-the-cap fixtures. No ceiling was re-tuned. (T-130.)

### Fixed
- Command-log argv redaction is now idempotent: a second redaction pass preserves
  the exact canonical `…(<n> chars)` output of the first instead of truncating its
  marker again. Known token shapes are now detected anywhere within free text and
  non-sensitive flag values, not only at the start of an argv slot. Property tests
  cover secret non-disclosure across token, sensitive-flag, and URL-userinfo shapes;
  arbitrary/large Unicode; and invalid Unix argv bytes.
- `redact_args` now treats URL userinfo as secret-bearing by default. In
  particular, clone URLs that put a GitHub/GitLab PAT in the username slot
  (`https://ghp_…@…`, `https://glpat-…@…`) no longer leak it to a command
  observer; the only visible userinfo is the conventional non-secret
  `ssh://git@host/…` transport identity. (T-131.)
- `redact_args`/`redact_value` no longer mistake part of a URL's path or query for
  embedded credentials. `mask_url_userinfo` now searches for the `userinfo@` only
  within the URL's **authority** component (up to the first `/`, `?`, or `#`), the
  same boundary `https_host` uses — so a credential-free URL with a port and a
  later `@`, e.g. `https://host:8443/dir/file@rev`, logs verbatim instead of
  collapsing to `https://<redacted>@rev` (the port's `:` had made the whole
  `host:8443/dir/file` look like `user:secret` userinfo). A genuine embedded
  credential (`scheme://user:secret@host…`) is still masked, host/path kept. (T-120.)
- `clone_dest_cleanable` now returns `true` only when `dest` is *provably*
  absent (`read_dir` fails with `NotFound`) or an already-empty directory —
  previously **any** `read_dir` error (permission denied, transient I/O, a
  plain-file `dest`) was treated as cleanable, which could tell
  `cleanup_failed_clone_dest` to `remove_dir_all` a pre-existing, non-empty
  directory it merely failed to read. (T-085.)

## [0.7.0] - 2026-07-19

### Added
- `clone_dest_cleanable` / `cleanup_failed_clone_dest` — the R7 failed-clone
  cleanup helper (compute whether a clone destination is safe to remove *before*
  running the clone; best-effort `remove_dir_all` on the error path only, never
  touching a non-empty pre-existing destination). Consolidates the
  byte-identical logic previously duplicated in `vcs_git::clone_repo` and
  `vcs_jj::git_clone`, which now call these helpers. (T-082.)

### Changed
-

### Fixed
-

## [0.6.0] - 2026-07-10

### Added

- **Host context for credential requests.** `ManagedClient::with_expected_host(host)`
  records the remote host a client targets; the auto-injected forge token-env path
  (`prepare`) now passes it as the `CredentialRequest`'s host, so a **host-keyed**
  `CredentialProvider` resolves the secret for *that* host and never a neighbouring
  instance's. `resolve_credential`'s **fallback policy** is now spelled out and
  applies identically to read and write operations: no provider / `Ok(None)` / an
  empty (whitespace-only) secret → defer to ambient auth; `Err` → **fail-closed**
  abort (never a silent downgrade, and never a wrong host's secret). Clients without
  a host binding are unchanged — the request carries no host, and a host-keyed
  provider that can't place it defers to ambient. (T-045.)

- **Cancellation-aware retry backoff.** `ManagedClient::default_cancel_on(token)` now
  cuts a lock-contention retry backoff **short** the instant the token fires: a
  cancelled operation returns a structured `Error::Cancelled` promptly instead of
  sleeping out the remaining (possibly large `max_backoff`) delay before its next
  attempt. The token is still applied to the spawned process as before — it is now
  *also* observed by the retry loop. No further attempt is launched once the token
  fires, so the attempt count stays deterministic (no cancel-vs-retry race). The
  jitter/exponential/cap backoff maths and the no-token behaviour are unchanged.

### Changed

- **Breaking:** `retry_async` gained a second parameter,
  `cancel: Option<&processkit::CancellationToken>`, between `policy` and
  `should_retry`: `retry_async(policy, cancel, should_retry, op)`. When `Some`, the
  inter-attempt backoff aborts with `Error::Cancelled` the moment the token fires
  (before, during, or right at the end of a wait), launching no further attempt;
  pass `None` for the previous plain, uninterruptible backoff. Callers using
  `ManagedClient` are unaffected — it threads its `default_cancel_on` token through
  automatically.

- **Breaking (macro):** `at_forwarders!` gained a third section, `raw { fn view(args…)
  -> Ret => target; }`, and the raw escape hatches (`run`/`run_raw`/`run_args`/
  `run_raw_args`) moved out of `bare` into it. `bare` now forwards a method verbatim
  (dropping `dir`); `raw` forwards the view method to the client's **dir-taking**
  `target` (`self.$field.target(self.dir, args…)`), so a raw call through a `…At` view
  runs in the bound `dir` instead of the process cwd. A wrapper that lists `run*` under
  `bare` must move them to `raw` and add the matching `*_in` client methods (T-035).

### Fixed
-

## [0.5.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wave1.5a): is_invalid_input + is_resource_not_found classifiers (A2/A3)


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): content verbs preserve trailing bytes (H7)
- fix(wave2): don't retry a fetch timeout (avoids 3x deadline amplification) (R6)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(retry): lock-contention classifier + opt-in jittered RetryPolicy on git/jj mutations
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(credentials): git remote (HTTPS) credential injection via credential.helper (Phase 2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(credentials): treat an empty resolved secret as ambient (no injection)
- fix(cli-support): tighten lock-retry markers, credential robustness, flag-guard hardening
- fix(cli-support+jj): tighten transient marker, resolve_list match, conflict end-marker


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.1] - 2026-07-05

### Added
- **The `managed_client!` macro now generates a `Debug` impl** for every wrapper
  type it scaffolds (`Git`, `Jj`, `GitHub`, `GitLab`), delegating straight to the
  wrapped `ManagedClient` field — which already redacts its configured
  credential provider (`credentials.is_some()` only, never the secret) and
  carries no `R: Debug` bound. No wrapper crate needs its own hand-written impl.

### Changed
-

### Fixed
-

## [0.5.0] - 2026-07-05

### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wave1.5a): is_invalid_input + is_resource_not_found classifiers (A2/A3)


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): content verbs preserve trailing bytes (H7)
- fix(wave2): don't retry a fetch timeout (avoids 3x deadline amplification) (R6)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(retry): lock-contention classifier + opt-in jittered RetryPolicy on git/jj mutations
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(credentials): git remote (HTTPS) credential injection via credential.helper (Phase 2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(credentials): treat an empty resolved secret as ambient (no injection)
- fix(cli-support): tighten lock-retry markers, credential robustness, flag-guard hardening
- fix(cli-support+jj): tighten transient marker, resolve_list match, conflict end-marker


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Added

- feat(wave1.5a): is_invalid_input + is_resource_not_found classifiers (A2/A3)


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave0): data-loss & security bleeders (C1/C2/C3/H1/H5/P1)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): content verbs preserve trailing bytes (H7)
- fix(wave2): don't retry a fetch timeout (avoids 3x deadline amplification) (R6)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(retry): lock-contention classifier + opt-in jittered RetryPolicy on git/jj mutations
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(credentials): git remote (HTTPS) credential injection via credential.helper (Phase 2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(credentials): treat an empty resolved secret as ambient (no injection)
- fix(cli-support): tighten lock-retry markers, credential robustness, flag-guard hardening
- fix(cli-support+jj): tighten transient marker, resolve_list match, conflict end-marker


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.3.0] - 2026-07-03

### Added
- New optional **`serde`** feature exposing a **`json`** module with the two
  forge-parser JSON helpers shared by `vcs-github`/`vcs-gitlab`/`vcs-gitea`:
  `null_to_empty` (a `deserialize_with` that turns a present JSON `null` into an
  empty string) and `from_json(program, json)` (deserialize a CLI's `--json`
  output into `T`, mapping a parse failure to `Error::Parse` tagged with the
  binary name). Off by default — only the forge wrappers enable it, so the
  ambient-auth backends (`vcs-git`/`vcs-jj`) never pull in `serde`/`serde_json`.
- `https_host(url)` — extract the `host[:port]` (verbatim from an `https://` URL)
  to scope a credential helper to the host an operation targets.
- **`managed_client!` gained an optional `scrub_env = [ … ]`** clause: a client that
  supplies it scrubs those inherited env vars (via `default_env_remove`) on every
  instance it builds. `vcs-git` uses it to drop the repo-redirector vars (`GIT_DIR`,
  …) so a value leaking from the parent process can't retarget commands.
  (`docs/audit-2026-07.md` H4.)
- **`is_invalid_input(err)`** classifier — recognizes an input rejection from the
  argument guards (`reject_flag_like` / the validating newtypes), encoded as an
  `Error::Spawn` with `io::ErrorKind::InvalidInput`. Lets a caller/binding surface a
  bad argument as a `ValueError`, distinct from a real spawn/OS failure.
  (`docs/audit-2026-07.md` A2.)
- **`ManagedClient::run_untrimmed`** — like `run`, but returns stdout **verbatim**
  (no `trim_end`), for content-returning verbs where a trailing newline is part of
  the value. Exit-checked; no lock-retry. (`docs/audit-2026-07.md` H7.)

### Fixed
- **Corrected the jj lock-contention markers and made the git one locale-stable.**
  `is_lock_contention` matched jj strings that jj never emits; it now matches jj's
  actual `"Failed to lock working copy"` / `"Failed to lock operation heads store"`,
  and matches git's **locale-stable** `index.lock` path fragment (not the translated
  `': File exists'` suffix), so lock-retry works on a non-English runner.
  (`docs/audit-2026-07.md` H2.)
- **`is_transient_fetch_error` no longer classifies a `Timeout` as transient**, so a
  timed-out `fetch` is **not** retried. A `.timeout()`-bounded run that expired already
  spent the caller's full deadline; retrying it up to `FETCH_ATTEMPTS` times multiplied
  the wall-clock (a black-holed remote under a 120 s deadline blocked ≈ 6 min, 3× the
  advertised ceiling). Fast transient failures (DNS, dropped connection, io-level
  interrupted/would-block) still retry. Inherited by `vcs-git`/`vcs-jj`'s fetch retry
  and by the `is_transient_fetch_error` classifier on both facades
  (`vcs_core::Error` and `vcs_forge::Error`). (`docs/audit-2026-07.md` R6.)

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the `processkit` types surfaced in this crate's public API
  (`Error`/`ProcessResult`/…) 1.x — **breaking** for a downstream that pins `processkit`
  `0.x` directly. processkit is semver-stable from 1.0, so future 1.x updates are
  non-breaking.
- **`ManagedClient::output` → `output_string` (breaking).** Mirrors processkit's
  crate-wide `output`→`output_string` rename (one name per operation; disambiguates from
  `std`'s bytes-returning `output`), keeping `ManagedClient`'s verb set a faithful mirror
  of `CliClient`. Update `mc.output(..)` to `mc.output_string(..)`.
- **`ManagedClient::parse`/`try_parse` now require `T: Send` and the parser `+ Send`
  (breaking).** Matches processkit 1.x's tightened bounds; a real parser closure is
  already `Send`, so callers are unaffected in practice.
- **`git_credential_helper(cred)` → `git_credential_helper(cred, expect_host)`
  (breaking).** The new `expect_host: Option<&str>` scopes the helper to a host
  (see Security below); pass `None` for the previous ungated behavior.

### Security
- **The inline git credential helper can be scoped to a host.** When
  `git_credential_helper` is given `Some(host)`, the emitted snippet reads git's
  credential request and releases the secret only for a matching host — so an HTTP
  redirect or a submodule fetch to a *different* host can't extract the token.
  `None` keeps the prior ungated behavior. (`docs/audit-2026-07.md` H5.)

## [0.2.0] - 2026-06-27

### Added
- **Credential provisioning (opt-in).** A new `credentials` module: the
  `CredentialProvider` async trait (dyn-compatible, matching processkit's
  `ProcessRunner` pattern) plus the `Credential`/`Secret` types (`Secret` redacts
  itself in `Debug`/`Display`) and built-in adapters (`StaticCredential`,
  `EnvToken`, `provider_fn`). `ManagedClient` gained `with_credentials` +
  `with_token_env` + `resolve_credential`: when a token-env binding is set it
  injects the resolved token into every command's environment (the forge
  `GH_TOKEN`/`GITLAB_TOKEN` path); `git_credential_helper` builds a git
  `credential.helper` invocation that keeps the secret out of `argv`. Default is
  no provider → ambient CLI auth, unchanged. Adds an `async-trait` dependency.
  `ManagedClient` also gained an `exit_code` verb (used by the forge clients).
- **Lock-contention retry.** `is_lock_contention(&Error)` classifies a *pre-execution*
  **whole-repository** lock-acquisition failure (git's `index.lock`, jj's
  working-copy / op-heads lock) — the one error class safe to retry on a mutation,
  since the command never ran. Per-ref lock failures (`cannot lock ref`,
  `<ref>.lock`) are deliberately *excluded*: a multi-ref `push`/`fetch` can fail a
  ref lock after earlier refs already moved, where a retry would not be idempotent.
  `RetryPolicy` (attempts + exponential backoff + full jitter)
  and the `retry_async` executor express the strategy; `ManagedClient` is a
  `CliClient` wrapper that applies it to every command (the `vcs-git`/`vcs-jj`
  clients now hold one). Retry is opt-in (default `RetryPolicy::none()`). Adds a
  `tokio` (time) dependency for the backoff sleep.
- `signalled_is_terminal_not_transient` test — pins that an `Error::Signalled`
  (signal-killed process) is terminal, not a transient fetch error (so it is
  never auto-retried), even when its captured stderr contains an otherwise-transient
  marker.

### Changed
- Bumped `processkit` to **0.11.0** (from 0.9.1). The classifiers' input `Error`
  gained partial output on the `Timeout`/`Signalled` variants and new first-class
  variants (`Signalled`/`NotFound`/`CassetteMiss`); the `#[non_exhaustive]`
  fall-through keeps every classifier returning "no" for unfamiliar variants. The
  0.10→0.11 step is light for us: processkit's **`stats` feature is now opt-in**
  (we never used the metrics surface, so default builds are leaner with no code
  change), `OutputEvent` now carries an `OutputLine` (we don't stream output
  events), and a cancel-precedence race fix plus a control-character-sanitizing
  one-line `Error` `Display` (0.10.2) come for free — no API change on our side.

### Removed
- The **`cancellation`** feature — cancellation is now core in processkit 0.10, so
  `Error::Cancelled` is always constructible (the
  `cancelled_is_not_transient_or_otherwise_classified` test is now unconditional).
  Breaking for anyone who enabled `vcs-cli-support/cancellation`.

### Fixed
- **Lock-retry safety:** `is_lock_contention` no longer classifies per-ref lock
  failures (`cannot lock ref`, `<ref>.lock`/`packed-refs.lock`) — a multi-ref
  `push`/`fetch` can fail a ref lock after earlier refs moved, where a retry would
  not be idempotent. It now matches only the whole-repo/working-copy locks
  (`index.lock`, jj working-copy / op-heads), which are genuinely pre-execution.
- `reject_flag_like` now also refuses an interior NUL, and applies the leading-`-`
  check to the *trimmed* value (so `" --flag"` with leading whitespace is refused).
- `EnvToken` treats a whitespace-only environment value as unset (`None` → ambient),
  and `git_credential_helper`'s inline helper emits nothing when its secret env var
  is unset/empty (git falls through to ambient instead of using an empty credential).
  `ManagedClient::resolve_credential` likewise drops a whitespace-only secret (not
  just an empty one), so every adapter shares one "no usable credential ⇒ ambient" rule.
- `ManagedClient::output` dropped its dead lock-retry wrapper (it returns `Ok` on a
  non-zero exit, so the retry predicate could never fire); credential injection on
  `output` is unchanged.
- **Transient-fetch classifier tightened:** dropped the bare `timed out` marker from
  `is_transient_fetch_error`'s list. It subsumed the specific `connection timed out`
  / `operation timed out` entries and would also match unrelated non-network
  "timed out" messages (a lock wait, a hook), triggering a spurious fetch retry. The
  specific timeout phrases are retained.

## [0.1.0] - 2026-06-08

### Added
- Initial release: the `processkit`-coupled plumbing the CLI wrappers share —
  `reject_flag_like` (the argv injection guard, parameterized by program name),
  the `FETCH_ATTEMPTS`/`FETCH_BACKOFF` fetch-retry policy, and the error
  classifiers `is_merge_conflict` / `is_nothing_to_commit` /
  `is_transient_fetch_error`. Extracted from the copies previously duplicated
  across `vcs-git` and `vcs-jj` so the transient-failure marker list and the
  classifiers can no longer drift between backends.

### Changed
- Bumped `processkit` to **0.8** — `Error` (taken by the classifiers) stays
  `#[non_exhaustive]`; an unfamiliar variant classifies as "no" on every
  classifier (covered by a test). Breaking for consumers matching
  `processkit::Error` exhaustively.
- New off-by-default **`cancellation`** feature (forwards to
  `processkit/cancellation`): the classifiers only match `Exit`/`Timeout`, so
  `Error::Cancelled` already falls through every one to "no"; the feature only lets
  a test construct the variant to pin that (not transient, not a conflict, not
  nothing-to-commit) as a first-class assertion.
- `reject_flag_like` also refuses whitespace-only values (as meaning-changing as
  empty ones), not just empty and leading-`-`.

### Fixed
-

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.8.0...HEAD
[0.8.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.7.0...vcs-cli-support-v0.8.0
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.6.0...vcs-cli-support-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.2...vcs-cli-support-v0.6.0
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.1...vcs-cli-support-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.0...vcs-cli-support-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.4.0...vcs-cli-support-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.3.0...vcs-cli-support-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.2.0...vcs-cli-support-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.1.0...vcs-cli-support-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-cli-support-v0.1.0
