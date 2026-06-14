# Changelog — vcs-mcp

All notable changes to the `vcs-mcp` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-mcp-v<version>`.

## [Unreleased]

### Changed
- **Tool JSON output reflects the updated `vcs-core`/`vcs-forge` DTOs (breaking for
  wire consumers).** `repo_snapshot` now nests upstream tracking under one
  `tracking` object (`{branch, ahead, behind}` or `null`) instead of three flat
  `upstream`/`ahead`/`behind` fields; release results carry `body`/`draft`/
  `prerelease`; issue results carry `body`/`url`; PR check `bucket` is the typed
  `CheckBucket` value.
- Bumped `processkit` to **0.10.1**. Test doubles moved to `processkit::testing`;
  cancellation is now core (no feature flag).

### Fixed
-

## [0.1.0] - 2026-06-08

### Added
- Initial release: `vcs-mcp`, a Model Context Protocol (MCP) server exposing the
  `vcs-core` (`Repo`) and `vcs-forge` (`Forge`) operations as agent-callable
  tools. A lib (`VcsMcpServer`, hermetically testable) plus the `vcs-mcp` binary,
  which serves MCP over **stdio** for an `mcpServers` config entry. The workspace's
  **first binary crate** and **second runtime-tokio** crate (after `vcs-watch`).
- **Read tools** (always available, annotated `readOnlyHint`): `repo_snapshot`,
  `repo_info`, `repo_status`, `repo_diff_stat`, `repo_branches`,
  `repo_current_branch`, `repo_conflicts`, `repo_worktrees`, `repo_try_merge`
  (a rollback merge probe); forge: `forge_auth_status`, `forge_repo_view`,
  `forge_pr_list`, `forge_pr_view`, `forge_pr_checks`, `forge_issue_list`,
  `forge_issue_view`, `forge_release_list`, `forge_release_view`. Each returns
  the facade DTO as JSON (via the facades' optional `serde` feature).
- **Mutating tools** (gated, annotated `destructiveHint`): `repo_commit`,
  `repo_checkout`, `repo_fetch`, `repo_push`, `repo_create_worktree`,
  `repo_remove_worktree`; forge: `forge_pr_create`, `forge_pr_merge`,
  `forge_pr_close`, `forge_issue_create`. Outside the write gate they reject up
  front — naming the tool — before spawning anything.
- **`WriteGate`** — the server's write policy (`None` / `All` /
  `Set(HashSet<tool name>)`), checked by every mutating tool under its own name.
  `VcsMcpServer::new` takes it in place of a coarse bool.
- **CLI:** `--repo <path>` (default cwd), `--forge github|gitlab|gitea` (override),
  `--allow-write` (every mutation), `--allow-tools <name,…>` (a per-tool
  allowlist; comma-separated, repeatable, accumulates; `--allow-write` wins when
  both are given), `--timeout <seconds>` (per-command deadline, default 120; `0`
  disables), `--help`. With neither write flag the server is read-only. The
  forge is auto-detected from the `origin` remote (`ForgeKind::from_remote_url`)
  — works on a colocated jj repo; a pure-jj repo with no git remote has no
  forge, and the `forge_*` tools then return a clear "no forge configured"
  error.
- **Hardened by default:** the binary opens the repo with a hardened git client
  (`Git::hardened()` — repo hooks and `core.fsmonitor` disabled, repo-redirecting
  `GIT_*` scrubbed, system config skipped), so serving a repository you didn't
  create can't execute its hooks even on a read tool. jj has no repo-local hooks.
  Every git/forge command also runs under the `--timeout` deadline so a stalled
  network call can't hang a request. The server advertises its identity as
  `vcs-mcp` (with the crate version) over the MCP wire.
- The tool logic, write-gating, serialization, and the `#[tool_router]`/
  `#[tool_handler]` wiring are covered hermetically (a `ScriptedRunner`-backed
  `Repo`, plus an in-process rmcp client round-trip over an in-memory duplex
  transport); `#[ignore]` tests drive the read tools and a gated mutation against a
  real temporary git repo.

### Notes
- Built on [`rmcp`](https://crates.io/crates/rmcp) (the official MCP Rust SDK).
  Read-only by default. The wrappers' argv injection guards apply under every
  tool.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-mcp-v0.1.0
