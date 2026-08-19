# vcs-mcp — the MCP server

`vcs-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) **server**
that exposes the toolkit's typed repository operations as MCP **tools**, so an
agent harness (Claude Code, an IDE assistant, any MCP client) drives a git/jj repo
— and its forge — through **structured, validated calls** instead of raw shell.
Each tool wraps a [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/) (`Repo`) or [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)
(`Forge`) operation and returns its DTO as JSON. The binary drives git through a
**hardened** client (`Git::hardened()` — repo hooks and `core.fsmonitor` disabled,
with the code-execution `GIT_*` variables scrubbed, and a network operation refused
when the repository overrides `core.sshCommand`) and tool
arguments are injection-guarded (the wrappers keep caller values out of flag
position — flag-VALUE slots plus `reject_flag_like` on the few bare positionals), so
serving a repository you didn't create can't run its hooks or smuggle a flag into
argv — and, **on a git-backed repo**, can't redirect its ssh transport either. That
last guarantee belongs to the hardened `Git` client alone: a valid `.jj` wins backend
detection, so on a **jj or colocated** repo `repo_fetch`/`repo_push` run
`jj git fetch` / `jj git push`, which still honour the repository's
`core.sshCommand` — see "A hardened git client" below.

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

Serving GitHub under a specific `gh` account — for a machine that holds several
logins, without `gh auth switch` rewriting which one is active globally:

```json
{
  "mcpServers": {
    "vcs": {
      "command": "vcs-mcp",
      "args": ["--repo", "/path/to/repo", "--gh-account", "work-acct"]
    }
  }
}
```

Install it with `cargo install vcs-mcp` (or point `command` at a built binary).

### CLI flags

```text
vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
        [--allow-tools <name,…>] [--timeout <seconds>]
        [--max-output-bytes <n>] [--log-commands]
        [--ssh-command <command>] [--trust-repo-ssh-command]
        [--gh-account <login> | --gh-token-env <VAR>]
