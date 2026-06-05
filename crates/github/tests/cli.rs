//! Integration tests for `vcs-github`. Ignored by default (require the `gh`
//! binary). The repo/pr/issue commands need network + authentication and are
//! not exercised here — their JSON parsing is covered by the hermetic unit
//! tests in `src/parse.rs` and the scripted-runner tests in `src/lib.rs`. Run
//! with `cargo test -p vcs-github -- --ignored`.

use vcs_github::{GitHub, GitHubApi};

#[tokio::test]
#[ignore = "requires the gh binary"]
async fn version_mentions_gh() {
    let v = GitHub::new()
        .version()
        .await
        .expect("gh should be installed");
    assert!(v.to_lowercase().contains("gh"), "unexpected: {v}");
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
    assert!(
        releases.iter().any(|r| r.tag_name == "vcs-git-v0.4.0"),
        "expected the released tag, got {releases:?}"
    );

    let release = gh
        .release_view(dir, "vcs-git-v0.4.0")
        .await
        .expect("release_view");
    assert_eq!(release.tag_name, "vcs-git-v0.4.0");
    assert!(!release.body.is_empty(), "release notes were curated");
    assert!(!release.url.is_empty());
}
