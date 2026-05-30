# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project

`vcs-toolkit-rs` is a Rust toolkit for automating **Git**, **Jujutsu**, and
**GitHub** through CLI process execution — the crates shell out to the official
`git`, `jj`, and `gh` binaries and capture their output rather than
reimplementing each tool's protocol.

It is a Cargo workspace of four **independently versioned and published**
library crates:

| Path | crates.io name | Drives |
|---|---|---|
| `crates/process` | `vcs-process` | the shared job-backed process launcher |
| `crates/git` | `vcs-git` | `git` |
| `crates/jj` | `vcs-jj` | `jj` |
| `crates/github` | `vcs-github` | `gh` (GitHub CLI) |

The three wrappers expose the same shape: a `run<I, S>(args)` helper that
executes the underlying binary and returns trimmed stdout (or an `io::Error`
carrying stderr on failure), plus typed wrappers like `version()`. Keep that
shape consistent across crates when adding command wrappers.

`vcs-process` is the one shared crate the wrappers depend on. It launches every
child inside an OS **job** — a Windows [Job Object] with
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, or a Linux [cgroup v2] killed via
`cgroup.kill` (falling back to a POSIX process group when no writable cgroup is
available) — so the whole process tree dies with the parent. The platform FFI
lives only here (`crates/process/src/{windows,linux,other}.rs`); the wrappers
just call `vcs_process::run`. v1 guarantees kill-on-close; resource limits are a
future extension, observable via `Job::mechanism()`.

[Job Object]: https://learn.microsoft.com/windows/win32/procthread/job-objects
[cgroup v2]: https://docs.kernel.org/admin-guide/cgroup-v2.html

## Build, test, run

```bash
cargo build                         # build all crates
cargo test                          # all unit + integration tests (workspace)
cargo test -p vcs-git               # scope to one crate
cargo test <name>                   # tests matching a substring
cargo test -- --ignored             # tests needing the real git/jj/gh binaries
cargo clippy --all-targets -- -D warnings   # lint (CI treats warnings as errors)
cargo fmt --all --check             # format check (CI gate)
```

Tests that invoke the real `git` / `jj` / `gh` binaries are marked `#[ignore]`
so `cargo test` stays hermetic on CI; run them locally with `--ignored`.
Integration tests (if added) live in each crate's `tests/` dir — each file is
compiled as its own crate; prefer shared helpers in `tests/common/mod.rs`.

## Code style

- **Comment the *why*, not the *what*.** The code already says what it does;
  comments explain the non-obvious reason — a workaround, a wire contract, a
  performance trade-off. Don't narrate obvious lines.
- **Match the surrounding code.** Follow the existing module's naming, idioms,
  error-handling style, and comment density. Keep the three crates parallel:
  new wrappers should look the same in `vcs-git`, `vcs-jj`, and `vcs-github`.
- **Reuse before you add.** Search for an existing helper before writing a new
  one; avoid duplicating logic.
- **Conventional-commit subjects.** Write commit subjects as
  `type(scope): summary` — `feat`, `fix`, `refactor`, `perf`, `docs`, `test`,
  `chore`, `ci`, etc. Use the crate as the scope where it helps
  (`feat(git): ...`). These feed the changelog; see "Releasing".
- **Keep it formatted and lint-clean.** Run `cargo fmt` and
  `cargo clippy --all-targets` before considering work done.

## Dependency management

This workspace fixes **no** allow-list of crates — add whatever a crate
genuinely needs. The three wrappers stay lean: their only dependency is
`vcs-process` (which itself pulls in just `windows-sys` on Windows and `libc` on
Linux for the job FFI). Don't add more to a wrapper unless there's a real
reason. The convention is about *how* you add dependencies, not *which*:

- **Document every dependency.** Each entry in `Cargo.toml` gets an inline
  comment explaining *why* it's there. A future reader should never guess.
- **Pin major versions** (`"1"`, `"0.22"`) and enable only the features used.
- **Shared deps** go in the root `[workspace.dependencies]` and are referenced
  from a member with `<crate>.workspace = true`.
- **Commit `Cargo.lock`.** Reproducible builds — it's tracked, not ignored.
- **Platform-specific deps** go under a cfg target table, e.g.
  `[target.'cfg(windows)'.dependencies]`, with the same "why" comment.

## Local-only files

`.gitignore` carves out `*.local.md`, `task_plan.md`, `findings.md`,
`progress.md` — use those names freely for scratch notes; they won't be
committed.

## Releasing and the changelog

Each crate releases **independently** — they do not share a version.

