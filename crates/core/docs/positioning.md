# When to use vcs-toolkit (vs `gitoxide` / `git2`)

vcs-toolkit and the in-process git libraries solve different problems, and the
choice is usually clear once you name it. vcs-toolkit **shells out to the
installed `git` / `jj` / `gh` binaries** and parses their output — every command
async, run inside an OS job (via [`processkit`](https://crates.io/crates/processkit))
so the process tree dies with the parent. [`gitoxide`](https://crates.io/crates/gix)
(the `gix` crate) and [`git2`](https://crates.io/crates/git2) (bindings to the C
`libgit2`) are **in-process git object-database libraries** — they read and write
git's on-disk format directly, no subprocess, no `git` binary in sight.

That one architectural fact drives everything below. This page spells out the
trade-off so you can pick deliberately rather than by reflex.

## The core trade-off

vcs-toolkit gives you the **installed binary's exact behaviour, configuration, and
credentials** — the same `git` / `jj` / `gh` the user runs from their shell. A
command honours their `~/.gitconfig`, their credential helpers, their SSH agent,
their hooks, their aliases, their commit-signing setup, their `gh auth` session,
and — for jj — Jujutsu's entire model, which no library reimplements. You are not
approximating the user's git; you *are* their git. When your automation pushes,
the same credential helper fires; when it commits, the same signing key signs;
when it rebases, the same `merge.conflictStyle` shapes the markers you then parse.

When ambient auth isn't what you want — a CI job with a short-lived token, an agent
acting for several accounts — `Git`/`GitHub`/`GitLab` take an opt-in
`CredentialProvider` (`with_credentials(...)`) layered *on top of* this model: the
per-operation secret is injected without touching the user's stored credentials, and
the default path stays exactly as above.

The cost is real and worth stating plainly:

- **A process spawn per command.** Each call forks a binary, pipes its output,
  and parses it. That's microseconds of git-internal work wrapped in
  milliseconds of process overhead — mitigated by the OS-job containment (so the
  spawn is cheap to *contain*, not cheap to *make*) and by batched reads like
  `Repo::snapshot()` that fold several queries into one pass, but never zero.
- **A dependency on the binaries being installed.** No `git` / `jj` / `gh` on
  `PATH`, no vcs-toolkit. (Unit tests dodge this via the scripted runner, but
  production needs the real tools.)

gitoxide and git2 invert both: in-process speed (no spawn, no pipe, no parse) and
no binary dependency — but they reimplement git's semantics in their own code, and
they cover *only git*. Neither knows anything about jj or GitHub.

## Use vcs-toolkit when

- You need **byte-identical behaviour to the user's CLI** — the same output their
  `git status` / `jj log` / `gh pr view` would print, because you're running it.
- You must **honour real credentials** — credential helpers, SSH keys, `gh auth`
  tokens, OS keychains — without reimplementing auth. The binary already knows how.
- You need **jj** (Jujutsu) or **GitHub** (`gh`). Neither gitoxide nor git2
  touches these. jj has its own object model, operation log, and conflict
  representation; `gh` is GitHub's REST/GraphQL surface. vcs-toolkit wraps both.
- You're **automating a workflow** — PR lifecycle, rebase/merge orchestration,
  fetch-then-rebase-then-push handshakes, programmatic conflict resolution —
  rather than reading objects at scale. You want the *commands*, not the *database*.
- You want a **thin, auditable layer**. The wrapper is, almost literally, the
  command you'd type; what runs is inspectable (argv recording, the `tracing`
  feature) and easy to reason about. There's no second git implementation to trust.
- **Subprocess containment matters.** A crashing or `Ctrl-C`'d parent must not
  leave an orphaned `git gc` or a hung `gh` — the OS job reaps the whole tree.

## Use gitoxide / git2 when

- You need to **read or traverse the object database at high throughput** — pack
  access, blame over a huge history, walking millions of commits — where a
  per-operation process spawn would dominate the cost. In-process wins decisively.
- You must **run where `git` isn't installed** — an embedded tool, a sandboxed or
  statically-linked binary, a container with no git. The library *is* the git.
- You want **fine-grained in-process control** over refs, objects, the index, or
  trees — building exactly the commit you want from parts, rather than scripting
  the porcelain that would assemble it.
- You're **building a git implementation or server** — a forge backend, a custom
  transport, an alternative client — not automating a human's everyday workflow.

## Comparison

vcs-toolkit covers jj via `vcs-jj` and the three forges via `vcs-github` /
`vcs-gitlab` / `vcs-gitea` (unified by `vcs-forge`); gitoxide and git2 cover git
only. Per-operation cost for vcs-toolkit is a (contained, sometimes batched) process
spawn; the libraries are in-process. At a glance:

| | vcs-toolkit | gitoxide (`gix`) | git2 (`libgit2`) |
|---|---|---|---|
| Model | subprocess to the CLIs | in-process, pure Rust | in-process, C bindings |
| Honours user config / credentials / hooks | yes — it *is* the user's binary | no | no |
| Covers jj (Jujutsu) | yes | no | no |
| Covers forges (GitHub/GitLab/Gitea) | yes | no | no |
| Per-operation cost | a process spawn | none — in-process | none — in-process |
| Binary dependency | needs the CLIs on `PATH` | none | none (links `libgit2`) |
| Fidelity to the CLI | exact — same binary & version | reimplementation | reimplementation |
| Maturity | young, consumer-driven | fast, maturing | mature C core |

Read it as: the libraries trade the user's *exact* git for speed and
independence; vcs-toolkit trades speed and independence for the user's exact git —
plus jj and GitHub, which the libraries don't reach at all.

## Not either/or

These compose cleanly, and a serious tool often uses both. Reach for
gitoxide or git2 on the **hot path** — high-volume object reads, history
traversal, blame — where avoiding a spawn per operation is the whole point.
Reach for vcs-toolkit for **workflow automation** — driving a rebase, opening and
merging a PR, resolving conflicts, syncing with a remote — and for **anything jj
or GitHub**, which the libraries simply don't cover. One reads the database fast;
the other drives the tools the user already trusts. Using both isn't a compromise;
it's the right factoring.

## See also

- [Process model, errors & observability](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — the OS-job
  containment, the spawn-and-parse model, and the structured `Error` behind every
  command vcs-toolkit runs.
- [README](https://github.com/ZelAnton/vcs-toolkit-rs#readme) — the overview, the crate table, and the quick start.
