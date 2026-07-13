# Changelog — vcs-gitlab

All notable changes to the `vcs-gitlab` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-gitlab-v<version>`.

## [Unreleased]

### Added
- **MR review methods.** `GitLabApi::mr_approve(dir, number)` (`glab mr approve
  <id>`) records an approval, and `mr_revoke(dir, number)` (`glab mr revoke <id>`)
  withdraws it — GitLab's approve/revoke review model (there is no "request
  changes" action, unlike GitHub's `pr review --request-changes`). Both take the
  `u64` MR id (never flag-like — no positional guard needed) and return `Result<()>`;
  both are `at`-forwarded, and both have defaulted `Error::Unsupported` trait bodies
  so external implementers keep compiling. The exact argv is pinned by hermetic
  tests (the live commands mutate a real MR's approval state).

### Changed
-

### Fixed
-

## [0.6.0] - 2026-07-10

### Added
- **`glab` version floor + capability gate.** New `GitLabCapabilities` (`version:
  GitLabVersion`), probed via `GitLabApi::capabilities()` (`glab --version`, parsed
  with the shared `vcs-diff` version parser the way `vcs-git`/`vcs-jj` do — the
  first dotted-numeric token wins, so any build/commit trailer is ignored; an
  unrecognisable banner is an `Error::Parse`). `is_supported()` /
  `ensure_supported()` gate on the crate's declared floor **glab ≥ 1.25.0** — the
  modern `glab` line whose `--output json` read surface, `mr update`/`mr checkout`/
  `mr merge` lifecycle verbs, and `api` this crate all drive. A too-old `glab` is
  now rejected up front with a clear "needs glab ≥ 1.25.0, found 1.20.0" message
  rather than failing deep inside an operation with a cryptic `unknown
  command`/`unknown flag`. `GitLabVersion` (an alias of `vcs_diff::Version`) is
  re-exported, and the bound `GitLabAt` view forwards `capabilities()`.
- `MergeRequest`/`Issue` gained `labels: Vec<String>` (GitLab's REST API already
  reports these as plain strings) and `assignees: Vec<String>` (flattened from
  the REST `assignees` array of User objects' `username`).
- `GitLabApi::mr_checkout(dir, number)` — check a merge request's source branch
  out into the working copy (`glab mr checkout <id>`); the branch is fetched and
  switched to, so a build/test/edit runs against the MR locally. Mutates the
  working copy. Mirrored on the `GitLabAt` bound view. **Defaulted** to
  `Error::Unsupported` on the trait so external implementers keep compiling.
- `MrMerge` — a `#[non_exhaustive]` merge spec (`strategy` + `auto` +
  `delete_branch`), built through `MrMerge::merge()`/`squash()`/`rebase()` then
  `.auto()`/`.delete_branch()`. Shares the shape of `vcs-github`'s `PrMerge` and
  `vcs-gitea`'s `PrMerge` so the `vcs-forge` facade drives one merge spec across
  all three backends.

### Changed
- deps: bump `mockall` to 0.15 (unified workspace dependency, was 0.13 per-crate).
- **Breaking:** the raw escape hatches on the bound view (`GitLabAt::run`/`run_raw`/
  `run_args`/`run_raw_args`) now run **in the bound `dir`** instead of the process's
  current directory. Previously they sat in the `bare` forwarder group, so
  `glab.at(dir).run(…)` silently ran in the process cwd — a bound handle whose raw
  call could target a *different* project (`glab` infers the project from the cwd's
  remote) than the one it was bound to, now consistent with `api`. New dir-taking
  client methods `GitLab::run_in`/`run_raw_in`/`run_args_in`/`run_raw_args_in` back the
  bound forwarders (argv forwarded verbatim; only the cwd is bound). The
  **process-cwd** escape hatch is unchanged and still reached by calling
  `run`/`run_raw`/… on `GitLab` itself (`glab.run(…)`) — migrate a caller that relied
  on `glab.at(dir).run(…)` running in the process cwd to `glab.run(…)`. (T-035.)
- **Breaking:** `GitLabApi::mr_merge` takes an `MrMerge` spec instead of a bare
  `MergeStrategy` — `mr_merge(dir, id, MergeStrategy::Squash)` →
  `mr_merge(dir, id, MrMerge::squash())`. The `GitLabAt` bound view moves to the
  same spec. `glab mr merge` can express **neither** the gh-style `auto`
  (merge-once-requirements-met) nor `delete_branch` option — glab's own
  `--auto-merge` is a different, merge-when-pipeline contract — so setting either
  on `MrMerge` now returns a structured `Error::Unsupported` rather than silently
  merging without it (which, for an irreversible merge, could produce the wrong
  side effects). The default (neither set) is unchanged: the plain immediate merge.

### Fixed
- `mr_create`, `mr_edit`, `issue_create`, and `mr_comment` now refuse a
  description/comment body that is *exactly* `"-"` before spawning glab,
  surfacing an `Error::Spawn` with `io::ErrorKind::InvalidInput` — glab treats
  a bare `-` as a request to open `$EDITOR`/read from stdin rather than the
  literal string, which would otherwise hang a headless caller indefinitely
  (glab has no timeout of its own on the prompt).

