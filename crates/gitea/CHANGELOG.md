# Changelog — vcs-gitea

All notable changes to the `vcs-gitea` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-gitea-v<version>`.

## [Unreleased]

### Added
- **`is_view_absence` — tell a confirmed issue/PR absence apart from a format drift.**
  `tea` has no single-item view, so `GiteaApi::issue_view`/`pr_view` synthesize one by
  paging `tea … list`; a number that is genuinely absent can therefore only be reported
  as a `processkit::Error::Parse` — the **same variant** a real format drift (our
  positional parser rejecting tea's output, or tea's `unknown output type`) uses. The
  new `vcs_gitea::is_view_absence(&err)` classifier recognises the deliberate
  "confirmed absent" sentinel **by structure** (a shared message prefix the producing
  sites and the classifier build from one constant), so a consumer — e.g. the scheduled
  live CLI-drift test gate, or a caller mapping absence to its own `Ok(None)` —
  distinguishes "this issue/PR doesn't exist" from "tea's output format drifted"
  **without matching the error message text by hand**, and a real drift is never
  misread as an absence. This makes the absence-vs-drift split a **documented, typed
  contract** rather than an incidental string shape. The `issue_view`/`pr_view`
  signatures and the absence `Error::Parse` value (its message included) are
  **unchanged**, so existing consumers — including the `vcs-forge` facade's
  `is_resource_not_found` — keep working; the page-safety-bound miss stays a distinct,
  loud `Error::Parse` for which `is_view_absence` is `false`.
- **Issue lifecycle methods.** `GiteaApi::issue_close(dir, number)` (`tea issues
  close <index>`) and `issue_reopen(dir, number)` (`tea issues reopen <index>`)
  flip an issue's state and return `Result<()>`; `issue_comment(dir, number, body)`
  (`tea comment <index> <body>`) posts a comment and returns the command output,
  reusing the shared `tea comment` subcommand (Gitea's issue/PR index space is
  shared, like `pr_comment`). The comment body is a bare positional, so it is
  guarded with `reject_flag_like`. All three are `at`-forwarded and have defaulted
  `Error::Unsupported` trait bodies so external implementers keep compiling; the
  exact argv is pinned by hermetic tests.

### Changed
- **Read operations now use `--output csv`, not `--output json`.** `auth_status`,
  `pr_list`, `pr_view`, `issue_list`, `issue_view`, and `release_list` ask `tea` for
  its quoted-DSV `csv` output and parse it positionally, replacing the previous
  `--output json` path. `json` is **not** supported on this crate's declared floor —
  `tea` 0.9.x's table printer (`modules/print/table.go`) has no `json` case, so
  `--output json` fell through to its `default` arm and printed `unknown output type
  'json', available types are: …` to stdout **with exit code 0**, which a JSON parser
  then rejected. (`json` was only added in `tea` 0.10.0; newer `tea` also makes the
  unknown-format arm exit non-zero.) `csv` is supported across the whole `tea` 0.9+
  line, so the read surface now works on the floor as well as the latest release.
  Verified by reading tea's source at the `v0.9.2`/`v0.10.0`/`v0.14.2` tags and
  corroborated by the sibling F# port hitting the same `unknown output type 'json'`
  against live `tea` 0.9.2; the `#[ignore]` real-`tea` tests in `tests/cli.rs` are the
  ongoing live confirmation. The positional DSV reader handles both of tea's CSV
  dialects — the naive always-quoted `","`-joined format (0.9.x–0.13.x) and the
  RFC-4180 `encoding/csv` format (0.14.x), including quoted multi-line issue bodies.
- **`issue_view` is now synthesized by paging `tea issues list`.** `tea`'s bare-index
  single-issue view (`tea issues <n>`) renders Markdown and ignores `--output`, so it
  can't be parsed structurally; `issue_view` now pages `tea issues list --state all`
  (`--output csv`) and filters by number, like `pr_view` — finding a closed issue and
  one past the server's ~50-row page cap. The public `GiteaApi::issue_view` signature
  and the `Issue` DTO are **unchanged**; a genuine absence is an `Error::Parse`.

### Fixed
- **`pr_edit` now fails safely instead of silently listing PRs.** `tea` 0.9.2 has no
  `pr edit` subcommand and degrades an unknown subcommand into `pr list`; the concrete
  client implementation was removed so `GiteaApi::pr_edit` now uses its default
  `Error::Unsupported` result without spawning `tea`. Use the Gitea REST API to edit a
  pull request.
