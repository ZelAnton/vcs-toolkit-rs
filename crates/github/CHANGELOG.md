# Changelog — vcs-github

All notable changes to the `vcs-github` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-github-v<version>`.

## [Unreleased]

### Added
-

### Changed
- `run` now launches `gh` inside an OS job (Windows Job Object / Linux cgroup v2)
  via `vcs-process`, so the process tree is killed on close — no orphaned
  subprocesses.

### Fixed
-

## [0.1.0] - 2026-05-29

### Added
- Initial skeleton: `run` CLI-execution helper and `version()` over the `gh` binary.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-github-v0.1.0
