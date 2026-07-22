//! Integration tests for `vcs-github`. Ignored by default (require the `gh`
//! binary). The repo/pr/issue commands need network + authentication and are
//! not exercised here — their JSON parsing is covered by the hermetic unit
//! tests in `src/parse.rs` and the scripted-runner tests in `src/lib.rs`. Run
//! with `cargo test -p vcs-github -- --ignored`.

use vcs_github::{GitHub, GitHubApi, GitHubHost};

/// Whether `gh` is on PATH (a successful `--version` spawn).
async fn gh_present() -> bool {
    GitHub::new().version().await.is_ok()
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn version_mentions_gh() {
    let v = GitHub::new()
        .version()
        .await
        .expect("gh should be installed");
    assert!(v.to_lowercase().contains("gh"), "unexpected: {v}");
}

// The real `gh --version` banner must parse into a version at/above the crate
// floor. This is the "modern real binary" arm of the version-gate check the
// scheduled-drift lane runs (the hermetic unit tests in `src/lib.rs` cover the
// minimum and unrecognisable arms): if a future `gh` reshapes its `--version`
// output so the shared parser can't read it, `capabilities()` returns
// `Error::Parse` and this fails, flagging the drift.
#[tokio::test]
#[ignore = "requires the gh binary"]
async fn capability_version_gate_real_binary() {
    if !gh_present().await {
        eprintln!("skipping: gh not installed");
        return;
    }
    let caps = GitHub::new().capabilities().await.expect("gh capabilities");
    assert!(
        caps.is_supported(),
        "the installed gh ({}) is below vcs-github's supported floor",
        caps.version
    );
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn auth_status_does_not_error() {
    // Reports the bool whether or not the user is logged in; must not error.
    let _authed = GitHub::new()
        .auth_status()
        .await
        .expect("auth_status should not error");
}

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn auth_status_for_host_does_not_error() {
    // The host-scoped probe reports the bool for one host (`gh auth status
    // --hostname github.com`) whether or not the user is logged in; it must not
    // error, just like the unscoped `auth_status`.
    let _authed = GitHub::new()
        .auth_status_for(&GitHubHost::github_com())
        .await
        .expect("auth_status_for should not error");
}

// Read-only, auth-gated checks against this very repository (it has real
// Actions runs and releases). Skipped politely when gh isn't authenticated.
// PR-scoped reads (`pr_checks`, `pr_feedback`) have NO live coverage — the
// repo has no PRs; their parsing is covered hermetically.

/// `Some(client)` when gh is installed AND authenticated; `None` → skip.
async fn authed() -> Option<GitHub> {
    let gh = GitHub::new();
    match gh.auth_status().await {
        Ok(true) => Some(gh),
        _ => {
            eprintln!("skipping: gh not authenticated");
            None
        }
    }
}

#[tokio::test]
#[ignore = "requires the gh binary, auth, and network"]
async fn run_list_and_view_round_trip() {
    let Some(gh) = authed().await else { return };
    let dir = std::path::Path::new(".");

    let runs = gh.run_list(dir, 3, None).await.expect("run_list");
    assert!(!runs.is_empty(), "this repo has Actions runs");
    let first = &runs[0];
    assert!(first.database_id > 0);
    assert!(!first.workflow_name.is_empty(), "got {first:?}");
    assert!(!first.url.is_empty());

    let viewed = gh.run_view(dir, first.database_id).await.expect("run_view");
    assert_eq!(viewed.database_id, first.database_id);
    assert_eq!(viewed.workflow_name, first.workflow_name);
}

#[tokio::test]
#[ignore = "requires the gh binary, auth, and network"]
async fn release_list_and_view_round_trip() {
    let Some(gh) = authed().await else { return };
    let dir = std::path::Path::new(".");

    let releases = gh.release_list(dir).await.expect("release_list");
    let released_tag = releases
        .first()
        .map(|release| release.tag_name.clone())
        .expect("this repo has releases");

    let release = gh
        .release_view(dir, &released_tag)
        .await
        .expect("release_view");
    assert_eq!(release.tag_name, released_tag);
    // `release_view` fetches body/url, so both are `Some` and non-empty (the lean
    // `release_list` leaves them `None`).
    assert!(
        release.body.as_deref().is_some_and(|b| !b.is_empty()),
        "release notes were curated"
    );
    assert!(release.url.as_deref().is_some_and(|u| !u.is_empty()));
}

// --- Cassette recording ------------------------------------------------
//
// The two tests below are not part of the ordinary suite: they drive a real,
// authenticated `gh` against this very repository and (re)write the
// human-readable JSON cassettes `src/lib.rs`'s hermetic unit tests replay
// (`release_view_requests_view_fields`, `run_list_and_view_replay_recorded_cassette`).
// See CONTRIBUTING.md, "Updating a `gh` CLI cassette", for when/how to run
// these and how a cassette diff should read on review.
//
// `processkit`'s `record` feature is enabled unconditionally for this crate's
// dev/test profile (see `[dev-dependencies]` in Cargo.toml), so no extra
// `--features` flag is needed here — just `--ignored` to opt into the ones
// that spawn a real `gh`.
//
// Run with: `cargo test -p vcs-github -- --ignored record_`
mod record {
    use super::*;
    use processkit::JobRunner;
    use processkit::testing::RecordReplayRunner;
    use std::path::PathBuf;

    fn cassette_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/cassettes")
            .join(name)
    }

    #[tokio::test]
    #[ignore = "records a live cassette against gh; requires network + an authenticated gh"]
    async fn record_release_round_trip() {
        let runner =
            RecordReplayRunner::record(cassette_path("release_round_trip.json"), JobRunner::new());
        let gh = GitHub::with_runner(&runner);
        let dir = std::path::Path::new(".");

        let releases = gh.release_list(dir).await.expect("release_list");
        let tag = releases
            .first()
            .map(|r| r.tag_name.clone())
            .expect("this repo has releases");
        gh.release_view(dir, &tag).await.expect("release_view");

        runner.save().expect("save release cassette");
    }

    #[tokio::test]
    #[ignore = "records a live cassette against gh; requires network + an authenticated gh"]
    async fn record_run_round_trip() {
        let runner =
            RecordReplayRunner::record(cassette_path("run_round_trip.json"), JobRunner::new());
        let gh = GitHub::with_runner(&runner);
        let dir = std::path::Path::new(".");

        let runs = gh.run_list(dir, 3, None).await.expect("run_list");
        let id = runs
            .first()
            .map(|r| r.database_id)
            .expect("this repo has Actions runs");
        gh.run_view(dir, id).await.expect("run_view");

        runner.save().expect("save run cassette");
    }
}