- **PR merge argv now puts flags before the index.** `GiteaApi::pr_merge` emits
  `tea pr merge --style <style> <index>`. This is required by tea 0.9.x's urfave-cli
  parser, which stops parsing flags after the first bare positional.
- **Empty/header-only reads parse as an empty list; `unknown output type` is a loud
  error.** tea always prints a DSV header even for zero rows, and some builds print
  nothing — both now read as an empty list rather than an error, while the
  `unknown output type` diagnostic (the exit-0 signal that a requested `--output`
  format is unsupported) is turned into an `Error::Parse` instead of a silently-empty
  result, so a format regression can't pass unnoticed.
- **Format-drift test gate no longer degrades into an environment-skip.** The
  `#[ignore]` real-`tea` suite (`tests/cli.rs`) and the weekly `gitea-live` workflow
  lane now classify a format mismatch — our parser rejecting tea's output, **or** tea
  reporting `unknown output type` — as an explicit test **failure**, not a silent skip;
  only a genuine environment error (no repo, no login, network) is skipped. The
  `gitea-live` lane also runs against both the `tea` 0.9.x floor and the latest release,
  so a floor-only format regression is exercised instead of missed by a latest-only run.

## [0.7.0] - 2026-07-19

### Added
- **PR review methods.** `GiteaApi::pr_approve(dir, number)` (`tea pr approve
  <index>`) records an approving review, and `pr_reject(dir, number, body)` (`tea pr
  reject <index> <reason>`) requests changes with a required reason. The `reason` is
  a bare positional, so it is guarded with `reject_flag_like` (a leading `-` or empty
  value is refused before spawning), like `pr_comment`'s body. Both are
  `at`-forwarded, return `Result<()>`, and have defaulted `Error::Unsupported` trait
  bodies so external implementers keep compiling. The exact argv is pinned by
  hermetic tests (the live commands mutate a real PR's review state).

### Changed
-

### Fixed
-

## [0.6.0] - 2026-07-10

### Added
- **`tea` version floor + capability gate.** New `GiteaCapabilities` (`version:
  GiteaVersion`), probed via `GiteaApi::capabilities()` (`tea --version`, parsed
  with the shared `vcs-diff` version parser the way `vcs-git`/`vcs-jj` do — the
  first dotted-numeric token wins, so any emoji/build trailer is ignored; an
  unrecognisable banner is an `Error::Parse`). `is_supported()` /
  `ensure_supported()` gate on the crate's declared floor **tea ≥ 0.9.0** — the
  `tea` line whose `--output json`/`--fields` read surface, `pr create`/`merge`/
  `close`/`checkout` lifecycle verbs, and `comment` this crate all drive. A too-old
  `tea` is now rejected up front with a clear "needs tea ≥ 0.9.0, found 0.8.0"
  message rather than failing deep inside an operation with a cryptic `unknown
  command`/`unknown flag`. `GiteaVersion` (an alias of `vcs_diff::Version`) is
  re-exported, and the bound `GiteaAt` view forwards `capabilities()`. Adds a
  `vcs-diff` dependency (the shared version type/parser).
- `GiteaApi::pr_checkout(dir, number)` — check a pull request's branch out into
  the working copy (`tea pr checkout <n>`); the head branch is fetched and
  switched to, so a build/test/edit runs against the PR locally. Mutates the
  working copy. Mirrored on the `GiteaAt` bound view. **Defaulted** to
  `Error::Unsupported` on the trait so external implementers keep compiling.
- `PrMerge` — a `#[non_exhaustive]` merge spec (`strategy` + `auto` +
  `delete_branch`), built through `PrMerge::merge()`/`squash()`/`rebase()` then
  `.auto()`/`.delete_branch()`. Shares the shape of `vcs-github`'s `PrMerge` and
  `vcs-gitlab`'s `MrMerge` so the `vcs-forge` facade drives one merge spec across
  all three backends.

### Changed
- deps: bump `mockall` to 0.15 (unified workspace dependency, was 0.13 per-crate).
- **Breaking:** the raw escape hatches on the bound view (`GiteaAt::run`/`run_raw`/
  `run_args`/`run_raw_args`) now run **in the bound `dir`** instead of the process's
  current directory. Previously they sat in the `bare` forwarder group, so
  `tea.at(dir).run(…)` silently ran in the process cwd — a bound handle whose raw call
  could target a *different* repository than the one it was bound to. New dir-taking
  client methods `Gitea::run_in`/`run_raw_in`/`run_args_in`/`run_raw_args_in` back the
  bound forwarders (argv forwarded verbatim; only the cwd is bound). The
  **process-cwd** escape hatch is unchanged and still reached by calling
  `run`/`run_raw`/… on `Gitea` itself (`tea.run(…)`) — migrate a caller that relied on
  `tea.at(dir).run(…)` running in the process cwd to `tea.run(…)`. (T-035.)
- **Breaking:** `GiteaApi::pr_merge` takes a `PrMerge` spec instead of a bare
  `MergeStrategy` — `pr_merge(dir, n, MergeStrategy::Squash)` →
  `pr_merge(dir, n, PrMerge::squash())`. The `GiteaAt` bound view moves to the
  same spec. `tea pr merge` can express **neither** the gh-style `auto`
  (merge-once-checks-pass) nor `delete_branch` option, so setting either on
  `PrMerge` now returns a structured `Error::Unsupported` rather than silently
  merging without it (which, for an irreversible merge, could produce the wrong
  side effects). The default (neither set) is unchanged: the plain `--style` merge.

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


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forge): gitea pr_view page-miss signal + release key aliases; gh pr_checks case-insensitive; forge pr_comment empty-body guard
- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts
- fix(gitea): re-model tea --output json parsers to tea's table/detail shape (not REST)

