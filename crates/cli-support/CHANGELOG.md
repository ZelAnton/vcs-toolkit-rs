# Changelog — vcs-cli-support

All notable changes to the `vcs-cli-support` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-cli-support-v<version>`.

## [Unreleased]

### Added
- **Credential provisioning (opt-in).** A new `credentials` module: the
  `CredentialProvider` async trait (dyn-compatible, matching processkit's
  `ProcessRunner` pattern) plus the `Credential`/`Secret` types (`Secret` redacts
  itself in `Debug`/`Display`) and built-in adapters (`StaticCredential`,
  `EnvToken`, `provider_fn`). `RetryingClient` gained `with_credentials` +
  `with_token_env` + `resolve_credential`: when a token-env binding is set it
  injects the resolved token into every command's environment (the forge
  `GH_TOKEN`/`GITLAB_TOKEN` path); `git_credential_helper` builds a git
  `credential.helper` invocation that keeps the secret out of `argv`. Default is
  no provider → ambient CLI auth, unchanged. Adds an `async-trait` dependency.
  `RetryingClient` also gained an `exit_code` verb (used by the forge clients).
- **Lock-contention retry.** `is_lock_contention(&Error)` classifies a *pre-execution*
  lock-acquisition failure (git `index.lock`/ref lock/`packed-refs.lock`, jj's
  working-copy lock) — the one error class safe to retry on a mutation, since the
  command never ran. `RetryPolicy` (attempts + exponential backoff + full jitter)
  and the `retry_async` executor express the strategy; `RetryingClient` is a
  `CliClient` wrapper that applies it to every command (the `vcs-git`/`vcs-jj`
  clients now hold one). Retry is opt-in (default `RetryPolicy::none()`). Adds a
  `tokio` (time) dependency for the backoff sleep.
- `signalled_is_terminal_not_transient` test — pins that an `Error::Signalled`
  (signal-killed process) is terminal, not a transient fetch error (so it is
  never auto-retried), even when its captured stderr contains an otherwise-transient
  marker.

### Changed
- Bumped `processkit` to **0.10.1** (from 0.9.1). The classifiers' input `Error`
  gained partial output on the `Timeout`/`Signalled` variants and new first-class
  variants (`Signalled`/`NotFound`/`CassetteMiss`); the `#[non_exhaustive]`
  fall-through keeps every classifier returning "no" for unfamiliar variants.

### Removed
- The **`cancellation`** feature — cancellation is now core in processkit 0.10, so
  `Error::Cancelled` is always constructible (the
  `cancelled_is_not_transient_or_otherwise_classified` test is now unconditional).
  Breaking for anyone who enabled `vcs-cli-support/cancellation`.

### Fixed
-

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-cli-support-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-cli-support-v0.1.0
