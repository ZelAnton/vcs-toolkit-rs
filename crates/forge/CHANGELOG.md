# Changelog — vcs-forge

All notable changes to the `vcs-forge` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-forge-v<version>`.

## [Unreleased]

### Added
- **Version-aware `capabilities()`.** `ForgeCapabilities` gains `version:
  Option<vcs_diff::Version>` (the installed `gh`/`glab`/`tea` version, `None` for an
  `Unknown` backend or an unrecognisable banner) and `supported: bool` (whether the
  installed CLI meets the backend wrapper's declared version floor — gh ≥ 2.0,
  glab ≥ 1.25, tea ≥ 0.9). `Forge::capabilities()` now probes the CLI version
  alongside auth, and a CLI **below the floor** zeroes the per-op flags exactly
  like an unauthed one — so the map never advertises a command an old CLI can't
  run. An unrecognisable `--version` banner degrades to `supported: false` /
  `version: None` (conservatively unavailable) rather than failing the probe; a
  genuine spawn/timeout failure still propagates. `vcs_diff::Version` is re-exported
  as `vcs_forge::Version`, and `ForgeCapabilities` gains `.version(v)` / `.supported()`
  builders.
- `ForgePr`/`ForgeIssue` gained `labels: Option<Vec<String>>` and
  `assignees: Option<Vec<String>>` (additive on the `#[non_exhaustive]` DTOs, plus
  chained `.labels(...)`/`.assignees(...)` setters) — GitHub and GitLab report
  `Some(..)` (an empty `Some(vec![])` is a confirmed "none"); Gitea's `tea` has no
  such columns, so both are `None` there (unknown, never a false empty list).
- `Forge::pr_checkout(number)` / `ForgeApi::pr_checkout` — check a PR/MR's branch
  out into the bound working copy, dispatching to `gh pr checkout` / `glab mr
  checkout` / `tea pr checkout`. **Mutates the working copy.** Supported on all
  three real backends (an `Unknown` handle returns `Unsupported`). The `ForgeApi`
  trait method is **defaulted** to `Error::Unsupported` so external implementers
  keep compiling. `ForgeOp` gained a `PrCheckout` variant (added to `ForgeOp::ALL`)
  so `Forge::supports(ForgeOp::PrCheckout)` reports it available — the one
  `ForgeOp` every real backend supports (only `Unknown` lacks it).
- `PrMerge` — the unified merge spec (`strategy` + `auto` + `delete_branch`),
  built through `PrMerge::merge()`/`squash()`/`rebase()` (or `PrMerge::new(strategy)`)
  then `.auto()`/`.delete_branch()`. Generalises the per-CLI merge specs
  (`vcs-github`'s `PrMerge`, `vcs-gitlab`'s `MrMerge`, `vcs-gitea`'s `PrMerge`) into
  one shape the facade drives across all three backends.

### Changed

- **Breaking: unified DTOs now model "the backend can't report this field" as
  `None`, distinct from a confirmed `false`/empty.** The ambiguous sentinels that
  couldn't tell "unknown" from a real value are replaced by a per-field support
  contract:
  - `ForgePr::draft` `bool` → `Option<bool>` (Gitea is `None` — `tea` has no draft
    flag; GitHub/GitLab report `Some`).
  - `ForgeRepo::private` `bool` → `Option<bool>` (GitLab is `None` when `glab` omits
    `visibility` — an absent visibility is *unknown*, never a false `Some(false)`).
  - `ForgeRelease::url` `String` → `Option<String>`, `ForgeRelease::draft` /
    `prerelease` `bool` → `Option<bool>` (GitHub's lean `release_list` leaves `url`
    `None`; GitLab has no draft/pre-release concept so both are `None`; Gitea has no
    release-page URL so `url` is `None`).

  The `serde`/MCP JSON contract follows: an unknown field serialises to `null`,
  distinct from a confirmed `false`/`[]`. **Builder setters that took no argument now
  take the value** (`ForgePr::draft(bool)`, `ForgeRepo::private(bool)`,
  `ForgeRelease::draft(bool)`/`prerelease(bool)`), and the `labels`/`assignees`/`url`
  setters record a *confirmed* `Some(..)`; a freshly `new()`-built DTO leaves every
  support-gated field `None`. Update a `match`/field read to handle the `Option`
  (e.g. `pr.draft` → `pr.draft == Some(true)`), and `.draft()` → `.draft(true)`.
- **Breaking:** `Forge::pr_merge` / `ForgeApi::pr_merge` take a `PrMerge` spec
  instead of a bare `MergeStrategy` — `pr_merge(n, MergeStrategy::Squash)` →
  `pr_merge(n, PrMerge::squash())`. `PrMerge`'s `auto`/`delete_branch` options are
  **GitHub-only** (`gh pr merge --auto --delete-branch`); on GitLab/Gitea,
  requesting either now returns a structured `Unsupported` rather than silently
  merging without it (which, for an irreversible merge, could produce the wrong
  side effects). `Error::is_unsupported()` now also classifies a wrapper-level
  `Unsupported` bubbling up through `Error::Forge` (the option-can't-be-expressed
  case), not just the facade's own `Error::Unsupported` (whole-operation-missing).
- Internal only (no public API change): the GitHub backend now drives
  `vcs-github`'s spec-typed `pr_close(dir, number, PrClose)` instead of the removed
  positional `delete_branch: bool`. `Forge::pr_close(PrClose)` keeps its existing
  signature.

### Fixed
-

## [0.5.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(forge): gitea PR head_branch strips fork owner: prefix (M26); is_unauthorized keys gh's no-auth phrase instead of the bare 'auth login' verb (M27)
- fix(github): PR draft flag is read from gh's isDraft, not hardcoded false


### Added

- feat(a4): public builder constructors for forge return DTOs (custom ForgeApi backends)


### Changed

- refactor(a5): Forge::issue_create takes an IssueCreate spec (extensible, mirrors PrCreate)
- refactor(a7): make data-carrying RepoEvent/Error variants #[non_exhaustive] (field-safe)
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(forge): is_unauthorized / is_rate_limited error classifiers
- feat(forge): github_with_token / gitlab_with_token convenience constructors
- feat(wave1.5a): is_invalid_input + is_resource_not_found classifiers (A2/A3)


### Changed

- refactor(forge)!: rename vcs_github::Repo + vcs_gitlab::Project to RepoView
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(wave1.5b): Forge::pr_close takes a PrClose spec, not a bare delete_branch bool (A1)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- review: write-gate repo_try_merge, forge Error classifier parity, forge_pr_mark_ready MCP tool
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forge): gitea pr_view page-miss signal + release key aliases; gh pr_checks case-insensitive; forge pr_comment empty-body guard
- fix(forge): github CI aggregate maps all-unknown checks to Pending (gitlab parity)
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor(core+forge): macro-mirror VcsRepo/ForgeApi trait decl + delegating impl (Wave S)
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts
- fix(gitea): re-model tea --output json parsers to tea's table/detail shape (not REST)

