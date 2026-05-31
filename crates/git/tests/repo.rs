//! End-to-end tests for the typed `vcs-git` client against a real temporary
//! repository. Ignored by default (require the `git` binary); run with
//! `cargo test -p vcs-git -- --ignored`.

mod common;

use std::path::{Path, PathBuf};

use common::TempDir;
use vcs_git::{Git, GitApi};

/// Give the repo a deterministic identity so commits don't depend on global config.
fn configure(dir: &Path) {
    for (key, val) in [
        ("user.email", "test@example.com"),
        ("user.name", "Test"),
        ("commit.gpgsign", "false"),
    ] {
        std::process::Command::new(vcs_git::BINARY)
            .current_dir(dir)
            .args(["config", key, val])
            .status()
            .expect("git config");
    }
}

#[tokio::test]
#[ignore = "requires the git binary"]
async fn init_status_add_commit_log_cycle() {
    let tmp = TempDir::new("cycle");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);

    // Untracked file shows up in status.
    std::fs::write(dir.join("file.txt"), "hello\n").expect("write file");
    let status = git.status(dir).await.expect("status");
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].code, "??");
    assert_eq!(status[0].path, "file.txt");

    // Stage + commit, then status is clean.
    git.add(dir, &[PathBuf::from("file.txt")])
        .await
        .expect("add");
    git.commit(dir, "initial commit").await.expect("commit");
    assert!(git.status(dir).await.expect("status").is_empty());

    // Log reflects the commit, with the enriched fields.
    let log = git.log(dir, 10).await.expect("log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "initial commit");
    assert_eq!(log[0].author, "Test");
    assert_eq!(log[0].hash.len(), 40, "full sha expected");
    assert!(!log[0].short_hash.is_empty() && log[0].hash.starts_with(&log[0].short_hash));
    assert!(
        log[0].date.starts_with("20"),
        "ISO date expected, got {:?}",
        log[0].date
    );

    // Branch introspection + create/checkout.
    let branch = git.current_branch(dir).await.expect("current_branch");
    assert!(!branch.is_empty());
    git.create_branch(dir, "feature")
        .await
        .expect("create_branch");
    git.checkout(dir, "feature").await.expect("checkout");
    assert_eq!(git.current_branch(dir).await.expect("branch"), "feature");
    let branches = git.branches(dir).await.expect("branches");
    assert!(branches.iter().any(|b| b.name == "feature"));

    // rev_parse resolves HEAD to the commit hash.
    assert_eq!(
        git.rev_parse(dir, "HEAD").await.expect("rev-parse"),
        log[0].hash
    );
}

#[tokio::test]
#[ignore = "requires the git binary"]
async fn diff_is_empty_tracks_worktree_changes() {
    let tmp = TempDir::new("diff");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "one\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "add a").await.expect("commit");

    assert!(
        git.diff_is_empty(dir).await.expect("clean"),
        "no changes yet"
    );

    std::fs::write(dir.join("a.txt"), "two\n").expect("modify");
    assert!(
        !git.diff_is_empty(dir).await.expect("dirty"),
        "unstaged change should be visible"
    );
}

// End-to-end check of the `-z` rename parsing: a real `git mv` must surface as a
// rename entry carrying both the new path and the original (`orig_path`).
#[tokio::test]
#[ignore = "requires the git binary"]
async fn status_reports_rename_with_orig_path() {
    let tmp = TempDir::new("rename");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("old.txt"), "hello\n").expect("write");
    git.add(dir, &[PathBuf::from("old.txt")])
        .await
        .expect("add");
    git.commit(dir, "add old").await.expect("commit");

    // Stage a rename, then read it back through the typed status.
    std::process::Command::new(vcs_git::BINARY)
        .current_dir(dir)
        .args(["mv", "old.txt", "new.txt"])
        .status()
        .expect("git mv");

    let status = git.status(dir).await.expect("status");
    let renamed = status
        .iter()
        .find(|e| e.code.starts_with('R'))
        .expect("a rename entry");
    assert_eq!(renamed.path, "new.txt", "new path");
    assert_eq!(
        renamed.orig_path.as_deref(),
        Some("old.txt"),
        "original path"
    );
}
