# `vcs-agent/v1` executable contract

This document freezes the first agent-facing binary contract. The executable is
named `vcs-agent`; its first implementation is version `0.1.0`. Contract and
binary versions are independent: additive binary releases may continue emitting
`vcs-agent/v1`, while a breaking wire or exit change requires a new contract
version.

The executable is an application facade over vcs-toolkit's typed clients, not a
mirror of every Git, Jujutsu, or forge method. The v1 taxonomy contains `probe`,
`inspect`, `changes`, `commit`, `publish`, `ci status`, and `ci wait`. `probe`,
`inspect`, `changes`, `commit`, `publish`, `ci status`, and `ci wait` are implemented.
Publication is intentionally limited to checked Git/GitHub capabilities; unsupported
backend/forge combinations fail before the corresponding mutation and never silently
invoke a lower-level command. The production
source assertion in `crates/agent/src/main.rs` checks that the executable has no
raw subprocess constructor.

## Read-only outcomes

`inspect --repo <path>` discovers Git or Jujutsu through `vcs-core`, then emits
the detected backend, repository root and bound working directory, branch or
bookmark, revision and (for Jujutsu) change identity, split dirtiness counts,
conflict and operation state, remotes, classified forge, auth facts, and
capabilities. An absent forge is `detection: "absent"`; a detected forge whose
CLI cannot be probed uses a structured `unavailable` fact rather than a guessed
value. Nullable fields mean the backend did not establish the fact, never a
fabricated zero or empty identity.

`changes --repo <path>` defaults to `--mode summary`. Summary returns changed
paths and aggregate line counts. `--mode full` additionally returns structured
per-file hunks and lines. `--content-max-bytes` (1,024 through 1,048,576; default
262,144) is projected onto the typed backend client's `OutputBudget`. Crossing
that content budget is an `output_limit` error: no partial diff is returned.
The independent `--max-output-bytes` budget still bounds the final envelope.

Paths use `{display, encoding, value}`. UTF-8 paths use `encoding: "utf-8"`;
non-UTF-8 Unix paths use hex-encoded OS bytes, and non-Unicode Windows paths use
hex-encoded UTF-16 units. This makes status paths round-trippable. Without
`--include-machine-paths`, both absolute and repository-relative paths use the
`redacted` encoding with `value: null`.

## Checked exact-path commit

`commit --repo <path>` is a checked mutation, not a convenience wrapper around
an ambient commit. It additionally requires `--write-intent commit`, one exact
`--expected-revision <id>`, a non-empty `--message`, and one or more repeated
`--path` values. Each selected path must be a non-empty, non-flag-like,
repo-relative file path with no absolute prefix, parent traversal, empty or dot
component, duplicate, directory expansion, or one-sided rename. Every path must
appear as an exact leaf in the live typed status before mutation; Git status is
queried with `--untracked-files=all` so an untracked directory can never stand
in for all descendants. Deletions and symlinks remain exact leaf entries. An
unchanged path is refused rather than reported later as included.

Preflight takes a live typed snapshot and fails with `denied` while conflicts or
an in-progress operation exist, when the current revision is unavailable, or
when it does not equal `--expected-revision`. Git also carries that identity into
the atomic ref update, so the preflight is not the write authority by itself.
Repeating a request after a success cannot create another commit because its
expected identity no longer matches. Git accepts byte-faithful non-UTF-8 paths.
Jujutsu checked commit is structured `unsupported` before any snapshot/commit
mutation because its typed CLI surface has no atomic expected-operation/change
guard equivalent to Git's expected-old ref update.
Before Git preparation, an active `filter` attribute on any selected path and
`commit.gpgSign=true` are refused. Consequently checked commit does not execute
a repository-selected clean filter or signing program; the focused real-Git
tests `git_checked_commit_rejects_an_active_clean_filter_before_it_executes` and
`git_checked_commit_rejects_configured_signing_before_the_program_executes`
install executable negative controls and verify that neither helper runs.

The only mutation call is the typed `Repo::commit_paths_checked`, which carries
the expected identity to the backend boundary. On Git it prepares the commit
from the expected tree through a temporary index, verifies the prepared object's
exact path diff, and installs it with native atomic
`update-ref HEAD <new> <expected-old>`. A concurrent HEAD advance makes that CAS
fail stale and leaves the prepared object unreachable; no T-193 commit is
installed. Repository hooks (including commit hooks, `post-index-change`, and
`reference-transaction`) are deliberately not executed: arbitrary hook code
could mutate unrelated index/worktree state and make the preservation claim
unprovable. The success envelope exposes this as
`repository_hooks_executed: false`. After a
successful CAS, only selected index entries are reset, preserving unrelated
staged/unstaged/untracked state. Jujutsu is refused as unsupported before
preparation rather than claiming a weaker stale guard. Postflight also requires an advanced revision,
clear repository state, no selected paths left in the working-copy change set,
and exact equality of the complete unrelated status-entry set before and after
the mutation. A new, removed, or changed unrelated entry therefore becomes
`outcome_unknown`. Only then does success report
the repository, before/after revision identity and included paths observed from
the created commit diff
(both old and new sides for renames), plus
`unrelated_changes_preserved: true`. Its semantics explicitly state that no
push, switch, conflict repair, or working-copy content edit occurred. Git may
update index entries for selected paths but preserves unrelated staged,
unstaged, and untracked state. No Jujutsu commit-success envelope is valid until
that backend gains an atomic expected-identity mutation primitive.

