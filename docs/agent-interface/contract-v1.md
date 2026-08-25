# `vcs-agent/v1` executable contract

This document freezes the first agent-facing binary contract. The executable is
named `vcs-agent`; its first implementation is version `0.1.0`. Contract and
binary versions are independent: additive binary releases may continue emitting
`vcs-agent/v1`, while a breaking wire or exit change requires a new contract
version.

The executable is an application facade over vcs-toolkit's typed clients, not a
mirror of every Git, Jujutsu, or forge method. The v1 taxonomy reserves `probe`,
`inspect`, `changes`, `commit`, `publish`, `ci status`, and `ci wait`. Only
`probe` is implemented in the initial skeleton; invoking another reserved outcome
returns `unsupported` and never silently invokes a lower-level command.

## Envelope and compatibility

Every machine success or failure is one complete JSON document on stdout. The
schema is committed at
[`crates/agent/schema/envelope.v1.schema.json`](../../crates/agent/schema/envelope.v1.schema.json).
Every envelope contains these fields:

- `contract_version`: exactly `vcs-agent/v1`;
- `binary_version` and `operation`;
- `status`: `success` or `error`;
- `data` on success, or a structured `error` on failure;
- `warnings` and an optional, classified `fallback`.

Clients are compatible when they support the reported contract version. They
must ignore additive object fields and route unknown error kinds by the reported
exit band, but must not interpret another contract version as v1. The schema
therefore validates stable field shapes without closing objects against additive
fields or closing the operation/error vocabularies against future identifiers.
`probe` reports the minimum and maximum compatible contract and the schema
identifier. Golden success and invalid-input examples are committed under
[`crates/agent/tests/fixtures`](../../crates/agent/tests/fixtures).

Machine output is bounded before it is written. The default ceiling is 65,536
bytes; callers may select 1,024 through 1,048,576 bytes. If a success envelope
would exceed the ceiling, the entire success document is discarded and replaced
with a complete `output_limit` failure. Partial content is never decoded or
emitted as a valid success.

## Error kinds and exit bands

The `error.kind` vocabulary and exact exit codes are stable within v1:

| Kind | Exit | Meaning |
|---|---:|---|
| `invalid_input` | 2 | Invocation or typed input is invalid |
| `unsupported` | 10 | Outcome/capability is unavailable |
| `denied` | 20 | Policy, permission, or explicit safety gate refused the operation |
| `backend` | 30 | Git/Jujutsu domain operation failed |
| `forge` | 31 | GitHub/GitLab/Gitea domain operation failed |
| `authentication` | 32 | Required forge/backend identity is unavailable or rejected |
| `timeout` | 40 | Operation deadline or inactivity deadline elapsed |
| `cancelled` | 41 | Caller cancellation fired |
| `output_limit` | 42 | Content or machine result exceeded its fail-loud budget |
| `external_command` | 50 | A supervised, non-domain external command failed |
| `internal` | 70 | The application could not honor its own contract |

Exit `0` is success. The stable bands are caller `2..=19`, policy `20..=29`,
domain `30..=39`, lifecycle `40..=49`, external command `50..=59`, and internal
`70..=79`. Future additive kinds use an unused value in the matching band.

ProcessKit timeouts, cancellation, output overflow, and permission failures map
structurally before the operation's domain mapping. Error detail is bounded and
redacted; it never includes captured stdout wholesale.

## Streams, redaction, and execution

Machine envelopes go only to stdout. Human diagnostics go only to stderr, so a
caller can parse stdout without filtering log lines. Credentialed HTTP(S) URLs,
secret-shaped assignments, and bearer tokens are redacted. Machine-local paths
are absent/redacted by default and can be included only with
`--include-machine-paths`; that option never disables credential redaction.

`probe` is non-mutating: it reads no repository and spawns no child. Future VCS
and forge paths must use existing typed vcs-toolkit clients. Their execution
policy is centrally defined with a ProcessKit cancellation token, per-operation
deadline, fail-loud output budget, and ProcessKit process-tree containment.
There is no production `std::process::Command` path for `git`, `jj`, `gh`,
`glab`, or `tea`, and there is no raw-command escape hatch.

ProcessKit-CLI integration is executable composition, not a Rust plugin contract:

```text
processkit-cli run -- vcs-agent <outcome> ...
```
