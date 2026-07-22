# Changelog — vcs-github

All notable changes to the `vcs-github` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-github-v<version>`.

## [Unreleased]

### Added
- **GitHub Actions run control.** Three new `GitHubApi` methods close the CI
  automation loop alongside the existing read-only `run_list`/`run_view`/
  `run_watch`. `workflow_dispatch(dir, WorkflowDispatch)` fires a
  `workflow_dispatch` event (`gh workflow run <workflow> [--ref <ref>]
  [--raw-field key=value …]`) through a new `#[non_exhaustive]` `WorkflowDispatch`
  builder (`new(workflow)` + chained `.git_ref(..)` / `.field(k, v)`); it returns
  `Result<()>` because GitHub's dispatch API replies `204 No Content` with no run
  id (poll `run_list` for the started run). Inputs are emitted with `--raw-field`,
  **not** `--field` — the latter's `@value` reads a *file*, so the raw form keeps a
  value like `@/etc/passwd` a literal string. `run_rerun(dir, id, RerunScope)`
  reruns a completed run (`gh run rerun <id>`), where the new `#[non_exhaustive]`
  `Copy` enum `RerunScope::{All, FailedOnly}` selects `--failed` (a direct argument,
  since one toggle doesn't reach the builder bar). `run_cancel(dir, id)` cancels an
  in-progress run (`gh run cancel <id>`). The bare `<workflow>` positional is
  flag-injection guarded (like `release_view`'s tag); the `u64` run ids can never
  look like a flag, so they need none; `--ref`/`--raw-field` values ride in
  flag-VALUE slots (verbatim-safe, like `--branch`). Exit codes follow gh's
  convention (`gh help exit-codes`, empirically checked against gh 2.95.0: **0**
  success, **1** failure such as an unknown workflow / already-completed run,
  **4** unauthenticated). All three are `at`-forwarded and have defaulted
  `Error::Unsupported` trait bodies so external implementers keep compiling; the
  exact argv is pinned by hermetic tests.
- **Issue lifecycle methods.** `GitHubApi::issue_close(dir, number)` (`gh issue
  close <n>`) and `issue_reopen(dir, number)` (`gh issue reopen <n>`) flip an
  issue's state and return `Result<()>`; `issue_comment(dir, number, body)`
  (`gh issue comment <n> --body <body>`) posts a comment and returns its URL. The
  comment body rides in a flag-VALUE slot, so a leading `-` is safe (no positional
  guard needed). All three are `at`-forwarded and have defaulted
  `Error::Unsupported` trait bodies so external implementers keep compiling; the
  exact argv is pinned by hermetic tests.
- **`PullRequest`/`Issue`/`Release` gain `author`/timestamp/`milestone` fields.**
  `PullRequest` and `Issue` gain `author: String`, `created_at: String`,
  `updated_at: String` (all `gh --json` RFC 3339/login fields), and
  `milestone: Option<String>`; `Release` gains `author: String`. `author` and
  `milestone` flatten gh's nested `{"login": …}`/`{"title": …}` objects (a `null`
  author — a deleted account — becomes an empty string; a `null` milestone
  becomes `None`). `PR_FIELDS`/`ISSUE_LIST_FIELDS`/`ISSUE_VIEW_FIELDS`/
  `RELEASE_LIST_FIELDS`/`RELEASE_VIEW_FIELDS` are widened accordingly.

### Changed
-

### Fixed
-

## [0.11.0] - 2026-07-19

### Added

- Add examples/ directories to vcs-core and vcs-forge
- Add a Windows integration CI lane with real git/jj binaries
- Add a scheduled CI lane against latest jj/glab/tea releases
- Add Dependabot config for GitHub Actions and Cargo
- Add scripts/gate as a single local CI-gate entry point
- Add path-scoped history log to git and jj clients
- Add report-only test coverage measurement to CI
- Add backoff retry and terminal-failure signaling to vcs-watch loop
- Add a capability/version gate for forge CLI wrappers
- Add configurable output budget for large content operations


### Changed

- Reconcile processkit 2.1.1 migration with second concurrent crate-version release
- github, gitlab, forge, mcp: add typed pr_diff/mr_diff
- forge, github, gitlab: add labels/assignees to ForgePr/ForgeIssue
- github, gitlab, gitea, forge, mcp: add pr_checkout
- Replace remaining positional bool parameters with typed spec structs
- Unify PR/MR merge spec across forge wrappers
- Bind bound-view raw command escape hatches to their directory
- Model unknown forge DTO values instead of false defaults
- Bind bound-view raw command escape hatches to their directory
- Apply rustfmt to the five CLI wrapper crates
- Support GitHub Enterprise token and host-scoped auth probe
- Avoid interactive editor for dash-body input in glab commands
- Make retry backoff sleep sensitive to cancellation
- Pass host context into CredentialProvider for repo-scoped operations
- Support argv-limit-safe bulk path operations in git add/commit/log
- Turn the release workflow into a SemVer and package gate
- Include the MIT license text in every published crate package
- Include the MIT license text in every published crate package
- Unify mockall to a single workspace-level version
- Stabilize the vcs-watch dependency boundary and tracing feature
- Restore canonical repository instructions and refresh the stability matrix
- Make path-carrying APIs lossless for non-UTF-8 filenames
- Release: vcs-diff v0.6.0, vcs-cli-support v0.6.0, vcs-git v0.10.0, vcs-jj v0.10.0, vcs-github v0.10.0, vcs-gitlab v0.6.0, vcs-gitea v0.6.0, vcs-forge v0.6.0, vcs-testkit v0.6.0, vcs-core v0.8.0, vcs-watch v0.6.0, vcs-mcp v0.6.0


### Fixed

- Fix CRLF line endings introduced into 13 Cargo.toml manifests


### Changed

- Reconcile processkit 2.1.1 migration with concurrent crate-version release
- Release: vcs-diff v0.5.2, vcs-cli-support v0.5.2, vcs-git v0.9.2, vcs-jj v0.9.2, vcs-github v0.9.2, vcs-gitlab v0.5.2, vcs-gitea v0.5.2, vcs-forge v0.5.2, vcs-testkit v0.5.2, vcs-core v0.7.2, vcs-watch v0.5.2, vcs-mcp v0.5.2


### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(github): PR draft flag is read from gh's isDraft, not hardcoded false


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

- fix(github): null-tolerant Review/Comment string fields (uniform with the crate's other --json DTOs)
- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)
- fix(wave2): bound gh run watch's discarded output buffer (R5)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
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
- fix(forge): github CI aggregate maps all-unknown checks to Pending (gitlab parity)


### Added

- feat(github): PR lifecycle — merge/ready/close, checks, runs, review/comment/feedback, issues, releases
- feat: injection guards + validating newtypes, Git::hardened, typed conflict model
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts


### Added

- feat: cwd-bound handles, wider facade, new ops, VcsRepo trait


### Changed

- deps: processkit 0.6 — probe() predicates + transient fetch-retry
- Release: vcs-git v0.4.0, vcs-jj v0.4.0, vcs-github v0.4.0, vcs-core v0.2.0


### Changed

- Release: vcs-git v0.3.1, vcs-jj v0.3.1, vcs-github v0.3.1, vcs-core v0.1.0


### Added

- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics
- feat(github): query PRs by head->base branch; allow head in pr_create


### Changed

- deps: bump processkit 0.4 -> 0.5; absorb breaking API changes
- Release: vcs-git v0.3.0, vcs-jj v0.3.0, vcs-github v0.3.0


### Changed

- Release: vcs-git v0.2.1, vcs-jj v0.2.1, vcs-github v0.2.1


### Changed

- Release: vcs-git v0.2.0, vcs-jj v0.2.0, vcs-github v0.2.0


### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

## [0.10.0] - 2026-07-10

### Added
- **`gh` version floor + capability gate.** New `GitHubCapabilities` (`version:
  GitHubVersion`), probed via `GitHubApi::capabilities()` (`gh --version`, parsed
  with the shared `vcs-diff` version parser the way `vcs-git`/`vcs-jj` do — the
  first dotted-numeric token wins, so gh's `(date)`/release-URL trailers are
  ignored; an unrecognisable banner is an `Error::Parse`). `is_supported()` /
  `ensure_supported()` gate on the crate's declared floor **gh ≥ 2.0.0** — the
  first modern `gh` line whose `--json` read surface, `pr edit`/`pr checkout`/
  `pr ready` lifecycle verbs, and `api` this crate all drive. A too-old `gh` is now
  rejected up front with a clear "needs gh ≥ 2.0.0, found 1.14.0" message rather
  than failing deep inside an operation with a cryptic `unknown command`/`unknown
  flag`. `GitHubVersion` (an alias of `vcs_diff::Version`) is re-exported, and the
  bound `GitHubAt` view forwards `capabilities()`.
- **GitHub Enterprise Server (GHES) credentials + host-scoped auth.** A new
  `GitHubHost` type models the target host (SaaS `github.com` vs a GHES host),
  built via `GitHubHost::github_com()`, `GitHubHost::new("ghe.example.com")`, or
  `GitHubHost::from_remote_url(url)` (HTTPS/SSH/scp-like remotes; userinfo and port
  dropped). `GitHub::with_host(host)` binds a client to it: a supplied credential
  is then injected into the environment variable `gh` reads for **that** host —
  `GH_TOKEN` for github.com, `GH_ENTERPRISE_TOKEN` for a GHES host — plus `GH_HOST`
  is pinned, so an enterprise secret never lands in the github.com token env (nor
  vice versa) and the secret stays out of `argv`. `GitHubApi::auth_status_for(&host)`
  probes a single host (`gh auth status --hostname <host>`), so a broken or absent
  session for a *different* host can't turn the check into a false negative for the
  host you target; it is **defaulted** on the trait (external implementers keep
  compiling) and mirrored on the `GitHubAt` bound view. An empty, malformed, or
  undeterminable host is a diagnosable error (`GitHubHost::new`/`from_remote_url`
  return `Err`), never a silent fall back to the github.com token. Without a host
  binding the client is unchanged (github.com / `GH_TOKEN`).
- **Host-keyed credential providers.** A `GitHub::with_host(host)`-bound client now
  also carries the (canonical) host in every operation's `CredentialRequest`, so a
  **host-keyed** `CredentialProvider` returns the secret for *that* instance and
  nothing else — one provider safely serves several host-bound clients without
  cross-injecting a neighbour's token. A provider `Err` is fail-closed (the op aborts
  before `gh` spawns) and `Ok(None)` defers to ambient auth, for read and write
  alike. Without a host binding the request carries no host (unchanged). (T-045.)
