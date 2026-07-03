# Changelog — vcs-forge

All notable changes to the `vcs-forge` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-forge-v<version>`.

## [Unreleased]

### Added
-

### Changed
-

### Fixed
-

## [0.3.0] - 2026-07-03

### Added
- `Error::is_unauthorized()` and `Error::is_rate_limited()` classifiers — detect an
  authentication failure (missing/invalid token, "not logged in") or a rate-limit
  (HTTP 429 / "API rate limit exceeded" / secondary-abuse limit) from the forge
  CLI's output, so a caller (or a language binding) can map them to dedicated
  errors instead of string-matching `stderr`. Conservative/phrase-based: a miss
  degrades to a generic forge error.
- **`Error::is_invalid_input()`** and **`Error::is_resource_not_found()`** classifiers.
  `is_invalid_input` recognizes a bad argument the facade refused (the facade's
  `InvalidInput`, or a wrapper guard like a flag-like Gitea comment body).
  `is_resource_not_found` recognizes a referenced PR/MR/issue/repo/release that
  doesn't exist (high-precision `gh`/`glab` phrasings + the Gitea `pr_view` miss),
  distinct from the `gh`/`glab`/`tea` *binary* being missing (`is_not_found`). A
  binding maps them to `ValueError` / `NotFoundError`. (`docs/audit-2026-07.md` A2, A3.)
- `Forge::github_with_token(cwd, token)` and `Forge::gitlab_with_token(cwd, token)`
  convenience constructors — a real-runner handle that authenticates with an explicit
  token (injected as `GH_TOKEN` / `GITLAB_TOKEN`) instead of the CLI's ambient login,
  without hand-building the underlying client. `token` takes `impl Into<Secret>`, so a
  plain `&str`/`String` works. `Secret` is now re-exported (`vcs_forge::Secret`).
  **Gitea has no such constructor**: `tea` reads credentials from its own config and
  has no token-via-environment override, so it authenticates only through `tea login`
  (documented on [`Forge::gitea`]).

### Changed
- **`Forge::pr_close` / `ForgeApi::pr_close` take a `PrClose` spec, not a bare
  `delete_branch` bool (breaking).** `pr_close(number, true)` didn't say what `true`
  meant — and the flag is **GitHub-only** (GitLab/Gitea have no `--delete-branch` and
  silently ignored it). It's now `pr_close(PrClose::new(number).delete_branch())`, whose
  doc states the per-backend honesty, and `#[non_exhaustive]` leaves room to grow.
  Behavior unchanged. (`docs/audit-2026-07.md` A1.)
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` (`vcs_forge::processkit`, incl.
  `Error`/`ProcessResult`) 1.x — **breaking** for a downstream that pins `processkit`
  `0.x` directly. No behaviour change. processkit is semver-stable from 1.0, so future
  1.x updates are non-breaking.
- **Renamed the inject constructors `Forge::for_github`/`for_gitlab`/`for_gitea`/
  `for_unknown` → `from_github`/`from_gitlab`/`from_gitea`/`from_unknown`
  (breaking).** This mirrors `vcs-core`'s `Repo::from_git`/`from_jj` naming. The
  force-backend constructors `github()`/`gitlab()`/`gitea()` are unchanged.
  Update callers of `Forge::for_*` to `Forge::from_*`.

### Fixed
- **`Forge::supports()` reports `false` for every op on an `Unknown` backend.** It
  wrongly returned `true`, so a UI listing `ForgeOp::ALL` rendered every operation as
  available even though each one returns `Unsupported` on an unclassified handle. Now
  it agrees with `capabilities()`'s all-`false` map. (`docs/audit-2026-07.md` H10.)
- **Docs:** `Forge::pr_comment` now documents that comment-body handling differs by
  backend — GitHub/GitLab take the body in a flag-value slot (a `-`-leading body is
  fine), while Gitea's `tea comment` takes it positionally and rejects a flag-like
  body (one whose first non-space char is `-`, e.g. a Markdown bullet). The previous
  doc wrongly claimed every backend used a flag-value slot. No behavior change.
- **Docs:** `pr_list` / `issue_list` / `release_list` now note that Gitea returns at
  most **~50** rows per its server page cap (GitHub/GitLab return up to 100), and
  `pr_view` notes it **pages** the Gitea listing so it finds a PR past that cap. No
  behavior change in this crate (the fix is in `vcs-gitea`). (`docs/audit-2026-07.md` H8.)

## [0.2.0] - 2026-06-27

### Added
- Re-export of `processkit` itself (`vcs_forge::processkit`) so a `vcs-forge`-only
  consumer can match the wrapped `Error::Forge(processkit::Error::…)` without a
  direct `processkit` dependency (mirrors `vcs_core::processkit`).
- **Capability introspection** — `Forge::supports(ForgeOp) -> bool` reports which
  *varying* operations a backend ships (`ForgeOp`: `RepoView`/`PrMarkReady`/
  `PrChecks`/`ReleaseView` — the ops Gitea lacks), so a consumer can hide an
  unavailable action instead of calling it and handling `Unsupported`. New types
  `ForgeOp` (+ `ForgeOp::ALL`).
- **`Forge::capabilities() -> Result<ForgeCapabilities>`** and the
  `ForgeCapabilities` flat map surfaced by the `forge_info` MCP tool — carries
  `pr_create`/`pr_comment`/`pr_edit`/`pr_checks`/`pr_merge`/`issue_create`/`authed`,
  each the intersection of "the CLI ships the command" and the live auth probe
  (spawned at most once). `ForgeCapabilities::all_false()` is the all-`false`
  shape. (Serialized snake_case under the `serde` feature.)
- `ForgeRelease` now carries `body: Option<String>` (release notes; GitHub &
  GitLab, `None` on Gitea), `draft: bool`, and `prerelease: bool` (GitHub & Gitea;
  always `false` on GitLab, which has no such concept). Additive on the
  `#[non_exhaustive]` DTO.
