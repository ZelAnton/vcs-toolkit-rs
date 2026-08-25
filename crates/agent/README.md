# vcs-agent

`vcs-agent` is the small, outcome-oriented executable interface for repository
agents. Its machine surface is deliberately narrower than the Rust wrappers and
the MCP server: callers ask for an outcome, receive one bounded versioned JSON
document, and decide explicitly whether an `unsupported` result permits a lower-
level fallback.

The initial `0.1.0` delivery implements the non-mutating `probe` outcome:

```text
cargo run -p vcs-agent -- probe
```

`probe` reports the `vcs-agent/v1` contract and schema identity, binary version,
implemented and reserved outcomes, compatible VCS/forge families, ProcessKit
containment/cancellation facts, fail-loud output limits, error kinds, and stable
exit bands. The shared policy fixes an initial 120-second per-operation deadline.
`probe` itself reads no repository and spawns no command.

Machine results — success and failure — are complete JSON documents written to
stdout. Short human diagnostics are written only to stderr. The default machine
result ceiling is 64 KiB and can be set between 1 KiB and 1 MiB with
`--max-output-bytes`; an oversized result is replaced by a complete
`output_limit` error and is never truncated into valid-looking success JSON.
Credentialed URLs, secret-shaped fields, and machine-local paths are redacted.
Paths may be included only when a future operation requires them and the caller
passes `--include-machine-paths`; secret redaction remains mandatory.

The normative v1 wire and exit contract is documented in
[`docs/agent-interface/contract-v1.md`](../../docs/agent-interface/contract-v1.md).
The executable schema and golden fixtures live in [`schema/`](schema) and
[`tests/fixtures/`](tests/fixtures).

## Execution boundary

The application policy carries a ProcessKit `CancellationToken`, a deadline, and
vcs-toolkit's fail-loud `OutputBudget`. Future repository and forge outcomes must
project those values onto the existing typed `vcs-core`/`vcs-forge` clients.
There is no raw-command escape hatch and production code contains no direct
`std::process::Command` path for `git`, `jj`, `gh`, `glab`, or `tea`.

Long-running workflows can be supervised by executable composition:

```text
processkit-cli run -- vcs-agent <outcome> ...
```

`vcs-agent` does not link ProcessKit-CLI internals or implement a plugin host.
