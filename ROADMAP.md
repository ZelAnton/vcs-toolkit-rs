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

> **Planning layout.** This file holds **committed work**. Open ideas not yet
> committed live in [`ideas/`](ideas/) (`next-` = reconsider first, `later-` = further
> out / consumer-gated); settled rejections and scope boundaries live in
> [`decisions/`](decisions/). See [`ideas/README.md`](ideas/README.md) for the
> today / next / later / won't-do bucket scheme.

---

## Active roadmap (do now)

The R1–R10 wave (2026-06-09 / 2026-06-10 sweeps) is **shipped** — summary below. With it
cleared, the active list is empty; the next committed items get promoted from
[`ideas/next-*`](ideas/) (forge `capabilities()`, `examples/`, MCP HTTP transport) when work
resumes. Settled-against items live in [`decisions/`](decisions/).

### ✅ Shipped — the R1–R10 wave (2026-06-10)

- **R1 ✅** — jj `create_worktree` rolls back (forget workspace + remove dir) when the
  `bookmark_create` step fails, with a hermetic `ScriptedRunner` test
  (`crates/core/src/jj_backend.rs`).
- **R2 ✅** — root cause fixed upstream (processkit 0.9.1 untruncated `Error::Exit`, now
  adopted); added a >4 KiB classification regression test (`crates/cli-support/src/lib.rs`).
- **R3 ✅** — report-only `cargo-semver-checks` CI job (`continue-on-error`; advisory while
  every crate is 0.x, ready to enforce at 1.0).
- **R4 ✅** — proptest panic-freedom for the gitea `tea` parsers (`crates/gitea/src/parse.rs`;
  `proptest` added to vcs-gitea dev-deps).
- **R5 ✅** — gitlab issue/release integration round-trips added (graceful-skip without a live
  GitLab); the hermetic argv/parse coverage in `crates/gitlab/src/lib.rs` was found already
  comprehensive (the line-count gap vs github was a CI-platform difference, now documented).
- **R6 ✅** — `SECURITY.md` / `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` + `.github/` issue & PR
  templates (and the README "Conventions" link repointed off the untracked `AGENTS.md` to
  `CONTRIBUTING.md`).
- **R7 ✅** — `keywords` + `categories` on all 12 crate manifests.
- **R8 ✅ (resolved by analysis — won't implement)** — our exit-code idioms
  (`probe` / `exit_code` / `output`) are already correct; `ok_codes` would not improve them.
  See [`decisions/wont-do-2026-06.md`](decisions/wont-do-2026-06.md).
- **R9 ✅** — `is_transient_fetch_error` now also consults processkit's io-level
  `Error::is_transient()` (`crates/cli-support/src/lib.rs`), with a test.
- **R10 ✅** — `timeout_grace` on the git/jj fetch ops (graceful terminate-then-kill on a
  per-client timeout; `FETCH_TIMEOUT_GRACE` in `vcs-cli-support`).

---

## Completed program (history)

The §1–§7 program below is **complete** — retained as the design record (what shipped
and why, with the empirical CLI facts discovered along the way). It is history, not a
worklist; live work is the Active roadmap above.

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
  adoption pending a processkit release). Finding: processkit (0.6+) already
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
  (processkit 0.6+), the `tracing` feature, and `ScriptedRunner::fallback` as
  a dry-run harness. A first-class `on_command` hook is listed in the 5.2
  spec as a secondary, optional upstream ask.

## 6. Longer-horizon directions (independent of today's consumers)

Where the toolkit could go as a general-purpose "typed CLI automation" SDK,
regardless of what the current consumers need. Being executed as a program of
waves: **A** = 6.2+6.3+6.7 (safety substrate — ✅ done), **B** = 6.9+6.10
(✅ done), **C** = 6.4+6.5+6.11+6.12 (✅ done; 6.5 spec-only), **D** = 6.1
(forges — ✅ done), **E** = 6.6 (watching — ✅ done), **F** = 6.8 (vcs-mcp — ✅
done). The §6 wave program (A–F) is **complete**; remaining §6 items below are
additive follow-ups, not a blocking wave.

