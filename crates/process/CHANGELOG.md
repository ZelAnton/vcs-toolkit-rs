# Changelog — vcs-process

All notable changes to the `vcs-process` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-process-v<version>`.

## [Unreleased]

### Added
- Initial release: `Job` (Windows Job Object / Linux cgroup v2 with a POSIX
  process-group fallback), `Child`, the `Mechanism` reporter, and the one-shot
  `run` helper. Child processes are launched with kill-on-close so the whole
  tree dies with the parent — no orphaned `git`/`jj`/`gh` subprocesses.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/commits/main
