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

#[tokio::test]
#[ignore = "requires the git binary"]
async fn git_create_then_blocking_cleanup() {
    let tmp = TempDir::new("git-cleanup");
    let dir = tmp.path();
    init_repo(dir);
    let repo = Repo::open(dir).expect("open");

    let wt = tmp.path().join("wt");
    repo.create_worktree(&wt, "feat", "HEAD")
        .await
        .expect("create_worktree");
    assert!(wt.join("seed.txt").exists(), "worktree populated");

    // Synchronous cleanup (the Drop-time path) removes it.
    repo.cleanup_worktree_blocking(&wt).expect("cleanup");
    assert!(!wt.exists(), "worktree removed");
}

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn jj_create_then_blocking_cleanup() {
    let tmp = TempDir::new("jj-cleanup");
    let dir = tmp.path();
    init_jj_repo(dir);
    let repo = Repo::open(dir).expect("open");

    let ws = tmp.path().join("ws");
    repo.create_worktree(&ws, "feat", "@")
        .await
        .expect("create_worktree");
    assert!(ws.exists(), "workspace dir created");

    // Synchronous cleanup resolves the workspace name by path, deletes the dir,
    // and forgets it.
    repo.cleanup_worktree_blocking(&ws).expect("cleanup");
    assert!(!ws.exists(), "workspace dir removed");
}

// `try_merge` probes both outcomes against a real git repo without leaving any
// trace, and the abort/continue cycle drives a real conflicted merge to ground.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn git_try_merge_and_abort_continue_cycle() {
    use vcs_core::vcs_git::GitApi;
    use vcs_core::{MergeProbe, OperationState};

    let tmp = TempDir::new("probe-git");
    let dir = tmp.path();
    init_repo(dir);
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    };

    // Diverge: "conflicting" edits seed.txt at the base; main edits it too.
    git(&["checkout", "-q", "-b", "conflicting"]);
    std::fs::write(dir.join("seed.txt"), "theirs\n").unwrap();
    git(&["commit", "-aqm", "theirs"]);
    git(&["checkout", "-q", "-"]);
    std::fs::write(dir.join("seed.txt"), "ours\n").unwrap();
    git(&["commit", "-aqm", "ours"]);
    // And a non-conflicting side branch touching a different file.
    git(&["checkout", "-q", "-b", "clean-side"]);
    std::fs::write(dir.join("side.txt"), "side\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "side"]);
    git(&["checkout", "-q", "-"]);

    let repo = Repo::open(dir).expect("open");
    let head_before = repo
        .git()
        .expect("git backend")
        .rev_parse(dir, "HEAD")
        .await
        .expect("rev-parse");

    // Conflict probe: reports the path, leaves no merge state, moves nothing.
    assert_eq!(
        repo.try_merge("conflicting").await.expect("try_merge"),
        MergeProbe::Conflicts(vec!["seed.txt".to_string()])
    );
    assert_eq!(
        repo.in_progress_state().await.expect("state"),
        OperationState::Clear
    );
    assert!(repo.changed_files().await.expect("status").is_empty());

    // Clean probe: same guarantees.
    assert!(
        repo.try_merge("clean-side")
            .await
            .expect("try_merge")
            .is_clean()
    );
    assert_eq!(
        repo.git()
            .expect("git backend")
            .rev_parse(dir, "HEAD")
            .await
            .expect("rev-parse"),
        head_before,
        "a probe must not move HEAD"
    );
    assert!(repo.changed_files().await.expect("status").is_empty());

    // Real conflicted merge → continue is blocked → abort clears it.
    assert!(
        repo.git()
            .expect("git backend")
            .merge_commit(dir, "conflicting", false, None)
            .await
            .is_err()
    );
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Conflict
    );
    assert_eq!(
        repo.abort_in_progress().await.expect("abort"),
        OperationState::Clear
    );

    // Again, but resolve and continue to completion this time.
    assert!(
        repo.git()
            .expect("git backend")
            .merge_commit(dir, "conflicting", false, None)
            .await
            .is_err()
    );
    std::fs::write(dir.join("seed.txt"), "resolved\n").unwrap();
    git(&["add", "seed.txt"]);
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Clear
    );
    assert!(repo.conflicted_files().await.expect("conflicts").is_empty());
}