### New forges

- **6.1 ✅ Forge wrappers beyond GitHub.** Shipped `vcs-gitlab` (`glab`) and
  `vcs-gitea` (`tea`), mirroring `vcs-github`'s shape, plus a `vcs-forge` facade
  (`Forge` + the object-safe `ForgeApi`) that dispatches the **lean PR/MR
  lifecycle** — auth, repo view, list/view/create/merge/mark-ready/close, CI
  status — across all three with unified DTOs (`ForgePr`/`ForgePrState`/
  `ForgeRepo`/`CiStatus`), the way `vcs-core` sits over git/jj. A forge has no
  filesystem marker, so `Forge` is constructed explicitly (optionally via
  `ForgeKind::from_remote_url`). Gitea's `tea` lacks a repo view, draft toggle,
  and checks command, so those return `Error::Unsupported` for that backend. The
  argv + JSON shapes are pinned by hermetic fixtures; the `#[ignore]` smoke tests
  check real-binary integration (`version`/`auth_status`, CI installs `glab`/`tea`
  best-effort). The create/merge lifecycle argv tracks the documented CLIs but
  isn't exercised end-to-end in CI (needs a live forge). Future, additive: issues,
  releases, reviews/comments per forge.

### Safety for untrusted input and untrusted repos

- **6.2 ✅ Typed argument newtypes + injection guards.** Shipped as two
  layers: automatic guards on every exposed positional (a leading-`-`/empty
  value is refused before spawning — verified git/jj parse such values as
  flags), plus optional validating newtypes `RefName`/`RevSpec` (vcs-git)
  and `RevsetExpr` (vcs-jj). Signatures stay `&str` — a full newtype
  migration would be breaking churn with no added safety once the guards
  exist (recorded decision). Paths already went through `--`/embedding.
- **6.3 ✅ Hardened execution profile.** Shipped as `Git::harden()` /
  `Git::hardened()`: hooks off via env-based config
  (`core.hooksPath=/dev/null`, verified to suppress hooks on Windows),
  `core.fsmonitor=false`, repo-redirecting `GIT_*` scrubbed, system config
  skipped, prompts off — applied to every command via processkit's
  `default_env`/`default_env_remove` (no upstream work needed). jj
  deliberately has no equivalent (no repo-local hooks; documented).

### Performance

- **6.4 ✅ Batched snapshot queries.** `Repo::snapshot() -> RepoSnapshot`
  collects branch, upstream, ahead/behind, HEAD, dirtiness, change count, and
  operation state in **one or two** spawns instead of N. git uses a single
  `status --porcelain=v2 --branch -z` (a new `vcs_git::BranchStatus` +
  `parse_porcelain_v2` — branch/upstream/ahead-behind/changes/unmerged in one
  call) plus the cheap in-progress fs probe; jj uses one `log -r @` template
  (commit id + bookmarks + `empty` + `conflict`) plus a change count only when
  dirty. Documented asymmetry: `upstream`/`ahead`/`behind` are always `None` on
  jj (no git-style upstream tracking).
- **6.5 Persistent query sessions — spec delivered upstream** (toolkit adoption
  pending a processkit release). `git cat-file --batch` / `gh api --paginate`-style
  long-lived children for fast repeated object/metadata reads need a capability
  `processkit` doesn't expose, and we do not fork it. *Finding:* the requirements
  note handed to the ProcessKit project asks for a **persistent-process API** — a
  child spawned once and held inside the same OS job, with a framed
  request/response pipe (write a query line, read a length- or NUL-delimited
  response), explicit cancellation and cleanup-on-drop, and a `ScriptedRunner`
  analogue that replays canned framed responses so a batch consumer stays
  hermetically testable (the same testability requirement as the §5.2 streaming
  hooks). Until that ships, batch reads go through one spawn per query (or the
  batched `snapshot` of 6.4 for the common case).

### Repo events

