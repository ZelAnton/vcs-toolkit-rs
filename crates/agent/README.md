# vcs-agent

`vcs-agent` is the small, outcome-oriented executable interface for repository
agents. Its machine surface is deliberately narrower than the Rust wrappers and
the MCP server: callers ask for an outcome, receive one bounded versioned JSON
document, and decide explicitly whether an `unsupported` result permits a lower-
level fallback.

The `0.1.0` v1 read surface implements `probe`, `inspect`, and `changes`:

```text
cargo run -p vcs-agent -- probe
cargo run -p vcs-agent -- inspect --repo .
cargo run -p vcs-agent -- changes --repo . --mode full --content-max-bytes 262144
```

`probe` reports the `vcs-agent/v1` contract and schema identity, binary version,
implemented and reserved outcomes, compatible VCS/forge families, ProcessKit
containment/cancellation facts, fail-loud output limits, error kinds, and stable
exit bands. The shared policy fixes one 120-second deadline for the complete
outcome, including every sequential repository and forge query it composes.
`probe` itself reads no repository and spawns no command. `inspect` reports
backend, repository/working-copy state, remotes, forge/auth/capability facts;
`changes` reports a summary or structured full diff.

Machine results — success and failure — are complete JSON documents written to
stdout. Short human diagnostics are written only to stderr. The default machine
result ceiling is 64 KiB and can be set between 1 KiB and 1 MiB with
`--max-output-bytes`; an oversized result is replaced by a complete
`output_limit` error and is never truncated into valid-looking success JSON.
Credentialed URLs, secret-shaped fields, and machine-local paths are redacted.
Operation paths use a lossless encoding object and are included only when the
caller passes `--include-machine-paths`; secret redaction remains mandatory.
Full diff capture has a separate fail-loud `--content-max-bytes` ceiling (default
256 KiB), so a typed backend overflow cannot become valid-looking truncated JSON.

The normative v1 wire and exit contract is documented in
[`docs/agent-interface/contract-v1.md`](../../docs/agent-interface/contract-v1.md).
The executable schema and golden fixtures live in [`schema/`](schema) and
[`tests/fixtures/`](tests/fixtures).

## Execution boundary

The application policy carries a ProcessKit `CancellationToken`, one aggregate
outcome deadline, and vcs-toolkit's fail-loud `OutputBudget`. Repository and
forge outcomes project those values onto the existing typed
`vcs-core`/`vcs-forge` clients; the aggregate deadline cancels the shared token
when the complete composition exhausts its budget.
There is no raw-command escape hatch and production code contains no direct
`std::process::Command` path for `git`, `jj`, `gh`, `glab`, or `tea`.

Long-running workflows can be supervised by executable composition:

```text
processkit-cli run -- vcs-agent <outcome> ...
```

`vcs-agent` does not link ProcessKit-CLI internals or implement a plugin host.

On Git, these read outcomes leave refs, index, and working-copy content alone.
On Jujutsu they deliberately read the live working copy: `jj` may snapshot an
unsnapshotted filesystem edit and append an operation-log entry. The machine
result reports that distinction instead of claiming strict no-op reads or using
the stale `--ignore-working-copy` view.