## [0.5.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(forge)!: rename vcs_github::Repo + vcs_gitlab::Project to RepoView
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- refactor(api): git current_branch -> Option; gitlab mr id -> number (pre-1.0 consistency)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.1] - 2026-07-05

### Added
- **`GitLab<R>` now implements `Debug`**, via the shared `vcs_cli_support::managed_client!`
  macro (no code change here). No `R: Debug` bound; a token configured via
  `with_token` is never printed, only whether a credential provider is set.

### Changed
-

### Fixed
-

## [0.5.0] - 2026-07-05

### Changed

- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(forge)!: rename vcs_github::Repo + vcs_gitlab::Project to RepoView
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- refactor(api): git current_branch -> Option; gitlab mr id -> number (pre-1.0 consistency)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Added

- feat(wrappers): re-export ProcessRunner + JobRunner so consumers needn't depend on processkit directly


### Changed

- refactor(forge)!: rename vcs_github::Repo + vcs_gitlab::Project to RepoView
- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(cli-support): share one at_forwarders! macro across the 5 wrappers
- refactor(cli-support): managed_client! macro for the common wrapper scaffold
- refactor(cli-support): hoist forge JSON helpers (null_to_empty, from_json) behind a serde feature
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)


### Added

- feat(retry+ci): is_transient classifier (R9), fetch timeout_grace (R10), report-only semver-checks CI (R3), >4KiB classification regression test (R2)
- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
- feat(credentials): CredentialProvider abstraction + forge (gh/glab) token injection (Phase 1)
- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- refactor: adopt processkit 0.10 direct-arg-list verbs (drop self.core.command double-mention) + envs() for env sets
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- refactor(api): git current_branch -> Option; gitlab mr id -> number (pre-1.0 consistency)
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(forges): tolerate JSON null in optional string fields; jj self-rename guard
- fix(watch+testkit+forge+gitlab): doc + isolation minors


### Added

- feat(forge): vcs-gitlab + vcs-gitea + vcs-forge facade (Wave D)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.3.0] - 2026-07-03

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_gitlab::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` types (`Error`/`ProcessResult`/…)
  1.x — **breaking** for a downstream that pins `processkit` `0.x` directly. No
  behaviour change. processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.
- **Renamed the `repo_view` DTO `Project` → `RepoView` (breaking).** The struct
  returned by `repo_view` (and re-exported at the crate root) is now `RepoView`,
  for a consistent name across the forge wrappers (its fields are still GitLab's
  REST `Project` object); update `use vcs_gitlab::Project` to
  `use vcs_gitlab::RepoView`. Fields and behaviour are unchanged.
- **Renamed `GitLabApi::mr_ready` → `mr_mark_ready` (breaking).** The draft→ready
  method (and its `at(dir)` bound form) is now `mr_mark_ready`, matching
  `vcs-github`'s `pr_mark_ready`; the emitted `glab mr update <id> --ready`
  command is unchanged. Update callers of `mr_ready` to `mr_mark_ready`.
- Internal: the JSON parse helpers `null_to_empty` (the `null → ""`
  `deserialize_with`) and `from_json` (the `Error::Parse`-mapping decoder) now come
  from `vcs_cli_support::json` instead of being defined locally, so the three forge
  parsers share one convention. Requires cli-support's new `serde` feature (enabled
  via the dependency). No public API or behaviour change.

### Fixed
- **`api` now targets the bound repository, not the process's current directory
  (breaking: `api(endpoint)` → `api(dir, endpoint)`).** It builds `glab api` with the
  repo dir as its working directory, so a relative endpoint resolves the project from
  *that* repo's remote. Previously it ran in the process cwd, so a client bound to
  `/repo-a` while the process sat in `/repo-b` hit the **wrong project**. The `at(dir)`
  bound form (`GitLabAt::api(endpoint)`) is unchanged. (`docs/audit-2026-07.md` H9.)

## [0.2.0] - 2026-06-27

### Added
- **Per-operation credentials (opt-in).** `GitLab::with_credentials(provider)`
  accepts a `CredentialProvider` (re-exported from `vcs-cli-support`, along with
  `Credential`/`Secret`/`StaticCredential`/`EnvToken`/`provider_fn`), plus the
  convenience `GitLab::with_token(token)` / `with_env_token(var)` for the common
  cases. The resolved token is injected as `GITLAB_TOKEN` on every `glab` invocation
  — never in `argv` — overriding the ambient login. Default is no provider →
  ambient `glab` auth, unchanged. (Internally the client now wraps
  `vcs-cli-support`'s `ManagedClient`
  instead of the `cli_client!` macro; the public constructor/builder surface is
  unchanged.)
- `GitLabApi::api(endpoint)` — the `glab api` escape hatch for any unmodelled
  REST/GraphQL endpoint (mirrors `GitHubApi::api`), with the same flag-injection
  guard on `endpoint`.
- `Release::description` — release notes (GitLab's `description`), surfaced by the
  `vcs-forge` facade as `ForgeRelease::body`.
- `mr_comment(dir, number, body)` — add a comment to a merge request, returning
  the command's output (`glab mr note <number> -m <body>`). `-m` is a flag-VALUE
  position so no argv-guard is needed.
- `mr_edit(dir, number, MrEdit)` — edit a merge request's title and/or description
  (`glab mr update <number> [--title <title>] [--description <body>] --yes`).
  `--yes` skips the confirmation prompt. A new `MrEdit` builder (`new()`,
  `.title(..)`, `.body(..)`) carries the optional fields; absent flags are
  not emitted. An empty string is treated as a real value (glab clears the
  field on `--title ""` / `--description ""`), not as `None`. The trait
  methods are **defaulted** to `Error::Unsupported` so external implementers
  keep compiling when the crate bumps — only the `GitLab` concrete impl and
  the regenerated `MockGitLabApi` override them.

### Changed
- Documented that `CiStatus::Pending` also covers GitLab's *blocked-awaiting-action*
  pipeline states (`manual`/`scheduled`/`waiting_for_resource`): they bucket as
  `Pending` ("not known to be done"), so a poller looping until a pipeline leaves
  `Pending` must bound its wait — a `manual` pipeline stays blocked until triggered.
  Behaviour unchanged; doc-only clarification.
- **The `mr_*` methods' id parameter is renamed `id` → `number` (breaking).**
  `mr_view`/`mr_merge`/`mr_ready`/`mr_close`/`mr_comment`/`mr_edit`/`mr_checks` now
  take `number: u64`, matching this crate's own `issue_*` methods and the other
  forge wrappers (`vcs-github`/`vcs-gitea`) and facade — the value is still GitLab's
  project-scoped `iid`. Call sites pass it positionally, so most are unaffected.
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
  behind a feature. Downstream that enabled `vcs-gitlab/cancellation` should drop it.

### Fixed
- **Tolerate a JSON `null` in optional string fields.** GitLab's REST API (which
  `glab` emits verbatim) sends a *present* `null` — not an absent key — for an
  empty optional value (an issue/MR with no `description`, a project with no
  `default_branch`, a release with no `name`/`released_at`/`description`). The
  `#[serde(default)]` attribute only covers an absent key, so a present `null`
  previously failed the **entire** parse with "invalid type: null, expected a
  string". These fields now deserialize a `null` to an empty string, so the most
  common real responses parse.

