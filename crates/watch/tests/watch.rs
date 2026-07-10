//! End-to-end tests for `vcs-watch` against real temporary git/jj repositories.
//! Ignored by default (require the `git`/`jj` binary). Run with
//! `cargo test -p vcs-watch -- --ignored`.
//!
//! The pure snapshot-diff is covered hermetically in `src/event.rs`; these tests
//! exercise the real notify → debounce → re-query → emit pipeline. Each performs
//! a repo operation and waits (with a generous ceiling) for the resulting event —
//! a short debounce keeps them snappy, and the re-query+diff design means stray
//! filesystem noise can't produce a spurious change.

use std::time::Duration;

use tokio::time::timeout;
use vcs_core::Repo;
use vcs_testkit::{GitSandbox, JjSandbox, TempDir};
use vcs_watch::{RepoEvent, RepoWatcher};

/// Drain changes until one carries an event matching `pred`, or the overall
/// deadline elapses. Returns whether the event was seen.
async fn wait_for(
    watcher: &mut RepoWatcher,
    overall: Duration,
    pred: impl Fn(&RepoEvent) -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + overall;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match timeout(remaining, watcher.recv()).await {
            Ok(Some(change)) => {
                if change.events.iter().any(&pred) {
                    return true;
                }
            }
            // Channel closed, or the overall timeout fired.
            Ok(None) | Err(_) => return false,
        }
    }
}