Timeout and cancellation remain their ProcessKit lifecycle kinds during
preflight and Git preparation. An observed terminal nonzero CAS rejection is a
safe stale refusal because the ref was not updated. A timeout, cancellation, or
other unobservable CAS result is `outcome_unknown` (exit 43) even if a subsequent
HEAD read differs from the prepared commit: the prepared ref may have been
installed and immediately advanced again. Failures after an observed successful
CAS are likewise unknown; neither case becomes a false success. A caller recovers by
inspecting current state and retrying with the original expected revision: a
commit that actually advanced is then rejected as stale, while an unchanged
revision permits a fresh checked attempt.

## Checked publish and recovery

`publish` requires `--write-intent publish`, a full expected local object ID, the
expected pre-push remote value (`<id>` or `absent`), explicit remote/source/target,
forge/account, and PR title/body. The current typed capability boundary is Git on
`origin` plus GitHub. Preflight proves branch, local revision, remote URL/forge,
active account, repository visibility, capabilities, a unique source/target PR (if
one exists), and the exact remote ref before an ordinary exact-SHA refspec push.
Jujutsu, non-origin routing, and GitLab/Gitea return structured `unsupported` before
push or PR/MR mutation.

After push, an exact remote-ref postflight is mandatory. A retry may report the push
as `already_present`, and PR discovery makes create idempotent. If an error is
followed by exact observed state it is `recovered_after_error`; if the irreversible
result cannot be proved it is `outcome_unknown`. Error details carry checkpoints such
as `push_succeeded_pr_failed`, while success carries verified irreversible-step and
PR number/URL/source/target evidence. Schema fixtures and validator negative mutations
mechanically check the exact-revision and verified-step claims.

## Exact-revision CI

`ci status` and `ci wait` require an explicit branch and expected published revision.
The GitHub implementation requests `headSha` and filters by exact equality; a recent
run for another SHA, an incomplete run, no exact match, or duplicate workflow match
cannot satisfy success. `ci wait` shares one caller deadline and cancellation token,
uses the typed `run_watch` 300-second inactivity watchdog, and reports its bounded
256 KiB/256-line diagnostic policy in the success evidence. Interruption after PR
publication is classified with checkpoint `pr_succeeded_ci_interrupted`. GitLab and
Gitea CI are structured `unsupported` until their typed facades expose equivalent
exact-revision evidence.

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
| `outcome_unknown` | 43 | A mutation returned but its exact postflight could not be proved |
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

`probe` is non-mutating: it reads no repository and spawns no child. `inspect`
and `changes` do not mutate Git refs, index, or working-copy content. Their Git
read-only invariant is exercised by the real-backend tests under
`crates/agent/tests`. Jujutsu differs honestly: the outcomes query the live
working copy, so normal `jj` reads may snapshot a bare filesystem edit and add a
reversible op-log entry. The envelope therefore reports
`working_copy_snapshot: "live-jj-snapshot"` and
`operation_log_may_advance: true`; it does not claim the stale, non-recording
`--ignore-working-copy` view is current.

All repository and forge paths use existing typed vcs-toolkit clients. Their
execution policy is centrally defined with a ProcessKit cancellation token, one
deadline for the complete outcome (including its sequential typed queries), a
fail-loud output budget, and ProcessKit process-tree containment.
There is no production `std::process::Command` path for `git`, `jj`, `gh`,
`glab`, or `tea`, and there is no raw-command escape hatch.

ProcessKit-CLI integration is executable composition, not a Rust plugin contract:

```text
processkit-cli run -- vcs-agent <outcome> ...
```

The versioned, fail-closed supervision requirements and mechanical cross-binary
proof are specified in the
[`vcs-agent.processkit-cli/v1` profile](processkit-cli-profile-v1.md). A normal
`vcs-agent` build has no ProcessKit-CLI dependency. When the optional binary is
provided, its `probe --json` surface and lifecycle schema are checked before
launch; incompatibility is a failure, while only an unprovided binary is a
skip. The child result remains one complete `vcs-agent/v1` JSON document in
bounded capture, and `runner_exit` preserves either the child code or the
runner-imposed timeout/cancellation class.