- `PullRequest`/`Issue` gained `labels: Vec<String>` and `assignees: Vec<String>`,
  parsed from `gh --json labels,assignees`'s nested `[{"name": …}]`/
  `[{"login": …}]` shapes and flattened to plain strings.
- `GitHubApi::pr_checkout(dir, number)` — check a pull request's branch out into
  the working copy (`gh pr checkout <n>`); the head branch is fetched and switched
  to, so a build/test/edit runs against the PR locally. Mutates the working copy.
  Mirrored on the `GitHubAt` bound view. **Defaulted** to `Error::Unsupported` on
  the trait so external implementers keep compiling.

### Changed

- deps: bump `mockall` to 0.15 (unified workspace dependency, was 0.13 per-crate).
- **Breaking:** the raw escape hatches on the bound view (`GitHubAt::run`/`run_raw`/
  `run_args`/`run_raw_args`) now run **in the bound `dir`** instead of the process's
  current directory. Previously they sat in the `bare` forwarder group, so
  `gh.at(dir).run(…)` silently ran in the process cwd — a bound handle whose raw call
  could target a *different* repository (`gh` infers the repo from the cwd's remote)
  than the one it was bound to, now consistent with `api`. New dir-taking client
  methods `GitHub::run_in`/`run_raw_in`/`run_args_in`/`run_raw_args_in` back the bound
  forwarders (argv forwarded verbatim; only the cwd is bound). The **process-cwd**
  escape hatch is unchanged and still reached by calling `run`/`run_raw`/… on `GitHub`
  itself (`gh.run(…)`) — migrate a caller that relied on `gh.at(dir).run(…)` running
  in the process cwd to `gh.run(…)`. (T-035.)
- **Breaking:** `Release::body` and `Release::url` are now `Option<String>`
  (were `String`). `release_list` doesn't request either field (RELEASE_LIST_FIELDS
  omits them), so an absent value now reads as the honest `None` ("not fetched")
  rather than a false empty string; `release_view` fills both as `Some`. A present
  JSON `null` also reads as `None`. Update a read to unwrap the `Option` (e.g.
  `release.body` → `release.body.as_deref()`). This is what lets the `vcs-forge`
  facade surface a release's `url` as `Some` only when it was actually fetched.
