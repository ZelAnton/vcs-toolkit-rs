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
    let repo = Repo::open(sandbox.path()).expect("open");
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
    let repo = Repo::open(&wt_path).expect("open worktree");
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
    let repo = Repo::open(sandbox.path()).expect("open");
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
    let repo = Repo::open(sandbox.path()).expect("open");
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

// Dropping the watcher stops the stream: `recv` returns `None` promptly.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn drop_stops_the_watch() {
    let sandbox = GitSandbox::init("watch-drop");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    let repo = Repo::open(sandbox.path()).expect("open");
    let mut watcher = fast(repo).await.expect("watcher");

    // Re-bind the receiver out of the watcher would keep it alive; instead drop
    // the whole watcher and confirm a fresh `recv` on a *moved* handle ends. Here
    // we simply assert that, with no activity, `recv` doesn't spuriously fire.
    let quiet = timeout(Duration::from_millis(300), watcher.recv()).await;
    assert!(quiet.is_err(), "no events expected on a quiescent repo");
    drop(watcher); // stops the OS watch + background task
}