// jj `try_merge`: a real two-parent conflict is reported and the probe is fully
// rolled back (working copy and op log state restored).
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn jj_try_merge_reports_conflicts_and_rolls_back() {
    use vcs_core::MergeProbe;
    use vcs_core::vcs_jj::JjApi;

    let tmp = TempDir::new("probe-jj");
    let dir = tmp.path();
    init_jj_repo(dir);
    let jj = |args: &[&str]| {
        let status = Command::new("jj")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("jj command");
        assert!(status.success(), "jj {args:?} failed");
    };

    // Two siblings off root() editing the same file; a bookmark marks side-a.
    std::fs::write(dir.join("c.txt"), "base\n").unwrap();
    jj(&["describe", "-m", "base"]);
    jj(&["new", "root()", "-m", "side-a"]);
    std::fs::write(dir.join("c.txt"), "aaa\n").unwrap();
    jj(&["bookmark", "create", "side-a", "-r", "@"]);
    jj(&["new", "root()", "-m", "side-b"]);
    std::fs::write(dir.join("c.txt"), "bbb\n").unwrap();

    let repo = Repo::open(dir).expect("open");
    let before = repo
        .jj()
        .expect("jj backend")
        .current_change(dir)
        .await
        .expect("current_change");

    assert_eq!(
        repo.try_merge("side-a").await.expect("try_merge"),
        MergeProbe::Conflicts(vec!["c.txt".to_string()])
    );

    // Rolled back: same working-copy change, no conflict, no merge child left.
    let after = repo
        .jj()
        .expect("jj backend")
        .current_change(dir)
        .await
        .expect("current_change");
    assert_eq!(after.change_id, before.change_id, "working copy restored");
    assert!(
        !repo
            .jj()
            .expect("jj backend")
            .has_workingcopy_conflict(dir)
            .await
            .expect("conflict probe"),
        "probe must not leave a conflicted working copy"
    );
}

// A multi-commit rebase that re-conflicts on the next patch: continue must
// report `Conflict` (not an error), then drive to `Clear` once resolved.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn git_continue_drives_rebase_through_two_conflicts() {
    use vcs_core::OperationState;
    use vcs_core::vcs_git::GitApi;

    let tmp = TempDir::new("rebase-restop");
    let dir = tmp.path();
    init_repo(dir);
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    };

    // A two-commit stack off the base, each editing seed.txt.
    git(&["branch", "-q", "stack"]);
    std::fs::write(dir.join("seed.txt"), "ours\n").unwrap();
    git(&["commit", "-aqm", "ours"]);
    git(&["branch", "-q", "onto"]);
    git(&["checkout", "-q", "stack"]);
    std::fs::write(dir.join("seed.txt"), "s1\n").unwrap();
    git(&["commit", "-aqm", "s1"]);
    std::fs::write(dir.join("seed.txt"), "s2\n").unwrap();
    git(&["commit", "-aqm", "s2"]);

    let repo = Repo::open(dir).expect("open");

    // The rebase stops on the first commit's conflict.
    assert!(
        repo.git()
            .expect("git backend")
            .rebase(dir, "onto")
            .await
            .is_err()
    );
    assert_eq!(
        repo.in_progress_state().await.expect("state"),
        OperationState::Rebase
    );

    // Blocked until resolved; then the continue stops on the NEXT conflict.
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Conflict
    );
    std::fs::write(dir.join("seed.txt"), "r1\n").unwrap();
    git(&["add", "seed.txt"]);
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Conflict,
        "the second patch must re-conflict"
    );

    // Resolve the second conflict; the rebase completes.
    std::fs::write(dir.join("seed.txt"), "r2\n").unwrap();
    git(&["add", "seed.txt"]);
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Clear
    );
    assert!(repo.conflicted_files().await.expect("conflicts").is_empty());
}
