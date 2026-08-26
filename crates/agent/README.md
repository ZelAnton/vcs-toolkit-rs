# vcs-agent

`vcs-agent` is the small, outcome-oriented executable interface for repository
agents. Its machine surface is deliberately narrower than the Rust wrappers and
the MCP server: callers ask for an outcome, receive one bounded versioned JSON
document, and decide explicitly whether an `unsupported` result permits a lower-
level fallback.

The `0.1.0` v1 surface implements `probe`, `inspect`, `changes`, checked
exact-path `commit`, checked `publish`, and exact-revision `ci status`/`ci wait`:

```text
cargo run -p vcs-agent -- probe
cargo run -p vcs-agent -- inspect --repo .
cargo run -p vcs-agent -- changes --repo . --mode full --content-max-bytes 262144
cargo run -p vcs-agent -- commit --repo . --write-intent commit \
  --expected-revision <inspect-revision> --message "selected files" \
  --path src/lib.rs --include-machine-paths
cargo run -p vcs-agent -- publish --repo . --write-intent publish \
  --expected-revision <full-sha> --expected-remote-revision absent \
  --remote origin --source feature --target main --forge github \
  --expected-account agent --title "Checked publish" --body ""
cargo run -p vcs-agent -- ci wait --repo . --forge github --source feature \
  --expected-revision <full-sha> --wait-seconds 1800 --poll-seconds 10
```

`probe` reports the `vcs-agent/v1` contract and schema identity, binary version,
implemented and reserved outcomes, compatible VCS/forge families, ProcessKit
containment/cancellation facts, fail-loud output limits, error kinds, and stable
exit bands. The shared policy fixes one 120-second deadline for the complete
outcome, including every sequential repository and forge query it composes.
`probe` itself reads no repository and spawns no command. `inspect` reports
backend, repository/working-copy state, remotes, forge/auth/capability facts;
`changes` reports a summary or structured full diff. `commit` requires explicit
write intent, a non-empty message and exact repo-relative file paths, and the
revision identity obtained from preflight. It refuses stale or conflicted state,
directory/traversal/flag-like ambiguity, unchanged selections, active selected-path
clean filters, and configured commit signing before mutation. Its success envelope
proves the before/after identities, the included path set, and exact preservation
of the unrelated status-entry set; it never pushes, switches, or repairs conflicts.

Checked publish currently supports a Git repository, the explicit `origin` remote,
and GitHub. It verifies the full local object ID, branch, remote/forge identity,
active account, repository visibility, capabilities, and expected remote ref before
the first mutation. Jujutsu and GitLab/Gitea return structured `unsupported` before
push or PR/MR mutation. A retry recognizes an already-pushed exact SHA and an existing
unique source/target PR; ambiguous identity, unexpected remote advancement, or an
unprovable mutation result never becomes success. These claims are exercised by
`app::tests` and the committed `publish-success-git.v1.json` fixture.

GitHub CI status/wait filters runs by exact `headSha`, rejects a recent different
revision and duplicate workflow matches, and reports success only after every
selected run is terminal with conclusion `success`. `ci wait` uses one aggregate
deadline plus the typed GitHub 300-second inactivity watchdog and bounded diagnostic
capture. Rust schema tests and `scripts/agent-interface/test_validate.py` check the
negative controls and committed `ci-*-success-github.v1.json` fixtures.

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

Long-running workflows can be supervised by executable composition while preserving
the child JSON and terminal exit classification:

```text
processkit-cli run --timeout 1800 --capture-max-bytes 65536 --no-echo -- \
  vcs-agent ci wait <arguments>
```

`vcs-agent` does not link ProcessKit-CLI internals or implement a plugin host.
The optional composition is pinned and tested by the
[`vcs-agent.processkit-cli/v1` profile](https://github.com/zelanton/vcs-toolkit-rs/blob/main/docs/agent-interface/processkit-cli-profile-v1.md):
it performs an exact machine probe before launch, retains the complete child
JSON in bounded capture, and checks terminal lifecycle/exit classification.
Ordinary builds and packages do not require ProcessKit-CLI to be installed.

On Git, these read outcomes leave refs, index, and working-copy content alone.
On Jujutsu they deliberately read the live working copy: `jj` may snapshot an
unsnapshotted filesystem edit and append an operation-log entry. The machine
result reports that distinction instead of claiming strict no-op reads or using
the stale `--ignore-working-copy` view.

Checked commit uses the existing typed `Repo::commit_paths` facade. Git maps the
selection to commit-only literal pathspecs and may update the selected paths'
index entries while retaining unrelated staged, unstaged, and untracked state.
Jujutsu maps UTF-8 paths to exact filesets and has no Git index; its before/after
evidence includes both revision and change IDs. A lifecycle failure remains a
structured timeout/cancellation result, while a completed mutation whose
postflight cannot be proved returns `outcome_unknown` (exit 43). Repeating the
same request is fail-closed because its expected pre-mutation revision is stale.
