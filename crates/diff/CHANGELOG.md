# Changelog — vcs-diff

All notable changes to the `vcs-diff` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-diff-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.1.0] - 2026-06-08

### Added
- Initial release: the shared git-format unified-diff model and parser —
  `ChangeKind`, `DiffLine`, `Hunk`, `FileDiff`, `DiffStat`, and `parse_diff` —
  plus the `Version` type and `parse_dotted_version`. Extracted from the
  byte-identical copies previously carried by `vcs-git` and `vcs-jj` (and the
  third `ChangeKind`/`DiffStat` copy in `vcs-core`), so the parser and the
  version `Ord` can no longer drift between backends. Dependency-free (std
  only); property-tested for panic-freedom.
- Optional `serde` feature: derives `serde::Serialize` on the public DTOs
  (`DiffStat`, `ChangeKind`, `DiffLine`, `Hunk`, `FileDiff`, `Version`) so a
  consumer (e.g. `vcs-mcp`) can emit them as JSON. **Off by default** — the crate
  stays std-only unless the feature is enabled; enums serialize as their variant
  names, structs keep their snake_case field names.

### Changed
-

### Fixed
-

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-diff-v0.1.0