- **Breaking:** `GitHubApi::pr_close` drops its trailing positional
  `delete_branch: bool` for a named `#[non_exhaustive]` `PrClose` spec —
  `pr_close(dir, number, true)` → `pr_close(dir, number,
  PrClose::new().delete_branch())` — so the flag reads at the call site (mirroring
  `PrMerge`). The `GitHubAt` bound view moves to the same spec.
- **No API change here**, noted for the ecosystem: this crate's `PrMerge`
  (`strategy` + `auto` + `delete_branch`) is now the reference shape the sibling
  wrappers adopt for a **unified merge spec** — `vcs-gitlab` gained `MrMerge`,
  `vcs-gitea` gained `PrMerge`, and the `vcs-forge` facade a `PrMerge` DTO, each
  with the same fields. `gh pr merge` is the only backend that can express `auto`
  (`--auto`) and `delete_branch` (`--delete-branch`); GitLab/Gitea report those two
  options `Unsupported`. `GitHubApi::pr_merge` is unchanged.

### Fixed
-

## [0.9.2] - 2026-07-06

### Added

- feat: add Debug to Forge/Backend and the five CLI wrapper clients


### Changed

- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Fixed

- fix(github): PR draft flag is read from gh's isDraft, not hardcoded false


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

- fix(github): null-tolerant Review/Comment string fields (uniform with the crate's other --json DTOs)
- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)
- fix(wave2): bound gh run watch's discarded output buffer (R5)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
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
- fix(forge): github CI aggregate maps all-unknown checks to Pending (gitlab parity)


