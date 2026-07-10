//! End-to-end tests for the `vcs-core` facade against a real temporary git
//! repository. Ignored by default (require the `git` binary); run with
//! `cargo test -p vcs-core -- --ignored`.
//!
//! Scaffolding (throwaway repos, raw scenario steps) comes from `vcs-testkit`;
//! the typed facade under test does the rest.

use vcs_core::{BackendKind, ChangeKind, OperationState, Repo, WorktreeCreate};
use vcs_testkit::{GitSandbox, JjSandbox, git, jj};

/// A git sandbox with the one seed commit the facade tests build on.
fn seeded_git() -> GitSandbox {
    let repo = GitSandbox::init("facade");
    repo.commit_file("seed.txt", "seed\n", "initial");
    repo
}

// The batched snapshot against real git: branch, a local-tracking upstream with
// ahead/behind, dirtiness, and a Clear operation — all from `status --porcelain=v2
// --branch`.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn snapshot_git_branch_upstream_ahead_and_dirty() {
    let sandbox = seeded_git();
    let dir = sandbox.path();
    let repo = Repo::discover(dir).expect("open");
    let branch = repo
        .current_branch()
        .await
        .expect("branch")
        .expect("named branch");

    // Clean, no upstream configured yet.
    let s = repo.snapshot().await.expect("snapshot");
    assert_eq!(s.branch.as_deref(), Some(branch.as_str()));
    assert!(!s.dirty && s.change_count == 0);
    assert!(s.tracking.is_none());
    assert_eq!(s.operation, OperationState::Clear);
    assert!(s.head.is_some());

    // Track a *local* branch as upstream (no remote needed), then commit ahead and
    // leave an untracked file so the snapshot is dirty.
    git(dir, &["branch", "base"]); // base = the seed commit
    git(dir, &["branch", "--set-upstream-to=base"]); // current branch tracks base
    sandbox.commit_file("a.txt", "a\n", "ahead by one"); // +1 vs base
    sandbox.write("dirty.txt", "x\n"); // an untracked change

    let s = repo.snapshot().await.expect("snapshot");
    let tracking = s.tracking.as_ref().expect("upstream tracking");
    assert_eq!(tracking.branch, "base");
    assert_eq!(tracking.ahead, Some(1), "one commit ahead of base");
    assert_eq!(tracking.behind, Some(0));
    assert!(s.dirty);
    assert!(s.change_count >= 1);
}

// The batched snapshot against real jj: dirtiness + change count from the `@`
// change, a bookmark as the branch, and the documented no-upstream asymmetry.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn snapshot_jj_dirty_bookmark_and_no_upstream() {
    let sandbox = JjSandbox::init("snap-jj");
    let dir = sandbox.path();
    let repo = Repo::discover(dir).expect("open");

    // A fresh empty `@`: clean, no git-style upstream.
    let s = repo.snapshot().await.expect("snapshot");
    assert!(!s.dirty && s.change_count == 0);
    assert!(s.tracking.is_none());
    assert_eq!(s.operation, OperationState::Clear);
    assert!(s.head.is_some());

    // A new file makes `@` dirty (jj snapshots it) with a change count.
    sandbox.write("new.txt", "new\n");
    let s = repo.snapshot().await.expect("snapshot");
    assert!(s.dirty);
    assert!(s.change_count >= 1);

    // A bookmark on `@` surfaces as the branch.
    sandbox.bookmark("feature");
    let s = repo.snapshot().await.expect("snapshot");
    assert_eq!(s.branch.as_deref(), Some("feature"));
}

#[tokio::test]
#[ignore = "requires the git binary"]
async fn open_detects_git_and_reports_changes() {
    let sandbox = seeded_git();
    let dir = sandbox.path();

    // Detection + handle.
    let repo = Repo::discover(dir).expect("open");
    assert_eq!(repo.kind(), BackendKind::Git);
    assert!(repo.git().is_some() && repo.jj().is_none());

    // A committed-clean working copy has no changes.
    assert!(repo.changed_files().await.expect("status").is_empty());

    // An edit shows up as a modification; a new file as added.
    sandbox.write("seed.txt", "changed\n");
    sandbox.write("new.txt", "new\n");
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

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn open_detects_jj_and_reports_changes() {
    let sandbox = JjSandbox::init("facade-jj");
    let dir = sandbox.path();

    // Detection routes a jj repo to the jj backend.
    let repo = Repo::discover(dir).expect("open");
    assert_eq!(repo.kind(), BackendKind::Jj);
    assert!(repo.jj().is_some() && repo.git().is_none());

    // A new file in the working copy shows up (jj snapshots it) as added.
    sandbox.write("new.txt", "new\n");
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
    let sandbox = seeded_git();
    let dir = sandbox.path();
    let repo = Repo::discover(dir).expect("open");

    let wt = dir.join("wt");
    repo.create_worktree(WorktreeCreate::new(wt.as_path(), "feat").base("HEAD"))
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
    let sandbox = JjSandbox::init("jj-cleanup");
    let dir = sandbox.path();
    let repo = Repo::discover(dir).expect("open");

    let ws = dir.join("ws");
    repo.create_worktree(WorktreeCreate::new(ws.as_path(), "feat").base("@"))
        .await
        .expect("create_worktree");
    assert!(ws.exists(), "workspace dir created");

    // Synchronous cleanup resolves the workspace name by path, deletes the dir,
    // and forgets it.
    repo.cleanup_worktree_blocking(&ws).expect("cleanup");
    assert!(!ws.exists(), "workspace dir removed");
}

// The blocking Drop-path must refuse the repository's MAIN workspace, same as the
// async `remove_worktree` — deleting its directory would wipe the whole repo.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn jj_blocking_cleanup_refuses_the_main_workspace() {
    let sandbox = JjSandbox::init("jj-cleanup-main");
    let dir = sandbox.path();
    let repo = Repo::discover(dir).expect("open");

    let err = repo
        .cleanup_worktree_blocking(dir)
        .expect_err("the main workspace must be refused, not wiped");
    assert!(
        err.to_string().contains("main workspace"),
        "refusal message: {err}"
    );
    assert!(dir.join(".jj").exists(), "the repository must survive");
}

