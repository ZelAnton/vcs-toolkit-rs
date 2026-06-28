# Changelog — vcs-cli-support

All notable changes to the `vcs-cli-support` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-cli-support-v<version>`.

## [Unreleased]

### Added
- New optional **`serde`** feature exposing a **`json`** module with the two
  forge-parser JSON helpers shared by `vcs-github`/`vcs-gitlab`/`vcs-gitea`:
  `null_to_empty` (a `deserialize_with` that turns a present JSON `null` into an
  empty string) and `from_json(program, json)` (deserialize a CLI's `--json`
  output into `T`, mapping a parse failure to `Error::Parse` tagged with the
  binary name). Off by default — only the forge wrappers enable it, so the
  ambient-auth backends (`vcs-git`/`vcs-jj`) never pull in `serde`/`serde_json`.

### Changed
-

### Fixed
-

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.2.0...HEAD
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.1.0...vcs-cli-support-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-cli-support-v0.1.0
