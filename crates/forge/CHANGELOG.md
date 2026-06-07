# Changelog — vcs-forge

All notable changes to the `vcs-forge` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-forge-v<version>`.

## [Unreleased]

### Added
- Initial release: a backend-agnostic facade over `vcs-github`, `vcs-gitlab`, and
  `vcs-gitea` — the forge analogue of `vcs-core`. `Forge<R>` is a cwd-bound handle
  dispatching the common forge operations to whichever CLI backs it; the
  object-safe `ForgeApi` trait mirrors the inherent methods for `&dyn ForgeApi`.
- Explicit construction (`Forge::github`/`gitlab`/`gitea` over the real runner;
  `Forge::for_github`/`for_gitlab`/`for_gitea` over an explicit client), plus a
  pure `ForgeKind::from_remote_url` host classifier (forges have no filesystem
  marker, so there is no auto-detection).
- Unified DTOs (`#[non_exhaustive]`): `ForgePr` + `ForgePrState`
  (`Open`/`Closed`/`Merged`, normalising the three forges' state spellings),
  `ForgeRepo`, `CiStatus` (`Passing`/`Failing`/`Pending`/`None`), `MergeStrategy`,
  and the `PrCreate` spec (`PrCreate::new(title, body).source(b).target(b)` —
  mapped to each CLI's own head/base flags).
- The lean lifecycle: `auth_status`, `repo_view`, `pr_list`, `pr_view`,
  `pr_create(PrCreate)`, `pr_merge`, `pr_mark_ready`, `pr_close`, `pr_checks`.
- **Issues + releases**: `issue_list` / `issue_view(number)` /
  `issue_create(title, body)` and `release_list` / `release_view(tag)`, with the
  unified `ForgeIssue` (+ `ForgeIssueState` — any case of "closed" maps to
  `Closed`, every other state reads as live `Open`) and `ForgeRelease`
  (`published_at: Option<String>`, `None` for an unpublished draft) DTOs.
  `body`/`url` on `ForgeIssue` are best-effort (empty from GitHub's lean
  `issue_list`; filled by `issue_view` everywhere). `ForgeRelease.url` is
  **always empty on Gitea** — `tea releases list` exposes no release-page URL.
- An `Error::Unsupported { forge, operation }` variant: Gitea's `tea` has no
  current-repo view, draft toggle, checks command, or single-release view, so
  `repo_view`, `pr_mark_ready`, `pr_checks`, and `release_view` return it for the
  Gitea backend (the call does not spawn). `Error::is_unsupported()` /
  `is_transient_fetch_error()` classifiers.
- Optional `serde` feature: derives `serde::Serialize` on the public DTOs
  (`ForgeKind`, `ForgePr`, `ForgePrState`, `ForgeIssue`, `ForgeIssueState`,
  `ForgeRelease`, `ForgeRepo`, `CiStatus`, `MergeStrategy`, `PrCreate`) so a
  consumer (e.g. `vcs-mcp`) can emit them as JSON. **Off by default.**

### Changed
- Bumped `processkit` to **0.7** — `Error::Forge` wraps the now-`#[non_exhaustive]`
  `processkit::Error`, which gains variants. Breaking for consumers matching
  the wrapped error exhaustively.
- `pr_create` doc honesty: it returns the CLI's success output — a URL on
  GitHub/GitLab, but a textual summary on Gitea (tea prints no URL and has no
  flag to shape the create output). `issue_create` mirrors the contract (tea
  ends its textual summary with the URL).

### Fixed
- GitLab `repo_view` no longer reports a project with **absent** `visibility`
  as private — `ForgeRepo.private` is `false` unless the forge positively says
  non-public (never claim privacy that isn't proven).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/commits/main/crates/forge