## [0.5.1] - 2026-07-05

### Added
- **`Forge`/`Backend` now implement `Debug`**, symmetric with `vcs_core::Repo`'s.
  Hand-written rather than derived: it avoids forcing an `R: Debug` bound onto
  the generic runner type parameter, and it never formats the inner
  `GitHub`/`GitLab`/`Gitea` client — `Backend` prints only its discriminant
  (`GitHub(..)`/`GitLab(..)`/`Gitea(..)`, or plain `Unknown` for the no-client
  backend) via `finish_non_exhaustive`, so a credential token set via
  `with_token` can't leak through `{:?}`.

### Changed
-

### Fixed
-

## [0.5.0] - 2026-07-05

### Added
-

### Changed
-

### Fixed
- **`ForgePr::draft` now reflects a GitHub PR's real draft status** instead of
  always being `false`. `vcs-github` gained the `isDraft` field, so the GitHub
  mapper reads `pr.is_draft` rather than hardcoding `false`; GitLab already
  reported it. (Gitea's `tea` PR list still carries no draft flag, so it remains
  `false` there — documented on the field.)
- **`Error::is_unauthorized` no longer false-fires on `gh`'s wrong-remote hint.** The
  bare `gh auth login` *suggestion* verb was an auth marker, but `gh` also prints it in
  a misconfiguration message ("none of the git remotes … point to a known GitHub host …
  please use `gh auth login`") — so a caller prompted a futile re-login instead of
  surfacing the wrong/absent forge remote. The bare verb is replaced by the unique
  phrase of gh's genuine no-auth message (`get started with github cli`), so that real
  failure — and `not logged in` / `HTTP 401` / `bad credentials` / `authentication
  required`/`failed` — still classify, while the hint does not. (`docs/audit-2026-07.md`
  M27.)

## [0.4.0] - 2026-07-03

### Added
- **Public builder constructors for the return DTOs** `ForgePr`, `ForgeIssue`,
  `ForgeRelease`, and `ForgeRepo` (e.g. `ForgePr::new(number, title, state).source_branch(s)
  .target_branch(t).url(u)`, `ForgeRelease::new(tag).title(t).body(notes)`), plus
  chainable presence-only setters on **`ForgeCapabilities`**
  (`ForgeCapabilities::all_false().pr_create().pr_merge().authed()`). They're
  `#[non_exhaustive]`, so a consumer writing a custom `ForgeApi` backend (a new forge)
  or a test double previously couldn't build one to return — including a `capabilities()`
  override reporting anything but all-`false`. The builders make them constructible
  outside `vcs-forge`. Mirrors `vcs-core`'s return-DTO builders. Additive.
  (`docs/audit-2026-07.md` A4.)
- `Error::unsupported(forge, operation)` — the public constructor for the now-
  `#[non_exhaustive]` `Error::Unsupported` variant (see Changed).
- `IssueCreate` — the open-an-issue spec (`IssueCreate::new(title, body)`), mirroring
  `PrCreate`'s shape and `#[non_exhaustive]` so labels/assignees can be added later
  without a breaking signature change.

### Changed
- **`Error::Unsupported { forge, operation }` is now `#[non_exhaustive]` (breaking).** A
  `match` arm binding its fields must add `..` (`Error::Unsupported { forge, .. }`), and
  the variant can no longer be built by struct literal outside the crate — construct it
  through the new **`Error::unsupported(forge, operation)`** instead (which an external
  `ForgeApi` impl needs to return it). Both changes let the variant gain context (e.g. a
  hint) after 1.0 without a breaking bump. Most callers use `is_*` classifiers, not
  field-matching, so are unaffected. (`docs/audit-2026-07.md` A7.)
- **`Forge::issue_create` / `ForgeApi::issue_create` take an `IssueCreate` spec, not
  bare `(title, body)` args (breaking).** `issue_create("T", "B")` →
  `issue_create(IssueCreate::new("T", "B"))`. Consistent with `pr_create(PrCreate)`,
  and the spec is the extension point for issue labels/assignees (which a bare-`&str`
  signature couldn't grow without breaking) — the reason to reshape it before 1.0.
  Behavior unchanged. (`docs/audit-2026-07.md` A5.)

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.5.2...HEAD
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.5.1...vcs-forge-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.5.0...vcs-forge-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.4.0...vcs-forge-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.3.0...vcs-forge-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.2.0...vcs-forge-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-forge-v0.1.0...vcs-forge-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-forge-v0.1.0