### Added

- feat(github): PR lifecycle — merge/ready/close, checks, runs, review/comment/feedback, issues, releases
- feat: injection guards + validating newtypes, Git::hardened, typed conflict model
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts


### Added

- feat: cwd-bound handles, wider facade, new ops, VcsRepo trait


### Changed

- deps: processkit 0.6 — probe() predicates + transient fetch-retry
- Release: vcs-git v0.4.0, vcs-jj v0.4.0, vcs-github v0.4.0, vcs-core v0.2.0


### Changed

- Release: vcs-git v0.3.1, vcs-jj v0.3.1, vcs-github v0.3.1, vcs-core v0.1.0


### Added

- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics
- feat(github): query PRs by head->base branch; allow head in pr_create


### Changed

- deps: bump processkit 0.4 -> 0.5; absorb breaking API changes
- Release: vcs-git v0.3.0, vcs-jj v0.3.0, vcs-github v0.3.0


### Changed

- Release: vcs-git v0.2.1, vcs-jj v0.2.1, vcs-github v0.2.1


### Changed

- Release: vcs-git v0.2.0, vcs-jj v0.2.0, vcs-github v0.2.0


### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

## [0.9.1] - 2026-07-05

### Added
- **`GitHub<R>` now implements `Debug`**, via the shared `vcs_cli_support::managed_client!`
  macro (no code change here). No `R: Debug` bound; a token configured via
  `with_token` is never printed, only whether a credential provider is set.

### Changed
-

### Fixed
-

## [0.9.0] - 2026-07-05

### Added
-

### Changed
- **`pr list`/`pr view` now request the `isDraft` JSON field**, exposed as
  `PullRequest::is_draft`. `PR_FIELDS` previously omitted it, so a PR's draft
  status was invisible (the `vcs-forge` `ForgePr::draft` was consequently always
  `false` for GitHub). The field deserializes with `#[serde(default)]` for
  robustness (defaults to `false` if a payload omits it).

### Fixed
-

## [0.8.0] - 2026-07-03

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

