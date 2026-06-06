# vcs-toolkit-rs documentation

The full guide set for [vcs-toolkit-rs](../README.md) — a Rust toolkit that
automates **Git**, **Jujutsu**, and **GitHub** by shelling out to the official
`git` / `jj` / `gh` binaries and capturing their output. Every command is async
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
| [vcs-git](git.md) | `vcs-git` | the `git` binary — status, commits, branches, worktrees, diff, blame, merge/rebase, remotes, tags |
| [vcs-jj](jj.md) | `vcs-jj` | the `jj` (Jujutsu) binary — changes, bookmarks, the operation log, workspaces, squash/split/absorb, git sync |
| [vcs-github](github.md) | `vcs-github` | the `gh` CLI — pull requests, issues, Actions runs, releases, reviews |
| [vcs-core](core.md) | `vcs-core` | a backend-agnostic facade that detects git-vs-jj and dispatches the operations both share |
| [vcs-testkit](testkit.md) | `vcs-testkit` | throwaway git/jj sandboxes and a bare remote for integration tests |

## Cross-cutting topics

These apply across the wrapper crates:

- **[Conflict resolution](conflicts.md)** — the typed conflict-marker models in
  `vcs_git::conflict` and `vcs_jj::conflict`: parse marker soup into structured
  regions, re-render byte-exact, and resolve to a chosen side.
- **[Testing & mocking](testing.md)** — the three test seams (depend on the
  trait, the `mock` feature, inject a `ScriptedRunner`/`RecordingRunner`), the
  dry-run harness, and real-binary integration tests with `vcs-testkit`.
- **[Security & hardening](security.md)** — the automatic injection guards, the
  `RefName` / `RevSpec` / `RevsetExpr` validating newtypes, and `Git::hardened()`
  for running against repositories you didn't create.
- **[Process model, errors & observability](process-model.md)** — OS-job
  containment and the platform table, per-client timeouts, the
  `processkit::Error` variants and how to branch on them structurally, and the
  four observability seams (argv recording, streaming, the `tracing` feature,
  the dry-run harness).

## How the guides relate

```
                          README.md  (overview, quick start)
                              │
                       docs/README.md  (you are here)
                              │
        ┌──────────┬──────────┼──────────┬───────────┐
     git.md     jj.md     github.md    core.md     testkit.md
        │          │                      │            │
        └────┬─────┴──────────┬───────────┴─────┬──────┘
        conflicts.md     security.md       testing.md
                         process-model.md
```

`core.md` sits over `git.md` / `jj.md` (it dispatches to them); the cross-cutting
guides are referenced from every per-crate guide's *See also* footer.

## Reference

- Per-crate API docs (rustdoc): build locally with `cargo doc --no-deps --open`.
- Per-crate changelogs: `crates/<crate>/CHANGELOG.md`.
- Project roadmap: [ROADMAP.md](../ROADMAP.md).
- Contributing / build conventions: [AGENTS.md](../AGENTS.md).
