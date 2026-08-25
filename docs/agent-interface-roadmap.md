# Agent interface roadmap

## Status and intent

This roadmap evolves vcs-toolkit from an MCP-first agent integration into a
transport-neutral agent interface. The existing Rust crates remain the source of
truth for Git, Jujutsu, and forge semantics. A new outcome-oriented command-line
interface becomes the primary executable contract for local agents; an Agent Skill
teaches workflows over that contract; MCP remains a supported adapter rather than
the product's only agent-facing entry point.

The executable name is `vcs-agent`. The initial contract task confirmed the
`vcs-agent/v1` envelope, outcome taxonomy, error kinds, exit bands, output limits,
redaction policy, and executable-composition boundary. The normative contract is
[`docs/agent-interface/contract-v1.md`](agent-interface/contract-v1.md).

## Problem statement

The current MCP server exposes a broad, low-level surface. An agent that also has a
shell already knows `git`, `jj`, `gh`, `glab`, and `tea`, so it can bypass MCP instead
of discovering, selecting, and sequencing dozens of tools. A write gate in MCP is not
a security boundary when the same host still permits unrestricted VCS mutations
through the shell.

The desired interface must therefore optimize for actual agent behavior:

- offer a familiar executable that is easy to select from a shell-capable harness;
- express user outcomes rather than mirror every wrapped CLI method;
- return bounded, versioned, machine-readable results and structured failures;
- preserve the typed parsing, validation, credentials, cancellation, and process-tree
  containment already implemented by vcs-toolkit and ProcessKit;
- teach selection, sequencing, verification, and honest fallback through a Skill;
- keep MCP available for hosts where MCP is the appropriate transport;
- measure whether the new interface improves selection and completion instead of
  assuming that packaging alone changes model behavior.

## Architectural decision

### Build a separate `vcs-agent` binary

The agent CLI belongs in this workspace because its domain contract is composed from
`vcs-core`, `vcs-forge`, and the per-backend crates. It should not rebuild argv,
parsers, credential handling, or forge differences at the binary layer.

The binary is an application facade, not another general VCS wrapper. Its initial
surface should remain small and outcome-oriented:

- `probe` — report the agent contract, versions, available backends, and optional
  supervisor compatibility without mutating a repository;
- `inspect` — return repository, working-copy, remote, forge, authentication, and
  capability facts in one bounded result;
- `changes` — return a summary or bounded structured diff;
- `commit` — commit exactly named paths and return before/after revision evidence;
- `publish` — perform the checked push and PR/MR publication handshake;
- `ci status` / `ci wait` — report terminal CI for the intended revision, never a
  merely recent or branch-adjacent run;
- conflict operations only after the core workflow is proven; they should reuse the
  existing structured conflict model rather than expose raw marker editing.

There is deliberately no general raw-command escape hatch in this facade. Unsupported
work returns a structured `unsupported` result so the Skill can make a visible,
auditable fallback to a lower-level CLI.

### Depend on ProcessKit-rs through its public API

vcs-toolkit already builds its real clients on ProcessKit's public `ProcessRunner`,
`JobRunner`, `CliClient`, cancellation, deadline, output-budget, and test-double
surfaces. `vcs-agent` must preserve that route for every `git`/`jj`/forge subprocess;
it must not add direct `std::process::Command` paths for those operations.

The agent binary may use ProcessKit directly for top-level cancellation, deadlines,
structured error classification, and any composed operation that is not already
covered by a facade. A missing primitive is first demonstrated against the current
public ProcessKit API. Only a genuine generic gap may become a proposed upstream
addition; vcs-toolkit must not fork containment or teardown semantics locally.

### Compose with ProcessKit-CLI; do not turn it into a plugin host yet

ProcessKit-CLI is a single-purpose process runner. Its supported contract is its
binary surface, reserved exit-code range, and versioned JSONL lifecycle stream. Its
Rust library target is explicitly an internal, unstable implementation detail. Its
existing agent-skill/marketplace files distribute instructions for that runner; they
are not a runtime extension API.

The initial integration therefore uses executable composition:

```text
processkit-cli run [supervision options] -- vcs-agent <operation> [arguments]
```

