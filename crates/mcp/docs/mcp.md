# vcs-mcp — the MCP server

`vcs-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) **server**
that exposes the toolkit's typed repository operations as MCP **tools**, so an
agent harness (Claude Code, an IDE assistant, any MCP client) drives a git/jj repo
— and its forge — through **structured, validated calls** instead of raw shell.
Each tool wraps a [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/) (`Repo`) or [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)
(`Forge`) operation and returns its DTO as JSON. The binary drives git through a
**hardened** client (`Git::hardened()` — repo hooks and config disabled) and tool
arguments are injection-guarded (the wrappers keep caller values out of flag
position — flag-VALUE slots plus `reject_flag_like` on the few bare positionals), so
serving a repository you didn't create can't run its hooks or smuggle a flag into argv.

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
        [--max-output-bytes <n>]
```

| Flag | Effect |
|---|---|
| `--repo <path>` | Repository to serve (default: the current directory); git vs jj is detected from the path. |
| `--forge <github\|gitlab\|gitea>` | Force the forge for the PR/MR tools. Default: auto-detect from the `origin` remote. |
| `--allow-write` | Enable **all** mutating tools. Off by default — read tools only. |
| `--allow-tools <name,…>` | Enable **only the named** mutating tools (comma-separated; repeatable — occurrences accumulate). Tool names are the method names from the catalogue below (the canonical set is `vcs_mcp::WRITE_TOOLS`); an unknown/misspelled name is **rejected up front** with an error listing the valid write tools, rather than being silently inert. Read tools are unaffected. `--allow-write` wins when both are given. |
| `--timeout <seconds>` | Per-command deadline so a stalled fetch/forge call can't hang a request (default: 120; `--timeout 0` disables it). |
| `--max-output-bytes <n>` | Ceiling on content-tool output in bytes (`repo_show_file`, `forge_pr_diff`); default: 10485760 (10 MiB), `0` disables it. Exceeding it returns `OutputTooLarge` rather than a truncated result. |
| `-h`, `--help` | Print usage and exit. |

## Tool catalogue

### Read tools (always available, `readOnlyHint`)