- **6.6 ✅ Watching.** Shipped `vcs-watch`: `RepoWatcher` filesystem-watches
  `.git`/`.jj` (jj wins when colocated; worktree gitlinks resolved), debounces
  the write burst, **re-queries** `vcs-core`'s batched `snapshot()` (+
  `local_branches`), and **diffs** against the previous state to emit typed
  `RepoEvent`s (`HeadMoved`, `BranchSwitched`, `BranchCreated`/`Deleted`,
  `WorkingCopyChanged`, `UpstreamChanged`, `AheadBehindChanged`,
  `OperationChanged`, `ConflictChanged`). Each settled change is a `RepoChange {
  snapshot, events }` (bundled state + deltas) on an async `recv()` stream;
  re-query+diff makes raw-event noise (ref temp-renames, `index.lock`, reflog) a
  no-op. Decisions: raw `notify` + a custom debounce (default 250 ms / 1 s
  ceiling); watch scope configurable (state-dir default, opt-in working-tree).
  The pure diff is hermetically unit-tested; the debounce → re-query pipeline
  is hermetically fake-time tested (§7 Wave R), with the notify bridge covered
  by `#[ignore]` real-repo tests. This is the workspace's first runtime-tokio +
  streaming crate; the `stream` feature adds an `impl futures_core::Stream`
  (§7 Wave R). Future, additive: `.gitignore`-aware working-tree filtering.

### Structured conflicts

- **6.7 ✅ Typed conflict model.** Shipped as `vcs_git::conflict` (git's
  `merge`/`diff3`/`zdiff3` styles, variable marker size, CRLF preserved —
  also parses jj's `git` marker style) and `vcs_jj::conflict` (jj's native
  `diff` and `snapshot` styles, `conflict N of M` counters): structured
  regions, byte-exact `render`, and a `resolve(side)` writer. Nuance
  recorded: in jj's default `diff` style one side is stored as a unified
  diff against the base, so `resolve` reconstructs it by applying the diff.
  Round-tripped against real conflicts in integration tests.

### Agent-facing surface

- **6.8 ✅ `vcs-mcp`.** Shipped an MCP server crate (a lib + the `vcs-mcp`
  binary, on the official `rmcp` SDK over stdio) exposing the typed operations
  of **both facades** — `vcs-core` (git/jj) and `vcs-forge` (PR/MR, issues,
  releases) — as MCP tools. Read tools are always on (annotated
  `readOnlyHint`); the ten mutating tools are **gated behind a `WriteGate`**
  (annotated `destructiveHint`, reject up front when outside the gate):
  `--allow-write` enables all mutations, `--allow-tools <name,…>` a per-tool
  allowlist (§7 Wave A). The forge is auto-detected from the `origin` remote
  (`--forge` overrides). Returns the facade DTOs as JSON via a new **optional
  `serde` feature** on `vcs-diff`/`vcs-core`/`vcs-forge` (off by default —
  default builds stay serde-free). The safety substrate (injection guards,
  hardened profile) applies under every tool. Future, additive: more tools, an
  HTTP transport.

### Quality and project maturity

- **6.9 ✅ CLI version matrix in CI.** A Linux `integration` job runs the
  `#[ignore]` suites against jj **0.38 / 0.40 / 0.42** (floor / mid / latest,
  installed by pinned `gh release download`) plus the floor on an older-git
  image — catching CLI/template drift before users do. Pre-validated locally
  against jj 0.42: zero drift (the §4/§6 surface still parses). The hermetic
  3-OS `test` job stays on runner-default versions.
- **6.10 ✅ Fuzz and property-test the parsers.** `proptest` (stable, in the
  CI gate) fuzzes every pure parser in vcs-git/vcs-jj for panic-freedom on
  arbitrary + structure-biased input, plus a byte-exact `render(parse(x))==x`
  invariant on the conflict modules. It **found a real bug**: `parse_porcelain`
  byte-sliced a status record assuming ASCII codes and panicked on a leading
  multibyte char — fixed (boundary-safe `get`) with a regression test. An
  optional `fuzz/` dir (cargo-fuzz, nightly, workspace-excluded) carries
  libFuzzer targets for the two conflict parsers.
