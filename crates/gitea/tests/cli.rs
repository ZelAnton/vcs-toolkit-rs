//! Integration tests for `vcs-gitea`. Ignored by default (require the `tea`
//! binary). Run with `cargo test -p vcs-gitea -- --ignored`.
//!
//! `tea` is rarely pre-installed, so each test **skips gracefully** (prints and
//! returns) when the binary is absent, rather than failing — CI installs it
//! best-effort.
//!
//! The list/view tests below are the **definitive check** of the `tea --output
//! json` contract our parsers model (the hermetic unit tests can only confirm the
//! parser against *assumed* fixtures). They run a real `tea … list` and assert
//! the output is **not** an `Error::Parse` — a parse error means tea's real shape
//! diverged from our structs, the exact bug class this crate's re-model fixed.
//! Any *other* error (no Gitea repo, not authenticated, network) is an
//! environment skip, so they need a live, authenticated Gitea repo to be
//! meaningful: point `VCS_GITEA_TEST_REPO` at one (defaults to the cwd). **Run
//! these against a real `tea` before the crate's first release.**

use std::path::PathBuf;

use vcs_gitea::{Gitea, GiteaApi};

/// Whether `tea` is on PATH (a successful `--version` spawn).
async fn tea_present() -> bool {
    Gitea::new().version().await.is_ok()
}

/// The repo dir the list/view tests run against (`VCS_GITEA_TEST_REPO`, else cwd).
fn test_repo() -> PathBuf {
    std::env::var_os("VCS_GITEA_TEST_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"))
}

/// Fail only on the contract violation this suite hunts — a parser that doesn't
/// match `tea`'s real output (`Error::Parse`). Every other error (the dir is not
/// a Gitea repo, no login, network) is an environment skip, not a test failure.
fn assert_not_parse_error<T>(label: &str, result: processkit::Result<T>) {
    match result {
        Ok(_) => {} // tea produced output and our parser accepted it.
        Err(processkit::Error::Parse { message, .. }) => {
            panic!("{label}: tea output did not match the parser (contract drift): {message}");
        }
        Err(other) => eprintln!("skipping {label}: {other}"),
    }
}

#[tokio::test]
#[ignore = "requires the tea binary"]
async fn version_runs() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    let v = Gitea::new().version().await.expect("tea version");
    assert!(!v.trim().is_empty(), "expected a version string");
}

#[tokio::test]
#[ignore = "requires the tea binary"]
async fn auth_status_does_not_error() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    // Reports the bool whether or not a login is configured; must not error.
    let _authed = Gitea::new()
        .auth_status()
        .await
        .expect("auth_status should not error");
}

// The three list shapes (PR / issue / release tables) and the issue detail view
// must deserialize from REAL `tea --output json` without an `Error::Parse`. These
// are the only structural validation of the table/detail contract the parsers
// model — point `VCS_GITEA_TEST_REPO` at a populated, authenticated Gitea repo.
#[tokio::test]
#[ignore = "requires the tea binary + a real Gitea repo/login"]
async fn list_outputs_match_the_parsers() {
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return;
    }
    let tea = Gitea::new();
    let dir = test_repo();

    assert_not_parse_error("pr_list", tea.pr_list(&dir).await);
    assert_not_parse_error("issue_list", tea.issue_list(&dir).await);
    assert_not_parse_error("release_list", tea.release_list(&dir).await);

    // issue_view goes through tea's *typed* detail path (a different shape from
    // the list); probe #1 (a non-Parse error is fine if it doesn't exist).
    assert_not_parse_error("issue_view", tea.issue_view(&dir, 1).await);
}