// `try_merge` probes both outcomes against a real git repo without leaving any
// trace, and the abort/continue cycle drives a real conflicted merge to ground.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn git_try_merge_and_abort_continue_cycle() {
    use vcs_core::vcs_git::GitApi;
    use vcs_core::{MergeProbe, OperationState};

    let sandbox = seeded_git();
    let dir = sandbox.path();

    // Diverge: "conflicting" edits seed.txt at the base; main edits it too.
    sandbox.git(&["checkout", "-q", "-b", "conflicting"]);
    sandbox.write("seed.txt", "theirs\n");
    sandbox.git(&["commit", "-aqm", "theirs"]);
    sandbox.git(&["checkout", "-q", "-"]);
    sandbox.write("seed.txt", "ours\n");
    sandbox.git(&["commit", "-aqm", "ours"]);
    // And a non-conflicting side branch touching a different file.
    sandbox.git(&["checkout", "-q", "-b", "clean-side"]);
    sandbox.commit_file("side.txt", "side\n", "side");
    sandbox.git(&["checkout", "-q", "-"]);

    let repo = Repo::discover(dir).expect("open");
    let head_before = repo
        .git()
        .expect("git backend")
        .rev_parse(dir, &vcs_core::vcs_git::RevSpec::new("HEAD").unwrap())
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
            .rev_parse(dir, &vcs_core::vcs_git::RevSpec::new("HEAD").unwrap())
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
            .merge_commit(
                dir,
                vcs_core::vcs_git::MergeCommit::branch(
                    vcs_core::vcs_git::RevSpec::new("conflicting").unwrap()
                )
            )
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
            .merge_commit(
                dir,
                vcs_core::vcs_git::MergeCommit::branch(
                    vcs_core::vcs_git::RevSpec::new("conflicting").unwrap()
                )
            )
            .await
            .is_err()
    );
    sandbox.write("seed.txt", "resolved\n");
    sandbox.git(&["add", "seed.txt"]);
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

    let sandbox = JjSandbox::init("probe-jj");
    let dir = sandbox.path();

    // Two siblings off root() editing the same file; a bookmark marks side-a.
    sandbox.write("c.txt", "base\n");
    sandbox.describe("base");
    jj(dir, &["new", "root()", "-m", "side-a"]);
    sandbox.write("c.txt", "aaa\n");
    sandbox.bookmark("side-a");
    jj(dir, &["new", "root()", "-m", "side-b"]);
    sandbox.write("c.txt", "bbb\n");

    let repo = Repo::discover(dir).expect("open");
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

    let sandbox = seeded_git();
    let dir = sandbox.path();

    // A two-commit stack off the base, each editing seed.txt.
    sandbox.branch("stack");
    sandbox.write("seed.txt", "ours\n");
    sandbox.git(&["commit", "-aqm", "ours"]);
    sandbox.branch("onto");
    sandbox.checkout("stack");
    sandbox.write("seed.txt", "s1\n");
    sandbox.git(&["commit", "-aqm", "s1"]);
    sandbox.write("seed.txt", "s2\n");
    sandbox.git(&["commit", "-aqm", "s2"]);

    let repo = Repo::discover(dir).expect("open");

    // The rebase stops on the first commit's conflict.
    assert!(
        repo.git()
            .expect("git backend")
            .rebase(dir, &vcs_core::vcs_git::RevSpec::new("onto").unwrap())
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
    sandbox.write("seed.txt", "r1\n");
    git(dir, &["add", "seed.txt"]);
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Conflict,
        "the second patch must re-conflict"
    );

    // Resolve the second conflict; the rebase completes.
    sandbox.write("seed.txt", "r2\n");
    git(dir, &["add", "seed.txt"]);
    assert_eq!(
        repo.continue_in_progress().await.expect("continue"),
        OperationState::Clear
    );
    assert!(repo.conflicted_files().await.expect("conflicts").is_empty());
}