## [0.5.1] - 2026-07-05

### Added
- **`Gitea<R>` now implements `Debug`.** Added by hand (not via
  `vcs_cli_support::managed_client!` — `Gitea` is scaffolded by the external
  `processkit::cli_client!` macro instead, which doesn't generate one), no
  `R: Debug` bound, delegating to the wrapped `processkit::CliClient`'s own
  Debug-safe impl. `tea` is ambient-auth-only, so there's no token to leak, but
  the impl stays consistent with the other four CLI wrapper types.

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
- **A fork PR's `head_branch` is now a flat branch name.** tea renders a cross-fork
  PR's head as `owner:branch` (and `<marker>:branch` for a deleted fork), unlike the
  plain branch it renders for a same-repo PR — and unlike GitHub/GitLab's flat head. The
  parser now strips the `owner:` prefix, so `head_branch` is always the bare branch (a
  same-repo head, which has no `:`, is unchanged). (`docs/audit-2026-07.md` M26.)

## [0.4.0] - 2026-07-03

### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forge): gitea pr_view page-miss signal + release key aliases; gh pr_checks case-insensitive; forge pr_comment empty-body guard
- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts
- fix(gitea): re-model tea --output json parsers to tea's table/detail shape (not REST)

## [0.3.0] - 2026-07-03

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_gitea::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` types (`Error`/`ProcessResult`/…)
  1.x — **breaking** for a downstream that pins `processkit` `0.x` directly. No
  behaviour change. processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.
- Internal: the JSON parse helpers `null_to_empty` (the `null → ""`
  `deserialize_with`) and `from_json` (the `Error::Parse`-mapping decoder) now come
  from `vcs_cli_support::json` instead of being defined locally, so the three forge
  parsers share one convention. Requires cli-support's new `serde` feature (enabled
  via the dependency). No public API or behaviour change.

### Fixed
- **`pr_view` no longer returns a false "not found" for a PR past the server's page
  cap.** The Gitea server clamps an API page to `MAX_RESPONSE_ITEMS` (default 50) and
  `tea` makes one call per page, so the previous single `tea pr list --limit 999` was
  silently capped at ~50 rows — a PR numbered beyond that got a confident false
  `Error::Parse "no such PR"`. `pr_view` now **pages** through (`--page N`) until the
  PR is found or a page comes back empty (a genuine absence), so it finds a PR
  regardless of repo size. The walk stops on an *empty* page (not a short one), so it
  is robust to an instance whose page cap is below the request; and the list parsers
  now read empty/whitespace stdout as an empty list (some `tea` builds print nothing,
  not `[]`, for an empty result) rather than a spurious parse error.
  (`docs/audit-2026-07.md` H8.)
- **Documented the list verbs' honest server-side cap.** `pr_list` / `issue_list` /
  `release_list` return **at most ~50** rows on a default Gitea instance (the same
  `MAX_RESPONSE_ITEMS` clamp), not the "up to 100" the old `--limit 100` comment
  implied; the docs now say so and point at `run` (`--page N`) for the rest. Behavior
  unchanged — only the misleading comment/doc is corrected. (`docs/audit-2026-07.md` H8.)

## [0.2.0] - 2026-06-27

### Added
- `pr_comment(dir, number, body)` — add a comment to a pull request,
  returning the command's output (`tea comment <index> <body>`). Gitea PRs
  and issues share the `index` space and the same `tea comment` subcommand
  hits both. The `body` is a bare positional, so it is argv-guarded with
  `reject_flag_like` (a leading `-` or empty value is rejected before any
  process spawns) — the first such guard in this crate.
- `pr_edit(dir, number, PrEdit)` — edit a pull request's title and/or
  description (`tea pr edit <index> [--title <title>] [--description <body>]`).
  A new `PrEdit` builder (`new()`, `.title(..)`, `.body(..)`) carries the
  optional fields; absent flags are not emitted. An empty string is treated
  as a real value (tea clears the field on `--title ""` / `--description ""`),
  not as `None`. The trait methods are **defaulted** to `Error::Unsupported`
  so external implementers keep compiling when the crate bumps — only the
  `Gitea` concrete impl and the regenerated `MockGiteaApi` override them.
- `vcs-cli-support` added as a direct dependency (for `reject_flag_like`,
  needed by `pr_comment`).

### Changed
- Documented that **Gitea authentication is ambient**: unlike the new
  `vcs-github`/`vcs-gitlab` per-operation `with_credentials` token providers,
  `tea` has no non-interactive per-invocation token mechanism (it authenticates
  from `tea login add` only), so `Gitea` offers no credential injection.
  `vcs-cli-support`'s `CredentialService::Gitea` is reserved for if/when `tea`
  gains env-token support.
- Bumped `processkit` to **0.11.0** (from 0.9.1), a major breaking release ahead
  of processkit's 1.0 freeze. Breaking for downstream via the re-exported
  `processkit::Error`: `Error::Timeout`/`Signalled` now carry partial
  `stdout`/`stderr`, `Error::Signalled`/`NotFound`/`CassetteMiss` are first-class
  variants, the blanket `From<io::Error>` is gone, and `Invocation::cwd` is now
  `Option<PathBuf>`.

### Removed
- The **`cancellation`** feature — cancellation is always available now
  (processkit 0.10 made it core), so the `cli_client!`-generated
  `default_cancel_on(token)` and the re-exported `CancellationToken` no longer sit
  behind a feature. Downstream that enabled `vcs-gitea/cancellation` should drop it.

### Fixed
- `pr_view` reports a *possible page-miss* when its `tea pr list --limit 999`
  listing fills the cap and the requested number isn't found — distinguishing
  "this PR exists but is beyond the page" from a flat "no such PR" on a very large
  repo (instead of an indistinguishable not-found either way).
- `Release` JSON parsing gained `serde` **aliases** for the cleaned/camelCase/raw
  key forms (`tag_name`/`tag-name`/`tagName`/`Tag-Name`, `published_at`/…) alongside
  tea's current quirky `toSnakeCase` keys (`tag-_name`, `published _at`), so a
  future `tea` that fixes its header casing doesn't silently break release parsing.
- The typed single-issue view (`tea issues <n>`) tolerates a JSON `null`
  `body`/`url` (an issue with no description) instead of failing the whole parse —
  `#[serde(default)]` only covered an absent key, not a present `null`.

## [0.1.0] - 2026-06-08

### Added
- Initial release: `GiteaApi` trait + `Gitea` client wrapping the `tea` CLI,
  mirroring `vcs-github`'s shape (async, `#[non_exhaustive]` DTOs, the structured
  `processkit::Error`, the `mock` feature → `MockGiteaApi`, and the
  `Gitea::with_runner` scripted-runner seam).
