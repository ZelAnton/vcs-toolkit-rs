# Contributing

Contributions land via pull requests into `main`. Thanks for helping out!

This is a Cargo workspace of independently-versioned crates that wrap the
`git` / `jj` / `gh` / `glab` / `tea` command-line tools as typed, async Rust APIs.
Start with the [README](README.md) for the overview and the
[guide set in `docs/`](docs/README.md) for per-crate depth.

## Building & testing

```bash
cargo build --workspace
cargo test                              # hermetic unit + doc tests (no real binaries)
cargo test --workspace --all-features   # incl. the mock layer + ScriptedRunner
cargo test -- --ignored                 # real-binary integration suites
                                        # (need git / jj / gh / glab / tea on PATH)
```

Before opening a PR, run the full local gate — it reproduces the CI jobs
(`fmt`, `clippy` in both configurations, `doc`, `msrv`, feature isolation, `test`,
`cargo deny check`, `cargo package`) in the same order and with the same flags as
[`.github/workflows/ci.yml`](.github/workflows/ci.yml), skipping with an explicit
message whatever can't run locally (no `nightly`/MSRV toolchain, no `cargo-deny`,
the multi-version `integration` job):

```bash
scripts/gate          # full gate — run before pushing
scripts/gate --fast   # fmt + clippy + test only, for quick local iteration
scripts/gate --help
```

`scripts/gate` requires a POSIX shell (bash); see the comment at the top of the
script. Its composition is kept in sync with `ci.yml` by hand, not generated —
if you change a CI job, update `scripts/gate` too.

The pure parsers are property-tested (`proptest`) for panic-freedom, and CI runs the
`--ignored` suites against several `jj` versions to catch CLI/template drift. A separate
weekly, non-gating [scheduled drift lane](.github/workflows/scheduled-cli-drift.yml)
re-runs them against the *actual latest* jj/glab/tea and stands up a one-shot **live
Gitea** to exercise the real create → merge PR lifecycle (plus issues/releases)
end-to-end through `vcs-forge`/`vcs-gitea`, reporting drift as a tracking issue instead
of failing a PR. The test seams (the interface trait, the `mock` feature, and injecting a
`ScriptedRunner` / `RecordingRunner`) are documented in the
**[testing guide](crates/testkit/docs/testing.md)** — production code depends on the
trait, so tests need no real binary, temp repo, or network.

## Benchmarks

Run the parser benchmarks locally with a POSIX shell:

```bash
bash scripts/bench
# or a single crate while iterating:
cargo bench --release -p vcs-git
```

The fixtures are generated in each benchmark source, rather than stored as large
repository files. `vcs-diff` measures parsing a 1,200-file unified diff with
additions, deletions, and modifications; `vcs-git` measures a 2,500-record
porcelain-v2 status plus mixed merge/diff3 conflict parsing and exact rendering;
`vcs-jj` covers the equivalent native snapshot-style conflict path. Criterion writes
the HTML report to `target/criterion/report/index.html` (with per-benchmark reports
beneath `target/criterion/`); open it in a browser to compare the current sample with
the saved baseline and inspect the distributions/charts.

CI deliberately runs only `cargo bench --no-run --locked`, which compiles every
benchmark and fails on a compile error without collecting timing data or enforcing a
performance threshold. Run the benchmarks locally for numbers: shared CI runners are
too noisy for performance conclusions.

## Conventions

### Dependency management

- This workspace fixes **no allow-list of crates**. Declare each shared
  dependency once in `[workspace.dependencies]` and reference it from members
  with `<crate>.workspace = true` when more than one crate needs it.
- **Every dependency carries an inline "why" comment** in `Cargo.toml`, and
  `Cargo.lock` stays committed.
- Pin major versions and enable only the features actually used.

### Code & docs

