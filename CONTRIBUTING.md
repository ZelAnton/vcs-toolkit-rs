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
  every crate directory and do not add an `include` list that omits it — a crate
  with no `include` list already ships every tracked file. CI runs
  `cargo package --list` for all published crates and compares each local copy
  with the root `LICENSE`; add the same file before publishing a new crate.
  Set only `license.workspace = true` in Cargo.toml — don't also set
  `license-file`, since cargo warns when both a `license` SPDX expression and a
  `license-file` are given (`license-file` is only for non-standard licenses; see
  [the manifest docs](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields)).
- **Multi-option commands take a builder/spec** rather than a long positional list —
  the trigger is **≥2 options, or any bare `bool`** (a bare boolean at a call site is
  ambiguous, so it becomes a presence-only setter or a spec field).
- Keep new code in the style of the surrounding code; `cargo fmt` and the clippy gate
  above are the baseline.
- **[Extending vcs-toolkit-rs](docs/extending.md)** — the full contributor workflow for
  adding capabilities: CLI methods, facade operations, MCP tools, and decision records.

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
