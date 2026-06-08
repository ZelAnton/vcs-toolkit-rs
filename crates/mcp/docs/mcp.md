# vcs-mcp — the MCP server

`vcs-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) **server**
that exposes the toolkit's typed repository operations as MCP **tools**, so an
agent harness (Claude Code, an IDE assistant, any MCP client) drives a git/jj repo
— and its forge — through **structured, validated calls** instead of raw shell.
Each tool wraps a [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/) (`Repo`) or [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)
(`Forge`) operation and returns its DTO as JSON. The binary drives git through a
**hardened** client (`Git::hardened()` — repo hooks and config disabled) and every
tool argument flows through the wrappers' injection guards (`reject_flag_like`), so
serving a repository you didn't create can't run its hooks or smuggle a flag into
argv.

It's the workspace's **first binary crate** — a thin `vcs-mcp` binary over a
hermetically-testable library (`VcsMcpServer`) — and its **second runtime-tokio**
crate (after [`vcs-watch`](https://docs.rs/vcs-watch/latest/vcs_watch/guide/)).

**Read tools are always available; mutating tools are gated.** Every mutation is
registered and annotated `destructiveHint`, but rejects calls unless the server's
**write gate** covers it: `--allow-write` enables every mutation, `--allow-tools
repo_commit,forge_pr_create` enables only the named ones.

## Launching the server

The binary speaks MCP over **stdio**; point a client at it through an
`mcpServers` config entry. Read-only over the current directory:

```json
{
  "mcpServers": {
    "vcs": {
      "command": "vcs-mcp",
      "args": ["--repo", "/path/to/repo"]
    }
  }
}
```

Allowing mutations and forcing a forge:

```json
{
  "mcpServers": {
    "vcs": {
      "command": "vcs-mcp",
      "args": ["--repo", "/path/to/repo", "--forge", "github", "--allow-write"]
    }
  }
}
```

Install it with `cargo install vcs-mcp` (or point `command` at a built binary).

### CLI flags

```text
vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
        [--allow-tools <name,…>] [--timeout <seconds>]
