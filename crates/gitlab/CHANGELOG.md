# Changelog — vcs-gitlab

All notable changes to the `vcs-gitlab` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-gitlab-v<version>`.

## [Unreleased]

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
- `mr_comment(dir, id, body)` — add a comment to a merge request, returning
  the command's output (`glab mr note <id> -m <body>`). `-m` is a flag-VALUE
  position so no argv-guard is needed.
- `mr_edit(dir, id, MrEdit)` — edit a merge request's title and/or description
  (`glab mr update <id> [--title <title>] [--description <body>] --yes`).
  `--yes` skips the confirmation prompt. A new `MrEdit` builder (`new()`,
  `.title(..)`, `.body(..)`) carries the optional fields; absent flags are
  not emitted. An empty string is treated as a real value (glab clears the
  field on `--title ""` / `--description ""`), not as `None`. The trait
  methods are **defaulted** to `Error::Unsupported` so external implementers
  keep compiling when the crate bumps — only the `GitLab` concrete impl and
  the regenerated `MockGitLabApi` override them.

### Changed
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
-

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitlab-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-gitlab-v0.1.0