This gives a long-running `publish` or `ci wait` workflow a durable run id, bounded
capture, hard/idle deadlines, lifecycle JSONL, inspection, cancellation, and
out-of-band supervision. Short, already-contained queries such as `inspect` can run
`vcs-agent` directly. Inside either form, every VCS child remains managed through
ProcessKit-rs by the vcs-toolkit clients.

Do not add dynamic-library loading, Rust ABI plugins, or domain-specific VCS commands
to ProcessKit-CLI. Do not depend on its internal Rust modules. An upstream request is
justified only if the interoperability task demonstrates a concrete gap that cannot
be solved by its published binary contract without duplicating lifecycle semantics or
losing required evidence. Any such request must be additive and generic to process
supervision, not specific to vcs-toolkit.

## Contract principles

### Machine output

The first implementation task must define and golden-test a versioned envelope. The
exact field names are part of that task, but the contract must distinguish:

- contract version and operation;
- success, unsupported, denied, invalid-input, backend, forge, authentication,
  timeout, cancellation, output-limit, and external-command failures;
- repository root, backend, forge kind, and relevant before/after revision identity;
- result data from bounded typed DTOs;
- warnings and a machine-readable fallback reason;
- terminal versus still-running state for CI and supervised operations.

Machine output goes to stdout; diagnostics go to stderr. Secrets, credentialed URLs,
and machine-local paths not required by the result are redacted. Large content is
refused or explicitly budgeted, never silently truncated into valid-looking JSON.

### Mutation policy

The CLI is not a substitute for host permissions, but it must make safe behavior the
easy behavior:

- mutations require an explicit operation and explicit repository;
- commit accepts an exact non-empty path set and preserves unrelated changes;
- push/publication reports the local revision, remote revision, and resulting PR/MR;
- CI success is accepted only for the intended revision and a terminal conclusion;
- destructive or remote-publishing operations expose their intent in machine output;
- no command silently changes backend, forge, account, branch, or fallback path;
- unsupported operations fail structurally before the Skill considers raw CLI use.

### Skill behavior

Ship one umbrella Skill first, not one Skill per command. Its trigger description must
cover repository inspection, change preparation, publication, CI verification, and
conflict handling while excluding ordinary file search and editing.

The Skill must:

1. prefer `vcs-agent` when an operation is supported;
2. run `probe`/`inspect` before the first mutation in a workflow;
3. preserve unrelated workspace state and select exact paths;
4. use ProcessKit-CLI supervision for operations whose duration or descendant risk
   warrants lifecycle evidence;
5. verify the resulting local revision, remote revision, PR/MR, and terminal CI as
   applicable;
6. use raw `git`/`jj`/forge CLIs only after `unsupported`, a missing executable, or a
   documented need for exact low-level diagnostic output;
7. report the fallback reason rather than silently bypassing the preferred interface.

The Skill cannot enforce this boundary by itself. Hosts that require prohibition of
raw VCS mutations must enforce that with their command policy, sandbox, or approvals.

## Evaluation strategy

Before optimizing metadata, maintain a versioned golden prompt corpus with expected
routing and outcome evidence:

- direct prompts that name vcs-toolkit or `vcs-agent`;
- indirect prompts such as “commit only these files” or “open a PR and wait for CI”;
- negative prompts such as source search or file editing that should not invoke the
  VCS interface;
- unsupported prompts that should fall back visibly;
- mutation prompts with unrelated dirty files;
- Git, Jujutsu, GitHub, GitLab, and Gitea variants where their capabilities differ.

Record at least:

- preferred-interface selection rate;
- false activation rate on negative prompts;
- raw-CLI bypass rate and classified fallback reason;
- invalid command/argument rate;
- number of calls needed to complete the outcome;
- preservation of unrelated workspace state;
- exact-revision publication and terminal-CI correctness;
- denied or unsafe mutation attempts.

The corpus and result schema must be usable without a live model in ordinary CI. Live
model evaluations are an opt-in evidence tier whose results are recorded separately,
not a nondeterministic merge gate.

## Delivery phases

### Phase 0 — Evidence and contract

- Establish the golden prompt corpus, routing policy, metrics, and repeatable result
  recorder.
- Freeze the v1 executable name, command taxonomy, JSON/error envelope, exit behavior,
  output budgets, and compatibility policy.
- Record the ProcessKit-rs and ProcessKit-CLI boundaries above as an architecture
  decision backed by current public contracts.

