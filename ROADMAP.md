# Roadmap

Planned future work, in priority order. The toolkit currently has no external
users, so API, architecture, and interfaces may all change freely — nothing
here is constrained by backward compatibility.

Items are driven by the two real consumers (`vcs-flow-rs` and
`agent-workspace`): everywhere they still shell out through the `run`/`run_raw`
escape hatches or hand-roll orchestration on top of the typed API is a signal
of a gap worth closing. File references below point at consumer code as it
stood when this document was written; treat them as evidence, not as live
links.

## 1. Close the remaining consumer escape hatches — ✅ done

Small typed methods; each was a place a consumer built argv by hand.
**Status:** implemented — 1.2 and 1.3 turned out to be already covered by
existing APIs (the consumer code predates them); the rest shipped as described
below.

| # | Status | Gap | Evidence | API |
|---|---|---|---|---|
| 1.1 | ✅ | Read a jj commit description | `vcs-flow-rs crates/commit/src/vcs.rs:158` (`jj log -r <revset> -T description`) | `JjApi::description(dir, revset) -> String` (wrapper over `template_query`, `--limit 1`) |
| 1.2 | ✅ already covered | `jj squash … --use-destination-message` with filesets | `vcs.rs:205` | `squash_paths(dir, from, into, filesets, use_destination_message)` already exists |
| 1.3 | ✅ already covered | git push with an explicit refspec + `-u` | `vcs.rs:501` (`git push -u origin local:remote`) | `push(dir, GitPush)` with `GitPush::refspec(local, remote_branch).remote(_).set_upstream()` already exists |
| 1.4 | ✅ | fetch from a *named* remote | `vcs.rs:265` (`git fetch origin`; typed `fetch()` is bare) | `GitApi::fetch_from(dir, remote)` / `JjApi::git_fetch_from(dir, remote)` + facade `Repo::fetch_from(remote)`, retried like `fetch` |
| 1.5 | ✅ | List git conflicted files | `vcs.rs:518` (`git diff --name-only --diff-filter=U`) | `GitApi::conflicted_files(dir)`; jj already had `resolve_list` |
| 1.6 | ✅ | Unified conflict listing on the facade | both consumers dispatch by hand | `Repo::conflicted_files() -> Vec<String>` (git `diff-filter=U` / jj `resolve_list -r @`) |
| 1.7 | ✅ | Dirty-tree check ignoring untracked | `vcs.rs:342` (`git status --porcelain --untracked-files=no`) | `GitApi::status_tracked(dir)` + facade `Repo::has_tracked_changes()` (jj: equals `has_uncommitted_changes`) |

## 2. Orchestration primitives — ✅ done

Both consumers independently built the same machinery on top of the typed
API — the strongest possible signal it belongs here. These are *separate
primitives*, not a false cross-backend abstraction (the merge / op-rollback
divergence stays deliberately non-unified, as documented in `vcs-core`).
**Status:** implemented as described, with two shape adjustments found during
design: the transaction closure receives a bound `JjAt` (rollback on `Err`
only — panic-rollback is impossible without async `Drop`), and
`switch_with_stash` is inherent on `Git` rather than a `GitApi` trait method
(composed operation, wrong mock surface for the trait).

- **2.1 ✅ jj transaction with op-log rollback.** Both consumers capture
  `op_head` before a mutation chain and `op_restore` on failure. Shipped as
  `Jj::transaction(dir, |tx| async { … })` (also on `JjAt`): snapshots the
  operation id, hands the closure a bound `JjAt`, restores on `Err`.
- **2.2 ✅ Dry-run merge.** `agent-workspace` probes with `merge --no-commit` +
  abort; jj-side it merges into a throwaway change and op-restores. Shipped as
  `Repo::try_merge(source) -> MergeProbe`
  (`MergeProbe = Clean | Conflicts(Vec<String>)`), with guaranteed rollback —
  a failing rollback propagates instead of misreporting.
- **2.3 ✅ Abort/continue as one state machine.** Shipped as
  `Repo::abort_in_progress()` and `Repo::continue_in_progress()` returning the
  fresh post-call `OperationState` (git: `merge --abort` / `rebase --abort` /
  the `_continue` twins, with `Conflict` reported while unresolved paths block;
  jj: reporting no-ops — rollback goes through 2.1).
- **2.4 ✅ Stash-safe branch switch.** `agent-workspace`'s sequencing (a failed
  checkout leaves the changes safe) shipped as
  `Git::switch_with_stash(dir, branch)` (also on `GitAt`), with a clean-tree
  fast path that skips the stash round-trip.

## 3. Widen `vcs-github` for PR-lifecycle automation — ✅ done

