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
wrappers' argv injection guards still underneath. **Read tools are always
available; mutating tools are gated** behind `--allow-write` (all mutations) or
`--allow-tools <name,…>` (a per-tool allowlist) and annotated `destructiveHint`.

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-mcp/latest/vcs_mcp/guide/)

## The binary

```text
vcs-mcp [--repo <path>] [--forge github|gitlab|gitea] [--allow-write]
        [--allow-tools <name,…>] [--timeout <seconds>]
```

The server drives git through a **hardened** client (`Git::hardened()` — repo
hooks and config disabled, so serving a repository you didn't create can't run its
hooks) and bounds every command with `--timeout` (default 120s; `0` disables) so a
stalled fetch/forge call can't hang a request.

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
jj repo too); pass `--forge` to override. With neither write flag, only the read
tools are callable; `--allow-tools repo_commit,repo_push` grants exactly those
mutations and nothing else.

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
