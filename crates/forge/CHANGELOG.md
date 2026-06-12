# Changelog — vcs-forge

All notable changes to the `vcs-forge` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-forge-v<version>`.

## [Unreleased]

### Added
- Re-export of `processkit` itself (`vcs_forge::processkit`) so a `vcs-forge`-only
  consumer can match the wrapped `Error::Forge(processkit::Error::…)` without a
  direct `processkit` dependency (mirrors `vcs_core::processkit`).
- **Capability introspection** — `Forge::supports(ForgeOp) -> bool` and
  `Forge::capabilities() -> ForgeCapabilities` report which varying operations a
  backend supports (Gitea lacks `repo_view`/`pr_mark_ready`/`pr_checks`/
  `release_view`), so a consumer can hide an unavailable action instead of
  calling it and handling `Unsupported`. New types `ForgeOp` (+ `ForgeOp::ALL`)
  and `ForgeCapabilities`.
- `ForgeRelease` now carries `body: Option<String>` (release notes; GitHub &
  GitLab, `None` on Gitea), `draft: bool`, and `prerelease: bool` (GitHub & Gitea;
  always `false` on GitLab, which has no such concept). Additive on the
  `#[non_exhaustive]` DTO.
- `ForgeIssue::body`/`url` are now populated by GitHub's `issue_list` too (its
  lean field list was widened), not just `issue_view`.

## [0.1.1] - 2026-06-12

### Added
- `PrEdit` — the unified edit-a-PR/MR spec (optional `title` and/or `body`),
  built with `PrEdit::new()` and chained `.title(..)` / `.body(..)` setters.
  Mirrors `PrCreate`'s shape.
- `ForgeCapabilities` — the flat capability map surfaced by the new
  `Forge::capabilities` / `forge_info` MCP tool. Carries `pr_create`,
  `pr_comment`, `pr_edit`, `pr_checks`, `pr_merge`, `issue_create`, and
  `authed`; the latter intersects with the static "what the CLI ships" map.
  `ForgeCapabilities::all_false()` is the all-`false` shape used for the
  `Unknown` forge.
- `ForgeKind::Unknown` variant — additive on a `#[non_exhaustive]` enum;
  the new `Forge::for_unknown(cwd)` ctor returns a handle whose
  `capabilities()` is the all-`false` shape without spawning, and whose
  every operation returns `Error::Unsupported`. Useful for an auto-detector
  that wants to surface "tried, no luck" rather than guess a forge kind.
- `Forge::pr_comment(number, body)` — post a comment to an existing PR/MR.
  Routes to `vcs-github::GitHub::pr_comment`, `vcs-gitlab::GitLab::mr_comment`,
  and `vcs-gitea::Gitea::pr_comment`; `Unknown` returns `Unsupported`.
- `Forge::pr_edit(number, PrEdit)` — edit a PR/MR's title and/or body.
  Rejects both-`None` with `Error::InvalidInput` BEFORE any spawn
  (spec §2: explicit-error path); routes to the three per-forge wrappers.
- `Forge::capabilities()` — the flat capability map. Spawns `auth status`
  / `login list` exactly once; the per-forge static "ships the command" map
  is a constant. `Unknown` returns the all-`false` shape without spawning.
- `Error::InvalidInput(String)` — new variant for the facade's refused-input
  cases (currently just `pr_edit` both-`None`); surfaces as a client-fixable
  error from the MCP layer.
- `ForgeApiExt` trait extension — the three new methods (`pr_comment`,
  `pr_edit`, `capabilities`) live on a separate trait auto-implemented for
  any `T: ForgeApi`, with default bodies that return `Error::Unsupported`
  / the all-`false` capability map. External implementers of `ForgeApi`
  keep compiling when the crate bumps.

### Changed
- The re-exported `vcs_github::CheckRun::bucket` is now the typed `CheckBucket`
  enum (was `String`) — breaking for code reaching through `vcs_forge::vcs_github`.
  The CI aggregate (`Forge::pr_checks` → `CiStatus`) is unchanged.
- Bumped `processkit` to **0.11.0** (via the wrappers). Re-exported
  `processkit::Error` changed (partial `stdout`/`stderr` on `Timeout`/`Signalled`;
  new `Signalled`/`NotFound`/`CassetteMiss` variants; `Invocation::cwd: Option<PathBuf>`)
  — breaking for downstream.

### Removed
- The **`cancellation`** feature (which forwarded to
  `vcs-github`/`vcs-gitlab`/`vcs-gitea`) — cancellation is now core in
  processkit 0.10, so `default_cancel_on` is always available without a feature.

### Fixed
-

## [0.1.0] - 2026-06-08

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
- Bumped `processkit` to **0.8** — `Error::Forge` wraps the `#[non_exhaustive]`
  `processkit::Error`; `Error::Exit` Display gained a stderr-tail suffix. Breaking
  for consumers matching the wrapped error exhaustively, or bumping their own
  direct `processkit` separately (caret `"0.7"` does not span 0.8).
- New off-by-default **`cancellation`** feature, forwarding to each wrapper's —
  build a cancellable `GitHub`/`GitLab`/`Gitea` (via `default_cancel_on`) and hand
  it to `Forge::for_github`/… to cancel a long `run_watch`/fetch. No new API.
- `pr_create` doc honesty: it returns the CLI's success output — a URL on
  GitHub/GitLab, but a textual summary on Gitea (tea prints no URL and has no
  flag to shape the create output). `issue_create` mirrors the contract (tea
  ends its textual summary with the URL).

### Fixed
- GitLab `repo_view` no longer reports a project with **absent** `visibility`
  as private — `ForgeRepo.private` is `false` unless the forge positively says
  non-public (never claim privacy that isn't proven).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-forge-v0.1.0
