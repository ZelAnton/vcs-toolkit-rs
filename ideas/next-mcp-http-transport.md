# next: vcs-mcp HTTP/SSE transport

> **Status:** open idea (next, leaning later). From the 2026-06-09 development sweep.
> `vcs-mcp` (§6.8) is a stdio-only MCP server on the `rmcp` SDK. ROADMAP §6.8 already
> lists "an HTTP transport" as additive future work.

## Candidate

Add an HTTP/SSE transport behind a feature flag, so the server can serve remote /
multi-client agent harnesses instead of only a locally-spawned child process.

*Cost: real · Value: speculative-but-real*

**The win:** opens `vcs-mcp` to harnesses that don't co-locate the server — a hosted
agent, a multi-tenant setup, an editor talking to a remote box.

**Critical assessment:** non-trivial. It pulls in connection lifecycle, authn/authz (the
server gates ten mutating tools behind a `WriteGate` — an HTTP surface multiplies the
attack surface for that gate), and the **deferred cancel-on-disconnect plumbing** that
§6.14 already flags: HTTP makes "client went away mid-call" a first-class concern needing
a per-call cancellation token bridged from the transport. **No consumer is asking for it
yet.** Genuinely useful, but speculative — it stays `next` (leaning `later`) until an
agent harness actually needs a non-stdio server.

**Revisit:** when a consumer needs a remote/multi-client MCP server, *and* the
per-request cancellation story (§6.14, `vcs-mcp` cancellation note) is designed — the two
are coupled.