Exit condition: the new interface can be implemented and evaluated without depending
on undocumented behavior or an undecided extension-host design.

### Phase 1 — Read-only CLI

- Add the binary crate and `probe`.
- Implement `inspect` and `changes` over `vcs-core`/`vcs-forge`.
- Cover Git and Jujutsu, forge-present and forge-absent repositories, unsupported
  capabilities, output ceilings, redaction, cancellation, and hermetic runners.

Exit condition: an agent can understand a repository and its changes through a small,
stable JSON surface without calling raw VCS commands.

### Phase 2 — ProcessKit-CLI interoperability

- Define a pinned `processkit-cli probe` preflight for the supervision features the
  workflow uses.
- Add a cross-binary proof that a `vcs-agent` operation can run under
  `processkit-cli run`, retain its result, and produce a valid terminal lifecycle
  record with faithful exit classification.
- Test timeout, cancellation, bounded capture, and nested containment behavior.
- Produce an upstream request draft only if the published binary contract proves
  insufficient.

Exit condition: the two CLIs compose without linking private ProcessKit-CLI code and
without weakening ProcessKit-rs containment.

### Phase 3 — Checked mutations and publication

- Add exact-path commit with preflight and before/after evidence.
- Add checked push and PR/MR publication with explicit account/forge identity.
- Add exact-revision CI status and terminal wait, with cancellation and inactivity
  handling inherited from ProcessKit.
- Prove idempotent recovery where a remote step succeeded before a later step failed.

Exit condition: the common “prepare, publish, wait for CI” workflow completes without
raw CLI use and cannot claim success for the wrong revision.

### Phase 4 — Skill and packaging

- Add the umbrella Skill, references, and factual drift tests against the built CLI.
- Evaluate the Skill against the golden corpus and tune its trigger/fallback language.
- Package installation metadata only after the standalone Skill is stable.
- Document host-level enforcement separately from Skill guidance.

Exit condition: direct and indirect repository prompts select the intended workflow at
an acceptable measured rate, while negative prompts remain precise.

### Phase 5 — MCP convergence

- Move shared outcome orchestration below both CLI and MCP adapters.
- Reduce MCP discovery noise with capability-aware tool registration.
- Add explicit server instructions and intent-oriented metadata.
- Keep low-level MCP operations only where composition is materially useful; prefer
  outcome tools for the common workflows.
- Run the same golden corpus against CLI+Skill and MCP transports.

Exit condition: transport choice does not change semantics, safety checks, or evidence,
and MCP no longer needs to expose unavailable or disallowed tools merely to advertise
them.

## Initial backlog

The first executable tranche is tracked in `.work/Tasks_Queue.md`:

- T-189 — evaluation corpus and routing baseline;
- T-190 — `vcs-agent` binary skeleton and versioned machine contract;
- T-191 — read-only `inspect` and `changes` outcomes;
- T-192 — ProcessKit-CLI composition and nested-containment proof;
- T-193 — exact-path checked commit;
- T-194 — checked publication and exact-revision terminal CI;
- T-195 — umbrella Skill and factual/evaluation tests;
- T-196 — MCP convergence over shared outcome services.

The dependency graph in the queue is authoritative. Later phases should be expanded
only after the initial measurements and interoperability proof expose real gaps.

## Upstream decision gates

### ProcessKit-rs

No upstream change is assumed. File a request only when the current public API cannot
provide a generic containment, cancellation, deadline, output, or observation primitive
needed by the outcome service. The request must include a minimal reproducer, platform
matrix, required semantics, and why composition in this workspace cannot solve it.

### ProcessKit-CLI

No runtime extension API is requested initially. Reconsider only after Phase 2, and
only if all of the following are true:

1. `processkit-cli run -- vcs-agent ...` cannot preserve required lifecycle or result
   evidence;
2. the gap is generic to supervised external tools rather than VCS-specific;
3. an additive binary-contract change is insufficient;
4. the benefit exceeds the compatibility, discovery, trust, packaging, and
   cross-platform costs of an extension mechanism.

Dynamic Rust plugins are explicitly out of scope unless a separate future design
solves ABI stability, signing/trust, version negotiation, isolation, installation,
and Windows/Linux/macOS loading behavior. External executable composition remains the
default because it already supplies process isolation and independent versioning.
