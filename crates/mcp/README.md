# vcs-mcp

[![crates.io](https://img.shields.io/crates/v/vcs-mcp.svg)](https://crates.io/crates/vcs-mcp) [![docs.rs](https://img.shields.io/docsrs/vcs-mcp)](https://docs.rs/vcs-mcp) [![downloads](https://img.shields.io/crates/d/vcs-mcp.svg)](https://crates.io/crates/vcs-mcp)

A [Model Context Protocol](https://modelcontextprotocol.io) **server** exposing
**git/jj** repository operations — and their **GitHub/GitLab/Gitea** forge — as
agent-callable **tools**. Part of the
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

Built on the [`vcs-core`](https://crates.io/crates/vcs-core) (`Repo`) and
[`vcs-forge`](https://crates.io/crates/vcs-forge) (`Forge`) facades: each tool
wraps a typed operation and returns its DTO as JSON, so an agent harness drives a
repository through **structured, validated calls** instead of raw shell — with the
wrappers' argv injection guards still underneath. Discovery advertises only
tools available for the selected backend, configured forge, and write gate.
Mutations require `--allow-write` or `--allow-tools <name,…>` and remain
annotated `destructiveHint`.

Prefer `outcome_*` for inspect, changes, checked commit/publish, and
exact-revision CI. They share the same preflight, evidence, error mapping,
credential isolation, deadlines, and fail-loud budgets as the `vcs-agent` CLI;
the compatible low-level `repo_*`/`forge_*` tools remain for narrower operations.

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-mcp/latest/vcs_mcp/guide/)

## The binary

```text
vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
        [--allow-tools <name,…>] [--timeout <seconds>]
        [--max-output-bytes <n>]
```

The server drives git through a **hardened** client (`Git::hardened()` — repo
hooks and `core.fsmonitor` disabled, so serving a repository you didn't create
can't run its hooks) and bounds every command with `--timeout` (default 120s; `0`
disables) so a stalled fetch/forge call can't hang a request. Content tools
(`repo_show_file`,
`forge_pr_diff`, and the working-copy read behind `repo_conflict_regions` /
`repo_resolve_conflict`) are further bounded by `--max-output-bytes` (default
10 MiB; `0` disables) so a giant blob, PR diff, or working-copy file can't be
buffered whole into memory — exceeding it returns an error rather than a silently
truncated result. The conflict tools read the filesystem directly (markers live
only in the working copy), so they get that ceiling from the server itself rather
than from the git/jj client the other content tools inherit it from.

The server speaks MCP over **stdio**; point a client at it via an `mcpServers`
config entry:

```json
{
  "mcpServers": {
    "vcs": {
      "command": "vcs-mcp",
      "args": ["--repo", "/path/to/repo", "--allow-write"]
    }
  }
}
```

The forge is auto-detected from the repo's `origin` remote (works on a colocated
jj repo too); pass `--forge` to override. With neither write flag, mutation
names are absent from `tools/list`; `--allow-tools repo_commit,repo_push`
advertises and grants exactly those mutations and nothing else. Forge names are
likewise absent when no forge is configured.

## The library

`VcsMcpServer` is independently embeddable over any `rmcp` transport:

```rust
use vcs_core::Repo;
use vcs_mcp::{VcsMcpServer, WriteGate};
use rmcp::{ServiceExt, transport::stdio};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let repo = Repo::discover(".")?;
let server = VcsMcpServer::new(repo, /* forge */ None, WriteGate::None);
server.serve(stdio()).await?.waiting().await?;
# Ok(()) }
```

**Runtime:** like [`vcs-watch`](https://crates.io/crates/vcs-watch), `vcs-mcp` uses
**tokio at runtime** (the rmcp server loop) — run it inside a tokio runtime.

## License

MIT