- fix(github): null-tolerant Review/Comment string fields (uniform with the crate's other --json DTOs)
- fix(wave2): gh/glab api() binds the repo dir instead of process cwd (H9)
- fix(wave2): bound gh run watch's discarded output buffer (R5)


### Added

- feat(api)!: Tier-1 interface — RepoSnapshot tracking cohesion, CheckBucket enum, unified git log, aligned status fields
- feat(forge)!: capability introspection (supports/capabilities), DTO field parity (labels/assignees/draft/prerelease), glab api() parity
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
- fix(forge): github CI aggregate maps all-unknown checks to Pending (gitlab parity)


### Added

- feat(github): PR lifecycle — merge/ready/close, checks, runs, review/comment/feedback, issues, releases
- feat: injection guards + validating newtypes, Git::hardened, typed conflict model
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- refactor: extract vcs-diff + vcs-cli-support foundational crates
- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: review follow-ups — docs, CI, Windows paths, mappers, and tests
- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts


### Added

- feat: cwd-bound handles, wider facade, new ops, VcsRepo trait


### Changed

- deps: processkit 0.6 — probe() predicates + transient fetch-retry
- Release: vcs-git v0.4.0, vcs-jj v0.4.0, vcs-github v0.4.0, vcs-core v0.2.0


### Changed

- Release: vcs-git v0.3.1, vcs-jj v0.3.1, vcs-github v0.3.1, vcs-core v0.1.0


### Added

- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics
- feat(github): query PRs by head->base branch; allow head in pr_create


### Changed

- deps: bump processkit 0.4 -> 0.5; absorb breaking API changes
- Release: vcs-git v0.3.0, vcs-jj v0.3.0, vcs-github v0.3.0


### Changed

- Release: vcs-git v0.2.1, vcs-jj v0.2.1, vcs-github v0.2.1


### Changed

- Release: vcs-git v0.2.0, vcs-jj v0.2.0, vcs-github v0.2.0


### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

## [0.7.0] - 2026-07-03

### Added
- Re-export of `processkit::ProcessRunner` and `JobRunner` (`vcs_github::{ProcessRunner,
  JobRunner}`) — so a consumer naming the client's runner type parameter (for
  `with_runner`, or to write a custom `ProcessRunner`) needn't add a direct `processkit`
  dependency. Joins the existing `Error`/`Result`/`ProcessResult` re-exports.

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). Crossing
  processkit's 1.0 makes the re-exported `processkit` types (`Error`/`ProcessResult`/…)
  1.x — **breaking** for a downstream that pins `processkit` `0.x` directly. No
  behaviour change. processkit is semver-stable from 1.0, so future 1.x updates are non-breaking.
- **Renamed the `repo_view` DTO `Repo` → `RepoView` (breaking).** The struct
  returned by `repo_view` (and re-exported at the crate root) is now `RepoView`,
  for a consistent name across the forge wrappers; update
  `use vcs_github::Repo` to `use vcs_github::RepoView`. Fields and behaviour are
  unchanged.
- **Renamed `GitHubApi::pr_ready` → `pr_mark_ready` (breaking).** The draft→ready
  method (and its `at(dir)` bound form) is now `pr_mark_ready`, for a clearer
  mark-ready verb; the emitted `gh pr ready <n>` command is unchanged. Update
  callers of `pr_ready` to `pr_mark_ready`.
- Internal: the JSON parse helpers `null_to_empty` (the `null → ""`
  `deserialize_with`) and `from_json` (the `Error::Parse`-mapping decoder) now come
  from `vcs_cli_support::json` instead of being defined locally, so the three forge
  parsers share one convention. Requires cli-support's new `serde` feature (enabled
  via the dependency). No public API or behaviour change.

