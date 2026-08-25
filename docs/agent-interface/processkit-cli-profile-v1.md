# ProcessKit-CLI interoperability profile v1

This profile proves executable composition without linking the internal
`processkit_cli` Rust library and without copying its registry, control-plane,
containment, or teardown implementation. It is defined mechanically by
[`processkit-cli-profile.v1.json`](processkit-cli-profile.v1.json); this prose
explains how to run and interpret that contract.

## Fail-closed preflight

An integration host supplies a ProcessKit-CLI executable explicitly with
`--processkit-cli` or `PROCESSKIT_CLI_BIN`. Before starting `vcs-agent`, the
harness runs the following exact shape (one `--require-surface` for every token
in the committed JSON profile):

```text
processkit-cli probe --json \
  --require-schema-version 1 \
  --require-exit-code-band 100-119 \
  --require-surface cancel \
  --require-surface cancel:--run-id \
  --require-surface probe \
  --require-surface probe:--json \
  --require-surface probe:--print-schema \
  --require-surface probe:--require-exit-code-band \
  --require-surface probe:--require-schema-version \
  --require-surface probe:--require-surface \
  --require-surface run \
  --require-surface run:--capture-dir \
  --require-surface run:--capture-max-bytes \
  --require-surface run:--cwd \
  --require-surface run:--detach \
  --require-surface run:--grace \
  --require-surface run:--jsonl \
  --require-surface run:--no-echo \
  --require-surface run:--run-id \
  --require-surface run:--timeout \
  --require-surface wait \
  --require-surface wait:--run-id \
  --require-surface wait:--timeout
```

The command above is the exact v1 preflight shape; the JSON profile is the
machine source of truth for the same token list. Exit `0` is accepted only with
`compatible: true`, an empty `mismatches` array, `probe_version: 1`,
`schema_version: 1`, and the exact `100..=119` band. Probe exit `110`, a
non-empty mismatch list, malformed JSON, a missing schema definition, or any
other nonzero exit is an incompatibility/failure. Once a binary was provided,
none of those states is reported as a skip.

The harness then obtains the producer schema with
`processkit-cli probe --json --print-schema`. It mechanically requires the
Draft 2020-12 `runnerExit` and `outputCaptured` definitions and the terminal
fields/sources used by this profile. Validation follows fields it consumes and
tolerates additive lifecycle events and fields within schema v1.

## Cross-binary evidence

Build `vcs-agent` normally, then run the gated tier:

```text
cargo build -p vcs-agent --locked
python scripts/agent-interface/processkit_cli_profile.py \
  --processkit-cli /provided/processkit-cli \
  --vcs-agent target/debug/vcs-agent \
  --repo . \
  --evidence-output target/processkit-cli-profile-evidence.json
```

On Windows, pass the `.exe` paths. Every child launch is shell-free and has the
public shape:

```text
processkit-cli run --jsonl <events> --capture-dir <capture> ... -- vcs-agent <operation> ...
```

The capture file retains the complete `vcs-agent/v1` JSON document. The runtime
evidence embeds that same parsed document; it does not replace the child result
with a summary. The harness validates these scenarios:

- successful `vcs-agent probe`: child, foreground runner, and `runner_exit` are
  all `0`/`child_exit`;
- structured unsupported outcome: the complete error envelope reports `10`,
  while the runner and terminal record faithfully preserve child exit `10`;
- overall timeout: lifecycle `timeout` plus terminal code/source `106`/`timeout`,
  with no invented child code;
- detached cancellation: public `cancel --run-id` followed by read-only
  `wait --run-id`, lifecycle `cancelled`, and terminal
  `108`/`control_cancel`; the harness never targets a PID or process name;
- bounded output: both capture files stop at the configured per-stream ceiling,
  while `output_captured` reports the full byte counters and `truncated: true`;
- nested execution: an outer ProcessKit-CLI run invokes `vcs-agent inspect`,
  which in turn uses the existing typed clients and ProcessKit-rs for backend
  children. The outer `run_started` mechanism and confirmed-empty
  `cleanup_finished` record are checked.

The last scenario proves successful nested execution and the outer lifecycle it
can observe. ProcessKit-CLI 0.3.1 does not expose a membership-attestation token
in its probe surface, so this profile does **not** claim that its JSONL alone
proves every inner process belonged to the outer kernel container. The
committed evidence labels this limit
`outer-lifecycle-observed-inner-membership-not-attested`; a generic upstream
request draft records what stronger proof would require.

## Gating states

| State | Result | Meaning |
|---|---|---|
| Neither `--processkit-cli` nor `PROCESSKIT_CLI_BIN` supplied | `skipped` | Optional interoperability tier was not requested. |
| A supplied path is missing/unusable | `failed` | Broken provisioning, never absence. |
| Probe/schema is incompatible | `failed` | Fail closed before any child launch. |
| Any scenario/evidence check fails | `failed` | Supplied integration does not prove the profile. |
| All six scenarios pass | `passed` | Evidence file may be written atomically. |

Normal build, test, and package paths never locate or install ProcessKit-CLI.
The committed profile/evidence projection is still validated offline in normal
CI; the live cross-binary tier activates only when its binary is explicitly
provided.