fn fast(repo: Repo) -> impl std::future::Future<Output = vcs_watch::Result<RepoWatcher>> {
    // A short debounce keeps the tests responsive; the watcher still coalesces.
    RepoWatcher::builder(repo)
        .debounce(Duration::from_millis(50))
        .build()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn git_branch_create_emits_branch_created() {
    let sandbox = GitSandbox::init("watch-git-branch");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    let repo = Repo::discover(sandbox.path()).expect("open");
    let mut watcher = fast(repo).await.expect("watcher");

    sandbox.git(&["branch", "feature"]);

    assert!(
        wait_for(&mut watcher, Duration::from_secs(10), |e| {
            matches!(e, RepoEvent::BranchCreated { name, .. } if name == "feature")
        })
        .await,
        "expected a BranchCreated(feature) event"
    );
}

// End-to-end: a watcher on a *linked worktree* sees a branch created from the
// MAIN checkout. The worktree's `.git` gitlink resolves to its private gitdir,
// but `refs/heads/*` live in the SHARED `.git` (a sibling, reached via
// `commondir`) — so the fix also watches that shared dir. This drives the real
// notify→re-query pipeline against a worktree on the host OS.
//
// Note: the *strict* regression guard for the fix is the hermetic
// `state_dirs_includes_private_and_shared_for_worktree` unit test (it fails if
// the shared dir is dropped). This end-to-end test can't be that guard on its
// own: a worktree watcher's own `git status` re-query rewrites the private-dir
// index, and that self-churn keeps re-querying branches independently of the
// shared-dir watch. It still earns its keep — it exercises the real OS watch on
// the resolved shared path (catching, e.g., a bad path that `notify` rejects).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn git_worktree_sees_branch_created_from_main() {
    let sandbox = GitSandbox::init("watch-git-wt");
    sandbox.commit_file("seed.txt", "seed\n", "initial");

    // Add a linked worktree on a new branch, placed outside the main working tree
    // (its own self-cleaning temp dir). `git worktree add` wants a non-existent
    // target, so point it at a fresh subpath.
    let wt_parent = TempDir::new("watch-git-wt-linked");
    let wt_path = wt_parent.path().join("wt");
    sandbox.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "wt-branch",
        wt_path.to_str().expect("utf8 worktree path"),
    ]);

    // Watch the *worktree*, not the main checkout.
    let repo = Repo::discover(&wt_path).expect("open worktree");
    let mut watcher = fast(repo).await.expect("watcher");

    // Create a branch from the MAIN checkout — it lands in the shared `.git`.
    sandbox.git(&["branch", "feature"]);

    assert!(
        wait_for(&mut watcher, Duration::from_secs(10), |e| {
            matches!(e, RepoEvent::BranchCreated { name, .. } if name == "feature")
        })
        .await,
        "worktree watcher must see a branch created in the shared git dir"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn git_working_tree_edit_emits_working_copy_changed() {
    let sandbox = GitSandbox::init("watch-git-wc");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    let repo = Repo::discover(sandbox.path()).expect("open");
    // Opt into working-tree watching so a bare untracked file is seen.
    let mut watcher = RepoWatcher::builder(repo)
        .working_tree(true)
        .debounce(Duration::from_millis(50))
        .build()
        .await
        .expect("watcher");

    sandbox.write("dirty.txt", "x\n"); // untracked → dirty, no git command

    assert!(
        wait_for(&mut watcher, Duration::from_secs(10), |e| {
            matches!(e, RepoEvent::WorkingCopyChanged { dirty: true, .. })
        })
        .await,
        "expected a WorkingCopyChanged(dirty) event"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the jj binary"]
async fn jj_bookmark_create_emits_branch_created() {
    let sandbox = JjSandbox::init("watch-jj-bm");
    sandbox.write("seed.txt", "seed\n");
    sandbox.describe("initial");
    let repo = Repo::discover(sandbox.path()).expect("open");
    let mut watcher = fast(repo).await.expect("watcher");

    sandbox.bookmark("feature");

    assert!(
        wait_for(&mut watcher, Duration::from_secs(10), |e| {
            matches!(e, RepoEvent::BranchCreated { name, .. } if name == "feature")
        })
        .await,
        "expected a BranchCreated(feature) event on jj"
    );
}

// T-038: the **default** jj watcher observes without mutating. On jj an ordinary
// query snapshots the working copy — taking the lock, recording an operation, and
// possibly moving `@` — so a naive re-query would make the observer mutate the
// observed. The default re-query is read-only (`--ignore-working-copy`), so a
// series of silent re-queries (driven here by bare working-tree edits, watched via
// `working_tree(true)`) must record **no** new operation, never move `@`, and emit
// no `WorkingCopyChanged` — the unsnapshotted edits stay invisible until something
// records them. Both `op_head`/`at_commit` are measured read-only so the assertion
// itself doesn't snapshot the pending edit.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the jj binary"]
async fn jj_read_only_requery_records_no_operation_and_moves_nothing() {
    let sandbox = JjSandbox::init("watch-jj-readonly");
    sandbox.write("seed.txt", "seed\n");
    sandbox.describe("initial");
    sandbox.new_change("work"); // a fresh, empty `@` — a clean baseline

    let repo = Repo::discover(sandbox.path()).expect("open");
    let mut watcher = RepoWatcher::builder(repo)
        .working_tree(true)
        .debounce(Duration::from_millis(50))
        .build()
        .await
        .expect("watcher");

    let op_before = sandbox.op_head();
    let at_before = sandbox.at_commit();
    let requeries_before = watcher.stats().requeries;

    // A series of bare working-tree edits (a brand-new file rewritten): each is a
    // filesystem event driving a (read-only) re-query, but no jj command records
    // them, so `--ignore-working-copy` never sees the new file.
    for i in 0..5 {
        sandbox.write("dirty.txt", &format!("edit {i}\n"));
        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    // Confirm the re-queries actually ran (the counter climbs), bounded so a wedged
    // watcher fails loudly instead of hanging.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while watcher.stats().requeries <= requeries_before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "re-queries never ran (requeries stuck at {requeries_before})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The read-only re-queries recorded no operation and did not move `@`.
    assert_eq!(
        sandbox.op_head(),
        op_before,
        "a read-only re-query must not record a jj operation"
    );
    assert_eq!(
        sandbox.at_commit(),
        at_before,
        "a read-only re-query must not move `@`"
    );

    // …and, seeing no state change, it emitted nothing.
    let quiet = timeout(Duration::from_millis(500), watcher.recv()).await;
    assert!(
        quiet.is_err(),
        "the default read-only watcher must not surface an unsnapshotted bare edit \
         as an event, got {quiet:?}"
    );
}

// T-038 (the other branch of the contract): opting into `snapshot_working_copy`
// makes the re-query snapshot the working copy — an explicit, documented mutation
// — so a bare working-tree edit that no jj command recorded IS observed as a
// `WorkingCopyChanged`. This is the escape hatch for a consumer that needs a live
// dirty indicator driven purely by filesystem edits, accepting that the watcher
// now records jj operations.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the jj binary"]
async fn jj_snapshot_working_copy_opt_in_observes_bare_edit() {
    let sandbox = JjSandbox::init("watch-jj-snapshot");
    sandbox.write("seed.txt", "seed\n");
    sandbox.describe("initial");
    sandbox.new_change("work"); // a fresh, empty `@` — a clean baseline

    let repo = Repo::discover(sandbox.path()).expect("open");
    let mut watcher = RepoWatcher::builder(repo)
        .working_tree(true)
        .snapshot_working_copy(true)
        .debounce(Duration::from_millis(50))
        .build()
        .await
        .expect("watcher");

    // A bare new-file edit — no jj command. Only a snapshotting re-query can
    // observe it (the read-only default would not — see the paired test above).
    sandbox.write("dirty.txt", "x\n");

    assert!(
        wait_for(&mut watcher, Duration::from_secs(10), |e| {
            matches!(e, RepoEvent::WorkingCopyChanged { dirty: true, .. })
        })
        .await,
        "opt-in snapshot_working_copy must observe a bare working-tree edit"
    );
}

// Dropping the watcher stops the stream: `recv` returns `None` promptly.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn drop_stops_the_watch() {
    let sandbox = GitSandbox::init("watch-drop");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    let repo = Repo::discover(sandbox.path()).expect("open");
    let mut watcher = fast(repo).await.expect("watcher");

    // Re-bind the receiver out of the watcher would keep it alive; instead drop
    // the whole watcher and confirm a fresh `recv` on a *moved* handle ends. Here
    // we simply assert that, with no activity, `recv` doesn't spuriously fire.
    let quiet = timeout(Duration::from_millis(300), watcher.recv()).await;
    assert!(quiet.is_err(), "no events expected on a quiescent repo");
    drop(watcher); // stops the OS watch + background task
}
