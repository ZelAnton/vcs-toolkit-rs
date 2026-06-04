//! End-to-end tests for the typed `vcs-jj` client against a real temporary
//! repository. Ignored by default (require the `jj` binary); run with
//! `cargo test -p vcs-jj -- --ignored`.

mod common;

use std::path::Path;
use std::process::Command;

use common::TempDir;
use vcs_jj::{Jj, JjApi, WorkspaceAdd};

/// Create a fresh jj repo in `dir` with a deterministic identity.
fn init_repo(dir: &Path) {
    let jj = |args: &[&str]| {
        Command::new(vcs_jj::BINARY)
            .current_dir(dir)
            .args(args)
            .status()
            .expect("jj command");
    };
    jj(&["git", "init"]);
    jj(&["config", "set", "--repo", "user.name", "Test"]);
    jj(&["config", "set", "--repo", "user.email", "test@example.com"]);
}

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn describe_new_and_log_cycle() {
    let tmp = TempDir::new("cycle");
    let dir = tmp.path();
    init_repo(dir);
    let jj = Jj::new();

    // Fresh working copy: an empty change with no description.
    let head = jj.current_change(dir).await.expect("current_change");
    assert!(!head.change_id.is_empty());
    assert!(head.empty, "fresh working copy should be empty");
    assert_eq!(head.description, "");

    // Describe it, then read it back.
    jj.describe(dir, "hello jj").await.expect("describe");
    assert_eq!(
        jj.current_change(dir)
            .await
            .expect("current_change")
            .description,
        "hello jj"
    );

    // Start a new change; it becomes the working copy.
    jj.new_change(dir, "second change").await.expect("new");
    assert_eq!(
        jj.current_change(dir)
            .await
            .expect("current_change")
            .description,
        "second change"
    );

    // Both changes are reachable from @.
    let log = jj.log(dir, "::@", 10).await.expect("log");
    assert!(
        log.len() >= 2,
        "expected at least two changes, got {}",
        log.len()
    );
    assert!(log.iter().any(|c| c.description == "hello jj"));

    // status_text returns something without erroring; parsed status of a fresh
    // (empty) working copy is an empty change list.
    jj.status_text(dir).await.expect("status_text");
    assert!(jj.status(dir).await.expect("status").is_empty());

    // A freshly described, unconflicted working copy reports no conflict
    // (delegates to the `conflict` template on `@`).
    assert!(
        !jj.has_workingcopy_conflict(dir)
            .await
            .expect("has_workingcopy_conflict")
    );
}

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn bookmark_create_set_and_list() {
    let tmp = TempDir::new("bookmarks");
    let dir = tmp.path();
    init_repo(dir);
    let jj = Jj::new();

    jj.describe(dir, "rooted").await.expect("describe");
    Command::new(vcs_jj::BINARY)
        .current_dir(dir)
        .args(["bookmark", "create", "mark", "-r", "@"])
        .status()
        .expect("bookmark create");
    // Move it via the typed API.
    jj.bookmark_set(dir, "mark", "@")
        .await
        .expect("bookmark_set");

    let bookmarks = jj.bookmarks(dir).await.expect("bookmarks");
    assert!(
        bookmarks.iter().any(|b| b.name == "mark"),
        "expected bookmark 'mark', got {bookmarks:?}"
    );

    // `bookmarks_all` exercises the real `bookmark list -a -T` template end-to-end
    // (the hermetic test only feeds canned output). A local `mark` plus its
    // colocated `mark@git` remote-tracking entry are both reported.
    let all = jj.bookmarks_all(dir).await.expect("bookmarks_all");
    assert!(
        all.iter().any(|b| b.name == "mark" && b.remote.is_none()),
        "expected local 'mark', got {all:?}"
    );
    assert!(
        all.iter()
            .any(|b| b.name == "mark" && b.remote.as_deref() == Some("git")),
        "expected remote-tracking 'mark@git', got {all:?}"
    );
}

