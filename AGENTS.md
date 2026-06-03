# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## Project

`vcs-toolkit-rs` is a Rust toolkit for automating **Git**, **Jujutsu**, and
**GitHub** through CLI process execution — the crates shell out to the official
`git`, `jj`, and `gh` binaries and capture their output rather than
reimplementing each tool's protocol.

It is a Cargo workspace of **independently versioned and published** crates: three
CLI wrappers, all built on the external
[`processkit`](https://crates.io/crates/processkit) crate (the job-backed process
launcher + `CliClient` core; was the prototype internal `vcs-process` crate), plus
one facade:

| Path | crates.io name | Drives |
|---|---|---|
| `crates/git` | `vcs-git` | `git` |
| `crates/jj` | `vcs-jj` | `jj` |
| `crates/github` | `vcs-github` | `gh` (GitHub CLI) |
| `crates/core` | `vcs-core` | — (facade over `vcs-git`/`vcs-jj`) |

Each **CLI wrapper** (`vcs-git`/`vcs-jj`/`vcs-github`) exposes the same shape — an **interface trait**
(`GitApi`/`JjApi`/`GitHubApi`) and a real client struct
(`Git`/`Jj`/`GitHub`) generic over a `processkit::ProcessRunner`. Methods are
**`async`** (tokio, via `#[async_trait]`), take `dir: &Path`, return parsed structs,
and fail with the structured `processkit::Error`; pure parsers live in each crate's
`parse.rs`. Each client wraps a single `core: processkit::CliClient<R>` field that
owns the binary name, runner, and optional `default_timeout` and provides the
`command`/`command_in` builders and the `text`/`capture`/`unit`/`code`/`parse`/
`try_parse` terminals — so a method is one line and a new wrapper is just a
`const BINARY`, the `processkit::cli_client!(pub struct X => BINARY)` macro (which
emits the `core` field, `new`/`Default`/`with_runner`/`default_timeout`), its
object-safe `*Api` trait, and its typed methods. The generic, ergonomic argument
types stay on `CliClient`, never on the trait. Keep this shape consistent across
crates and **keep the traits object-safe and `mockall`-friendly**
— no generic methods, no nested-reference lifetimes (use `&[PathBuf]`/`&[String]`,
not `&[&Path]`/`&[&str]`; use `Option<String>`, not `Option<&str>`) so `&dyn Api`,
`async-trait`, and `mockall` all work.

**Mockability is a first-class requirement.** Consumers depend on the trait and,
in tests, either enable the `mock` feature for a `mockall`-generated mock
(`MockGitApi`) or call `Git::with_runner(processkit::ScriptedRunner::new()…)` to
drive the real argument-building and parsing against canned `Reply`s. To assert the
*exact* built command (full args, cwd, env — and that a flag is absent), wrap any
runner in `processkit::RecordingRunner` and inspect the captured `Invocation`s;
`ProcessRunner` is implemented for `&R`, so pass `&rec` and keep the recorder. New
commands must keep these seams working (add a trait method + a hermetic
`ScriptedRunner`/`RecordingRunner` test).

