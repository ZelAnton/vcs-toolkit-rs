# Changelog — vcs-testkit

All notable changes to the `vcs-testkit` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-testkit-v<version>`.

## [Unreleased]

### Added
- Initial release: `TempDir` (unique, remove-on-drop), `configure_identity`,
  `GitSandbox` (init on `main` + deterministic identity; `commit_file`,
  `branch`, `checkout`, `rev_parse`, raw `git`), `BareRemote::seeded` (local
  clone/fetch/push fixture), and `JjSandbox` (`describe`, `new_change`,
  `bookmark`, raw `jj`). Synchronous, dependency-free, panics on failure —
  consolidates the scaffolding previously duplicated across the
  `vcs-git`/`vcs-jj`/`vcs-core` test suites.

### Changed
-

### Fixed
-

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/commits/main/crates/testkit