// Add a workspace, see it in the listing alongside `default`, then forget it —
// the core flow agent-workspace drives for jj.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn workspace_add_list_forget_cycle() {
    let tmp = TempDir::new("ws-main");
    let dir = tmp.path();
    init_repo(dir);
    let jj = Jj::new();

    // root() resolves to a real path.
    assert!(jj.root(dir).await.expect("root").exists());

    // A workspace path that doesn't exist yet, outside the repo.
    let ws_parent = TempDir::new("ws-linked");
    let ws_path = ws_parent.path().join("ws1");

    jj.workspace_add(dir, WorkspaceAdd::new("ws1", "@", ws_path.clone()))
        .await
        .expect("workspace add");

    let list = jj.workspace_list(dir).await.expect("list");
    assert!(list.iter().any(|w| w.name == "ws1"), "got {list:?}");
    assert!(list.iter().any(|w| w.name == "default"));

    jj.workspace_forget(dir, "ws1").await.expect("forget");
    assert!(
        !jj.workspace_list(dir)
            .await
            .expect("list2")
            .iter()
            .any(|w| w.name == "ws1"),
        "workspace should be gone after forget"
    );
}

// New surface against a real jj: the bound view, `reachable_bookmarks`, and
// `resolve_list` (empty when the revision has no conflicts).
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn reachable_bookmarks_and_resolve_list_cycle() {
    let tmp = TempDir::new("reachable");
    let dir = tmp.path();
    init_repo(dir);
    let jj = Jj::new();

    jj.describe(dir, "base").await.expect("describe");
    jj.bookmark_create(dir, "feature", "@")
        .await
        .expect("bookmark create");

    // The bound view drops the `dir` argument and resolves the same way.
    let reachable = jj.at(dir).reachable_bookmarks().await.expect("reachable");
    assert!(
        reachable.iter().any(|b| b.name == "feature"),
        "got {reachable:?}"
    );

    // A clean working copy has no conflicts → empty list (jj exits non-zero).
    assert!(
        jj.resolve_list(dir, "@")
            .await
            .expect("resolve_list")
            .is_empty()
    );

    // Build a real conflict: two children of base that edit the same file,
    // merged. `resolve_list` must return the actual conflicted path (this is the
    // case the format parser has to get right).
    let jj_raw = |args: &[&str]| {
        Command::new(vcs_jj::BINARY)
            .current_dir(dir)
            .args(args)
            .status()
            .expect("jj");
    };
    std::fs::write(dir.join("c.txt"), "base\n").expect("write base");
    jj_raw(&["new", "root()", "-m", "side-a"]);
    std::fs::write(dir.join("c.txt"), "aaa\n").expect("write a");
    let a = jj.current_change(dir).await.expect("a").change_id;
    jj_raw(&["new", "root()", "-m", "side-b"]);
    std::fs::write(dir.join("c.txt"), "bbb\n").expect("write b");
    let b = jj.current_change(dir).await.expect("b").change_id;
    jj_raw(&["new", &a, &b, "-m", "merge"]);

    let conflicts = jj.resolve_list(dir, "@").await.expect("resolve_list");
    assert_eq!(conflicts, ["c.txt"], "got {conflicts:?}");
}

// A renamed tracked file: jj `diff --summary` renders `R {old => new}`; status()
// must expose the real new path (and the old path), not the raw brace expression.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn status_exposes_rename_paths() {
    let tmp = TempDir::new("rename");
    let dir = tmp.path();
    init_repo(dir);
    let jj = Jj::new();

    std::fs::write(dir.join("old.rs"), "x\n").expect("write");
    jj.describe(dir, "base").await.expect("describe");
    jj.new_change(dir, "rename").await.expect("new");
    std::fs::rename(dir.join("old.rs"), dir.join("new.rs")).expect("rename");

    let changed = jj.status(dir).await.expect("status");
    let renamed = changed
        .iter()
        .find(|c| c.status == 'R')
        .unwrap_or_else(|| panic!("no rename entry in {changed:?}"));
    assert_eq!(renamed.path, "new.rs");
    assert_eq!(renamed.old_path.as_deref(), Some("old.rs"));
}
