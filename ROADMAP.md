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

## 2. Orchestration primitives

Both consumers independently built the same machinery on top of the typed
API — the strongest possible signal it belongs here. These are *separate
primitives*, not a false cross-backend abstraction (the merge / op-rollback
divergence stays deliberately non-unified, as documented in `vcs-core`).

- **2.1 jj transaction with op-log rollback.** Both consumers capture
  `op_head` before a mutation chain and `op_restore` on failure. Provide
  `Jj::transaction(dir, |tx| async { … })` (or an RAII `OpCheckpoint` guard)
  that snapshots the operation id and restores it on `Err`/panic.
- **2.2 Dry-run merge.** `agent-workspace` probes with `merge --no-commit` +
  abort; jj-side it merges into a throwaway change and op-restores. Unify as
  `Repo::try_merge(source) -> MergeProbe` where
  `MergeProbe = Clean | Conflicts(Vec<String>)`, with guaranteed rollback.
- **2.3 Abort/continue as one state machine.** `in_progress_state()` already
  reports `Merge`/`Rebase`/`Conflict`; add `Repo::abort_in_progress()` and
  `Repo::continue_in_progress()` (git: `merge --abort` / `rebase --abort` /
  the `_continue` twins; jj: no-op or `op_restore`).
- **2.4 Stash-safe branch switch.** Lift `agent-workspace`'s sequencing
  (checkout the target *before* `stash pop`, so a failed checkout leaves the
  stash intact) into `GitApi::switch_with_stash(dir, branch)`.

## 3. Widen `vcs-github` for PR-lifecycle automation

The `gh` wrapper is the thinnest crate (views + `pr_create`). Agent-style
consumers need the rest of the loop — "open a PR, watch CI, react to review,
merge":

- **3.1** `pr_merge` (merge/squash/rebase strategy, `--auto`,
  `--delete-branch`), `pr_ready`, `pr_close`
- **3.2** `pr_checks` (CI status per check) and `run_list` / `run_view` /
  `run_watch` for GitHub Actions runs
- **3.3** `pr_review` / `pr_comment`, plus reading reviews and comments
  (`pr view --json reviews,comments`)
- **3.4** `issue_create` / `issue_view`; `release_list` / `release_view`

## 4. Coverage gaps in the git/jj clients

Verified absent today; add as consumers (or new tools) demand them:

- **4.1 git:** `clone` (today `init` is the only way to obtain a repo!), tag
  operations (create/list/delete — release tooling), `show <rev>:<path>`
  (file content at a revision — review/agent tooling), `cherry_pick`,
  `revert`, `config_get`/`config_set`, `remote_add`/`remote_set_url`,
  `blame`.
- **4.2 jj:** `git clone`, `absorb` (fold edits into the changes that touched
  those lines — ideal for agent workflows), `split`, `duplicate`, `op_log`
  (the list; only head/restore/undo exist today), `evolog`, `file annotate`.

## 5. Infrastructure and quality

- **5.1 `vcs-testkit` crate.** Builders for temp repositories (git / jj /
  colocated; with commits, conflicts, a bare remote). Both consumers carry
  hundreds of lines of this in their test trees, and our own `--ignored`
  integration tests re-implement the same scaffolding.
- **5.2 Streaming / progress hooks** for long operations (clone/fetch/push):
  a per-line stderr callback. Likely a `processkit` capability first —
  written up as an upstream spec, same as the `Error::Exit.stdout` change
  (we do not fork processkit).
- **5.3 Capability detection.** jj's CLI changes between releases (the
  parsers here are validated against jj 0.38). `Jj::capabilities()` — a
  cached version probe that gates flags and fails with a clear
  "needs jj ≥ X" instead of an argv error.
- **5.4 Command observation hook** (`on_command(argv, dir)`) for tracing, UI
  progress, and a dry-run mode.

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