## [0.1.0] - 2026-06-08

### Added
- Initial release: `GitLabApi` trait + `GitLab` client wrapping the `glab` CLI,
  mirroring `vcs-github`'s shape (async, `#[non_exhaustive]` DTOs, the structured
  `processkit::Error`, the `mock` feature → `MockGitLabApi`, and the
  `GitLab::with_runner` scripted-runner seam).
- The **lean merge-request lifecycle**, deserializing `glab … --output json`
  (GitLab's REST JSON): `auth_status`, `repo_view` (`Project`),
  `mr_list`/`mr_view` (`MergeRequest`), `mr_create(MrCreate)`
  → URL, `mr_merge(id, MergeStrategy)` (merges **immediately** via
  `--auto-merge=false`, overriding glab's default merge-when-pipeline-succeeds;
  `--squash`/`--rebase`/default merge), `mr_ready`, `mr_close`, and `mr_checks`
  → `CiStatus` (the MR's bucketed `head_pipeline.status`).
- `auth_status` documents the glab exit-code caveat ([gitlab-org/cli#911]): a
  `true` is best-effort (glab can exit 0 while unauthenticated); `false`/timeout
  are faithful.

[gitlab-org/cli#911]: https://gitlab.com/gitlab-org/cli/-/issues/911
- Raw escape hatches `run`/`run_raw` (+ inherent `run_args`/`run_raw_args`), and
  a `GitLab::at(dir)` → `GitLabAt` bound view mirroring every project-scoped
  method.

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
- `Project.visibility` is now `Option<String>` (absent in the JSON → `None`
  instead of a misleading empty string).
- `auth_status` reports `false` on **any** non-zero exit (was: errored on exits
  other than 0/1), matching its "reports the bool, must not error" contract.
- `mr_create` now takes an `MrCreate` spec
  (`MrCreate::new(title, body).source(…).target(…)`) instead of positional
  `title, body, source, target` arguments, mirroring `vcs-git`'s `GitPush`
  builder style. The built argv is unchanged.

### Fixed
- `mr_list` passes `--per-page 100` — glab's default of 30 silently truncated
  larger result sets. The cap is now explicit and documented.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.6.0...HEAD
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.5.2...vcs-gitlab-v0.6.0
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.5.1...vcs-gitlab-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.5.0...vcs-gitlab-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.4.0...vcs-gitlab-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.3.0...vcs-gitlab-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.2.0...vcs-gitlab-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.1.0...vcs-gitlab-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-gitlab-v0.1.0