The `gh` wrapper is the thinnest crate (views + `pr_create`). Agent-style
consumers need the rest of the loop — "open a PR, watch CI, react to review,
merge". **Status:** implemented; gh CLI facts (exit codes, JSON shapes, flag
spellings) validated empirically on gh 2.93.

- **3.1 ✅** `pr_merge` (merge/squash/rebase strategy via a `PrMerge` builder,
  `--auto`, `--delete-branch`), `pr_ready`, `pr_close`
- **3.2 ✅** `pr_checks` → `Vec<CheckRun>` (gh's 0/8/1 outcome exit codes all
  return the parsed list; branch on `bucket`) and `run_list` / `run_view` /
  `run_watch` for GitHub Actions runs. `run_watch` returns the final
  `WorkflowRun` rather than an exit-code bool — gh exits 1 on failure but 2 on
  cancellation, so only `conclusion` reports the outcome faithfully.
- **3.3 ✅** `pr_review` (body embedded in `ReviewAction` — request-changes
  without a body is unrepresentable) / `pr_comment`, plus `pr_feedback`
  reading reviews and comments (`pr view --json reviews,comments`)
- **3.4 ✅** `issue_create` / `issue_view` (extends `Issue` with `body`/`url`);
  `release_list` / `release_view`

## 4. Coverage gaps in the git/jj clients — ✅ done

**Status:** implemented (client-level only — these stay off the facade by
design). Two behavioural surprises recorded during empirical validation:
git's default `merge` rebase backend auto-drops an emptied patch on
`--continue` — the "nothing to commit … skip" refusal that motivates
`rebase_skip` exists only under `rebase.backend=apply`; and `jj evolog -T`
renders in a *commit* context (bare `change_id` doesn't exist — the
`commit.`-method template form is required). Also: whether `jj git clone`
colocates by default depends on the jj version *and* `git.colocate` config, so
`git_clone` always passes the flag explicitly.

- **4.1 ✅ git:** `clone_repo` + `CloneSpec` (today `init` was the only way to
  obtain a repo!), tag operations (`tag_create`/`_create_annotated`/`_list`/
  `_delete` — release tooling), `show_file` (`show <rev>:<path>`, separators
  normalised — review/agent tooling), `cherry_pick`, `revert`,
  `config_get`/`config_set`, `remote_add`/`remote_set_url`, `blame` →
  `Vec<BlameLine>`, `rebase_skip`.
- **4.2 ✅ jj:** `git_clone`, `absorb` (fold edits into the changes that touched
  those lines — ideal for agent workflows), `split_paths`, `duplicate`,
  `op_log` → `Vec<Operation>` (the list; only head/restore/undo existed),
  `evolog`, `file_annotate` (+ bonus `file_show`, the twin of git's
  `show_file`).

## 5. Infrastructure and quality — ✅ done

- **5.1 ✅ `vcs-testkit` crate.** Shipped: `TempDir`, `configure_identity`,
  `GitSandbox`, `BareRemote::seeded`, `JjSandbox`, free `git()`/`jj()` raw
  steps — dependency-free, synchronous, panics on failure. Our own test
  suites migrated onto it (the 3× `TempDir` / 2× `bare_remote` / per-file
  init-helper duplication is gone); consumers use it as a crates.io
  dev-dependency.
- **5.2 ✅ Streaming / progress hooks — spec delivered upstream** (toolkit
  adoption pending a processkit release). Finding: processkit 0.6 already
  ships per-line callbacks (`Command::on_stdout_line`/`on_stderr_line`), so
  the requirements note handed to the ProcessKit project asks for
  hardening, not streaming: callback panic isolation (primary), documented
  ordering guarantees, and ScriptedRunner replaying canned output through
  handlers so streaming consumers are hermetically testable. We do not fork
  processkit.
- **5.3 ✅ Capability detection.** `capabilities()` on both clients →
  `GitCapabilities`/`JjCapabilities` (parsed version + `is_supported()` /
  `ensure_supported()` with a clear "needs jj ≥ 0.38, found 0.35.0"). jj's
  floor is precise (0.38, the empirically validated release); git gates the
  major only (validated on 2.54, expected ≥ 2.30 — an untested minor isn't
  hard-gated). Value types: callers cache the probe; the client holds no
  state.
- **5.4 ✅ Command observation** — satisfied by existing seams, documented in
  the README ("Observing commands"): wrap-the-runner argv observation
  (`RecordingRunner::new(JobRunner::new())`), live per-line streaming
  (processkit 0.6), the `tracing` feature, and `ScriptedRunner::fallback` as
  a dry-run harness. A first-class `on_command` hook is listed in the 5.2
  spec as a secondary, optional upstream ask.