### Fixed
- **`run_watch` no longer accumulates `gh run watch`'s output unboundedly.** `gh run
  watch` re-prints the full job table every ~3 s until the run ends, so a multi-hour
  watch grew its (entirely discarded — only the exit status matters) stdout to tens of
  MB. The retained buffer is now bounded (drop-oldest, last 256 lines / 256 KiB), so a
  long watch runs in constant memory; failure messages are unaffected.
  (`docs/audit-2026-07.md` R5.)
- **`api` now targets the bound repository, not the process's current directory
  (breaking: `api(endpoint)` → `api(dir, endpoint)`).** It builds `gh api` with the
  repo dir as its working directory, so a relative endpoint's `{owner}/{repo}`
  placeholder resolves against the bound repo. Previously it ran in the process cwd,
  so a client bound to `/repo-a` while the process sat in `/repo-b` hit the **wrong
  repository**. The `at(dir)` bound form (`GitHubAt::api(endpoint)`) is unchanged.
  (`docs/audit-2026-07.md` H9.)
- `Review` / `Comment` string fields now tolerate an explicit JSON `null` from `gh`
  (decoded as empty), matching how the crate's other `--json` DTOs already handle a
  present-but-null optional field — a null no longer fails the parse.

## [0.6.0] - 2026-06-27

### Added
- **Per-operation credentials (opt-in).** `GitHub::with_credentials(provider)`
  accepts a `CredentialProvider` (re-exported from `vcs-cli-support`, along with
  `Credential`/`Secret`/`StaticCredential`/`EnvToken`/`provider_fn`), plus the
  convenience `GitHub::with_token(token)` / `with_env_token(var)` for the common
  cases. The resolved token is injected as `GH_TOKEN` on every `gh` invocation —
  never in `argv` — overriding the ambient login. Default is no provider → ambient
  `gh` auth, unchanged. (Internally the client now wraps `vcs-cli-support`'s
  `ManagedClient` instead of the `cli_client!` macro; the public constructor/builder
  surface is unchanged.)
- `CheckBucket` enum (`Pass`/`Fail`/`Pending`/`Skipping`/`Cancel`/`Unknown`) with
  `is_failing`/`is_pending`/`is_passing`/`is_unknown` helpers — the typed form of
  gh's check categorisation, `#[non_exhaustive]` with an `Unknown` catch-all so a
  future gh bucket never breaks the parse. `is_unknown` distinguishes that catch-all
  (an unmodeled/missing bucket) from a deliberate `Skipping`, so an aggregator can
  treat it conservatively.
- `pr_edit(dir, number, PrEdit)` — edit a pull request's title and/or body
  (`gh pr edit <n> [--title <title>] [--body <body>]`). A new `PrEdit` builder
  (`new()`, `.title(..)`, `.body(..)`) carries the optional fields; absent
  flags are not emitted, so the argv reflects exactly the fields the caller
  set. An empty string is treated as a real value (gh clears the field on
  `--title ""` / `--body ""`), not as `None`. The trait method is
  **defaulted** to `Error::Unsupported` so external implementers keep
  compiling when the crate bumps — only the `GitHub` concrete impl and the
  regenerated `MockGitHubApi` override it.

### Changed
- `issue_list` now fetches `body` and `url` too (widened `--json` field list), so
  the listed `Issue`s carry them instead of leaving them empty until `issue_view`.
- **`CheckRun::bucket` is now `CheckBucket` (breaking)**, replacing the
  stringly-typed `String` — exhaustive matching instead of comparing string slices.
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
  behind a feature. Downstream that enabled `vcs-github/cancellation` should drop it.

### Fixed
- `pr_checks` detects gh's "no checks reported" (a PR with no checks → empty list)
  case-insensitively, so a capitalization tweak in gh's wording can't turn the
  no-checks case into a hard error.
- **Tolerate a JSON `null` in optional string fields.** `gh` emits a *present*
  `null` for some optional values — notably `headRefName`/`baseRefName` on a PR
  whose head branch was deleted, plus null `body`/`url`/timestamps. `#[serde(default)]`
  only covers an absent key, so a present `null` previously failed the whole parse
  with "invalid type: null, expected a string". These fields (on `PullRequest`,
  `Issue`, `WorkflowRun`, `CheckRun`, `Release`) now deserialize a `null` to an
  empty string.

## [0.5.0] - 2026-06-08

### Added
- PR lifecycle mutations: `pr_merge(dir, n, PrMerge)` — a `PrMerge` builder
  (`merge()`/`squash()`/`rebase()`, `.auto()`, `.delete_branch()`);
  `pr_ready(dir, n)`; `pr_close(dir, n, delete_branch)`.
- `pr_checks(dir, n)` → `Vec<CheckRun>` (`pr checks --json …`). gh signals the
  overall outcome via its exit code (0 pass / 8 pending / 1 some failed) but
  prints the same JSON for all three — all return the parsed list; branch on
  `CheckRun::bucket` (`pass`/`fail`/`pending`/`skipping`/`cancel`).