- The **lean pull-request lifecycle** `tea` supports: `auth_status` (a non-empty
  `login list`), `pr_list` (`PullRequest`), `pr_view` (synthesized by listing
  with `--state all` and filtering by number — `tea` has no single-PR view),
  `pr_create(PrCreate)`, `pr_merge(number, MergeStrategy)`
  (`--style merge|rebase|squash`), and `pr_close`.
- **Issues and releases**: `issue_list` (`Vec<Issue>`), `issue_view(number)` (the
  first-class `tea issues <n>` single-issue view), `issue_create(title, body)`,
  and `release_list` (`Vec<Release>`). No `release_view` — `tea releases` always
  lists.
- Raw escape hatches `run`/`run_raw` (+ inherent `run_args`/`run_raw_args`), and
  a `Gitea::at(dir)` → `GiteaAt` bound view mirroring every repo-scoped method.

### Notes
- Deliberately narrower than `vcs-github`/`vcs-gitlab`: `tea` exposes no
  current-repo view, no draft toggle, no PR-checks command, and no single-release
  view, so `repo_view`, `pr_mark_ready`, `pr_checks`, and `release_view` are
  absent (the `vcs-forge` facade reports them as `Unsupported` for the Gitea
  backend).
- **`tea --output json` is modeled, not the Gitea REST API.** Its **list**
  commands emit tea's print-*table* (a JSON array of string-maps; snake-cased
  column-header keys that can contain spaces/slashes; **all values strings**; no
  `html_url`, no nested branch objects), and its **detail** view (`issues <n>`) a
  separate *typed* object. The parsers select columns with `--fields` and
  string-parse the `index`. Consequences: a PR's merge state rides the `state`
  column (`"merged"`), and a `Release` carries **no web URL** (`tea releases`
  exposes only a tar/zip download URL, not surfaced). **This contract is derived
  by reading tea's source (`gitea.com/gitea/tea` `main`; the `PullFields`/
  `IssueFields` sets confirmed identical on the released v0.14.1), not validated
  end-to-end** — confirm it against a live `tea` (the `#[ignore]` integration
  tests in `tests/cli.rs`) before the first release.