| Tool | Params | Returns |
|---|---|---|
| `repo_snapshot` | — | The batched [`RepoSnapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/): branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, operation state. |
| `repo_info` | — | `{ backend, root, cwd, forge }` — git/jj, the repo root, the working dir, and the configured forge (or null). |
| `repo_status` | — | The working-copy changes (added/modified/deleted/renamed paths). |
| `repo_diff_stat` | — | Aggregate insertion/deletion/file counts for the working copy. |
| `repo_log` | `{ revspec_or_revset, max }` | Up to `max` commits reachable from `revspec_or_revset` (a git revspec or jj revset), most-recent-first. `author`/`date` are null on jj. |
| `repo_branches` | — | Local branch (git) / bookmark (jj) names. |
| `repo_current_branch` | — | The current branch/bookmark (null when detached/unset). |
| `repo_conflicts` | — | Paths with unresolved merge conflicts. |
| `repo_worktrees` | — | Attached worktrees (git) / workspaces (jj). |
| `forge_auth_status` | — | Whether the forge CLI reports an authenticated session. |
| `forge_repo_view` | — | The repository/project on the forge (`Unsupported` on Gitea). |
| `forge_pr_list` | — | Open pull/merge requests (up to 100; ~50 on Gitea). |
| `forge_pr_view` | `{ number }` | A single PR/MR by number (GitLab uses the project-scoped `iid`). |
| `forge_pr_checks` | `{ number }` | The PR/MR's coarse CI status (`Unsupported` on Gitea). |
| `forge_pr_diff` | `{ number }` | The PR/MR's diff, one file entry per changed file (`Unsupported` on Gitea). |
| `forge_issue_list` | — | Open issues (up to 100; ~50 on Gitea), as unified [`ForgeIssue`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_issue_view` | `{ number }` | A single issue by number, with body and URL filled. |
| `forge_release_list` | — | Releases, newest first (up to 100; ~50 on Gitea), as unified [`ForgeRelease`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_release_view` | `{ tag }` | A single release by tag (`Unsupported` on Gitea — filter `forge_release_list` instead). |
| `forge_info` | — | The forge identity + flat capability map: `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_checks, pr_merge, issue_create, version, supported, authed } }`. `kind` is `"github"` / `"gitlab"` / `"gitea"`; `version` is the installed CLI's `{major,minor,patch}` (or `null` when unknown/unrecognisable) and `supported` whether it meets the CLI's declared version floor; `authed` is the auth probe result; the per-op flags are the intersection of "the CLI ships the command", `supported`, and "the CLI is authenticated". |

### Mutating tools (gated behind the write gate, `destructiveHint`)

| Tool | Params | Effect |
|---|---|---|
| `repo_try_merge` | `{ source }` | Probe whether merging `source` would conflict — a **probe** that's always rolled back, so it has no net effect. Gated because it spawns a *real* trial merge that materializes working-tree content, which on an untrusted repo can run repo-local `filter`/`textconv` drivers the hardened client doesn't sandbox. |
| `repo_commit` | `{ paths, message }` | Commit exactly those paths (`git commit --only` / `jj commit <filesets>`). |
| `repo_checkout` | `{ reference }` | Switch the working copy to a branch/bookmark/revision (`git checkout` / `jj edit`). |
| `repo_rebase` | `{ onto }` | Rebase the current line onto a branch, bookmark, or revision. Returns `null` on success. Requires `--allow-write`. |
| `repo_abort_in_progress` | — | Abort the in-progress repository operation, if any. Returns `{ operation_state }`, the post-call state. On jj this is a reporting no-op; recover through the operation log instead. Requires `--allow-write`. |
| `repo_continue_in_progress` | — | Continue the in-progress repository operation after resolving conflicts. Returns `{ operation_state }`, the post-call state. On jj this is a reporting no-op; resolving conflicted files is the continuation, and recovery is through the operation log. Requires `--allow-write`. |
| `repo_new_child` | `{ reference }` | Start new work on top of a branch, bookmark, or revision. On git this checks out `reference`; on jj it creates an undescribed child change. Returns `null` on success. Requires `--allow-write`. |
| `repo_delete_branch` | `{ name, force? }` | Delete a local branch or bookmark. `force` defaults to `false`, deletes an unmerged git branch when true, and is ignored by jj. Returns `null` on success. Requires `--allow-write`. |
| `repo_rename_branch` | `{ old, new }` | Rename a local branch or bookmark. Returns `null` on success. Requires `--allow-write`. |
| `repo_fetch` | — | Fetch from the default remote (`git fetch` / `jj git fetch`). |
| `repo_push` | `{ branch }` | Push an existing branch/bookmark to `origin` (`git push -u origin <branch>` / `jj git push -b <branch>`). |
| `repo_create_worktree` | `{ path, branch, base }` | Create a worktree/workspace at `path` on a new `branch` from `base`. |
| `repo_remove_worktree` | `{ path, force? }` | Remove the worktree/workspace at `path`. Without `force`, a worktree with uncommitted changes is refused (both backends); the main worktree/workspace is always refused. |
| `forge_pr_create` | `{ title, body, source?, target? }` | Open a PR/MR (omit `source` for the current branch, `target` for the repo default); returns the CLI output (the URL on success). |
| `forge_pr_comment` | `{ number, body }` | Post a markdown comment to an existing PR/MR; returns the CLI output (the comment URL on success). On **Gitea**, PRs and issues share one `index` space and `tea comment` targets either — so a `number` that is actually an issue comments on that issue. |
| `forge_pr_edit` | `{ number, title?, body? }` | Edit a PR/MR's title and/or body. At least one of `title` or `body` must be set (both absent is rejected up front as `invalid_params`); an empty string is a real value (clears the field). |
| `forge_pr_merge` | `{ number, strategy, auto?, delete_branch? }` | Merge a PR/MR with `strategy` = `merge` \| `squash` \| `rebase`. `auto` (merge once requirements are met) and `delete_branch` are **GitHub-only** and default to `false`; on GitLab/Gitea, requesting either returns `invalid_params` rather than merging without it. |
| `forge_pr_close` | `{ number, delete_branch? }` | Close a PR/MR without merging (`delete_branch` also deletes the source branch, GitHub only). |
| `forge_pr_mark_ready` | `{ number }` | Mark a draft PR/MR ready for review (`Unsupported` on Gitea). |
| `forge_pr_checkout` | `{ number }` | Check out a PR/MR's branch into the local working copy (`gh pr checkout` / `glab mr checkout` / `tea pr checkout`). Mutates the working copy. |
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
2. **Tool annotations.** Mutating tools are annotated `destructiveHint` so an MCP
   client can surface a confirmation prompt; the genuinely read-only tools carry
   `readOnlyHint`. `repo_try_merge` is **write-gated** (not read-only): although it
   always rolls back and leaves no net trace, it spawns a *real* trial merge that
   materializes working-tree content, so it is treated like `repo_checkout` — see
   the next point.
3. **A hardened git client.** The binary opens the repo with `Git::hardened()`,
   which disables repo hooks and `core.fsmonitor`, pins a repo-local
   `core.sshCommand` empty, scrubs repo-redirecting and command-hook `GIT_*`
   variables, and skips system config — so serving a repository you didn't create
   can't execute its hooks (even on a read tool like `repo_status`). jj has no
   repo-local hooks, so its client needs no equivalent. **Residual:** `harden()`
   does *not* sandbox repo-local `filter.*` (smudge/clean) or `diff.*.textconv`
   drivers, which run when working-tree content is materialized (`repo_checkout`,
   the worktree tools, `repo_try_merge`) or a diff is produced. Those
   content-materializing tools are write-gated, so the default read-only mode does
   not expose the smudge-filter path; a `textconv` driver can still run on a diff of
   a **fully untrusted** repo, so sandbox the process (OS-level) for that case.
4. **Argv injection guards.** A tool parameter can't smuggle a leading-`-` flag
   into argv: the `vcs-core`/`vcs-forge` wrappers keep caller values out of flag
   position — typed (`u64`/`Path`) or flag-VALUE arguments, with `reject_flag_like`
   on the few bare positionals (a revision, a release tag, Gitea's comment body). A
   `body`/`title` that rides a flag-VALUE slot (e.g. a Markdown `- item` list or a
   `---` rule) is safe and passes through — the guard lives at the wrapper that owns
   the argv, not as a blanket leading-`-` refusal at the MCP seam.
5. **A per-command timeout.** Every git/forge command runs under the `--timeout`
   deadline (default 120s), so a stalled network call (`repo_fetch`, the `forge_*`
   tools) can't hang a request indefinitely.
6. **Serialized repo mutations.** rmcp dispatches a task per request, so the
   `repo_*` mutating tools are run **one at a time** behind a per-server write lock —
   two concurrent mutations (e.g. `repo_try_merge`'s materialize-then-rollback racing
   `repo_commit`) can't interleave and lose one's work. Read tools are **not**
   serialized, so a read can still observe transient mid-mutation state; the `forge_*`
   tools are remote calls and aren't behind this lock.
7. **A content-output budget.** `repo_show_file` and `forge_pr_diff` run under the
   `--max-output-bytes` ceiling (default 10 MiB), so a giant blob or PR diff can't
   be buffered whole into the server's (and then the JSON response's) memory —
   exceeding it returns `OutputTooLarge`, never a silently truncated result.

> Note the hardening, timeout, and output budget are how the **binary** constructs
> the `Repo`/`Forge`. A library embedder that builds a `VcsMcpServer` from
> `Repo::discover(".")` gets a plain, un-hardened client with no default timeout or
> output budget — harden and bound the client yourself
> (`Repo::from_git(root, cwd, Git::hardened().default_timeout(d).default_output_budget(b))`)
> if you serve untrusted repositories.

## Embedding the server

The library is independently usable — build a `VcsMcpServer` and serve it over any
[`rmcp`](https://crates.io/crates/rmcp) transport (the binary uses stdio):

```rust,ignore
use vcs_core::Repo;
use vcs_mcp::{VcsMcpServer, WriteGate};
use rmcp::{ServiceExt, transport::stdio};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let repo = Repo::discover(".")?;
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
