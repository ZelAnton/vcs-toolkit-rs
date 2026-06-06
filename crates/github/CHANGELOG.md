# Changelog — vcs-github

All notable changes to the `vcs-github` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-github-v<version>`.

## [Unreleased]

### Added
- PR lifecycle mutations: `pr_merge(dir, n, PrMerge)` — a `PrMerge` builder
  (`merge()`/`squash()`/`rebase()`, `.auto()`, `.delete_branch()`);
  `pr_ready(dir, n)`; `pr_close(dir, n, delete_branch)`.
- `pr_checks(dir, n)` → `Vec<CheckRun>` (`pr checks --json …`). gh signals the
  overall outcome via its exit code (0 pass / 8 pending / 1 some failed) but
  prints the same JSON for all three — all return the parsed list; branch on
  `CheckRun::bucket` (`pass`/`fail`/`pending`/`skipping`/`cancel`).
- Reviews and comments: `pr_review(dir, n, ReviewAction)` — the body travels in
  the variant (`Approve(Option<String>)` / `RequestChanges(String)` /
  `Comment(String)`), so an empty-body request-changes is unrepresentable;
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
-

### Fixed
-

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

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.4.0...HEAD
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.3.1...vcs-github-v0.4.0
[0.3.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.3.0...vcs-github-v0.3.1
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.2.1...vcs-github-v0.3.0
[0.2.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.2.0...vcs-github-v0.2.1
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-github-v0.1.0...vcs-github-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-github-v0.1.0