- **Each crate has its own `CHANGELOG.md`** ([Keep a Changelog](https://keepachangelog.com/));
  curate the `[Unreleased]` section as you work when a change is user-facing.
- **Published crates carry the full MIT text.** Keep a byte-identical `LICENSE` in
  every crate directory, set `license-file = "LICENSE"`, and do not add a
  restrictive `include` list that omits it. CI runs `cargo package --list` for all
  published crates and compares each local copy with the root `LICENSE`; add the same
  file and explicit field before publishing a new crate.
- **Multi-option commands take a builder/spec** rather than a long positional list —
  the trigger is **≥2 options, or any bare `bool`** (a bare boolean at a call site is
  ambiguous, so it becomes a presence-only setter or a spec field).
- Keep new code in the style of the surrounding code; `cargo fmt` and the clippy gate
  above are the baseline.
- **[Extending vcs-toolkit-rs](docs/extending.md)** — the full contributor workflow for
  adding capabilities: CLI methods, facade operations, MCP tools, and decision records.

### Updating a `gh` CLI cassette

A pilot in `vcs-github` replaces some hand-invented parser fixtures with
**recorded cassettes**: `processkit`'s `RecordReplayRunner` (the `record`
feature, enabled only for that crate's dev/test profile —
`crates/github/Cargo.toml`'s `[dev-dependencies]`, no effect on the published
library or any other workspace crate) runs the real `gh` **once** and captures
every `Invocation → ProcessResult` pair to a human-diffable JSON file under
`crates/github/tests/cassettes/`; the ordinary hermetic test suite then
replays that file — no subprocess, no network, deterministic. See the
[`processkit::cassette`](https://docs.rs/processkit) module docs for the
mechanism (portable match key, `match_on_cwd`/`match_on_env` opt-ins,
durability/concurrency of `save`).

Re-record a cassette when the wrapped `gh` subcommand's output shape changes
(a new/renamed `--json` field, a reshaped error) or you add a scenario:

```bash
# Requires network and an authenticated `gh` (`gh auth status`) against
# ZelAnton/vcs-toolkit-rs, whose real releases/Actions runs the cassettes
# capture. Re-run the specific `record_*` test(s) that cover the changed
# subcommand — this overwrites only that cassette file.
cargo test -p vcs-github -- --ignored record_release_round_trip
cargo test -p vcs-github -- --ignored record_run_round_trip
```

Then run `cargo test -p vcs-github` (no `--ignored`) to confirm the replaying
unit tests still pass against the freshly recorded file, and commit the
cassette alongside the code/test change that motivated re-recording — never on
its own in an unrelated PR.

**A cassette diff is a change to an external contract, not a routine data
update — review it explicitly, the way a `public-api.txt` diff is reviewed
below.** The cassette *is* "what `gh` actually printed" on the recording run;
a diff means that output changed (a new field, a reordered/renamed one, a
different error shape) and whoever reviews it must confirm the wrapper's
parser and any hermetic assertions still agree with the new shape before
approving — never accept a re-recorded cassette as a silent, pass-through
diff. A cassette also stores its invocation `program`/`args`/`cwd`/`stdout`/
`stderr` verbatim (only env *values* are redacted, kept as variable names
only) — skim a freshly recorded file for anything sensitive before committing
it, same as reviewing any other fixture that touched a real service.

**This is a distinct mechanism from the [scheduled CLI-drift
lane](.github/workflows/scheduled-cli-drift.yml)**, which this pilot does not
touch: a cassette answers "what did `gh` print when we last recorded", so it
must fail loudly (not skip) when it's missing or stale relative to the code
that expects it — replay's `Error::CassetteMiss` on an unmatched invocation
does exactly that. The scheduled lane answers a different question, "has the
*live* CLI's behavior drifted since then", running the real, latest binaries
on a schedule and reporting drift as a tracking issue rather than failing a
PR. Neither substitutes for the other, and neither may silently degrade into
an environment-skip.

### Public-API snapshots

Every published crate carries a committed **`crates/<crate>/public-api.txt`** — a
snapshot of its exported surface (public items, signatures, and the `Send`/`Sync`
auto-trait impls), generated with
[`cargo public-api`](https://github.com/cargo-public-api/cargo-public-api). The
`public-api` CI job (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml))
regenerates each crate's surface and **diffs it against the committed snapshot**,
failing — and printing the unified diff — on any drift. This turns the pre-1.0
public-API review in
[`crates/core/docs/stability.md`](crates/core/docs/stability.md) into a
*mechanical* gate: a new trait method, error variant, DTO field, or a changed
signature can no longer slip in unnoticed, and the diff in the job log is exactly
what to cross-check against that checklist.

**A surface change is accepted deliberately, never auto-generated.** When a change
moves the public API, CI fails against the now-stale snapshot. Do **not** blindly
regenerate: first read the diff the job prints (or run the command below and
inspect it), confirm every added/removed/changed item is intended and
changelog-worthy, and only then overwrite the snapshot — committing it *in the
same PR* as the code change so review sees the surface delta next to the code.

Regenerate a crate's snapshot with:

```bash
cargo +nightly-2026-06-25 public-api -p <crate> \
  --simplified --all-features > crates/<dir>/public-api.txt
# e.g. -p vcs-core > crates/core/public-api.txt
```

Two things about the toolchain:

- **Nightly, pinned.** `cargo public-api` reads rustdoc JSON, which is nightly
  only, so this is the one job that needs a nightly toolchain. It pins the
  *dated* `nightly-2026-06-25` (and `cargo-public-api` 0.52.0) **solely for this
  job** — it does **not** touch the `1.88` MSRV the rest of the workspace holds.
  Pinning keeps the type/auto-trait rendering reproducible, so a routine nightly
  bump can't spuriously fail the gate; regenerate with that same nightly
  (`rustup toolchain install nightly-2026-06-25`). Bumping the pinned nightly or
  the tool is a deliberate maintenance step: rendering can shift between
  nightlies, so it regenerates **every** `crates/*/public-api.txt` in one commit
  and updates the pin in both [`ci.yml`](.github/workflows/ci.yml) and
  [`scripts/gate`](scripts/gate).
- **Windows-shaped.** A few public items are OS-gated (e.g.
  `vcs-testkit::non_utf8_filename` is `#[cfg(unix)]`), so the surface differs by
  OS. The baselines are generated on **Windows** and CI checks them on
  `windows-latest`; regenerate on Windows (or let the CI job show you the diff) so
  the checking OS matches the generating one. `scripts/gate` runs this check only
  on Windows and skips it with a message elsewhere.

## Releasing

Maintainer-only, via the **Release** GitHub Actions workflow (manual
`workflow_dispatch` — pick the crate or `all`, and `patch` / `minor` / `major`). Each
crate is **versioned and published independently**: the workflow bumps the manifest,
promotes that crate's `CHANGELOG.md`, publishes to crates.io, tags
`<crate>-v<version>`, and creates the GitHub Release. docs.rs builds the API reference
from the published crate — there is no separate docs deploy.

Publish order follows the intra-workspace dependency graph: foundational crates
first (`vcs-diff`, `vcs-cli-support`), then the wrappers (`vcs-git`, `vcs-jj`,
`vcs-github`, `vcs-gitlab`, `vcs-gitea`), then the facades (`vcs-forge`,
`vcs-core`), and finally the crates that depend on a facade (`vcs-watch`,
`vcs-mcp`); `vcs-testkit` has no workspace dependency and can publish any time.
Each crate's `Cargo.toml` is the source of truth for its own version;
`scripts/release/lib.sh` is the source of truth for the publish order — keep
both in sync if the dependency graph changes. Intra-workspace dependencies use
`^MAJOR.MINOR` requirements and must stay in range when a dependency crosses a
minor or major version boundary; see
[crates/core/docs/stability.md](crates/core/docs/stability.md) for the
version/tier matrix and the external dependencies whose major bumps need
coordinated releases.

Before proposing something large, search the existing GitHub issues — it may
already be planned, deferred, or settled against.