- **The crate's `Cargo.toml` `version` is the single source of truth.** Bump it
  with the release; never let the manifest, the tag, and the published artifact
  drift apart.
- **Each crate has its own `CHANGELOG.md`** following
  [Keep a Changelog](https://keepachangelog.com/) +
  [Semantic Versioning](https://semver.org/). Curate the `[Unreleased]` section
  as you work — manual bullets always win over the `git-cliff` (`cliff.toml`)
  auto-fill, which buckets by commit-subject prefix
  (`feat`→Added, `fix`→Fixed, `remove`→Removed, `perf`/`refactor`/`ci`/…→Changed,
  `docs`/`chore`/`test`→skipped).
- **Tag per crate** as `<crate>-v<version>` (e.g. `vcs-git-v0.2.0`) so each
  crate's history and compare links stay independent, then
  `cargo publish -p <crate>`.
- **Publish order.** `vcs-process` must be on crates.io *before* the wrappers —
  `vcs-git`/`vcs-jj`/`vcs-github` depend on it by version, so a wrapper publish
  fails until the matching `vcs-process` version is live. Release `vcs-process`
  first whenever its version changed.
- **Release workflow.** `.github/workflows/release.yml` (`workflow_dispatch`)
  bumps a chosen crate, promotes its `[Unreleased]` heading to the new version +
  date (preserving the curated bullets), tags, and runs `cargo publish` (needs
  the `CRATES_IO_TOKEN` secret). CI hard-gates on `cargo package -p vcs-process`;
  the wrappers can only be package-checked once `vcs-process` is on crates.io, so
  CI just emits a notice for them until then. The final publish stays a
  deliberate, human-triggered action.

## Version control workflow

This repo uses [jujutsu (`jj`)](https://jj-vcs.github.io/jj/) colocated with
git. Use `jj` commands; the canonical workflow:

- **Per-prompt evaluation (mandatory).** Before any edits, run `jj st` and
  classify the incoming prompt against the current change description:

	| Signal in prompt | Category | Action |
	|---|---|---|
	| Same topic, refinement, follow-up of in-progress work | **Continuation** | Just work. jj auto-folds edits into the current change. |
	| Same change but goal has been refined or expanded | **Scope shift** | `jj describe -m "<refined summary>"`. **Don't** start a new change. |
	| Orthogonal topic, different area, "теперь сделай X" | **New work** | If current change is finished → `jj new -m "<summary>"` (descendant). If still in progress → `jj new @- -m "..."` (parallel sibling). |

	Reliable signals: word changes like "теперь" / "now" / "next" / "также сделай" / "and also" usually mean **new work** or **scope shift**. Imperative follow-ups inside the same scope ("исправь это", "fix this", "продолжи") mean **continuation**. When in doubt, ask the user.

	A `UserPromptSubmit` hook (`.claude/hooks/jj-prompt-reminder.sh`) injects this same checklist into context each turn — the hook is the reminder, this table is the rulebook.

- **Describe early.** When starting a new piece of work, immediately set the change description:
	```
	jj describe -m "Concise summary"
	```
	The description should reflect intent *before* the work — not be backfilled at commit time. Keep extending the same `jj` change for follow-ups; don't spawn one per edit.
- **Sync on the user's trigger.** When the user says `pull` (or `push`/`sync`), run the full handshake:
	1. `jj git fetch` first — picks up any remote movement.
	2. Rebase if `main@origin` advanced: `jj rebase -r @- -d main@origin`.
	3. `jj bookmark set main -r <rev>` then `jj git push --bookmark main`.

	Never push without an explicit signal from the user.
- **Undoing dropped work.** When the user decides to abandon something already done, reach for `jj`'s safety net rather than hand-cleanup:
	- `jj undo` (alias of `jj op undo`) reverses the last operation — describe, edit, squash, rebase, abandon, push, all of it. Repeatable.
	- `jj abandon <rev>` drops a specific change entirely; descendants auto-rebase.
	- `jj restore` discards working-copy edits back to the parent's tree.
	- `jj op log` is the full reflog if you need to go further back via `jj op restore <op-id>`.
- **No new bookmarks** unless the user explicitly asks. Work lives on `main`; that is the publish target.

## Windows / line endings

The working tree may carry CRLF line endings on Windows despite `.gitattributes`
mandating LF — that's stat-cache state from a pre-attributes checkout, not actual
file divergence. The committed blobs are LF; pushed commits are clean. Colocated
`jj st` may show phantom modifications for files that haven't been re-extracted
since `.gitattributes` was added. `.gitattributes` (`* text=auto eol=lf`) is what
keeps git and jj agreeing on the working copy.
