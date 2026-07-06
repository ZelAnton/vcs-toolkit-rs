# Changelog — vcs-testkit

All notable changes to the `vcs-testkit` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-testkit-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.5.1] - 2026-07-05

### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(git): current_branch handles unborn repos via symbolic-ref
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat: vcs-testkit crate, version capabilities, observation docs


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.0] - 2026-07-05

### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(git): current_branch handles unborn repos via symbolic-ref
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat: vcs-testkit crate, version capabilities, observation docs


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(git): current_branch handles unborn repos via symbolic-ref
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat: vcs-testkit crate, version capabilities, observation docs


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.3.0] - 2026-07-03

### Added
-

### Changed
- **Docs:** the testing guide gained a "Testing through a language binding (FFI)"
  section — the runner seam (`with_runner` + `processkit`'s `ScriptedRunner`) is the
  one that crosses an FFI boundary, so a binding (e.g. `vcs-toolkit-py`) wraps it
  rather than the Rust-only `mock`/trait seams.

### Fixed
-

## [0.2.0] - 2026-06-27

### Added
-

### Changed
-

### Fixed
- The git-sandbox environment scrub now also removes `GIT_CONFIG`,
  `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`, and `GIT_NAMESPACE` (alongside the
  existing `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`/`GIT_CONFIG_PARAMETERS`),
  so a host that exports any of those can no longer redirect a sandbox git
  invocation's config, object store, or ref namespace away from the temp repo.

## [0.1.0] - 2026-06-08

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
- Sandboxes are isolated from the **host** VCS configuration: every git
  invocation runs with `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL`/
  `GIT_CONFIG_SYSTEM` redirected to a nonexistent path (plus `--template=` on
  `init`), so a host-global `init.templateDir`/`core.hooksPath` can no longer
  inject hooks that execute during sandbox commits. jj invocations run with
  `JJ_CONFIG` isolated and `JJ_USER`/`JJ_EMAIL` pinned, making the
  `jj git init`-created working-copy commit's author deterministic
  (`test@example.com`) instead of inheriting the host identity. Repo-local
  hooks a test installs on purpose still run (`core.hooksPath` is deliberately
  not touched).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.5.1...HEAD
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.5.0...vcs-testkit-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.4.0...vcs-testkit-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.3.0...vcs-testkit-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.2.0...vcs-testkit-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-testkit-v0.1.0...vcs-testkit-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-testkit-v0.1.0