- **6.11 ✅ Cookbook and positioning docs.** `docs/cookbook.md` (task-oriented
  end-to-end recipes — prompt line via `snapshot`, PR-and-watch-CI, stash-safe
  switch, programmatic conflict resolution, backend detection, jj transaction)
  and `docs/positioning.md` ("when to use vcs-toolkit vs `gitoxide`/`git2`": use
  it for the installed binary's exact behaviour/config/credentials and for
  jj+GitHub, which the libraries don't cover; reach for gitoxide/git2 for
  in-process, no-subprocess object reads — with a fair comparison table).
- **6.12 ✅ Path to 1.0.** `docs/stability.md`: per-crate stability tiers, the
  SemVer/versioning policy (`0.x` allows breaking; strict after 1.0; independent
  per-crate versions), the MSRV policy (floor `1.88`, machine-checked via
  `rust-version`, bumps are minor), and a public-API review checklist for the
  1.0 gate (object-safety + mockability, `#[non_exhaustive]` coverage, structured
  errors, injection guards, no leaked internals, docs+tests).

### Upstream-gated (specs delivered to ProcessKit-rs)

- **6.13 ✅ Cancellable operations — adopted (processkit 0.8).** The
  client-cancellation spec landed in processkit 0.8: a **client-level**
  `CliClient::default_cancel_on(token)` re-emitted on the `cli_client!` wrappers
  (so `Git`/`Jj`/`GitHub`/… gain `default_cancel_on` when the `cancellation`
  feature is on), plus `Reply::pending()` so the path is hermetically testable.
  Adoption needed **zero new vcs-* API** exactly as predicted: an off-by-default
  `cancellation` feature on each wrapper (forwarded by `vcs-core`/`vcs-forge`)
  turns the builder on; a consumer builds a cancellable client and passes it
  through the existing `Repo::from_git`/`Forge::for_github` constructors, then a
  controller calls `token.cancel()` to kill every in-flight call (`Error::Cancelled`,
  treated as terminal by the fetch-retry). Shipped with it: hermetic paused-clock
  cancellation tests (`run_watch` in vcs-github, retried `fetch` in vcs-git, via
  `Reply::pending()`), an explicit `Cancelled → not transient` classifier test, a
  cookbook recipe, and the testing-guide pattern. (Per-command `Command::cancel_on`
  in the object-safe `*Api` traits stays rejected — the client-level default is the
  ergonomic, mock-friendly seam.)

