//! End-to-end tests for the `vcs-core` facade against a real temporary git
//! repository. Ignored by default (require the `git` binary); run with
//! `cargo test -p vcs-core -- --ignored`.

mod common;

use std::path::Path;
use std::process::Command;

use common::TempDir;
use vcs_core::{BackendKind, ChangeKind, Repo};

/// Create a fresh git repo in `dir` with a deterministic identity and one commit.
fn init_repo(dir: &Path) {
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "initial"]);
}

#[tokio::test]
#[ignore = "requires the git binary"]
async fn open_detects_git_and_reports_changes() {
    let tmp = TempDir::new("facade");
    let dir = tmp.path();
    init_repo(dir);

    // Detection + handle.
    let repo = Repo::open(dir).expect("open");
    assert_eq!(repo.kind(), BackendKind::Git);
    assert!(repo.git().is_some() && repo.jj().is_none());

    // A committed-clean working copy has no changes.
    assert!(repo.changed_files().await.expect("status").is_empty());

    // An edit shows up as a modification; a new file as added.
    std::fs::write(dir.join("seed.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    let changes = repo.changed_files().await.expect("status");
    assert!(
        changes
            .iter()
            .any(|c| c.path == "seed.txt" && c.kind == ChangeKind::Modified)
    );
    assert!(
        changes
            .iter()
            .any(|c| c.path == "new.txt" && c.kind == ChangeKind::Added)
    );

    // Partial commit of just the tracked edit.
    repo.commit_paths(&["seed.txt".to_string()], "edit seed")
        .await
        .expect("commit_paths");
    let after = repo.changed_files().await.expect("status");
    assert!(after.iter().all(|c| c.path != "seed.txt"));
}

/// Create a fresh jj repo (git-backed) in `dir` with a deterministic identity.
fn init_jj_repo(dir: &Path) {
    let jj = |args: &[&str]| {
        let status = Command::new("jj")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("jj command");
        assert!(status.success(), "jj {args:?} failed");
    };
    jj(&["git", "init"]);
    jj(&["config", "set", "--repo", "user.name", "Test"]);
    jj(&["config", "set", "--repo", "user.email", "test@example.com"]);
}

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn open_detects_jj_and_reports_changes() {
    let tmp = TempDir::new("facade-jj");
    let dir = tmp.path();
    init_jj_repo(dir);

    // Detection routes a jj repo to the jj backend.
    let repo = Repo::open(dir).expect("open");
    assert_eq!(repo.kind(), BackendKind::Jj);
    assert!(repo.jj().is_some() && repo.git().is_none());

    // A new file in the working copy shows up (jj snapshots it) as added.
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    let changes = repo.changed_files().await.expect("status");
    assert!(
        changes
            .iter()
            .any(|c| c.path == "new.txt" && c.kind == ChangeKind::Added),
        "expected new.txt added, got {changes:?}"
    );
}