```

| Flag | Effect |
|---|---|
| `--repo <path>` | Repository to serve (default: the current directory); git vs jj is detected from the path. |
| `--forge <github\|gitlab\|gitea>` | Force the forge for the PR/MR tools. Default: auto-detect from the `origin` remote. |
| `--allow-write` | Enable **all** mutating tools. Off by default — read tools only. |
| `--allow-tools <name,…>` | Enable **only the named** mutating tools (comma-separated; repeatable — occurrences accumulate). Tool names are the method names from the catalogue below (the canonical set is `vcs_mcp::WRITE_TOOLS`); an unknown/misspelled name is **rejected up front** with an error listing the valid write tools, rather than being silently inert. Read tools are unaffected. `--allow-write` wins when both are given. |
| `--timeout <seconds>` | Per-command deadline so a stalled fetch/forge call can't hang a request (default: 120; `--timeout 0` disables it). |
| `--max-output-bytes <n>` | Ceiling on content-tool output in bytes (`repo_show_file`, `repo_diff`, `forge_pr_diff`, and the working-copy read behind `repo_conflict_regions`/`repo_resolve_conflict`); default: 10485760 (10 MiB), `0` disables it. Exceeding it returns `OutputTooLarge` — or, for the direct filesystem read, the same refusal naming this ceiling — rather than a truncated result. |
| `--log-commands` | Log every git/jj/forge command the server runs — program, argv, working directory, exit code, and duration — to **stderr**, for diagnosing why the server behaves unexpectedly. Off by default. The log goes to stderr only, so the stdout JSON-RPC transport stays clean; argv values that could carry a secret (a token flag, a credentialed URL) are **redacted**, and long free text (a PR/issue body) is truncated. See the safety model below. |
| `--ssh-command <command>` | Run git's SSH network operations with this command, delivered as `GIT_SSH_COMMAND`. Also lifts the hardened client's `core.sshCommand` refusal (safety model, point 3) — and, because `GIT_SSH_COMMAND` outranks the config key, whatever the repository configured never runs. An empty value is rejected at startup (git would take it as a program named `""` and fail every SSH operation). |
| `--trust-repo-ssh-command` | Accept a `core.sshCommand` **the repository** configures, lifting the same refusal without pinning a command. It accepts whatever that repository says, including a value added later, so use it for a repository you own that deliberately carries its own ssh identity — overriding such a value would authenticate as somebody else. **`--ssh-command` wins when both are given**, whichever order they appear in: it is the narrower setting, and it keeps the repository's own value from running at all. |
| `--gh-account <login>` | Run the **forge** tools as this `gh` account instead of the machine's active one — the machine can hold several logins for the same host, but only one is active, and switching it (`gh auth switch`) rewrites global state outside this process. The account's token is resolved **once** with `gh auth token --user <login>` — lazily, on the first forge call that needs it — then cached for the life of the server and injected into each command's environment; only the login is ever an argument. Because the resolution is cached, a token rotated or revoked in `gh` mid-session is picked up only after a restart (safety model, point 11). GitHub only, and exclusive with `--gh-token-env` (see below). |
| `--gh-token-env <VAR>` | Take the GitHub token from environment variable `VAR` (the CI case). The flag value is the variable's **name**, not a token; the value is read per operation and injected into the command's environment, never argv. A `VAR` that is unset or blank falls back to the ambient `gh` login — `EnvToken`'s documented "no credential ⇒ ambient auth" behaviour, and the one way this flag differs from `--gh-account`, which is fail-closed (safety model, point 11). A value that could never *be* a variable name (one containing `=` or whitespace) is rejected at startup; a merely misspelled name is a name, so it reads as unset and lands on that fallback. GitHub only, and exclusive with `--gh-account`. |
| `-h`, `--help` | Print usage and exit. |

Both GitHub identity flags fail at startup rather than being quietly ignored:

- **Both at once** is an error. They name two *different* identities (a `gh`
  account on this machine vs. a token in the environment), and neither is a
  narrower form of the other, so there is no precedence rule that could be
  applied without silently running every call as an identity you didn't pick —
  the failure `--gh-account` exists to prevent. This is the one place the
  resolution differs from `--ssh-command` / `--trust-repo-ssh-command`, where one
  flag genuinely narrows the other and so wins. Repeating *one* of them is fine
  (last wins, as for `--repo`).
- **Either one on a non-GitHub forge** is an error naming the flag and the forge.
  They reach the `gh` client only, so on a GitLab/Gitea forge — or none at all,
  including an `origin` on an unrecognised host — they would otherwise be inert
  and leave every call on the ambient login the operator just tried to replace.
  The check runs after forge detection, so it covers a `--forge` that names
  another forge *and* an auto-detected one. (Handing the token to `glab`/`tea`
  instead is not an option: a GitHub token is not their credential.)

## Tool catalogue

### Read tools (always available)

These are the query tools — **always callable**, regardless of the write gate.
Their MCP *annotation* splits by whether the query can perturb the backend it
reads (see the Safety model's "annotation honesty on jj" note):

- **`readOnlyHint`** — genuinely read-only wherever they are supported:
  `repo_info` and `repo_conflict_regions` (neither spawns a backend command at
  all — the latter parses the working-copy file directly), `repo_op_log` (jj runs
  it with `--at-op=@ --ignore-working-copy`; Git reports `Unsupported`), and every
  `forge_*` read tool (they drive the forge CLI, not the repo working copy).
- **`destructiveHint = false` + `idempotentHint = true`** (not `readOnlyHint`) —
  every `repo_*` query that, on **jj**, runs a default working-copy-**snapshotting**
  command and so records a (reversible, append-only) op-log operation: `repo_status`,
  `repo_diff_stat`, `repo_diff`, `repo_snapshot`, `repo_log`, `repo_show_file`, `repo_annotate`,
  `repo_branches`, `repo_current_branch`, `repo_conflicts`, `repo_worktrees`. On git
  these are plain reads; the annotation is the honest backend-agnostic classification.

| Tool | Params | Returns |
|---|---|---|
| `repo_snapshot` | — | The batched [`RepoSnapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/): branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, operation state. |
| `repo_info` | — | `{ backend, root, cwd, forge }` — git/jj, the repo root, the working dir, and the configured forge (or null). |
| `repo_status` | — | The working-copy changes (added/modified/deleted/renamed paths). |
| `repo_diff_stat` | — | Aggregate insertion/deletion/file counts for the working copy. |
| `repo_diff` | — | The full parsed working-copy diff, one file entry per changed file — same scope as `repo_diff_stat` (git: working tree vs `HEAD`, excludes untracked files; jj: `@` vs its parent, includes newly-added files). Runs under the content-output budget (see below). |
| `repo_log` | `{ revspec_or_revset, max }` | Up to `max` commits reachable from `revspec_or_revset` (a git revspec or jj revset), most-recent-first. `author`/`date` are null on jj. |
| `repo_op_log` | `{ max }` | Up to `max` recent repository operations, newest first. jj only; runs with `--at-op=@ --ignore-working-copy`, so this true read records no snapshot or operation. Git reports `invalid_params` from structural `Unsupported`. |
| `repo_annotate` | `{ path, rev? }` | Per-line attribution at optional git revspec / jj revset. Each line has id, line, and content; `author`/`date` are null on jj. |
| `repo_branches` | — | Local branch (git) / bookmark (jj) names. |
| `repo_current_branch` | — | The current branch/bookmark (null when detached/unset). |
| `repo_conflicts` | — | Paths with unresolved merge conflicts. |
| `repo_conflict_regions` | `{ path }` | One conflicted file's markers **parsed into structure**, so an agent never has to scrape them: `{ backend, path, conflict_count, regions: [{ number, total, region }] }`. The `region` shape is the backend's own and is deliberately *not* flattened into a lossy union — on git it carries `ours`/`base`/`theirs` (base only in `diff3`/`zdiff3` style), their labels, and `marker_len`; on jj it carries the ordered `sections` (`Diff` with `from_label`/`to_label`, `Snapshot`, `Base`) plus jj's own `conflict N of M` counters. `path` is repo-relative and read from the **working copy**, where markers are materialized; a file with no markers returns `conflict_count: 0`, not an error. A true read: it spawns no git/jj command — which is also why the `--max-output-bytes` ceiling is applied to the file directly (a file past it is refused, never truncated). |
| `repo_worktrees` | — | Attached worktrees (git) / workspaces (jj). |
| `forge_auth_status` | — | Whether the forge CLI reports an authenticated session. A bare boolean, deliberately unchanged: it says a session exists, **not** which account it belongs to — read `forge_info`'s `auth` block for that. |
| `forge_repo_view` | — | The repository/project on the forge (`Unsupported` on Gitea). |
| `forge_pr_list` | `{ state?: open\|closed\|merged\|all, limit?: number }` | Defaults to open/100; merged-only is Unsupported on Gitea, whose server may clamp the limit. |
| `forge_pr_for_branch` | `{ source_branch }` | PRs/MRs with that source branch in any state, independent of target (`Unsupported` on Gitea). |
| `forge_pr_view` | `{ number }` | A single PR/MR by number (GitLab uses the project-scoped `iid`). |
| `forge_pr_checks` | `{ number }` | The PR/MR's coarse CI status (`Unsupported` on Gitea). |
| `forge_pr_diff` | `{ number }` | The PR/MR's diff, one file entry per changed file (`Unsupported` on Gitea). |
| `forge_issue_list` | `{ state?: open\|closed\|all, limit?: number }` | Defaults to open/100; Gitea may clamp the limit. Returns unified [`ForgeIssue`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_issue_view` | `{ number }` | A single issue by number, with body and URL filled. |
| `forge_release_list` | — | Releases, newest first (up to 100; ~50 on Gitea), as unified [`ForgeRelease`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)s. |
| `forge_release_view` | `{ tag }` | A single release by tag (`Unsupported` on Gitea — filter `forge_release_list` instead). |
| `forge_info` | — | The forge identity, flat capability map, and who the CLI acts as: `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_labels, pr_checks, pr_merge, pr_approve, pr_request_changes, issue_create, issue_close, issue_reopen, issue_comment, issue_labels, release_create, release_delete, version, supported, authed }, auth: { authed, active_account, accounts: [{ host, login, active }], repo_visible } }`. `kind` is `"github"` / `"gitlab"` / `"gitea"`; `version` is the installed CLI's `{major,minor,patch}` (or `null` when unknown/unrecognisable) and `supported` whether it meets the CLI's declared version floor; `authed` is the auth probe result; the per-op flags are the intersection of "the CLI ships the command", `supported`, and "the CLI is authenticated". `pr_request_changes` is always `false` for GitLab; `pr_labels`/`issue_labels` are `false` for Gitea. The `auth` block answers what `capabilities.authed` cannot: with several logins for one host the CLI runs as exactly one of them, so `active_account` names that identity and `repo_visible` says whether **this** repository is visible to it — `repo_visible: false` beside `authed: true` is why an otherwise-authenticated call fails with "Could not resolve to a Repository". GitHub-only for now, and every field is honestly optional (`null`/`[]` = unknown, never a negative answer): GitLab/Gitea report unknown without spawning, an unrecognised `gh auth status` format degrades to unknown rather than erroring, and `repo_visible` is probed only when a session exists. |

### Mutating tools (gated behind the write gate, `destructiveHint`)

| Tool | Params | Effect |
|---|---|---|
| `repo_try_merge` | `{ source }` | Probe whether merging `source` would conflict — a **probe** that's always rolled back, so it has no net effect. Gated because it spawns a *real* trial merge that materializes working-tree content, which on an untrusted repo can run repo-local `filter`/`textconv` drivers the hardened client doesn't sandbox. |
| `repo_resolve_conflict` | `{ path, side, index? }` | Keep one side of **every** conflict region in `path` and write the result to the working copy. `side` is `ours`, `base`, `theirs`, or — jj only, for a conflict with more than two sides — `side` plus a 0-based `index` (list the sides with `repo_conflict_regions` first). On git the path is then staged (`git add`), which is what clears the unmerged index entry; jj needs no such step (the working-copy content *is* the resolution). Returns `{ resolved, side, index, conflicts_resolved }`. Refused **before anything is written** when the request can't be honoured exactly: a path outside the repo, a path the backend does not currently report as conflicted (so a file merely *containing* marker-like text is never rewritten), a file over the `--max-output-bytes` ceiling, `base` where the conflict records none (git's 2-way `merge` style), `theirs` on an n-way jj conflict where it would be ambiguous, `side` on git, or an `index` alongside a named side. |
| `repo_commit` | `{ paths, message }` | Commit exactly those paths (`git commit --only` / `jj commit <filesets>`). |
| `repo_checkout` | `{ reference }` | Switch the working copy to a branch/bookmark/revision (`git checkout` / `jj edit`). |
| `repo_rebase` | `{ onto }` | Rebase the current line onto a branch, bookmark, or revision. Returns `{ rebased_onto }`. Requires `--allow-write`. |
| `repo_undo` | — | Undo the latest repository operation through top-level `jj undo`; returns `{ undone: true }`. jj only; Git reports `invalid_params` from structural `Unsupported`. Requires `--allow-write` (or `--allow-tools repo_undo`). |
| `repo_abort_in_progress` | — | Abort the in-progress repository operation, if any. Returns `{ operation_state }`, the post-call state. On jj this is a reporting no-op; inspect `repo_op_log` and recover with `repo_undo` instead. Requires `--allow-write`. |
| `repo_continue_in_progress` | — | Continue the in-progress repository operation after resolving conflicts. Returns `{ operation_state }`, the post-call state. On jj this is a reporting no-op; resolving conflicted files is the continuation, and recovery is available through `repo_op_log`/`repo_undo`. Requires `--allow-write`. |
| `repo_new_child` | `{ reference }` | Start new work on top of a branch, bookmark, or revision. On git this checks out `reference`; on jj it creates an undescribed child change. Returns `{ new_child_of }`. Requires `--allow-write`. |
| `repo_create_branch` | `{ name }` | Create a local branch or bookmark at the current head, without switching the working copy (`git branch <name>` / `jj bookmark create <name> -r @`). Returns `{ created_branch }`. Requires `--allow-write`. |
| `repo_delete_branch` | `{ name, force? }` | Delete a local branch or bookmark. `force` defaults to `false`, deletes an unmerged git branch when true, and is ignored by jj. Returns `{ deleted_branch, force }`. Requires `--allow-write`. |
| `repo_rename_branch` | `{ old, new }` | Rename a local branch or bookmark. Returns `{ renamed: { old, new } }`. Requires `--allow-write`. |
| `repo_fetch` | — | Fetch from the default remote (`git fetch` / `jj git fetch`). |
| `repo_push` | `{ branch }` | Push an existing branch/bookmark to `origin` (`git push -u origin <branch>` / `jj git push -b <branch>`). |
| `repo_create_worktree` | `{ path, branch, base }` | Create a worktree/workspace at `path` on a new `branch` from `base`. |
| `repo_remove_worktree` | `{ path, force? }` | Remove the worktree/workspace at `path`. Without `force`, a worktree with uncommitted changes is refused (both backends); the main worktree/workspace is always refused. |
| `forge_pr_create` | `{ title, body, source?, target?, labels? }` | Open a PR/MR (omit `source` for the current branch, `target` for the repo default); optionally apply labels; returns the CLI output (the URL on success). |
| `forge_pr_add_labels` / `forge_pr_remove_labels` | `{ number, labels }` | Add/remove one or more labels on GitHub/GitLab; Gitea returns `invalid_params` (`Unsupported`). |
| `forge_pr_comment` | `{ number, body }` | Post a markdown comment to an existing PR/MR; returns the CLI output (the comment URL on success). On **Gitea**, PRs and issues share one `index` space and `tea comment` targets either — so a `number` that is actually an issue comments on that issue. |
| `forge_pr_edit` | `{ number, title?, body? }` | Edit a PR/MR's title and/or body. At least one of `title` or `body` must be set (both absent is rejected up front as `invalid_params`); an empty string is a real value (clears the field). |
| `forge_pr_merge` | `{ number, strategy, auto?, delete_branch? }` | Merge a PR/MR with `strategy` = `merge` \| `squash` \| `rebase`. `auto` (merge once requirements are met) and `delete_branch` are **GitHub-only** and default to `false`; on GitLab/Gitea, requesting either returns `invalid_params` rather than merging without it. |
| `forge_pr_close` | `{ number, delete_branch? }` | Close a PR/MR without merging (`delete_branch` also deletes the source branch, GitHub only). |
| `forge_pr_mark_ready` | `{ number }` | Mark a draft PR/MR ready for review (`Unsupported` on Gitea). |
| `forge_pr_approve` | `{ number }` | Submit an approving review (`gh pr review --approve` / `glab mr approve` / `tea pr approve`). Supported on all three forges. |
| `forge_pr_request_changes` | `{ number, body }` | Submit a request-changes review with a required `body`/reason (`gh pr review --request-changes` / `tea pr reject`). `Unsupported` on **GitLab** (its review model is approve/revoke); an empty body is rejected up front as `invalid_params`. |
| `forge_pr_checkout` | `{ number }` | Check out a PR/MR's branch into the local working copy (`gh pr checkout` / `glab mr checkout` / `tea pr checkout`). Mutates the working copy. |
| `forge_issue_create` | `{ title, body, labels? }` | Open an issue and optionally apply labels; returns the CLI output (the URL on success). |
| `forge_issue_add_labels` / `forge_issue_remove_labels` | `{ number, labels }` | Add/remove one or more labels on GitHub/GitLab; Gitea returns `invalid_params` (`Unsupported`). |
| `forge_issue_close` | `{ number }` | Close an issue (`gh issue close` / `glab issue close` / `tea issues close`). Supported on all three forges. Returns `{ closed }`. |
| `forge_issue_reopen` | `{ number }` | Reopen a closed issue (`gh issue reopen` / `glab issue reopen` / `tea issues reopen`). Supported on all three forges. Returns `{ reopened }`. |
| `forge_issue_comment` | `{ number, body }` | Post a markdown comment to an existing issue; returns the CLI output (the comment URL on success). Maps to `gh issue comment --body` / `glab issue note -m` / `tea comment <n>` (on Gitea, issues and PRs share one `index` space). An empty body is rejected up front as `invalid_params`. |
| `forge_release_create` | `{ tag, title?, notes?, draft?, prerelease? }` | Create a release; returns the CLI output (the URL on success). `draft`/`prerelease` default to `false` and are **GitHub/Gitea-only** — GitLab returns `invalid_params` rather than creating without them. Asset uploads are not supported. |
| `forge_release_delete` | `{ tag }` | Delete a release by its Git tag (deletes the release only, not the underlying git tag). |

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

Gitea's wrapper reports `ErrorReason::Unsupported` for `repo_view`/`pr_checks`/
`release_view`; the server maps that to an MCP *invalid-request* (a client-facing
"this forge can't do that"), distinct from an internal forge/network failure.

## Safety model

The `vcs-mcp` binary applies, in order:

1. **Read-only by default.** With no write flag, only the read tools are
   callable; every mutation rejects up front. `--allow-write` flips all mutations
   on; `--allow-tools <name,…>` grants a **per-tool allowlist** (e.g. allow
   `repo_commit` and `repo_push` but not the worktree or forge mutations).
2. **Tool annotations.** Mutating tools are annotated `destructiveHint` so an MCP
   client can surface a confirmation prompt. Only the tools that are read-only on
   every backend where they are supported carry `readOnlyHint` (`repo_info`,
   `repo_conflict_regions`, `repo_op_log`, the
   `forge_*` reads); the
   `repo_*` queries that snapshot the jj working copy carry
   `destructiveHint = false` + `idempotentHint = true` instead of `readOnlyHint`,
   because on jj they record a (reversible) op-log operation — see the "annotation
   honesty on jj" note below. `repo_try_merge` uses the same non-destructive/
   idempotent annotation but is additionally **write-gated** (not read-only):
   although it always rolls back and leaves no net trace, it spawns a *real* trial
   merge that materializes working-tree content, so it is treated like
   `repo_checkout` — see the next point.
3. **A hardened git client.** The binary opens the repo with `Git::hardened()`,
   which disables repo hooks and `core.fsmonitor`, scrubs repo-redirecting and
   command-hook `GIT_*`
   variables, and skips system config — so serving a repository you didn't create
   can't execute its hooks (even on a read tool like `repo_status`). jj has no
   repo-local hooks, so its client needs no equivalent **for the hook vector** —
   that reasoning does not carry over to the SSH-transport check below, which a
   jj-backed repo never runs (see "Only the git backend" at the end of this point).
   **Residual:** `harden()`
   does *not* sandbox repo-local `filter.*` (smudge/clean) or `diff.*.textconv`
   drivers, which run when working-tree content is materialized (`repo_checkout`,
   the worktree tools, `repo_try_merge`) or a diff is produced. Those
   content-materializing tools are write-gated, so the default read-only mode does
   not expose the smudge-filter path; a `textconv` driver can still run on a diff of
   a **fully untrusted** repo, so sandbox the process (OS-level) for that case.
   A repo-local **`core.sshCommand`** is handled differently — it is neither
   neutralized nor ignored, but **refused**. git runs that key's value through a
   shell for the SSH transport, and it cannot be pinned away (an empty pin breaks
   SSH outright; a non-empty one silently changes which ssh binary and identity
   are used). So before each git network operation (`repo_fetch`, `repo_push`) the
   hardened client compares the repository's *effective* `core.sshCommand` with
   your *global* one and refuses when they differ — naming the key, the value
   found, and the two flags that continue. A `core.sshCommand` that lives only in
   your own global git config is **not** affected: it reads the same both ways.
   `--ssh-command <command>` pins your own (delivered as `GIT_SSH_COMMAND`, which
   outranks the repository's key) and `--trust-repo-ssh-command` accepts the
   repository's; `--ssh-command` wins if both are given. The check is a
   before-the-command read, not a sandbox — a repository rewritten *between* that
   read and git's own can still slip a value through — so a fully untrusted
   repository still belongs either read-only or inside an OS-level sandbox.
   **Only the git backend.** The check lives in the hardened `Git` client, and
   `Repo::discover_with` builds that client only for a repository it detects as
   **git**. A valid `.jj` **wins** over `.git`, so on a **colocated** checkout (both
   markers present) the server drives jj: `repo_fetch`/`repo_push` become
   `jj git fetch` / `jj git push`, no comparison runs, and jj's own git subprocess
   executes the repository's `core.sshCommand` (reproduced on jj 0.42 — the
   repo-local command ran on `jj git fetch` exactly as it does on a plain
   `git fetch`). `--ssh-command` / `--trust-repo-ssh-command` configure that same
   git client, so they are **no-ops** on a jj-backed repo. There is deliberately no
   `Jj::hardened()`, so for an untrusted **jj or colocated** repository this vector
   is not covered here at all: both network tools are write-gated, so the default
   read-only mode keeps them unreachable — otherwise use an OS-level sandbox.
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
   serialized, so a read can still observe transient mid-mutation state. Remote-only
   `forge_*` mutations are not behind this lock either. The local-working-copy
   exceptions — `forge_pr_checkout`, `forge_pr_merge`, and `forge_pr_close` (the
   latter two can delete a branch and switch the checkout) — take the same repo lock
   and therefore cannot interleave with `repo_*` mutations.
7. **A content-output budget.** `repo_show_file`, `repo_diff`, `forge_pr_diff`, and
   the working-copy read behind `repo_conflict_regions` / `repo_resolve_conflict`
   run under the `--max-output-bytes` ceiling (default 10 MiB), so a giant blob,
   diff, or working-copy file can't be buffered whole into the server's (and then
   the JSON response's) memory — exceeding it returns `OutputTooLarge` (or, for the
   filesystem read, the same refusal naming the ceiling), never a silently
   truncated result. The first three inherit the budget from the git/jj/forge
   **client** they run through, which enforces it on the subprocess's output pipe.
   The conflict tools spawn nothing for their read, so nothing pipe-side could
   enforce it: the server carries the same budget itself and applies it at the
   filesystem — first to the file's size, so an oversized file is refused before a
   byte of it is buffered, then to the read itself, so a file that grows past the
   ceiling mid-read still can't overrun it. `repo_resolve_conflict` is bounded end
   to end by that same read: what it writes is a parse of what it read, and a
   resolution only ever drops content. A library embedder sets the ceiling with
   `VcsMcpServer::with_output_budget` (the default is unlimited, as it is for a
   client with no configured budget).
8. **Annotation honesty on jj (no `readOnlyHint` on the snapshotting reads).** On a
   jj-backed repo, every `repo_*` query except `repo_info`, `repo_conflict_regions`
   and `repo_op_log` (`repo_status`,
   `repo_diff_stat`, `repo_diff`, `repo_snapshot`, `repo_log`, `repo_show_file`, `repo_annotate`,
   `repo_branches`, `repo_current_branch`, `repo_conflicts`, `repo_worktrees`) runs a
   plain jj command in jj's default working-copy-**snapshotting** mode: it imports any
   bare filesystem edit into a fresh `@` and records a new operation in the op log.
   The MCP spec defines `readOnlyHint` as "the tool does not modify its environment",
   and recording an op-log operation *is* a state change — so these tools deliberately
   **do not** claim `readOnlyHint`. They are annotated `destructiveHint = false` +
   `idempotentHint = true` instead: the op-log entry is append-only and fully
   reversible (`jj undo`) and changes no tracked content, refs, or bookmarks, and a
   re-run with no interim edit records nothing further — but it is not *nothing*, so
   the annotation stays honest rather than redefining `readOnlyHint` in prose. (On the
   git backend these same tools are ordinary reads; the annotation is the conservative
   cross-backend truth, since a read-only operation is trivially non-destructive and
   idempotent.) They remain **callable without a write gate** — a snapshot mutates no
   tracked content or refs, so it needs no `--allow-write`. A genuinely non-recording
   read exists through `repo_op_log`, which pairs `--ignore-working-copy` with
   `--at-op=@` so even operation-head reconciliation is suppressed. The same
   mode also exists internally, exposed as `vcs-core`'s
   `snapshot_readonly`/`local_branches_readonly` and used by `vcs-watch`'s polling
   loop) but is deliberately **not** wired into these MCP tools: it reports the state
   of the *last recorded* operation rather than the live working tree, so a bare edit
   no jj command has yet snapshotted would be silently invisible — the opposite of
   what an agent calling `repo_status`/`repo_diff` right after editing a file needs.
   That is why `--ignore-working-copy` is not an acceptable way to reclaim
   `readOnlyHint` here: it would trade a false annotation for stale reads.
9. **Repo containment for the two conflict tools.** `repo_conflict_regions` and
   `repo_resolve_conflict` are the only tools that touch the filesystem
   **directly** rather than through a git/jj subprocess — necessarily so, because
   conflict markers are materialized in the working copy and, on git, exist
   nowhere else (`git show HEAD:<path>` returns the clean blob; `git show :<path>`
   fails outright on an unmerged path). Every other tool inherits its path
   confinement from the subprocess running inside the repo, so these two make it
   explicit: an agent-supplied `path` must consist entirely of normal components,
   which rejects an absolute path, a Windows drive prefix, and any `..`
   traversal before a single byte is read or written. On Windows it additionally
   rejects a component naming a legacy DOS **device** (`CON`, `NUL`, `COM1`, …,
   with or without an extension): Win32 resolves those in every directory, so
   `<repo>\CON` opens the console rather than a file — and no repository can
   legitimately contain such a file, since Win32 won't let one be created.
   `repo_resolve_conflict` adds a further guard — the path must be one the backend
   *currently reports as conflicted* — so a file that merely **contains**
   conflict-marker-like text (a conflict-parser fixture, a quoted diff in
   documentation) can never be "resolved" into losing content. The
   `--max-output-bytes` ceiling applies to this direct filesystem path as well
   (guard 7 above), so "not through a subprocess" does not mean "unbounded".
   Residual: (a) a symlink *inside* the repo that points outside it is not
   followed-and-rejected here, the same exposure git itself has when it
   materializes a conflicted working tree; (b) `--timeout` bounds *commands*, and
   this read/write runs no command — on a wedged network filesystem the call can
   block for as long as the OS takes to fail it. The device-name guard removes the
   one case where that block would have been indefinite by construction; a stalled
   remote mount is left to the mount's own timeouts.
10. **Command logging is off by default, redacted, and stderr-only.** `--log-commands`
    wraps the git/jj/forge clients in a command-logging `ProcessRunner` decorator
    (`vcs_cli_support::logging::LoggingRunner`) so you can see exactly what the server
    spawns — program, argv, working directory, exit code, duration. It is a diagnostic
    surface over argv, so it is treated as security-sensitive: the log is written to
    **stderr only** (the stdout JSON-RPC transport is never touched), the process
    **environment is never logged** (that is the channel the forge token rides in —
    `GH_TOKEN`/`GITLAB_TOKEN` — and git's secret goes through `credential.helper`, so
    the token-carrying channel is out of scope by construction), and each argv value
    is redacted before it is written: the value after a sensitive flag (`--token`,
    `--password`, …) or the value of its `--flag=value` form is masked, a secret-shaped
    token (`ghp_`/`github_pat_`/`glpat-`/… prefix, an `x-access-token:` embed) is
    masked, a URL's embedded credentials are masked (host and path kept), and long free
    text (a PR/issue body, a commit message) is truncated. This is defence in depth on
    top of guard (4) above — the "token never rides in argv" contract — not a
    replacement for it.
11. **Forge identity is ambient by default, and explicit when it isn't.** With
    neither GitHub identity flag the forge tools authenticate exactly as the forge
    CLI would on its own — nothing is injected, and the machine's active `gh`
    account is used and left alone. `--gh-account <login>` picks a *different* one
    of the machine's logins **for this server only**: `vcs-github`'s
    `GhAccountToken` resolves that account's token with `gh auth token --user`
    (its own client, with the ambient token variables scrubbed from it, so an
    unrelated `GH_TOKEN` in the environment can't be echoed back in place of the
    account's) and injects it into each command's environment, so `gh auth switch`
    — which rewrites the user's global gh state — is never needed. Only that
    **injection** is per operation: the resolution runs **once**, lazily, on the
    first forge call that needs it, and `GhAccountToken` caches the result per
    `(login, host)` for the life of the provider — which this binary builds once,
    when it constructs the forge, so that is the life of the server process. The
    consequence is the one `vcs-github` documents for the cache: **a token rotated
    or revoked in `gh` mid-session keeps being used until the server is
    restarted**, so treat a restart as part of rotating that account's token. (A
    failed resolution is not cached, so a fixed cause is retried on the next call.)
    The flag is **fail-closed**: a login whose token can't be resolved fails the
    call naming the login, rather than quietly proceeding as the active account,
    which is the silent identity swap the flag exists to prevent.
    `--gh-token-env <VAR>` is the CI shape of the same seam. It caches nothing —
    `EnvToken` reads the named variable on every request — and is *not*
    fail-closed by design: an unset or blank `VAR` defers to the ambient login,
    which is what the `CredentialProvider` contract says a provider yielding no
    credential means. The startup check refuses only a value that could never *be*
    a variable name (one containing `=` or whitespace); a merely misspelled name
    is still a name, so it passes the check, reads as unset, and lands on exactly
    that ambient fallback. Neither flag puts a secret in argv: the flag values are
    an account **login** and a variable **name**, the tokens travel in the child's
    environment, and guard (10) never logs the environment — so `--log-commands`
    can print the identity in use but not the credential. The `gh auth token`
    probe runs under the same `--timeout` deadline as the commands, since a
    client's timeout bounds what the client spawns, not credential resolution.
    Both flags are refused at startup on a non-GitHub (or absent) forge and refuse
    each other; see "CLI flags" above. This all applies to the **binary**: a
    library embedder attaches a `CredentialProvider` to the client it builds.

> Note the hardening, timeout, and output budget are how the **binary** constructs
> the `Repo`/`Forge`. A library embedder that builds a `VcsMcpServer` from
> `Repo::discover(".")` gets a plain, un-hardened client with no default timeout or
> output budget — harden and bound the client yourself
> (`Repo::from_git(root, cwd, Git::hardened().default_timeout(d).default_output_budget(b))`)
> if you serve untrusted repositories — an un-hardened client also runs no
> `core.sshCommand` check, and the two opt-ins behind `--ssh-command` /
> `--trust-repo-ssh-command` are plain client builders
> (`with_ssh_command` / `trust_repo_ssh_command`) you apply the same way. The
> conflict tools' direct working-copy read takes its ceiling from the **server**,
> not the client, so bound it there too:
> `VcsMcpServer::new(...).with_output_budget(b)`.

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