- Reviews and comments: `pr_review(dir, n, ReviewAction)` — `ReviewAction`
  (`approve()` / `request_changes(body)` / `comment(body)`, `.with_body(..)`,
  `kind()`/`body()`) carries a required body for request-changes/comment by
  construction, so an empty-body request-changes is unrepresentable;
  `pr_comment(dir, n, body)` → URL; `pr_feedback(dir, n)` → `PrFeedback`
  (reviews + conversation comments from `pr view --json reviews,comments`,
  nested authors flattened).
- GitHub Actions runs: `run_list(dir, limit, branch)` / `run_view(dir, id)` →
  `WorkflowRun` (`conclusion` is an *empty string* until the run completes —
  gh's shape), and `run_watch(dir, id)` — blocks until the run finishes, then
  returns the final `WorkflowRun` (the watch exit code can't distinguish a
  failed run from a cancelled one, so the outcome is read via `run view`).
  `run_watch` under a client `default_timeout` is killed at the deadline.
- Issues and releases: `issue_create(dir, title, body)` → URL;
  `issue_view(dir, n)` (fills the new `Issue::body`/`Issue::url`);
  `release_list(dir)` / `release_view(dir, tag)` → `Release` (`is_latest` is
  reported by `list` only).
- All new dir-taking methods are mirrored on the `GitHubAt` bound view.
- Injection guards on the exposed positional arguments (`api` endpoint,
  `release_view` tag): a leading-`-` or empty value is refused **before**
  anything spawns. Flag-value positions (`--body`, `--branch`) need no
  guard — gh consumes the next token verbatim there.

### Changed
- **Breaking:** `pr_create` now takes a single `PrCreate` spec
  (`pr_create(dir, PrCreate)`) instead of the `(title, body, head, base)`
  argument list. Build it with `PrCreate::new(title, body)` plus the chained
  `.head(..)` / `.base(..)` setters. Argv unchanged.
- **Breaking:** `ReviewAction` is now a struct with **private** fields built via
  `approve()` / `request_changes(body)` / `comment(body)` (`.with_body(..)`,
  `kind()`/`body()` accessors, and the new public `ReviewKind` enum) instead of
  the `Approve(Option<String>)` / `RequestChanges(String)` / `Comment(String)`
  enum. This makes a body-less request-changes/comment unrepresentable. Argv
  unchanged.
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
  from `processkit`. The `run_watch` cancellation path is covered by a hermetic
  paused-clock test (`Reply::pending()`).
- Internal: the argv injection guard (`reject_flag_like`) now comes from the
  shared `vcs-cli-support` crate. No public API change.
- `auth_status` reports `false` on **any** non-zero exit (was: errored on exits
  other than 0/1), matching its "reports the bool, must not error" contract.

### Fixed
- `pr_list`/`pr_list_for_branch`/`issue_list`/`release_list` pass `--limit 100`
  — gh's default of 30 silently truncated larger result sets. The cap is now
  explicit and documented (use `run()` for more).

## [0.4.0] - 2026-06-04

### Added
- `GitHub::at(dir)` → `GitHubAt`, a cwd-bound view whose repo-scoped methods omit
  the leading `dir` argument (`gh.at(dir).pr_list()`).

### Changed
- Bumped `processkit` to 0.6; `auth_status` uses processkit's `probe()` (exit `0`/`1`
  → bool, anything else → error). No API change.

### Fixed
-

## [0.3.1] - 2026-06-03

### Added

- feat: Step B + 1d + 1e — error classifiers, status/diff_stat consistency, &[&str] ergonomics
- feat(github): query PRs by head->base branch; allow head in pr_create


### Changed

- deps: bump processkit 0.4 -> 0.5; absorb breaking API changes
- Release: vcs-git v0.3.0, vcs-jj v0.3.0, vcs-github v0.3.0


### Changed

- Release: vcs-git v0.2.1, vcs-jj v0.2.1, vcs-github v0.2.1


### Changed

- Release: vcs-git v0.2.0, vcs-jj v0.2.0, vcs-github v0.2.0


### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

## [0.3.0] - 2026-06-02

### Added
- Inherent `GitHub::run_args` / `run_raw_args` taking `&[&str]`, so callers
  needn't allocate a `Vec<String>` for the `run` escape hatch.