```

| Flag | Effect |
|---|---|
| `--repo <path>` | Repository to serve (default: the current directory); git vs jj is detected from the path. |
| `--forge <github\|gitlab\|gitea>` | Force the forge for the PR/MR tools. Default: auto-detect from the `origin` remote. |
| `--allow-write` | Enable **all** mutating tools. Off by default — read tools only. |
| `--allow-tools <name,…>` | Enable **only the named** mutating tools (comma-separated; repeatable — occurrences accumulate). Tool names are the method names from the catalogue below. Read tools are unaffected. `--allow-write` wins when both are given. |
| `--timeout <seconds>` | Per-command deadline so a stalled fetch/forge call can't hang a request (default: 120; `--timeout 0` disables it). |
| `-h`, `--help` | Print usage and exit. |

## Tool catalogue

### Read tools (always available, `readOnlyHint`)

| Tool | Params | Returns |
|---|---|---|
| `repo_snapshot` | — | The batched [`RepoSnapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/): branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, operation state. |
| `repo_info` | — | `{ backend, root, cwd, forge }` — git/jj, the repo root, the working dir, and the configured forge (or null). |
| `repo_status` | — | The working-copy changes (added/modified/deleted/renamed paths). |
| `repo_diff_stat` | — | Aggregate insertion/deletion/file counts for the working copy. |
| `repo_branches` | — | Local branch (git) / bookmark (jj) names. |
| `repo_current_branch` | — | The current branch/bookmark (null when detached/unset). |
| `repo_conflicts` | — | Paths with unresolved merge conflicts. |
| `repo_worktrees` | — | Attached worktrees (git) / workspaces (jj). |
| `repo_try_merge` | `{ source }` | Whether merging `source` would conflict — a **probe** that's always rolled back (read-only, but it spawns a real trial merge). |
| `forge_auth_status` | — | Whether the forge CLI reports an authenticated session. |
| `forge_repo_view` | — | The repository/project on the forge (`Unsupported` on Gitea). |
| `forge_pr_list` | — | Open pull/merge requests. |
| `forge_pr_view` | `{ number }` | A single PR/MR by number (GitLab uses the project-scoped `iid`). |
| `forge_pr_checks` | `{ number }` | The PR/MR's coarse CI status (`Unsupported` on Gitea). |
| `forge_issue_list` | — | Open issues (up to 100), as unified [`ForgeIssue`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_issue_view` | `{ number }` | A single issue by number, with body and URL filled. |
| `forge_release_list` | — | Releases, newest first (up to 100), as unified [`ForgeRelease`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_release_view` | `{ tag }` | A single release by tag (`Unsupported` on Gitea — filter `forge_release_list` instead). |

### Mutating tools (gated behind the write gate, `destructiveHint`)

| Tool | Params | Effect |
|---|---|---|
| `repo_commit` | `{ paths, message }` | Commit exactly those paths (`git commit --only` / `jj commit <filesets>`). |
| `repo_checkout` | `{ reference }` | Switch the working copy to a branch/bookmark/revision (`git checkout` / `jj edit`). |
| `repo_fetch` | — | Fetch from the default remote (`git fetch` / `jj git fetch`). |
| `repo_push` | `{ branch }` | Push an existing branch/bookmark to `origin` (`git push -u origin <branch>` / `jj git push -b <branch>`). |
| `repo_create_worktree` | `{ path, branch, base }` | Create a worktree/workspace at `path` on a new `branch` from `base`. |
| `repo_remove_worktree` | `{ path, force? }` | Remove the worktree/workspace at `path` (`force` overrides local changes, git only). |
| `forge_pr_create` | `{ title, body, source?, target? }` | Open a PR/MR (omit `source` for the current branch, `target` for the repo default); returns the CLI output (the URL on success). |
| `forge_pr_merge` | `{ number, strategy }` | Merge a PR/MR with `strategy` = `merge` \| `squash` \| `rebase`. |
| `forge_pr_close` | `{ number, delete_branch? }` | Close a PR/MR without merging (`delete_branch` also deletes the source branch, GitHub only). |
| `forge_issue_create` | `{ title, body }` | Open an issue; returns the CLI output (the URL on success). |

A gated call outside the write gate returns a clear error naming the tool
(`write tool "repo_push" is disabled; restart the server with --allow-write (all
mutations) or --allow-tools naming it`) **before** spawning anything. A forge tool with no forge configured returns
`no forge is configured for this repository (pass --forge github|gitlab|gitea)`.

## Forge auto-detection

When `--forge` is omitted, the server reads the repo's `origin` remote URL and
classifies its host via `ForgeKind::from_remote_url` (github.com → GitHub,
gitlab.com → GitLab, etc.). This works on a **colocated jj** repo too — it still
has a git `origin`. A **pure-jj** repo with no git remote (or an unrecognised
host) resolves to **no forge**, so the `forge_*` tools return the "no forge
configured" error while the `repo_*` tools work regardless. Pass `--forge` to
override the detection (e.g. a self-hosted GitLab/Gitea on a custom domain).

Gitea's wrapper reports `Error::Unsupported` for `repo_view`/`pr_checks`/
`release_view`; the server maps that to an MCP *invalid-request* (a client-facing
"this forge can't do that"), distinct from an internal forge/network failure.

## Safety model

The `vcs-mcp` binary applies, in order:

1. **Read-only by default.** With no write flag, only the read tools are
   callable; every mutation rejects up front. `--allow-write` flips all mutations
   on; `--allow-tools <name,…>` grants a **per-tool allowlist** (e.g. allow
   `repo_commit` and `repo_push` but not the worktree or forge mutations).
2. **`destructiveHint` annotations.** Mutating tools are annotated so an MCP client
   can surface a confirmation prompt; read tools carry `readOnlyHint`. (Note
   `repo_try_merge` is `readOnlyHint` even though it spawns a real trial merge — it
   always rolls back and leaves no trace.)
3. **A hardened git client.** The binary opens the repo with `Git::hardened()`,
   which disables repo hooks and `core.fsmonitor`, scrubs repo-redirecting `GIT_*`
   variables, and skips system config — so serving a repository you didn't create
   can't execute its hooks (even on a read tool like `repo_status`). jj has no
   repo-local hooks, so its client needs no equivalent.
4. **The wrappers' argv guards underneath.** Every argument flows through the
   `vcs-core`/`vcs-forge` facades, so the injection guards (`reject_flag_like`)
   apply — a tool parameter can't smuggle a leading-`-` flag into argv.
5. **A per-command timeout.** Every git/forge command runs under the `--timeout`
   deadline (default 120s), so a stalled network call (`repo_fetch`, the `forge_*`
   tools) can't hang a request indefinitely.

> Note the hardening and timeout are how the **binary** constructs the `Repo`/`Forge`.
> A library embedder that builds a `VcsMcpServer` from `Repo::open(".")` gets a
> plain, un-hardened client with no default timeout — harden and time-bound the
> client yourself (`Repo::from_git(root, cwd, Git::hardened().default_timeout(d))`)
> if you serve untrusted repositories.

## Embedding the server

The library is independently usable — build a `VcsMcpServer` and serve it over any
[`rmcp`](https://crates.io/crates/rmcp) transport (the binary uses stdio):

```rust,ignore
use vcs_core::Repo;
use vcs_mcp::{VcsMcpServer, WriteGate};
use rmcp::{ServiceExt, transport::stdio};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let repo = Repo::open(".")?;
let server = VcsMcpServer::new(repo, /* forge */ None, WriteGate::None);
server.serve(stdio()).await?.waiting().await?;
# Ok(()) }
```

`VcsMcpServer` is `Clone` (cheap — it holds `Arc` trait handles). The DTOs its
tools return serialize to JSON through the optional `serde` feature the facades
expose (`vcs-core` and `vcs-forge` are pulled in with `features = ["serde"]`).

## See also

- [vcs-core guide](https://docs.rs/vcs-core/latest/vcs_core/guide/) — the `Repo` facade behind the `repo_*` tools.
- [vcs-forge guide](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) — the `Forge` facade behind the `forge_*` tools.
- [Security & hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/) — the injection guards and hardened profile
  that apply under every tool.
- [crate docs](https://docs.rs/vcs-mcp) — quickstart.