- `ForgeIssue::body`/`url` are now populated by GitHub's `issue_list` too (its
  lean field list was widened), not just `issue_view`.
- `PrEdit` — the unified edit-a-PR/MR spec (optional `title` and/or `body`), built
  with `PrEdit::new()` and chained `.title(..)` / `.body(..)` setters. Mirrors
  `PrCreate`'s shape.
- `Forge::pr_comment(number, body)` — post a comment to an existing PR/MR (routes
  to `vcs-github`'s `pr_comment` / `vcs-gitlab`'s `mr_comment` / `vcs-gitea`'s
  `pr_comment`; `Unknown` returns `Unsupported`). An empty (or whitespace-only)
  body is rejected with `Error::InvalidInput` *before* any spawn — every backend
  passes the body in a `--body`/`--comment` flag-value slot (so a flag-like body is
  safe), but a blank comment is a caller bug, so it fails fast and uniformly.
  *(Correction: the "every backend … flag-value slot" claim is inaccurate — Gitea
  takes the body positionally; see the `[Unreleased]` ▸ Fixed entry.)*
- `Forge::pr_edit(number, PrEdit)` — edit a PR/MR's title and/or body. Rejects
  both-`None` with `Error::InvalidInput` *before* any spawn; routes to the three
  per-forge wrappers.
- `Error::is_not_found()` (the forge CLI binary `gh`/`glab`/`tea` isn't installed)
  and `Error::is_transient()` (a transient io/spawn hiccup) — completing the `is_*`
  classifier family so it matches `vcs_core::Error`'s, letting a caller branch on
  intent without reaching into the wrapped `processkit::Error`.
- `ForgeKind::Unknown` + `Forge::for_unknown(cwd)` — additive on the
  `#[non_exhaustive]` enum; a handle whose `capabilities()` is the all-`false`
  shape (no spawn) and whose every operation returns `Error::Unsupported`. Useful
  for an auto-detector that wants to surface "tried, no luck".
- `Error::InvalidInput(String)` — new `#[non_exhaustive]` variant for the facade's
  refused-input cases (currently `pr_edit` both-`None`); surfaces as a
  client-fixable error from the MCP layer.
- The new methods (`pr_comment`/`pr_edit`/`capabilities`) are added as **defaulted**
  methods directly on `ForgeApi` (default bodies return `Unsupported` / the
  all-`false` map), so external `ForgeApi` implementers keep compiling and the
  methods are callable through `&dyn ForgeApi`; the concrete `Forge` overrides all
  three with the real dispatch.

### Changed
- The re-exported `vcs_github::CheckRun::bucket` is now the typed `CheckBucket`
  enum (was `String`) — breaking for code reaching through `vcs_forge::vcs_github`.
  This type change is output-neutral for the CI aggregate (`Forge::pr_checks` →
  `CiStatus`); see **Fixed** for the separate all-`Unknown` → `Pending` aggregate fix.
- Bumped `processkit` to **0.11.0** (via the wrappers). Re-exported
  `processkit::Error` changed (partial `stdout`/`stderr` on `Timeout`/`Signalled`;
  new `Signalled`/`NotFound`/`CassetteMiss` variants; `Invocation::cwd: Option<PathBuf>`)
  — breaking for downstream.

### Removed
- The **`cancellation`** feature (which forwarded to
  `vcs-github`/`vcs-gitlab`/`vcs-gitea`) — cancellation is now core in
  processkit 0.10, so `default_cancel_on` is always available without a feature.

### Fixed
- `ForgeKind::from_remote_url` host extraction is now IPv6-aware: a bracketed
  scheme-URL authority (`https://[::1]:443/…`) yields the address `::1` instead
  of the literal `[`. The bracket is unwrapped **only** when its content is a
  genuine IPv6 literal (contains a colon), so a bracketed *name* such as
  `[gitlab.com]` is not unwrapped and cannot spoof a trusted SaaS host. No SaaS
  host is an IPv6 literal, so the classifier result is unchanged (`None`), but
  the underlying host parse is now correct for any future consumer.
- **`pr_checks` CI aggregation: a check set that is *all* unmodeled (`Unknown`)
  buckets now reports `Pending`, not `None`.** A PR that genuinely has checks (in a
  bucket a future `gh` introduces, or with a missing field) previously aggregated to
  `CiStatus::None` ("no CI ran"); it now reports `Pending` ("not known to be done"),
  matching how the GitLab mapper treats an unknown pipeline status — a cross-forge
  consistency fix. A deliberate `Skipping` check still doesn't move the needle, and a
  modeled pass alongside an unmodeled bucket still reports `Passing`.

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.3.0...HEAD
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.2.0...vcs-forge-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.1.0...vcs-forge-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-forge-v0.1.0
