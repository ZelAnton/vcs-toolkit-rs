# Fuzz targets

[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) targets for
public parsers that process untrusted VCS output. They complement the in-tree
`proptest` property tests (which run in the normal `cargo test` CI gate) with
continuous coverage-guided fuzzing.

This crate is **excluded from the workspace** (`exclude = ["fuzz"]` in the root
`Cargo.toml`) because cargo-fuzz needs **nightly Rust + libFuzzer**, so it never
touches the stable build, the MSRV, or the normal PR CI gate. Run it manually:

```bash
cargo install cargo-fuzz
cargo +nightly fuzz run git_conflict     # parse_conflicts panic-freedom + render roundtrip
cargo +nightly fuzz run jj_conflict      # the jj diff/snapshot grammar + side/base materializers
cargo +nightly fuzz run diff_parse       # forge PR unified-diff parser panic-freedom
cargo +nightly fuzz run git_porcelain_v2 # git status --porcelain=v2 parser panic-freedom
```

All targets reject invalid UTF-8 before calling their `&str` parsers. The two
conflict targets also assert their roundtrip invariants; `diff_parse` and
`git_porcelain_v2` assert panic-freedom for arbitrary UTF-8 received from a
forge or repository. A crash reproducer lands in `fuzz/artifacts/`; minimise
and add it as a regression unit test in the relevant parser.

Artifacts, corpora, and the build dir are git-ignored.

## Scheduled fuzzing

`.github/workflows/scheduled-fuzz.yml` runs all four targets weekly (Thursday
02:53 UTC) with a five-minute libFuzzer budget per target. It uses nightly and
`cargo-fuzz`, and caches each target's corpus between runs so the search keeps
building on previously discovered inputs.

This is a report-only scheduled lane, not a `ci.yml` or PR gate. If a target
crashes and writes a reproducer, the workflow uploads the contents of
`fuzz/artifacts` and creates or updates one tracking issue. After extracting the
downloaded artifact into `fuzz/`, its layout is `<target>/crash-*` at the
artifact root (there is no additional `artifacts/` directory). Setup/runtime
failures without a reproducer are reported in the workflow summary and are not
tracked as parser crashes. From the `fuzz/` directory, reproduce a failing input
with the same nightly cargo-fuzz command used by CI:

```bash
cd fuzz
cargo +nightly fuzz run <target> <target>/crash-<hash>
```

For a five-minute campaign matching scheduled CI, run:

```bash
cd fuzz
cargo +nightly fuzz run <target> -- -max_total_time=300
```

Replace `<target>` with `git_porcelain_v2` (or another listed target) and
`<hash>` with the downloaded filename. After confirming the failure, minimise
it if useful and add a regression unit test in the relevant parser.