- `pr_list_for_branch(dir, head, base)` — PRs that merge `head` into `base` in
  any state (`gh pr list --head <head> --base <base> --state all --json …`), each
  carrying its title, URL, and state.

### Changed
- `pr_create` gained a `head: Option<String>` parameter (before `base`) so a PR
  can target an explicit source branch (`gh pr create --head <head>`); `None`
  keeps the previous behaviour (head = current branch).
- Bumped `processkit` to 0.5. No change to the rest of this crate's public API.

### Fixed
-

## [0.2.1] - 2026-06-01

### Added
-

### Changed
- Bumped `processkit` to 0.4 — macOS/BSD process trees are now contained via a
  POSIX process group (`killpg` on drop) instead of an uncontained spawn.

### Fixed
-

## [0.2.0] - 2026-06-01

### Added

- feat(process): job-backed spawn (JobObject/cgroup) + publish setup
- feat: typed command wrappers, exec options, integration tests
- feat: mockable trait-based API + Runner injection
- feat: async (tokio) API, timeouts, structured errors, richer models
- feat: non_exhaustive result structs, optional tracing, cli_client! macro


### Changed

- Scaffold vcs-toolkit-rs workspace from rust-repo-template
- review: harden whole solution, fix potential issues
- refactor: portable Output model, CliClient core, richer test seam, -z git parsing
- refactor: replace internal vcs-process with external processkit 0.3
- ci: release workflow picks major/minor/patch with auto-increment (+ all-crates, first-release)
- Release: vcs-git v0.1.0, vcs-jj v0.1.0, vcs-github v0.1.0

## [0.1.0] - 2026-06-01

### Added
- `GitHubApi` trait + `GitHub` client with typed commands deserializing
  `gh … --json` into structs: `pr_list`/`pr_view` (`PullRequest`), `issue_list`
  (`Issue`), `repo_view` (`Repo`), `auth_status`, and raw `api`. Adds
  `serde`/`serde_json`.
- **Mockable by design:** consumers code against `GitHubApi`; `GitHub::with_runner`
  injects a fake process runner, and the `mock` feature generates `MockGitHubApi`
  (via `mockall`).
- `pr_create` and raw `run`/`run_raw` on `GitHubApi`.
- `PullRequest` gained `base_ref_name` and `url`; `Repo` now has `owner`, `url`,
  `is_private`, and `default_branch`.
- `GitHub::default_timeout` kills any command exceeding the deadline.

### Changed
- The API is now the `GitHub` client + `GitHubApi` trait — the original free
  functions are gone. Commands launch `gh` inside an OS job (Windows Job Object /
  Linux cgroup v2) via `processkit`, killed on close.
- **Now async (tokio):** every `GitHubApi` method is `async`; errors are the typed
  `processkit::Error` (JSON parse failures become `Error::Parse`).
  Adds `async-trait`.
- Built on the external **`processkit`** crate (the `CliClient` core, the
  `cli_client!` macro, the `ProcessRunner` seam, and the structured `Error`) —
  replacing the prototype internal `vcs-process` crate. `run_raw` now returns
  `processkit::ProcessResult<String>`.
- `PullRequest`/`Issue`/`Repo` are now `#[non_exhaustive]` — future fields won't
  be breaking changes.
- Optional `tracing` feature (forwards to `processkit/tracing`): a `debug` event
  per `gh` command.

### Fixed
- `auth_status` no longer reports "not authenticated" when `gh auth status` times
  out — a timeout surfaces as `processkit::Error::Timeout` (via `CliClient::code`,
  backed by processkit 0.3's first-class timeout error).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.11.0...HEAD
[0.11.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.10.0...vcs-github-v0.11.0
[0.10.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.9.2...vcs-github-v0.10.0
[0.9.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.9.1...vcs-github-v0.9.2
[0.9.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.9.0...vcs-github-v0.9.1
[0.9.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.8.0...vcs-github-v0.9.0
[0.8.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.7.0...vcs-github-v0.8.0
[0.7.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.6.0...vcs-github-v0.7.0
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.5.0...vcs-github-v0.6.0
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.4.0...vcs-github-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.3.1...vcs-github-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.3.0...vcs-github-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.2.1...vcs-github-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.2.0...vcs-github-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.1.0...vcs-github-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-github-v0.1.0