[`processkit`](https://crates.io/crates/processkit) is the external crate the
wrappers build on. It launches every child inside an OS **job** — a Windows
[Job Object] with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, or a Linux [cgroup v2]
killed via `cgroup.kill` (falling back to a POSIX process group when no writable
cgroup is available) — so the whole process tree dies with the parent. Its
`ProcessRunner` trait is the execution seam: `JobRunner` is the real one,
`ScriptedRunner`/`RecordingRunner` the test doubles. processkit guarantees
kill-on-close (observable via its `Mechanism`) and surfaces timeouts as a distinct
`Error::Timeout` (0.3+). **Do not vendor or fork processkit here** — if the
wrappers need a change in it, raise it as a requirement against the ProcessKit
project rather than working around it in a wrapper.

[Job Object]: https://learn.microsoft.com/windows/win32/procthread/job-objects
[cgroup v2]: https://docs.kernel.org/admin-guide/cgroup-v2.html

## Build, test, run

```bash
cargo build                         # build all crates
cargo test                          # all unit + integration tests (workspace)
cargo test -p vcs-git               # scope to one crate
cargo test <name>                   # tests matching a substring
cargo test -- --ignored             # tests needing the real git/jj/gh binaries
cargo test --workspace --features mock      # exercise the mockall mocks + ScriptedRunner
cargo clippy --all-targets -- -D warnings   # lint (CI treats warnings as errors)
cargo clippy --workspace --all-targets --features mock -- -D warnings
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
  error-handling style, and comment density. Keep the three wrapper crates
  parallel: new wrappers should look the same in `vcs-git`, `vcs-jj`, and `vcs-github`.
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
genuinely needs. The wrappers stay lean: `vcs-git` and `vcs-jj` depend on
`processkit` + `async-trait`; `vcs-github` additionally adds `serde`/`serde_json`
to deserialize `gh … --json`. `processkit` (external) brings the job FFI, the
`tokio` runtime, and the structured `Error`, so the wrappers don't depend on
`tokio` directly except `tokio` (`macros`, `rt-multi-thread`) as a `dev-dependency`
for `#[tokio::test]`. Don't add more to a wrapper unless there's a real reason. The
convention is about *how* you add dependencies, not *which*:

- **Document every dependency.** Each entry in `Cargo.toml` gets an inline
  comment explaining *why* it's there. A future reader should never guess.
- **Pin major versions** (`"1"`, `"0.22"`) and enable only the features used.
- **Shared deps** go in the root `[workspace.dependencies]` and are referenced
  from a member with `<crate>.workspace = true`.
- **Commit `Cargo.lock`.** Reproducible builds — it's tracked, not ignored.
- **Platform-specific deps** go under a cfg target table, e.g.
  `[target.'cfg(windows)'.dependencies]`, with the same "why" comment.
- **Test-only deps go behind a feature.** `mockall` is an `optional`
  dependency enabled only by each crate's `mock` feature
  (`mock = ["dep:mockall", …]`) so it never compiles into production. A
  *consumer* enables `mock` in `[dev-dependencies]` only — because Cargo unifies
  features, listing `vcs-git` as both a normal dep and a `mock`-enabled dep in
  one build would drag `mockall` into the release binary.

## Local-only files

`.gitignore` carves out `*.local.md`, `task_plan.md`, `findings.md`,
`progress.md` — use those names freely for scratch notes; they won't be
committed.

## Releasing and the changelog

Each crate releases **independently** — they do not share a version.

- **The crate's `Cargo.toml` `version` is the single source of truth.** The
  release workflow bumps it (you never type a version); never let the manifest,
  the tag, and the published artifact drift apart.
- **Each crate has its own `CHANGELOG.md`** following
  [Keep a Changelog](https://keepachangelog.com/) +
  [Semantic Versioning](https://semver.org/). Curate the `[Unreleased]` section
  as you work — manual bullets always win over the `git-cliff` (`cliff.toml`)
  auto-fill, which (only when `[Unreleased]` has no real bullets) buckets commits
  *touching that crate's directory* by subject prefix
  (`feat`→Added, `fix`→Fixed, `remove`→Removed, `perf`/`refactor`/`ci`/…→Changed,
  `docs`/`chore`/`test`→skipped).
- **Tag per crate** as `<crate>-v<version>` (e.g. `vcs-git-v0.2.0`) so each
  crate's history and compare links stay independent.
- **Publish order is almost flat.** The three CLI wrappers depend only on the
  already-published external `processkit` crate (by version), so each publishes
  independently. The **`vcs-core` facade is the one exception**: it depends on
  `vcs-git`/`vcs-jj`, so it publishes **last** (the `all` plan orders it after its
  deps), and its `^0.3` requirement on them must be kept in range when they cross
  a minor/major boundary (and the new version must be live on crates.io first). If
  a crate needs a newer `processkit`, bump the `[workspace.dependencies]` req and
  ensure that `processkit` version is live on crates.io first.
- **Release workflow.** `.github/workflows/release.yml` (`workflow_dispatch`,
  needs the `CRATES_IO_TOKEN` secret) is the only way to release. Pick **which
  crate** (`vcs-git`/`vcs-jj`/`vcs-github`/`vcs-core`, or **`all`**) and a **bump**
  (`patch`/`minor`/`major`) — the version is **never typed by hand**. For each
  selected crate it derives the next version from that crate's current
  `Cargo.toml` (a crate's **first release** — no `<crate>-v*` tag yet — ships the
  current version as-is, ignoring the bump), bumps it, auto-fills/promotes the
  changelog, then **publishes to crates.io before tagging** (so a failed publish
  strands nothing; an already-uploaded version counts as success), tags
  `<crate>-v<version>`, and creates a GitHub Release from the curated notes.
  `all` does every crate in one commit + atomic push, each bumped by the same
  chosen type from its own version. The publish stays a deliberate, human-triggered
  action.

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
