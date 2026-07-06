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

## [0.5.2] - 2026-07-06

### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(diff): unquote git-quoted paths so non-ASCII filenames aren't dropped
- fix(git): blame on SHA-256 repos; remote_head_branch/upstream surface timeouts


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.1] - 2026-07-05

### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(diff): unquote git-quoted paths so non-ASCII filenames aren't dropped
- fix(git): blame on SHA-256 repos; remote_head_branch/upstream surface timeouts


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.0] - 2026-07-05

### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(diff): unquote git-quoted paths so non-ASCII filenames aren't dropped
- fix(git): blame on SHA-256 repos; remote_head_branch/upstream surface timeouts


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Changed

- refactor(diff): hoist shared DiffSpec into vcs-diff (dedup git+jj)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(diff): unquote git-quoted paths so non-ASCII filenames aren't dropped
- fix(git): blame on SHA-256 repos; remote_head_branch/upstream surface timeouts


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.3.0] - 2026-07-03

### Added
- **`DiffSpec`** — the diff-request enum (`WorkingTree` / `Rev(String)`) that a
  wrapper's `diff`/`diff_text` takes, hoisted here from `vcs-git`/`vcs-jj` so both
  backends share one definition (re-exported as `vcs_git::DiffSpec` /
  `vcs_jj::DiffSpec`). Deliberately **not** `#[non_exhaustive]`: each backend must
  interpret every variant, so adding one is a breaking change caught at compile
  time. This crate defines it but has no method that consumes it.

### Changed
-

### Fixed
-

## [0.2.0] - 2026-06-27

### Added
-

### Changed
-

### Fixed
- **Git-quoted paths are now decoded instead of dropping the file.** git C-quotes a
  path (wraps it in `"…"` with `\NNN` octal/`\t`/`\"`/`\\` escapes) when it contains a
  control byte, a quote/backslash, or — with the default `core.quotePath=true` — **any
  non-ASCII byte** (e.g. `café.txt` → `"caf\303\251.txt"`). The parser only matched the
  *unquoted* `+++ b/` / `--- a/` / `rename` / `" b/"` forms, so a file with a non-ASCII
  (or tab/quote) name was **silently omitted** from `parse_diff`. It now unquotes the
  path on every source (`rename to`/`from`, `+++`/`---`, and the `diff --git` header
  fallback), so internationalised filenames parse correctly.
- A diff section whose path can't be resolved to a non-empty string (a malformed
  `diff --git … b/` with no path, and no `+++`/`---`/rename line) is now **dropped**
  rather than yielding a `FileDiff` with an empty `path`. A present-but-empty
  `+++ b/`/`--- a/` likewise falls through to the next path source instead of
  producing an empty path.

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.5.2...HEAD
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.5.1...vcs-diff-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.5.0...vcs-diff-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.4.0...vcs-diff-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.3.0...vcs-diff-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.2.0...vcs-diff-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.1.0...vcs-diff-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-diff-v0.1.0
