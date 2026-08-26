# Changelog — vcs-agent

All notable changes to the `vcs-agent` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently; tag releases as
`vcs-agent-v<version>`.

## [Unreleased]

### Added
- Add the `vcs-agent` binary and its non-mutating `probe` outcome. The command
  emits the versioned `vcs-agent/v1` success/error envelope, reports implemented
  and reserved outcomes plus ProcessKit execution capabilities, uses stable exit
  bands, redacts secrets and optional machine-local paths, and refuses oversized
  output without returning truncated success JSON.
- Add a committed JSON Schema, success/error fixtures, hermetic parser and
  rendering tests, structured ProcessKit error mapping, and a production-source
  proof that no raw VCS/forge subprocess path exists.
- Add read-only `inspect` and `changes` outcomes over the typed Git/Jujutsu and
  forge facades. They report nullable or structured unsupported repository,
  remote, forge, auth, and capability facts; distinguish summary from full
  structured diffs; enforce fail-loud content and machine-output budgets plus
  one deadline for the complete outcome; and disclose Jujutsu's unavoidable
  live working-copy snapshot and possible operation-log advancement.
- Add checked exact-path `commit` over `vcs-core`: explicit write intent,
  revision and repository-state preflight, literal Git/Jujutsu selection,
  before/after identity plus included-path evidence, fail-closed postflight,
  exact preservation checks for unrelated staged, unstaged, and untracked work,
  and pre-mutation refusal of selected-path clean filters and configured signing.
- Add checked Git/GitHub publication with exact local/remote revision proof,
  account/capability preflight, idempotent PR discovery/create, and explicit partial
  recovery checkpoints. Unsupported backend/forge combinations fail before mutation.
- Add exact-`headSha` GitHub `ci status` and `ci wait`, including terminal-success
  filtering, one aggregate deadline, cancellation, inactivity watchdog, and bounded
  diagnostic evidence.

### Changed
-

### Fixed
-
