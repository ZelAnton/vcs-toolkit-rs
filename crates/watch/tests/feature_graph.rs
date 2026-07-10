//! Feature-graph guard (T-055): the optional surfaces stay isolated, and the
//! `tracing` feature actually *forwards* to `vcs-core` (so the underlying
//! git/jj/processkit commands a re-query issues are traced too, not just the
//! watcher's own line).
//!
//! Driven by shelling out to `cargo tree`, so — like this crate's other
//! external-tool tests — it is `#[ignore]`d by default (the plain
//! `cargo test --workspace` smoke run stays fast and never re-enters cargo). Run
//! it explicitly:
//!
//! ```text
//! cargo test -p vcs-watch --test feature_graph -- --ignored
//! ```

use std::process::Command;

/// Run `cargo tree <args>` against this crate's manifest and return stdout.
/// Panics (failing the test) if cargo can't be launched or exits non-zero.
fn cargo_tree(args: &[&str]) -> String {
    // `CARGO` is set to the active cargo binary while tests run; fall back to
    // the one on `PATH` for a bare `cargo test` invocation.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");

    let mut cmd = Command::new(cargo);
    cmd.args(["tree", "--locked", "--manifest-path", manifest, "-p", "vcs-watch"]);
    cmd.args(args);

    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to run `cargo tree {}`: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`cargo tree {}` failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The **minimal** (no-default-features) build takes neither `tracing` nor
/// `futures-core` as a **direct** dependency — both stay behind their features.
/// (`futures-core` is present *transitively* via the tokio/processkit runtime
/// stack regardless, so isolation is about vcs-watch's own direct edge, which is
/// exactly what the feature flag controls — hence `--depth 1`.)
#[test]
#[ignore = "requires cargo on PATH; run with -- --ignored"]
fn minimal_build_has_no_direct_optional_deps() {
    // `--depth 1` = only vcs-watch's direct deps; `-e normal` drops dev-deps.
    let direct = cargo_tree(&["--no-default-features", "-e", "normal", "--depth", "1"]);
    assert!(
        !direct.contains("tracing"),
        "minimal build must not take `tracing` as a direct dep:\n{direct}"
    );
    assert!(
        !direct.contains("futures-core"),
        "minimal build must not take `futures-core` as a direct dep:\n{direct}"
    );
}

/// The `tracing` feature **forwards** to `vcs-core`: inverting the graph on the
/// `tracing` crate lists more than `vcs-watch` itself — `vcs-core` (and, through
/// it, the backends) enable it too. If the feature only did `dep:tracing`, the
/// reverse tree would name `vcs-watch` alone.
#[test]
#[ignore = "requires cargo on PATH; run with -- --ignored"]
fn tracing_feature_forwards_to_vcs_core() {
    let inverted = cargo_tree(&["--features", "tracing", "-e", "normal", "-i", "tracing"]);
    assert!(
        inverted.contains("vcs-core"),
        "the `tracing` feature must forward to `vcs-core/tracing` (its reverse \
         dependency tree should include vcs-core), got:\n{inverted}"
    );
}

/// The `stream` feature adds `futures-core` as a **direct** dependency of
/// `vcs-watch` (the minimal build has no such direct edge — see above).
#[test]
#[ignore = "requires cargo on PATH; run with -- --ignored"]
fn stream_feature_adds_direct_futures_core() {
    let direct = cargo_tree(&["--features", "stream", "-e", "normal", "--depth", "1"]);
    assert!(
        direct.contains("futures-core"),
        "the `stream` feature must take `futures-core` as a direct dep:\n{direct}"
    );
}
