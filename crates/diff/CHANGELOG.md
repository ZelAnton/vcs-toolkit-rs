# Changelog — vcs-diff

All notable changes to the `vcs-diff` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-diff-v<version>`.

## [Unreleased]

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.2.0...HEAD
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-diff-v0.1.0...vcs-diff-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-diff-v0.1.0
