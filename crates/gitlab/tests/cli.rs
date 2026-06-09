//! Integration tests for `vcs-gitlab`. Ignored by default (require the `glab`
//! binary). The list→view round-trips below run only against a configured GitLab
//! remote and **skip gracefully** when one isn't available — there's no GitLab
//! mirror in CI (unlike the GitHub repo that `vcs-github` tests against), so they
//! exist mainly for a maintainer to validate the shape end to end locally. The
//! argv + JSON parsing for every MR/issue/release command is pinned hermetically
//! in `src/lib.rs` regardless. Run with `cargo test -p vcs-gitlab -- --ignored`.
//!
//! `glab` is less universally installed than `git`/`gh`, so each test **skips
//! gracefully** (prints and returns) when the binary is absent, rather than
//! failing — CI installs it best-effort.

use vcs_gitlab::{GitLab, GitLabApi};

/// Whether `glab` is on PATH (a successful `--version` spawn).
async fn glab_present() -> bool {
    GitLab::new().version().await.is_ok()
}

#[tokio::test]
#[ignore = "requires the glab binary"]
async fn version_mentions_glab() {
    if !glab_present().await {
        eprintln!("skipping: glab not installed");
        return;
    }
    let v = GitLab::new().version().await.expect("glab version");
    // glab prints "glab version x.y.z …".
    assert!(v.to_lowercase().contains("glab"), "unexpected: {v}");
}

#[tokio::test]
#[ignore = "requires the glab binary"]
async fn auth_status_does_not_error() {
    if !glab_present().await {
        eprintln!("skipping: glab not installed");
        return;
    }
    // Reports the bool whether or not the user is logged in; must not error.
    let _authed = GitLab::new()
        .auth_status()
        .await
        .expect("auth_status should not error");
}

// list→view round-trips against a configured GitLab remote. They skip gracefully
// whenever `glab` is absent, unauthenticated, the repo has no GitLab remote, or the
// list is empty — so they never fail in CI, where no GitLab is configured.

#[tokio::test]
#[ignore = "requires the glab binary + a configured GitLab repo"]
async fn issue_list_and_view_round_trip() {
    if !glab_present().await {
        eprintln!("skipping: glab not installed");
        return;
    }
    let glab = GitLab::new();
    let dir = std::path::Path::new(".");
    let Ok(issues) = glab.issue_list(dir).await else {
        eprintln!("skipping: issue_list failed (no GitLab remote / auth?)");
        return;
    };
    let Some(first) = issues.first() else {
        eprintln!("skipping: repo has no issues");
        return;
    };
    let viewed = glab
        .issue_view(dir, first.number)
        .await
        .expect("issue_view");
    assert_eq!(viewed.number, first.number);
    assert_eq!(viewed.title, first.title);
}

#[tokio::test]
#[ignore = "requires the glab binary + a configured GitLab repo"]
async fn release_list_and_view_round_trip() {
    if !glab_present().await {
        eprintln!("skipping: glab not installed");
        return;
    }
    let glab = GitLab::new();
    let dir = std::path::Path::new(".");
    let Ok(releases) = glab.release_list(dir).await else {
        eprintln!("skipping: release_list failed (no GitLab remote / auth?)");
        return;
    };
    let Some(first) = releases.first() else {
        eprintln!("skipping: repo has no releases");
        return;
    };
    let viewed = glab
        .release_view(dir, &first.tag_name)
        .await
        .expect("release_view");
    assert_eq!(viewed.tag_name, first.tag_name);
    assert!(!viewed.url.is_empty());
}