### Changed
- Bumped `processkit` to **0.8** — the re-exported `Error`/`ProcessResult` carry
  through 0.8 (`Error` still `#[non_exhaustive]` with `NotReady`/`Unsupported` and
  feature-gated `Cancelled`/`ResourceLimit`; `Error::Exit` Display gained a
  stderr-tail suffix; `Command` is `#[must_use]`). **Breaking** for consumers that
  match the re-exported types exhaustively, or that bump their own direct
  `processkit` separately — caret `"0.7"` does not span 0.8, so bump together.
- Internal: the `CliClient` verbs the wrapper bodies call were renamed to one
  shared vocabulary (`text`→`run`, `capture`→`output`, `unit`→`run_unit`,
  `code`→`exit_code`); no public-API or built-argv change.
- New off-by-default **`cancellation`** feature: pulls in processkit's
  `cancellation`, so `cli_client!` emits `default_cancel_on(token)` on the client —
  build a cancellable client (every command it runs dies when the token fires) and
  pass it through the facade. No new vcs-* API; `CancellationToken` is re-exported
  from `processkit`.
- `auth_status` tolerates a non-zero `tea login list` exit (e.g. no config file
  yet) and reports `false` instead of erroring, matching its "reports the bool,
  must not error" contract.
- `pr_create` doc: tea prints a textual summary (no URL) and has no flag to
  shape the create output — documented instead of implied parity with gh/glab.
- `pr_create` now takes a `PrCreate` spec
  (`PrCreate::new(title, body).head(…).base(…)`) instead of positional
  `title, body, head, base` arguments, mirroring `vcs-git`'s `GitPush` builder
  style. The built argv is unchanged.

### Fixed
- `pr_list` passes `--limit 100` (tea's default page of 30 silently truncated
  larger sets), and `pr_view` — which lists and filters by number — uses
  `--limit 999`, so a PR beyond the first page is no longer a false "not found"
  (PRs beyond 999 still are; documented).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.7.0...HEAD
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.6.0...vcs-gitea-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.5.2...vcs-gitea-v0.6.0
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.5.1...vcs-gitea-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.5.0...vcs-gitea-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.4.0...vcs-gitea-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.3.0...vcs-gitea-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.2.0...vcs-gitea-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.1.0...vcs-gitea-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-gitea-v0.1.0