## 6. Longer-horizon directions (independent of today's consumers)

Where the toolkit could go as a general-purpose "typed CLI automation" SDK,
regardless of what the current consumers need. Unordered; each item should be
picked up only when a concrete use case appears.

### New forges

- **6.1 Forge wrappers beyond GitHub:** `vcs-gitlab` (`glab`), `vcs-gitea`
  (`tea`). Their PR/MR surfaces map closely onto `vcs-github`'s; a
  `ForgeApi` facade over them (the way `vcs-core` sits over git/jj) would let
  a tool target "the forge" instead of GitHub specifically.

### Safety for untrusted input and untrusted repos

- **6.2 Typed argument newtypes.** `RefName` (git `check-ref-format` rules),
  `Revset`, `Fileset`, `Refspec` with validating constructors, so a
  caller-supplied string can never smuggle a flag into argv; audit every
  builder for `--` separators. Matters as soon as any input is not a literal
  in the caller's source (UIs, bots, agents).
- **6.3 Hardened execution profile.** Cloning a repository and running `git`
  inside it executes that repository's hooks and honours its config —
  arbitrary code execution for any automation that touches repos it didn't
  create. Offer a profile that scrubs `GIT_*` env, pins config via `-c`,
  disables hooks (`core.hooksPath`), and keeps terminal prompts off
  (partially done today), so "inspect this untrusted checkout" is safe by
  construction.

### Performance

- **6.4 Batched snapshot queries.** `Repo::snapshot()` collecting branch,
  status, ahead/behind, and head metadata in one or two process spawns
  (git `for-each-ref`/`status -z` combined formats; one jj template query)
  instead of N round-trips — what prompt and TUI integrations actually need.
- **6.5 Persistent query sessions.** `git cat-file --batch`-style long-lived
  children for fast object/metadata reads. Needs a long-lived-process
  capability in `processkit` — written as an upstream spec, like the
  streaming hooks in §5.

### Repo events

- **6.6 Watching.** Filesystem-watch `.git`/`.jj`, debounce, re-query, and
  emit typed events (`HeadMoved`, `BranchCreated`, `WorkingCopyChanged`) —
  the foundation for status bars, TUIs, and daemons, and a layer no CLI
  provides by itself.

### Structured conflicts

- **6.7 Typed conflict model.** Parse conflict markers (`diff3`/`zdiff3`,
  jj's materialized conflicts) into structured regions — base/ours/theirs
  per hunk — plus a writer to apply a chosen resolution. This is the missing
  primitive for programmatic and assisted conflict resolution; today every
  tool re-greps `<<<<<<<` by hand.

### Agent-facing surface

- **6.8 `vcs-mcp`.** An MCP server crate exposing the typed operations as
  tools (read-mostly by default, mutations behind an explicit allowlist),
  built on the facade. Lets agent harnesses drive repositories through
  structured, validated calls instead of raw shell — the safety items above
  are the prerequisite.

### Quality and project maturity

- **6.9 CLI version matrix in CI.** Test against current and previous git/jj
  releases (jj's CLI moves fast; the parsers are validated against jj 0.38
  empirically). Pairs with the capability probes in §5 to catch parser drift
  before users do.
- **6.10 Fuzz and property-test the parsers.** They are pure functions over
  arbitrary CLI text — ideal `cargo-fuzz`/proptest targets.
- **6.11 Cookbook and positioning docs.** Task-oriented recipes, plus an
  explicit "when to use this vs `gitoxide`/`git2` bindings" guide (answer:
  when you want the installed binary's exact behaviour, config, and
  credentials — spell out the trade-off).
- **6.12 Path to 1.0.** Per-crate stability tiers, an MSRV policy, and a
  public API review once the consumer-driven phases (§1–§3) have settled the
  shape.

## Deliberately out of scope

1. **Copy-on-write worktree cloning (reflink) and its cross-process lock.**
   Stays in `agent-workspace`: the copy strategy is injected by the consumer,
   and `reflink-copy` is not a toolkit dependency. The toolkit's seam is
   `worktree_add(…, no_checkout)`, which already exists.
2. **A single cross-backend `merge`/`undo` button on the facade.** git merge
   and jj's `new_merge`+`squash`, and git history rewriting vs jj's op log,
   diverge for real; §2 exposes honest per-backend primitives instead.
3. **A blocking (non-async) API.** Both consumers run tokio; the only
   synchronous need is `Drop`-context cleanup, which the `blocking` helper
   modules already cover.
4. **Index-repair / batching policy after `--no-checkout`.** Application
   policy (progress UI, thresholds), not a CLI-wrapping primitive.
