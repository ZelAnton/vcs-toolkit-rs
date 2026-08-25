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

### Changed
-

### Fixed
-
