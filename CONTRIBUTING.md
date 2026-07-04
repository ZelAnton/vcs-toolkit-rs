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

Before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The pure parsers are property-tested (`proptest`) for panic-freedom, and CI runs the
`--ignored` suites against several `jj` versions to catch CLI/template drift. The test
seams (the interface trait, the `mock` feature, and injecting a `ScriptedRunner` /
`RecordingRunner`) are documented in the
**[testing guide](crates/testkit/docs/testing.md)** — production code depends on the
trait, so tests need no real binary, temp repo, or network.

## Conventions

- **Every dependency carries an inline "why" comment** in `Cargo.toml`, and
  `Cargo.lock` stays committed.
- **Each crate has its own `CHANGELOG.md`** ([Keep a Changelog](https://keepachangelog.com/));
  curate the `[Unreleased]` section as you work when a change is user-facing.
- **Multi-option commands take a builder/spec** rather than a long positional list —
  the trigger is **≥2 options, or any bare `bool`** (a bare boolean at a call site is
  ambiguous, so it becomes a presence-only setter or a spec field).
- Keep new code in the style of the surrounding code; `cargo fmt` and the clippy gate
  above are the baseline.

## Releasing

Maintainer-only, via the **Release** GitHub Actions workflow (manual
`workflow_dispatch` — pick the crate or `all`, and `patch` / `minor` / `major`). Each
crate is **versioned and published independently**: the workflow bumps the manifest,
promotes that crate's `CHANGELOG.md`, publishes to crates.io, tags
`<crate>-v<version>`, and creates the GitHub Release. docs.rs builds the API reference
from the published crate — there is no separate docs deploy.

Planned and parked work lives in [ROADMAP.md](ROADMAP.md), [`ideas/`](ideas/), and
[`decisions/`](decisions/) — check there before proposing something large; it may
already be planned, deferred, or settled against.
