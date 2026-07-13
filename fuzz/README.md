# Fuzz targets

[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) targets for
public parsers that process untrusted VCS output. They complement the in-tree
`proptest` property tests (which run in the normal `cargo test` CI gate) with
continuous coverage-guided fuzzing.

This crate is **excluded from the workspace** (`exclude = ["fuzz"]` in the root
`Cargo.toml`) because cargo-fuzz needs **nightly Rust + libFuzzer**, so it never
touches the stable build, the MSRV, or CI. Run it manually:

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
