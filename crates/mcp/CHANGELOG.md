# Changelog — vcs-mcp

All notable changes to the `vcs-mcp` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-mcp-v<version>`.

## [Unreleased]

### Added
- New read tool `repo_diff`: the full parsed working-copy diff (one file
  entry per changed file), the same scope `repo_diff_stat` already covers (git:
  working tree vs `HEAD`, excluding untracked files; jj: `@` vs its parent,
  including newly-added files). Surfaces `vcs-core`'s new `Repo::diff()`
  (`Vec<FileDiff>`) as JSON, modeled on `repo_show_file`/`forge_pr_diff` — it
  inherits the repo client's `OutputBudget`, so an over-budget diff errors with
  `OutputTooLarge` rather than a silently truncated result. Like every other
  jj-backed `repo_*` read, it lets jj snapshot the working copy (records an op-log
  entry), so it is annotated `destructiveHint = false` + `idempotentHint = true`
  rather than `readOnlyHint` — see the annotation change below. (T-068.)
- Two new write-gated PR/MR **review** tools: `forge_pr_approve` (approve — all
  three forges) and `forge_pr_request_changes` (request changes with a required
  body — GitHub/Gitea; `Unsupported` on GitLab, whose review model is
  approve/revoke). Both follow the existing forge-mutation write-gate pattern
  (`require_write`, `destructiveHint`) and require `--allow-write` (or
  `--allow-tools` naming them). `forge_info`'s capability map gains the matching
  `pr_approve` / `pr_request_changes` flags. (T-072.)
- Six new write-gated mutation tools: `repo_rebase`, `repo_abort_in_progress`,
  `repo_continue_in_progress`, `repo_new_child`, `repo_delete_branch`, and
  `repo_rename_branch`. They expose `vcs-core` facade methods for managing
  repository operations, branches/bookmarks, and new child revisions. All
  follow the existing write-gate pattern and require `--allow-write`.
- `--max-output-bytes <n>` caps content-tool output (`repo_show_file`,
  `forge_pr_diff`) at a default 10 MiB ceiling (`0` disables it), the same
  `OutputBudget` mechanism (T-049) already honoured by the library when a caller
  injects a budget-bound client. The binary previously served `OutputBudget::unlimited()`
  on both the repo (git/jj) and forge clients, so a giant blob or PR diff would
  buffer whole into the server's (and then the JSON response's) memory; exceeding
  the new default returns `OutputTooLarge` rather than a silently truncated
  result. (T-067.)

### Changed
- **MCP annotation fix (op-log honesty on jj).** The `repo_*` read tools that, on a
  jj backend, run a default working-copy-**snapshotting** jj command — and so record
  a (reversible) op-log operation — no longer advertise `readOnlyHint = true`, which
  the MCP spec defines as "does not modify its environment". Affected:
  `repo_status`, `repo_diff_stat`, `repo_diff`, `repo_snapshot`, `repo_log`,
  `repo_show_file`, `repo_branches`, `repo_current_branch`, `repo_conflicts`,
  `repo_worktrees`. They are now annotated `destructiveHint = false` +
  `idempotentHint = true` (the same honest classification `repo_try_merge` uses): the
  op-log snapshot is append-only/recoverable and changes no tracked content, refs, or
  bookmarks. `repo_info` and every `forge_*` read tool are genuinely read-only on both
  backends (no working-copy snapshot) and **keep** `readOnlyHint`. This is a
  metadata-only change — every tool stays callable in the default read-only mode (a
  snapshot is not a content/ref mutation, so none is write-gated) and no input/output
  shape changes; a client that auto-runs only `readOnlyHint` tools may now prompt for
  these jj-snapshotting reads. Documentation-only redefinition of `readOnlyHint` was
  rejected, and `--ignore-working-copy` is deliberately not used to reclaim it (it
  would report the last recorded operation rather than the live working tree — a stale
  read for a status/diff query). (T-068.)
- Serving a **bare** git repository (`git init --bare`, or a path at or under one
  with no working tree) now fails with the same classification `vcs-core` reports —
  `BareRepository: <path>` — instead of the generic
  `no git or jj repository found at or above <path>`. The server opens the repo
  through the new `Repo::discover_with`, so the bare-repo diagnostic (and any future
  discovery refinement) is shared with the facade rather than reimplemented in the
  binary; a non-repository path still reports "not a repository". No flags or tool
  shapes change. (T-061.)

### Fixed
- `repo_info`'s `root`/`cwd` no longer bypass the crate's non-UTF-8 path
  fail-closed policy (T-050): they used to serialize through
  `to_string_lossy`, silently substituting `U+FFFD` for a non-UTF-8
  root/working-directory (possible on Unix), unlike every other path-bearing
  DTO in this crate. They now serialize the borrowed `Path`s directly, so a
  non-UTF-8 root/cwd refuses the call with an explicit serialization error
  instead of returning corrupted JSON. The ordinary UTF-8 case is unchanged.
  (T-062.)
- `forge_pr_checkout` and `forge_pr_merge` (with `delete_branch`) now hold the
  same per-repo `write_lock` as `repo_*` mutations (via `begin_repo_write`)
  instead of only checking the write gate. Both locally mutate the working
  copy (checkout/switch and, for `forge_pr_merge`, deleting the local branch),
  so without the shared lock they could race a concurrent `repo_commit`/
  `repo_try_merge` and interleave with it. Purely remote forge mutations
  (`forge_pr_create`, `forge_issue_create`, `forge_pr_close`,
  `forge_pr_mark_ready`, `forge_pr_comment`, `forge_pr_edit`) are unaffected.
  (T-058.)

## [0.6.0] - 2026-07-10

### Added
- Documented **non-UTF-8 path policy (fail-closed)** for path-bearing results
  (`repo_status`'s `FileChange.path`, `repo_conflicts`'s list, `repo_diff`'s
  `FileDiff.path`): the facade carries each path losslessly as a `PathBuf`, and since
  JSON strings are UTF-8, a path that is not valid UTF-8 (possible on Unix) is now
  **refused with an explicit serialization error** rather than emitted with `U+FFFD`
  replacement — an agent is never handed a silently-corrupted path it would feed back
  into `repo_commit`. The ordinary UTF-8 case is unchanged (a plain JSON string).
  (T-050.)
- `repo_log` read tool: recent history (up to `max` commits reachable from a
  git revspec / jj revset), backed by the new `Repo::log` facade method. Always
  available (read-only, no `WriteGate`).
- `forge_pr_checkout` write tool (`{ number }`): check a PR/MR's branch out into
  the local working copy (`gh pr checkout` / `glab mr checkout` / `tea pr
  checkout`). Mutating, so it is `WriteGate`-gated (annotated `destructiveHint`)
  and added to `WRITE_TOOLS`; refused unless `--allow-write` or `--allow-tools
  forge_pr_checkout`.

### Changed
- **`forge_info` result** now carries two extra `capabilities` fields tracking the
  facade's version-aware capability map: `version` (the installed `gh`/`glab`/`tea`
  version as `{major,minor,patch}`, or `null` when unknown/unrecognisable) and
  `supported` (whether the CLI meets its declared version floor). The per-op flags
  are now the intersection of "the CLI ships the command", `supported`, and
  `authed` — so an old CLI below the floor reports its ops as unavailable rather
  than advertising a command that would fail deep with a cryptic error. Purely
  additive to the JSON object; existing fields are unchanged.
- **Breaking (tool schema):** the `forge_pr_merge` tool gained two optional
  boolean params, `auto` and `delete_branch` (both default `false`), tracking the
  facade's new unified `PrMerge` merge spec. Both are **GitHub-only** (`gh pr merge
  --auto --delete-branch`); on GitLab/Gitea, requesting either is rejected as
  `invalid_params` (the facade's `Unsupported`, a client-fixable request) rather
  than merging without it. `{ number, strategy }` calls keep working unchanged.
- Tool parameters that carry a git revspec / jj revset or a branch/bookmark name
  (e.g. `repo_log`, `repo_show_file`, `repo_checkout`) now surface a classifiable
  input-validation error when given a flag-like or malformed value: the `Repo`
  facade converts the JSON string into the backend's validated newtype at the
  boundary and rejects it **before** any child process spawns. No tool-schema
  change — the validation is enforced through the facade rather than the raw
  string being passed through.

### Fixed
- forge_pr_comment / forge_pr_edit: stop rejecting a legitimate leading-`-` body/title
  (a Markdown `- item` bullet list or `---` rule). These values ride in flag-VALUE
  slots on GitHub/GitLab (and Gitea's `--title`/`--description`), where a leading `-`
  is safe; the blanket MCP-layer `guard_argv_field` wrongly refused them for every
  backend. Argv-injection safety now lives solely at the wrapper layer, where the one
  bare positional (Gitea's `tea comment <n> <body>`) is still guarded by
  `reject_flag_like`. Behaviour is now uniform across forge_pr_create / forge_pr_edit /
  forge_pr_comment / forge_issue_create.

## [0.5.2] - 2026-07-06

### Changed

- core: rename Repo::open to Repo::discover; add strict Repo::open
- Release: vcs-diff v0.5.1, vcs-cli-support v0.5.1, vcs-git v0.9.1, vcs-jj v0.9.1, vcs-github v0.9.1, vcs-gitlab v0.5.1, vcs-gitea v0.5.1, vcs-forge v0.5.1, vcs-testkit v0.5.1, vcs-core v0.7.1, vcs-watch v0.5.1, vcs-mcp v0.5.1


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- refactor(a5): create_worktree takes a WorktreeCreate spec (branch/base not transposable)
- refactor(a5): Forge::issue_create takes an IssueCreate spec (extensible, mirrors PrCreate)
- review(0.4.0): whole-solution followups — MergeCheckPartial rename, is_merged test, mcp/core changelogs
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(wave1.5b): Repo::remove_worktree takes a WorktreeRemove spec, not a bare force bool (A1)
- refactor(wave1.5b): Forge::pr_close takes a PrClose spec, not a bare delete_branch bool (A1)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- review: write-gate repo_try_merge, forge Error classifier parity, forge_pr_mark_ready MCP tool
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(git): current_branch handles unborn repos via symbolic-ref


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.1] - 2026-07-05

### Changed

- core: rename Repo::open to Repo::discover; add strict Repo::open


### Changed

- Release: vcs-diff v0.5.0, vcs-cli-support v0.5.0, vcs-git v0.9.0, vcs-jj v0.9.0, vcs-github v0.9.0, vcs-gitlab v0.5.0, vcs-gitea v0.5.0, vcs-forge v0.5.0, vcs-testkit v0.5.0, vcs-core v0.7.0, vcs-watch v0.5.0, vcs-mcp v0.5.0


### Changed

- refactor(a5): create_worktree takes a WorktreeCreate spec (branch/base not transposable)
- refactor(a5): Forge::issue_create takes an IssueCreate spec (extensible, mirrors PrCreate)
- review(0.4.0): whole-solution followups — MergeCheckPartial rename, is_merged test, mcp/core changelogs
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(wave1.5b): Repo::remove_worktree takes a WorktreeRemove spec, not a bare force bool (A1)
- refactor(wave1.5b): Forge::pr_close takes a PrClose spec, not a bare delete_branch bool (A1)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- review: write-gate repo_try_merge, forge Error classifier parity, forge_pr_mark_ready MCP tool
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(git): current_branch handles unborn repos via symbolic-ref


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.5.0] - 2026-07-05

### Changed

- refactor(a5): create_worktree takes a WorktreeCreate spec (branch/base not transposable)
- refactor(a5): Forge::issue_create takes an IssueCreate spec (extensible, mirrors PrCreate)
- review(0.4.0): whole-solution followups — MergeCheckPartial rename, is_merged test, mcp/core changelogs
- Release: vcs-diff v0.4.0, vcs-cli-support v0.4.0, vcs-git v0.8.0, vcs-jj v0.8.0, vcs-github v0.8.0, vcs-gitlab v0.4.0, vcs-gitea v0.4.0, vcs-forge v0.4.0, vcs-testkit v0.4.0, vcs-core v0.6.0, vcs-watch v0.4.0, vcs-mcp v0.4.0


### Changed

- refactor!: interface-consistency renames (pr_mark_ready, Forge::from_* ctors, git fetch_branch)
- refactor(wave1.5b): Repo::remove_worktree takes a WorktreeRemove spec, not a bare force bool (A1)
- refactor(wave1.5b): Forge::pr_close takes a PrClose spec, not a bare delete_branch bool (A1)
- Release: vcs-diff v0.3.0, vcs-cli-support v0.3.0, vcs-git v0.7.0, vcs-jj v0.7.0, vcs-github v0.7.0, vcs-gitlab v0.3.0, vcs-gitea v0.3.0, vcs-forge v0.3.0, vcs-testkit v0.3.0, vcs-core v0.5.0, vcs-watch v0.3.0, vcs-mcp v0.3.0


### Fixed

- fix(forge+gitea+mcp): correct argv-safety docs for pr_comment body (per-backend)
- fix(wave0-followup): close cleanup_worktree_blocking repo-wipe + doc/register gaps
- fix(wave1): dead/degraded safety (H2/H3/H4/H6/H10/R1/R3)
- fix(wave2): gitea pr_view paginates past the server page cap; list caps documented (H8)


### Added

- feat(mcp): forge PR comment/edit + capability map + forge_info tool (#2)


### Changed

- deps: processkit 0.10.1 — testing-module imports, program-aware cassettes, cancellation core, Signalled/Timeout diagnostics
- meta: discoverability — sharpen descriptions/keywords/categories + README intro + GitHub topics
- build(deps): adopt processkit 0.11.0 (stats opt-in, OutputLine, cancel-race fix)
- review: write-gate repo_try_merge, forge Error classifier parity, forge_pr_mark_ready MCP tool
- Release: vcs-diff v0.2.0, vcs-cli-support v0.2.0, vcs-git v0.6.0, vcs-jj v0.6.0, vcs-github v0.6.0, vcs-gitlab v0.2.0, vcs-gitea v0.2.0, vcs-forge v0.2.0, vcs-testkit v0.2.0, vcs-core v0.4.0, vcs-watch v0.2.0, vcs-mcp v0.2.0


### Fixed

- fix(diff+mcp): drop empty-path diff sections; validate mcp --allow-tools names
- fix(git): current_branch handles unborn repos via symbolic-ref


### Added

- feat(mcp): vcs-mcp — MCP server over the facades (Wave F)
- feat(watch+ci+mcp): hermetic watch pipeline tests, requery timeout, stats, Stream; CI feature matrix; testable mcp args (Wave R)
- feat(api): facade push, forge issues+releases (+MCP tools), builder unification, MCP per-tool allowlist (Wave A)


### Changed

- Release: vcs-diff v0.1.0, vcs-cli-support v0.1.0, vcs-git v0.5.0, vcs-jj v0.5.0, vcs-github v0.5.0, vcs-gitlab v0.1.0, vcs-gitea v0.1.0, vcs-forge v0.1.0, vcs-testkit v0.1.0, vcs-core v0.3.0, vcs-watch v0.1.0, vcs-mcp v0.1.0


### Fixed

- fix: whole-solution review follow-ups — parser/config robustness, backend parity, watch worktrees, forge contracts

## [0.4.0] - 2026-07-03

### Added
-

### Changed
- Rebuilt against the `vcs-core` / `vcs-forge` spec reshapes: the `repo_create_worktree`
  and `forge_issue_create` handlers now build a `vcs_core::WorktreeCreate` /
  `vcs_forge::IssueCreate` and call the facades' new spec-taking signatures. **The MCP
  wire API is unchanged** — the JSON tool params (`{path, branch, base}`,
  `{title, body}`, `{number, delete_branch?}`, `{path, force?}`) are identical, so no MCP
  client is affected. (Transitive over `vcs-core` / `vcs-forge`; `docs/audit-2026-07.md`
  A5.)

### Fixed
-

## [0.3.0] - 2026-07-03

### Added
-

### Changed
- Bumped `processkit` to **1.1.0** (workspace floor now `"1"`, was `0.11.0`). `vcs-mcp`
  doesn't re-export `processkit` itself, but the bump is **breaking transitively** via
  the `vcs-core`/`vcs-forge` types it surfaces (their re-exported `processkit` is now
  1.x). No behaviour change here. processkit is semver-stable from 1.0, so future 1.x
  updates are non-breaking.
- **Docs:** the `forge_pr_list` / `forge_issue_list` / `forge_release_list` tool
  descriptions (a wire-visible contract an agent reads) now note that Gitea returns at
  most **~50** rows per its server page cap, not the "up to 100" of GitHub/GitLab.
  (`docs/audit-2026-07.md` H8.)

### Fixed
- **`repo_remove_worktree` inherits the `vcs-core` C1 safety fix.** Without `force`,
  removing a worktree with uncommitted changes is now refused (both backends), and the
  repository's main worktree/workspace is **always** refused — previously the jj path
  ignored `force` and could delete the main workspace, wiping the repo. The tool's
  `force` param doc (which wrongly said "git only") and description are corrected.
  (`docs/audit-2026-07.md` C1.)
- **`repo_checkout` no longer risks discarding unstaged edits** — the underlying git
  `checkout` now passes a trailing `--`, so a path-like reference errors instead of
  reverting that path from the index. (`docs/audit-2026-07.md` C2.)
- **The repo-mutating tools are serialized.** rmcp dispatches a task per request, so
  two concurrent mutations (e.g. `repo_try_merge`'s materialize-then-rollback racing
  `repo_commit`) could interleave and lose one's work. A per-server write mutex now
  runs the `repo_*` mutating tools one at a time. (`docs/audit-2026-07.md` R1.)

## [0.2.0] - 2026-06-27

### Added
- **Read tool** `forge_info` (always available, `readOnlyHint`): the forge
  identity + flat capability map. Returns
  `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_checks, pr_merge,
  issue_create, authed } }` where `kind` is `"github"` / `"gitlab"` /
  `"gitea"` and the per-op flags are the intersection of "the CLI ships
  the command" and "the CLI is authenticated" (a single `auth status` /
  `login list` probe is spawned; the rest is a static table). Errors with
  `invalid_params` ("no forge is configured for this repository …") when
  no forge is bound to the server, matching the other `forge_*` tools.
- **Mutating tools** (gated, `destructiveHint`):
  - `forge_pr_mark_ready({ number })` — mark a draft PR/MR ready for review
    (`Unsupported` on Gitea). Closes a parity gap: the `Forge` facade has
    `pr_mark_ready`, but no MCP tool surfaced it, so a draft→ready workflow wasn't
    drivable over MCP.
  - `forge_pr_comment({ number, body })` — post a markdown comment to an
    existing PR/MR; returns the CLI output (the comment URL on success).
  - `forge_pr_edit({ number, title?, body? })` — edit a PR/MR's title
    and/or body. At least one of `title` or `body` must be set; both
    absent is rejected up front as `invalid_params` (the facade's
    `Error::InvalidInput` mapped to an MCP `invalid_params` error). An
    empty string is a real value (clears the field) — it passes the
    belt-and-braces argv guard at the MCP seam and the wrapper's
    flag-VALUE-position pass-through.
- **Param structs**: `PrCommentParams`, `PrEditParams` (each
  `Deserialize` + `JsonSchema` — their schema is the tool's advertised
  input schema). `PrEditParams` is `Option`-typed on `title`/`body` so
  the JSON form can omit either (or both) without serde complaining.
- **Error mapping**: `vcs_forge::Error::InvalidInput` (a new variant on
  the facade's error, used by the both-`None` rejection on `pr_edit`) is
  mapped to MCP `invalid_params` alongside the existing
  `Error::Unsupported` mapping — both are client-fixable errors.
- **Pre-spawn argv guard** in the MCP layer (`guard_argv_field`): mirrors
  the wrappers' `reject_flag_like` for the `body` / `title` fields of
  the two new mutating tools. A leading-`-` is refused up front; an
  empty string is allowed (it clears the field). The wrappers still run
  their own guards — this is the second line of defence at the MCP seam.

### Changed
- **`repo_try_merge` is now write-gated (breaking).** It was a read tool
  (`readOnlyHint`), but it spawns a *real* trial merge that materializes working-tree
  content — which on an untrusted repository can run repo-local `filter`/`textconv`
  drivers the hardened client does not sandbox, the same code-execution class as
  `repo_checkout` (already gated). It now requires `--allow-write` (or
  `--allow-tools repo_try_merge`) and is in `WRITE_TOOLS`; its annotation is
  corrected to non-destructive/idempotent (it still rolls back, leaving no net
  trace). The default read-only mode therefore no longer exposes any working-tree-
  materializing operation; the MCP docs note the residual `textconv`-on-diff vector
  for fully untrusted repos.
- **Tool JSON output reflects the updated `vcs-core`/`vcs-forge` DTOs (breaking for
  wire consumers).** `repo_snapshot` now nests upstream tracking under one
  `tracking` object (`{branch, ahead, behind}` or `null`) instead of three flat
  `upstream`/`ahead`/`behind` fields; release results carry `body`/`draft`/
  `prerelease`; issue results carry `body`/`url`; PR check `bucket` is the typed
  `CheckBucket` value.
- Bumped `processkit` to **0.11.0**. Test doubles moved to `processkit::testing`;
  cancellation is now core (no feature flag).

### Fixed
- **`--allow-tools` validates tool names up front.** An unknown/misspelled name is
  now rejected with an error listing the valid write tools, instead of being added
  to a silently-inert allowlist (a typo never matched a real tool, so the intended
  write stayed disabled with no warning). The canonical set is the new public
  `vcs_mcp::WRITE_TOOLS`; `require_write` debug-asserts every gated tool is listed
  there, so the two can't drift.

## [0.1.0] - 2026-06-08

### Added
- Initial release: `vcs-mcp`, a Model Context Protocol (MCP) server exposing the
  `vcs-core` (`Repo`) and `vcs-forge` (`Forge`) operations as agent-callable
  tools. A lib (`VcsMcpServer`, hermetically testable) plus the `vcs-mcp` binary,
  which serves MCP over **stdio** for an `mcpServers` config entry. The workspace's
  **first binary crate** and **second runtime-tokio** crate (after `vcs-watch`).
- **Read tools** (always available, annotated `readOnlyHint`): `repo_snapshot`,
  `repo_info`, `repo_status`, `repo_diff_stat`, `repo_branches`,
  `repo_current_branch`, `repo_conflicts`, `repo_worktrees`, `repo_try_merge`
  (a rollback merge probe); forge: `forge_auth_status`, `forge_repo_view`,
  `forge_pr_list`, `forge_pr_view`, `forge_pr_checks`, `forge_issue_list`,
  `forge_issue_view`, `forge_release_list`, `forge_release_view`. Each returns
  the facade DTO as JSON (via the facades' optional `serde` feature).
- **Mutating tools** (gated, annotated `destructiveHint`): `repo_commit`,
  `repo_checkout`, `repo_fetch`, `repo_push`, `repo_create_worktree`,
  `repo_remove_worktree`; forge: `forge_pr_create`, `forge_pr_merge`,
  `forge_pr_close`, `forge_issue_create`. Outside the write gate they reject up
  front — naming the tool — before spawning anything.
- **`WriteGate`** — the server's write policy (`None` / `All` /
  `Set(HashSet<tool name>)`), checked by every mutating tool under its own name.
  `VcsMcpServer::new` takes it in place of a coarse bool.
- **CLI:** `--repo <path>` (default cwd), `--forge github|gitlab|gitea` (override),
  `--allow-write` (every mutation), `--allow-tools <name,…>` (a per-tool
  allowlist; comma-separated, repeatable, accumulates; `--allow-write` wins when
  both are given), `--timeout <seconds>` (per-command deadline, default 120; `0`
  disables), `--help`. With neither write flag the server is read-only. The
  forge is auto-detected from the `origin` remote (`ForgeKind::from_remote_url`)
  — works on a colocated jj repo; a pure-jj repo with no git remote has no
  forge, and the `forge_*` tools then return a clear "no forge configured"
  error.
- **Hardened by default:** the binary opens the repo with a hardened git client
  (`Git::hardened()` — repo hooks and `core.fsmonitor` disabled, repo-redirecting
  `GIT_*` scrubbed, system config skipped), so serving a repository you didn't
  create can't execute its hooks even on a read tool. jj has no repo-local hooks.
  Every git/forge command also runs under the `--timeout` deadline so a stalled
  network call can't hang a request. The server advertises its identity as
  `vcs-mcp` (with the crate version) over the MCP wire.
- The tool logic, write-gating, serialization, and the `#[tool_router]`/
  `#[tool_handler]` wiring are covered hermetically (a `ScriptedRunner`-backed
  `Repo`, plus an in-process rmcp client round-trip over an in-memory duplex
  transport); `#[ignore]` tests drive the read tools and a gated mutation against a
  real temporary git repo.

### Notes
- Built on [`rmcp`](https://crates.io/crates/rmcp) (the official MCP Rust SDK).
  Read-only by default. The wrappers' argv injection guards apply under every
  tool.

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.6.0...HEAD
[0.6.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.5.2...vcs-mcp-v0.6.0
[0.5.2]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.5.1...vcs-mcp-v0.5.2
[0.5.1]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.5.0...vcs-mcp-v0.5.1
[0.5.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.4.0...vcs-mcp-v0.5.0
[0.4.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.3.0...vcs-mcp-v0.4.0
[0.3.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.2.0...vcs-mcp-v0.3.0
[0.2.0]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-mcp-v0.1.0...vcs-mcp-v0.2.0
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-mcp-v0.1.0
