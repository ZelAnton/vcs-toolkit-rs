---
name: vcs-agent
description: Inspect repository state, review changes, commit exact files, publish Git or Jujutsu revisions, open or recover pull or merge requests, wait for exact-revision CI, and report conflicts through vcs-agent. Use for repository and revision workflows; do not use for ordinary file search, source reading, or editing files without a VCS outcome.
---

# vcs-agent workflow

Prefer the versioned `vcs-agent` interface for supported repository outcomes. This
skill is guidance: host sandbox, command rules, and approvals remain the enforcement
boundary for shell access and mutations.

Before using exact flags or interpreting an error, read
[`references/contract.v1.json`](references/contract.v1.json). It is validated against
the built binary and the committed ProcessKit-CLI integration profile.

## Route the request

Use this skill for repository inspection, bounded change review, exact-path commit,
publication, pull or merge request recovery, exact-revision CI status/wait, and
structured conflict reporting. Do not activate it for ordinary source search, file
reading, or file editing when no repository state or revision outcome is requested.

Use `vcs-agent` whenever `probe` reports the requested operation as supported. Raw
`git`, `jj`, `gh`, `glab`, or `tea` is allowed only when one of these facts applies:

- `vcs-agent` returned a structured `unsupported` result;
- the `vcs-agent` executable is absent;
- exact low-level diagnostic output unavailable from `vcs-agent` is necessary.

Tell the user which fact caused fallback before running the fallback command. Do not
treat a denied, invalid, authentication, timeout, cancelled, output-limit, or unknown
outcome as permission to bypass the interface.

## Inspect before mutation

1. Run `vcs-agent probe`, then `vcs-agent inspect --repo <PATH>` before the first
   mutation in the workflow. Treat non-v1 contracts and structured errors as data;
   do not reinterpret another contract as v1.
2. Record the exact repository path, backend, current revision, source branch or
   bookmark, configured remote, forge, and active account needed by the requested
   outcome. Resolve ambiguities before mutation; never silently change any identity.
3. Use `changes` when selecting work. Preserve unrelated state. For commit, pass one
   literal repo-relative leaf file per `--path`; never stage or commit the whole tree
   as a shortcut.

## Mutate and verify

- Commit only with `--write-intent commit`, the revision observed by `inspect` as
  `--expected-revision`, and the exact selected paths. Verify the returned before and
  after revisions and re-inspect unrelated state.
- Publish only with `--write-intent publish`, the exact local and expected remote
  revisions, remote, source, target, forge, expected account, title, and body. Accept
  recovered existing PR/MR evidence when the structured result proves the same
  source, target, forge, account, and revision.
- After publication, verify the remote revision and PR/MR identity from structured
  evidence. When CI applies, query or wait using the exact published revision. Accept
  success only from terminal runs whose revision equals that publication revision.
- On any mismatch or unknown outcome, stop and report the evidence. Do not retry an
  irreversible step until the next `inspect` establishes the current state.

## Apply ProcessKit-CLI deliberately

Run `probe`, short `inspect`/`changes`, commit, and ordinary `ci status` directly.
Wrap an operation only when its expected duration is at least 60 seconds or it has a
concrete descendant-cleanup risk for which the host needs independent terminal
lifecycle evidence. `ci wait` with a wait budget of 60 seconds or more meets the
duration threshold. Do not double-wrap short read-only operations.

Before the first wrapped command, perform the exact ProcessKit-CLI preflight defined
by `processkit_cli_profile` in the contract reference. A supplied missing or
incompatible ProcessKit-CLI is a failure, not a skip. After the run, read the terminal
`runner_exit` record and bounded capture; do not infer success from child output alone.
The profile proves outer lifecycle observation, not attested membership of every inner
process.

## Install from this checkout

Build or install the executable first:

```text
cargo install --locked --path crates/agent
```

For Codex, copy this complete `skills/vcs-agent` directory to
`$CODEX_HOME/skills/vcs-agent` (or `~/.codex/skills/vcs-agent` when `CODEX_HOME` is
unset), then validate the installed directory with Codex's `quick_validate.py` skill
validator. Keep the reference beside `SKILL.md`; installing only the entrypoint breaks
the factual contract. No other harness-specific installation layout is claimed.
