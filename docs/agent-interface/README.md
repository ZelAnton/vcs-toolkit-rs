# Agent-interface evaluation contract

This directory is the Phase 0 evidence contract described by
[`docs/agent-interface-roadmap.md`](../agent-interface-roadmap.md).  It is
versioned and reviewable: the corpus describes routing expectations, while a
result recording supplies evidence from one harness run.

The implemented executable contract is specified separately in
[`contract-v1.md`](contract-v1.md). Its committed schema and golden machine
results live with the [`vcs-agent` crate](../../crates/agent).

## Offline v1 corpus

[`corpus.v1.json`](corpus.v1.json) is the golden prompt corpus.  Each case has a
unique `case_id`, a backend/forge request, a preferred operation, an explicit
fallback allow-list and reason list, and invariants for call counts, unrelated
workspace state, exact revisions, and terminal CI.  It includes Git and
Jujutsu inspection/diff, exact-path commit, GitHub/GitLab/Gitea publication,
exact-revision CI waiting, conflict reporting, ordinary file search as a
negative prompt, and an unsupported low-level command.

The result envelope in [`result-schema.v1.json`](result-schema.v1.json) keeps
selection evidence separate from outcome evidence:

- `selection` records preferred-interface selection, false activation, raw CLI
  bypass, and a classified fallback reason;
- `calls` records preferred, fallback, raw CLI, and total calls;
- `workspace` records whether unrelated changes survived;
- `revision` records before/after/published identities and exact-revision
  terminal-CI evidence.

Validate the corpus, optional result fixture, and every committed
`vcs-agent/v1` machine fixture without a network or model call:

```text
python scripts/agent-interface/validate.py \
  --results docs/agent-interface/fixtures/results.v1.json
```

Create a byte-for-byte repeatable recording from a result fixture.  The recorder
does not add timestamps or ambient machine state; invoke it again with the same
inputs to obtain the same JSON:

```text
python scripts/agent-interface/record.py \
  --output docs/agent-interface/fixtures/recording.v1.json
```

The output envelope is defined by [`recording-schema.v1.json`](recording-schema.v1.json).
Every case retains its outcome status, complete calls breakdown (the required
preferred/fallback/raw/total channels), unrelated-state evidence, and the full
before/after/published revision block with terminal-CI revision and conclusion.
The recorder refuses to write any output when even one corpus case is missing.

The validator also checks the `inspect`/`changes` operation-specific shape,
path encodings, summary/full distinction, and disclosed Git/Jujutsu read
semantics. The recorder runs that same machine-fixture validation before it
writes an evaluation recording, so the two tools cannot silently disagree.
It rejects duplicate or unknown case IDs, unclassified fallbacks,
negative prompts that activate an interface, mismatched call totals, missing
unrelated-state evidence, and mutation/publication results without exact
revision evidence.  It also requires terminal CI evidence to be explicitly
verified and successful when the corpus marks it as required.

## MCP baseline

[`baseline-mcp.v1.json`](baseline-mcp.v1.json) is the current baseline record.
The local checkout has no live MCP evaluation harness or ambient credentials,
so its status is explicitly `no_data` and `metrics` is `null`.  `no_data` is a
known state, not a zero score.  A future local harness may replace the evidence
with a separately reviewed recording, but must retain the same versioned
envelope and never invent measurements for unavailable data.

## Optional live evaluation tier

Live model/forge evaluation is opt-in and is not a merge gate.  A live run must
be launched by an operator who supplies the harness, account/forge context, and
explicit credential policy; normal CI must remain usable with no network,
model, or forge credentials.  Live evidence is stored outside the deterministic
fixture and includes:

```json
{
  "schema_version": "agent-interface.live-recording.v1",
  "corpus_version": "1.0.0",
  "harness": {"name": "operator-selected", "model": "redacted", "network": true},
  "cases": [{
    "case_id": "inspect-status-git",
    "attempt": 1,
    "selection": {"selected_interface": "vcs-agent", "fallback_reason": null},
    "calls": {"preferred_interface": 1, "raw_cli": 0, "total": 1},
    "revision": {"exact_revision_verified": false, "terminal_ci": {"verified": false}}
  }],
  "status": "complete"
}
```

The live record must redact credentials and machine-local paths, preserve the
case IDs and evidence fields from the v1 result envelope, and state whether a
case was skipped or unavailable.  It must not be copied into the normal fixture
or used to make deterministic CI depend on a network or nondeterministic model
call.