- **6.14 Other processkit 0.8 features — evaluated, shelved (no consumer).** The
  0.8 bump also offered streaming hardening (R1–R3: handler-panic isolation,
  ordering, scripted-stream replay) and `ProcessRunner::start`, pipeline
  `unchecked()`/`|`, `ProcessResult::outcome()`, supervisor storm-guard, and
  `kill_on_parent_death`. The toolkit has **no consumer** for any: zero
  `on_*_line` streaming wrappers, zero `.pipe()` chains, no `Supervisor`,
  kill-on-drop already covers process teardown, and the transient classifier is
  message-based (so `outcome()` is a non-improvement). The one fan-out primitive
  with a real (if minor) consumer — `output_all` for jj-workspace enumeration —
  *was* adopted (see `vcs-jj`'s `workspace_roots`). Revisit the rest only when a
  consumer appears.

  - **`vcs-mcp` cancellation — deferred (request-lifecycle plumbing, not a feature
    flag).** The server deliberately has no `cancellation` feature: every client it
    builds already carries a `default_timeout` (configurable, surfaces as
    `Error::Timeout`), and it exposes no `run_watch` tool — so the unbounded-by-nature
    operation cancellation targets isn't reachable through mcp. The genuine gap is
    cancel-on-peer-disconnect / cancel-on-shutdown, which needs the server to own a
    token **per in-flight tool call** and bridge rmcp's cancellation/disconnect
    signal to it (rmcp's `#[tool]` dispatch doesn't hand that over for free) —
    strictly more than turning on `vcs-mcp/cancellation`. Pick it up if/when an
    agent harness needs soft-disconnect teardown.

## 7. Architecture program R → A → S (post-§6 fresh-eyes review)

A whole-workspace architecture review (2026-06-07; no users yet → breaking
changes free) found the design sound and focused the program on testability,
API completion, and extension-ritual cost. Three waves, each gated by the full
matrix + ≥2-pass adversarial review:

- **7.1 ✅ Wave R — reliability.** The vcs-watch debounce → ceiling → re-query
  pipeline became a free function over injected seams and is **hermetically
  fake-time tested** (9 paused-clock tests: coalescing, exact `max_wait`
  ceiling, transient skip + recovery, re-query deadline, teardown, backpressure,
  stream adapter); added `Builder::requery_timeout` (default 30 s, kills a
  wedged re-query as transient), `RepoWatcher::stats()` (lock-free health
  counters), and the `stream` feature. CI gained a **feature-isolation job**
  (each optional feature compiled solo per crate); classifier regression tests
  run against the real CLIs in the integration lane; forge host-classification
  and state mappers got proptests; `vcs-mcp` argv parsing became a testable
  function with a bin-test seed. Plus a real `diff3` parser fix the proptests
  surfaced (repeated base-marker line; seed committed).
- **7.2 ✅ Wave A — API completion (breaking).** Facade `Repo::push(branch)`
  (honest LCD; git `push -u origin` / jj `git push -b`); forge issues +
  releases unified end-to-end (`glab`/`tea` wrapper methods verified against
  the official docs → `ForgeIssue`/`ForgeRelease` DTOs → five `Forge`/`ForgeApi`
  methods → five MCP tools, `Unsupported` where `tea` can't); the **builder
  rule** ("≥2 options or any bare bool → spec/builder", now in AGENTS.md)
  applied across both levels (`CommitPaths`, `MergeCommit`, `MergeNoCommit`,
  `AnnotatedTag`, `SquashPaths`, gh/forge `PrCreate`, glab `MrCreate`, tea
  `PrCreate`; `ReviewAction` → kind+body struct keeping
  request-changes-requires-body unrepresentable); MCP `WriteGate` with
  `--allow-tools` per-tool allowlist; docs (escape-hatch routers in
  core.md/forge.md, the three call shapes, security decision notes).
- **7.3 ✅ Wave S — structural dedup.** A `facade_trait!` `macro_rules!` (one
  per facade — `vcs-core`, `vcs-forge`) now generates each trait decl **and** its
  delegating `impl … for Repo`/`Forge` from a single signature table, so the two
  can't silently drift; the real backend-`match` bodies stay hand-written on the
  inherent `impl` (the macro never owns a non-trivial body). Two sub-decisions
  resolved during the wave:
  - **automock spike — fell back (documented).** Adding `mockall::automock` to the
    generated traits is **impossible**: `macro_rules!` captures the method
    signatures as opaque `:ty` nonterminal fragments, which `automock`'s `syn`
    parser rejects ("unsupported type in this position"). The `:ty` capture alone
    is the cause (reproduced with the methods stripped to bare signatures — no
    docs, no `concat!`); `#[async_trait]` tolerates the fragments, `mockall` does
    not. The facade
    traits stay seam-tested over a fake runner (already what their docs recommend
    over mocking); no `mock` feature was added.
  - **marker-primitive extraction into vcs-diff — rejected (stop-the-line).** git's
    `marker_run` leaves the size constraint to call sites (variable
    `conflictMarkerSize`); jj bakes `n>=7` in (it lengthens all of a file's markers
    together). Disjoint vocabularies (`<=>|` vs `<%\+->`), structurally different
    parse loops, ~4 genuinely shared lines — any extraction bends one model. Both
    conflict modules stay independent.

## Boundaries and rejected ideas

The former **"Consciously rejected"** and **"Deliberately out of scope"** lists now live
in [`decisions/wont-do-2026-06.md`](decisions/wont-do-2026-06.md) — consolidated with one
reason each — so this roadmap holds only live and historical *work*. Open, not-yet-
committed ideas are in [`ideas/`](ideas/). (One former entry, **retry jitter**, has been
reopened as an active upstream proposal to ProcessKit-rs.)
