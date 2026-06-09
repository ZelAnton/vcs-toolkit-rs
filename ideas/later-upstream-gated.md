# later: upstream-gated adoptions (processkit)

> **Status:** open idea (later, externally gated). From the 2026-06-09 development sweep.
> Two performance/UX capabilities are **specs already delivered to ProcessKit-rs**;
> adoption here is blocked on (1) a processkit release that ships them and (2) a concrete
> consumer that needs them. §6.14 already records "no consumer for any of these."

## Candidates

### A. Streaming / progress hooks adoption (§5.2)
*Cost: moderate · Value: progress-UI capability · Gate: upstream + consumer*

processkit 0.6+ already ships per-line callbacks; the requirements note asked for
*hardening* (handler-panic isolation, ordering guarantees, scripted-stream replay for
hermetic tests) — and processkit 0.8 shipped exactly that (handler-panic isolation,
streaming `ScriptedRunner`). What's missing is a **toolkit consumer**: there are zero
`on_*_line` streaming wrappers in vcs-toolkit today. Wire streaming progress into a
long-running op (clone, fetch, `run_watch`) only when a consumer wants live progress.

### B. Persistent query sessions (§6.5)
*Cost: high · Value: real perf for repeated reads · Gate: upstream API + consumer*

`git cat-file --batch` / `gh api --paginate`-style long-lived children for fast repeated
object/metadata reads need a **persistent-process API** processkit doesn't yet expose
(spawn-once, framed request/response pipe, cancellation + cleanup-on-drop, plus a
`ScriptedRunner` analogue for hermetic tests). The spec is delivered upstream; until it
ships, batch reads go through one spawn per query (or the batched `snapshot()` of §6.4
for the common case).

## Assessment

Both are genuinely valuable and both are **externally gated** — pulling them forward
before the upstream API + a consumer exist would be speculative plumbing. **Revisit:**
when a processkit release exposes the needed primitive **and** a consumer (vcs-flow-rs /
agent-workspace) asks for streaming progress or fast batch reads. Track the upstream side
via `.hq/comms` threads to ProcessKit-rs.
