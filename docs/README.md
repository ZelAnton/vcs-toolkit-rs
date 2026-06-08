# vcs-toolkit-rs documentation

The full guide set for [vcs-toolkit-rs](../README.md) — a Rust toolkit that
automates **Git**, **Jujutsu**, **GitHub**, **GitLab**, and **Gitea** by shelling
out to the official `git` / `jj` / `gh` / `glab` / `tea` binaries and capturing
their output. Every command is async
(tokio), runs inside an OS **job** (so the process tree dies with the parent via
[`processkit`](https://crates.io/crates/processkit)), and fails with a
structured `processkit::Error`.

New here? Start with the root [README](../README.md) for the overview and the
quick start, then come back for depth.

## Per-crate guides

Each crate is versioned and published independently. The guides document every
public command grouped by theme, the parsed result types, the builder/config
types, and the validating newtypes — with worked examples throughout.

| Guide | Crate | Drives |
|---|---|---|
| [vcs-git](../crates/git/docs/git.md) | `vcs-git` | the `git` binary — status, commits, branches, worktrees, diff, blame, merge/rebase, remotes, tags |
| [vcs-jj](../crates/jj/docs/jj.md) | `vcs-jj` | the `jj` (Jujutsu) binary — changes, bookmarks, the operation log, workspaces, squash/split/absorb, git sync |
| [vcs-github](../crates/github/docs/github.md) | `vcs-github` | the `gh` CLI — pull requests, issues, Actions runs, releases, reviews |
| [vcs-gitlab](../crates/gitlab/docs/gitlab.md) | `vcs-gitlab` | the `glab` CLI — the lean merge-request lifecycle (list/view/create/merge/ready/close) + pipeline status |
| [vcs-gitea](../crates/gitea/docs/gitea.md) | `vcs-gitea` | the `tea` CLI — the lean pull-request lifecycle (list/view/create/merge/close) |
| [vcs-forge](../crates/forge/docs/forge.md) | `vcs-forge` | a forge-agnostic facade over GitHub/GitLab/Gitea — one PR/MR lifecycle across all three |
| [vcs-core](../crates/core/docs/core.md) | `vcs-core` | a backend-agnostic facade that detects git-vs-jj and dispatches the operations both share |
| [vcs-watch](../crates/watch/docs/watch.md) | `vcs-watch` | filesystem-watch a repo and stream typed state-change events (built on `vcs-core`) |
| [vcs-mcp](../crates/mcp/docs/mcp.md) | `vcs-mcp` | a Model Context Protocol server exposing the `vcs-core`/`vcs-forge` operations as agent-callable tools |
| [vcs-testkit](../crates/testkit/docs/testkit.md) | `vcs-testkit` | throwaway git/jj sandboxes and a bare remote for integration tests |

Two **foundational crates** sit below the wrappers (no guide of their own — their
types are re-exported by the wrappers, so you rarely name them directly):
`vcs-diff` (the std-only git-format diff model + parser and the `Version` type —
`git diff` and `jj diff --git` are byte-identical) and `vcs-cli-support` (the
`processkit`-coupled plumbing: the argv injection guard, fetch-retry policy, and
the error classifiers).

## Cross-cutting topics

These apply across the wrapper crates:

- **[Conflict resolution](../crates/git/docs/conflicts.md)** — the typed conflict-marker models in
  `vcs_git::conflict` and `vcs_jj::conflict`: parse marker soup into structured
  regions, re-render byte-exact, and resolve to a chosen side.
- **[Testing & mocking](../crates/testkit/docs/testing.md)** — the three test seams (depend on the
  trait, the `mock` feature, inject a `ScriptedRunner`/`RecordingRunner`), the
  dry-run harness, and real-binary integration tests with `vcs-testkit`.
- **[Security & hardening](../crates/git/docs/security.md)** — the automatic injection guards, the
  `RefName` / `RevSpec` / `RevsetExpr` validating newtypes, and `Git::hardened()`
  for running against repositories you didn't create.
- **[Process model, errors & observability](../crates/core/docs/process-model.md)** — OS-job
  containment and the platform table, per-client timeouts, the
  `processkit::Error` variants and how to branch on them structurally, and the
  four observability seams (argv recording, streaming, the `tracing` feature,
  the dry-run harness).
- **[Cookbook](../crates/core/docs/cookbook.md)** — task-oriented end-to-end recipes (a prompt line
  in one call, open-a-PR-and-watch-CI, cancel a long watch/fetch, stash-safe
  switch, programmatic conflict resolution, backend dispatch, jj transaction).
- **[When to use this vs `gitoxide`/`git2`](../crates/core/docs/positioning.md)** — the
  subprocess-vs-in-process trade-off and an honest comparison table.
- **[Stability, versioning & path to 1.0](../crates/core/docs/stability.md)** — per-crate stability
  tiers, the SemVer + MSRV policy, and the public-API review gate.

## How the guides relate

```
                          README.md  (overview, quick start)
                              │
                       docs/README.md  (you are here)
                              │
   ┌─────────┬─────────┬──────┴───┬─────────┬─────────┬──────────┐
 git.md    jj.md   github.md  gitlab.md  gitea.md  core.md   testkit.md
   │         │     └────┬─────────┴────┬────┘      │   │
   │         │       forge.md (over the three forges)  └─ watch.md (over core)
   │         │          │   └──────────────┬───────────┘
   │         │          └─ mcp.md (the MCP server, over core + forge)
   └────┬────┴───────────┬─────────────┬──────────────┘
   conflicts.md     security.md    testing.md
                    process-model.md
```

`core.md` sits over `git.md` / `jj.md`, `forge.md` over `github.md` /
`gitlab.md` / `gitea.md` (each facade dispatches to them), `watch.md` builds
on `core.md` (it re-queries `Repo::snapshot`), and `mcp.md` builds on **both**
facades (it exposes their operations as MCP tools); the cross-cutting guides are
referenced from every per-crate guide's *See also* footer.

## Reference

- Per-crate API docs (rustdoc): build locally with `cargo doc --no-deps --open`.
- Per-crate changelogs: `crates/<crate>/CHANGELOG.md`.
- Project roadmap: [ROADMAP.md](../ROADMAP.md).
- Contributing / build conventions: [AGENTS.md](../AGENTS.md).
