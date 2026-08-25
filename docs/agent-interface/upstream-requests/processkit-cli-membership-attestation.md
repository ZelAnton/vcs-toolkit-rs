# Draft: generic nested-container membership attestation

Status: draft only; not sent and no neighboring checkout was changed.

## Observed gap

The provided ProcessKit-CLI 0.3.1 binary can prove that an outer run started
with a named containment mechanism, produced versioned lifecycle events, and
finished cleanup with no observed members remaining. It can run a tool which
itself uses ProcessKit-rs, so nested supervision is reproducible. Its
`probe --json` surface does not, however, advertise a public operation that can
attest whether the calling process or a named nested child is a member of the
outer run's kernel container. Consequently, an adapter holding only the binary
and JSONL stream cannot turn “nested execution succeeded” into a stronger
membership claim.

## Minimal reproducer

1. Run `processkit-cli probe --json` and confirm schema v1 plus `run`, but no
   membership-attestation surface.
2. Start `processkit-cli run --run-id outer --jsonl outer.jsonl -- tool`, where
   `tool` creates its own ProcessKit container and child.
3. Observe outer `run_started`, child success, outer `cleanup_finished`, and
   `runner_exit`.
4. Try to prove from public binary output that the inner child was a member of
   the outer kernel container. The lifecycle stream has no authenticated
   membership verdict; PID/name inspection would be racy and is deliberately
   unacceptable.

Platforms to qualify: Windows Job Objects, Linux cgroup v2, Linux process-group
fallback, and macOS/no-kernel-containment fallback. The result must distinguish
“member”, “not a member”, and “the active mechanism cannot attest membership”.

## Requested generic capability

Consider an additive, probe-advertised binary operation which returns a
versioned, machine-readable membership verdict authenticated by the local
control transport/container backend. It should address a run by durable run id,
avoid caller-supplied PID adoption, preserve the existing reserved exit band,
and fail closed for ambiguous/stale runs or unsupported mechanisms. The
capability must be generic to any supervised external tool; it must not know
about VCS operations or `vcs-agent`.

An implementation that merely lists PIDs or asks the consumer to kill/check by
name would not close the gap. If a newer published ProcessKit-CLI already
advertises an equivalent versioned attestation surface, this draft should be
closed by raising the interoperability profile's minimum required surface and
adding cross-platform evidence, not by filing duplicate work.
