# Changelog — vcs-cli-support

All notable changes to the `vcs-cli-support` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-cli-support-v<version>`.

## [Unreleased]

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.2...HEAD
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.1...vcs-cli-support-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.5.0...vcs-cli-support-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.4.0...vcs-cli-support-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.3.0...vcs-cli-support-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.2.0...vcs-cli-support-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.1.0...vcs-cli-support-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-cli-support-v0.1.0
