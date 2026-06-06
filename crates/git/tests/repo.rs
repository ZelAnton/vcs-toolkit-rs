//! End-to-end tests for the typed `vcs-git` client against a real temporary
//! repository. Ignored by default (require the `git` binary); run with
//! `cargo test -p vcs-git -- --ignored`.

use std::path::PathBuf;

// Scaffolding from vcs-testkit; most tests here drive `git.init()` themselves
// (initialisation IS the subject), so they use `TempDir` + `configure_identity`
// rather than `GitSandbox::init`. Note `configure_identity` also pins
// `core.autocrlf=false`, keeping byte-exact content assertions valid on Windows.
use vcs_git::{Git, GitApi, WorktreeAdd};
use vcs_testkit::{BareRemote, TempDir, configure_identity as configure};

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
    vcs_testkit::git(dir, &["mv", "old.txt", "new.txt"]);

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

// Add a linked worktree on a new branch, see it in the porcelain listing, then
// remove it — the core flow agent-workspace drives.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn worktree_add_list_remove_cycle() {
    let tmp = TempDir::new("wt-main");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("f.txt"), "x\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "init").await.expect("commit");

    // common_dir points at the repo's `.git`.
    let common = git.common_dir(dir).await.expect("common_dir");
    assert!(common.to_string_lossy().contains(".git"), "{common:?}");

    // is_merged on real `branch --merged` output: a branch is merged into itself.
    let cur = git.current_branch(dir).await.expect("current_branch");
    assert!(git.is_merged(dir, &cur, &cur).await.expect("is_merged"));
    // No origin configured: `remote_head_branch` is `None`, not an error
    // (the `--quiet` path).
    assert!(
        git.remote_head_branch(dir)
            .await
            .expect("remote_head_branch")
            .is_none()
    );

    // A worktree path that doesn't exist yet, outside the repo.
    let wt_parent = TempDir::new("wt-linked");
    let wt = wt_parent.path().join("feature");

    git.worktree_add(
        dir,
        WorktreeAdd::create_branch(wt.clone(), "feature", "HEAD"),
    )
    .await
    .expect("worktree add");
    assert!(git.branch_exists(dir, "feature").await.expect("exists"));

    let list = git.worktree_list(dir).await.expect("list");
    assert!(
        list.iter().any(|w| w.branch.as_deref() == Some("feature")),
        "new worktree should be listed, got {list:?}"
    );

    git.worktree_remove(dir, &wt, true).await.expect("remove");
    assert!(
        !git.worktree_list(dir)
            .await
            .expect("list2")
            .iter()
            .any(|w| w.branch.as_deref() == Some("feature")),
        "worktree should be gone after remove"
    );
}

// New surface against a real git: the bound view (`git.at(dir)`) resolves the
// same as the dir-taking call, and `rev_parse_short` abbreviates HEAD.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn bound_view_and_rev_parse_short() {
    let tmp = TempDir::new("bound");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("f.txt"), "x\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "c1").await.expect("commit");

    // Bound view yields the same current branch as the dir-taking call.
    let bound = git.at(dir);
    assert_eq!(
        bound.current_branch().await.expect("branch"),
        git.current_branch(dir).await.expect("branch")
    );

    // `rev_parse_short` is a prefix of the full hash.
    let full = git.rev_parse(dir, "HEAD").await.expect("rev_parse");
    let short = bound.rev_parse_short("HEAD").await.expect("short");
    assert!(
        !short.is_empty() && full.starts_with(&short),
        "{short} vs {full}"
    );
}

// diff_text on an unborn repo (no commits) must not error on the unresolvable
// HEAD — it diffs against the empty tree and shows the staged additions.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn diff_text_works_on_unborn_repo() {
    let tmp = TempDir::new("unborn");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("f.txt"), "hello\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");

    assert!(git.is_unborn(dir).await.expect("is_unborn"));
    let diff = git
        .diff_text(dir, vcs_git::DiffSpec::WorkingTree)
        .await
        .expect("diff_text must not error on unborn repo");
    assert!(diff.contains("f.txt"), "expected the new file in: {diff}");
}

// A real merge conflict must surface through `conflicted_files`, and a tree
// whose only change is an untracked file must read as tracked-clean.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn conflicted_files_and_status_tracked() {
    let tmp = TempDir::new("conflict");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    let main = git.current_branch(dir).await.expect("branch");

    // Diverge: edit a.txt on both sides.
    git.create_branch(dir, "other").await.expect("branch");
    std::fs::write(dir.join("a.txt"), "main change\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "main edit").await.expect("commit");
    git.checkout(dir, "other").await.expect("checkout");
    std::fs::write(dir.join("a.txt"), "other change\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "other edit").await.expect("commit");

    // No conflicts before the merge.
    assert!(
        git.conflicted_files(dir)
            .await
            .expect("conflicted_files")
            .is_empty()
    );

    // The conflicting merge fails and leaves a.txt unmerged.
    assert!(git.merge_commit(dir, &main, false, None).await.is_err());
    assert_eq!(
        git.conflicted_files(dir).await.expect("conflicted_files"),
        ["a.txt"]
    );
    git.merge_abort(dir).await.expect("merge_abort");

    // An untracked file is uncommitted-dirty but tracked-clean.
    std::fs::write(dir.join("new.txt"), "untracked\n").expect("write");
    assert!(!git.status(dir).await.expect("status").is_empty());
    assert!(
        git.status_tracked(dir)
            .await
            .expect("status_tracked")
            .is_empty()
    );
}

// `merge_commit` with `no_ff` must create a real 2-parent merge commit even when
// a fast-forward was possible — the headline subtlety of the flag, and the one
// the conflict test (which only asserts the failing path) can't catch.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn merge_commit_no_ff_creates_a_merge_commit() {
    let tmp = TempDir::new("mergenoff");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    let main = git.current_branch(dir).await.expect("branch");

    // A feature branch one commit ahead; main does not move, so a plain merge
    // would fast-forward.
    git.create_branch(dir, "feature").await.expect("branch");
    git.checkout(dir, "feature").await.expect("checkout");
    std::fs::write(dir.join("b.txt"), "feature\n").expect("write");
    git.add(dir, &[PathBuf::from("b.txt")]).await.expect("add");
    git.commit(dir, "feature work").await.expect("commit");
    git.checkout(dir, &main).await.expect("checkout");

    git.merge_commit(dir, "feature", true, Some("merge feature".into()))
        .await
        .expect("merge_commit");

    // A 2-parent merge commit resolves `HEAD^2`; a fast-forward would not.
    assert!(
        git.resolve_commit(dir, "HEAD^2").await.is_ok(),
        "no_ff merge must create a 2-parent merge commit (HEAD^2 should resolve)"
    );
    assert_eq!(
        git.last_commit_message(dir).await.expect("msg").trim(),
        "merge feature"
    );
}

// `is_merged` must distinguish a branch already merged into the target from one
// that isn't — the hermetic test only feeds canned output, so this pins the real
// `branch --merged` semantics.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn is_merged_distinguishes_merged_and_unmerged() {
    let tmp = TempDir::new("ismerged");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    let main = git.current_branch(dir).await.expect("branch");

    // `done` branches off base and is merged back into main.
    git.create_branch(dir, "done").await.expect("branch");
    git.checkout(dir, "done").await.expect("checkout");
    std::fs::write(dir.join("b.txt"), "done\n").expect("write");
    git.add(dir, &[PathBuf::from("b.txt")]).await.expect("add");
    git.commit(dir, "done work").await.expect("commit");
    git.checkout(dir, &main).await.expect("checkout");
    git.merge_commit(dir, "done", true, Some("merge done".into()))
        .await
        .expect("merge_commit");

    // `pending` has a commit that was never merged into main.
    git.create_branch(dir, "pending").await.expect("branch");
    git.checkout(dir, "pending").await.expect("checkout");
    std::fs::write(dir.join("c.txt"), "pending\n").expect("write");
    git.add(dir, &[PathBuf::from("c.txt")]).await.expect("add");
    git.commit(dir, "pending work").await.expect("commit");
    git.checkout(dir, &main).await.expect("checkout");

    assert!(
        git.is_merged(dir, "done", &main)
            .await
            .expect("is_merged done"),
        "`done` was merged into main"
    );
    assert!(
        !git.is_merged(dir, "pending", &main)
            .await
            .expect("is_merged pending"),
        "`pending` was never merged into main"
    );
}

// `switch_with_stash` carries dirty state (tracked + untracked) across a branch
// switch, and restores it on the original branch when the checkout fails.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn switch_with_stash_carries_changes_and_restores_on_failure() {
    let tmp = TempDir::new("switch");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir); // pins core.autocrlf=false — the stash round-trip re-checks files out
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    git.create_branch(dir, "feature").await.expect("branch");

    // Dirty tree: a tracked edit and an untracked file both travel.
    std::fs::write(dir.join("a.txt"), "edited\n").expect("write");
    std::fs::write(dir.join("new.txt"), "untracked\n").expect("write");
    git.switch_with_stash(dir, "feature").await.expect("switch");
    assert_eq!(git.current_branch(dir).await.expect("branch"), "feature");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("read"),
        "edited\n"
    );
    assert!(dir.join("new.txt").exists(), "untracked file must travel");

    // A failing checkout restores the dirty state where it was.
    assert!(
        git.switch_with_stash(dir, "no-such-branch").await.is_err(),
        "checkout of a missing branch must fail"
    );
    assert_eq!(git.current_branch(dir).await.expect("branch"), "feature");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("read"),
        "edited\n"
    );
    assert!(dir.join("new.txt").exists(), "untracked file must survive");
}

// Clone from a local bare fixture: the worktree materialises and history reads.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn clone_repo_from_local_bare_remote() {
    let remote = BareRemote::seeded("clone");
    let tmp = TempDir::new("clone-dest");
    let dest = tmp.path().join("cloned");
    let git = Git::new();

    git.clone_repo(
        remote.url().as_str(),
        &dest,
        vcs_git::CloneSpec::new().branch("main"),
    )
    .await
    .expect("clone");
    assert!(dest.join("seed.txt").exists(), "worktree materialised");
    let log = git.log(&dest, 10).await.expect("log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "seed");
    assert_eq!(git.current_branch(&dest).await.expect("branch"), "main");
}

// Tag cycle, file-at-revision, config and remote management round-trips.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn tags_show_config_and_remotes_round_trip() {
    let tmp = TempDir::new("misc");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
    std::fs::write(dir.join("sub").join("f.txt"), "v1\n").expect("write");
    git.add(dir, &[PathBuf::from("sub/f.txt")])
        .await
        .expect("add");
    git.commit(dir, "base").await.expect("commit");

    // Tags: lightweight + annotated, list, delete.
    git.tag_create(dir, "v1", None).await.expect("tag");
    git.tag_create_annotated(dir, "v1.1", "first release", None)
        .await
        .expect("tag -a");
    assert_eq!(git.tag_list(dir).await.expect("list"), ["v1", "v1.1"]);
    git.tag_delete(dir, "v1").await.expect("delete");
    assert_eq!(git.tag_list(dir).await.expect("list"), ["v1.1"]);

    // show_file resolves a subdir path — exercised with BACKSLASH separators
    // on purpose (the Windows trap; normalised internally).
    assert_eq!(
        git.show_file(dir, "HEAD", r"sub\f.txt")
            .await
            .expect("show"),
        "v1"
    );

    // Config: set → get → unset key reads as None.
    git.config_set(dir, "vcs.test", "yes").await.expect("set");
    assert_eq!(
        git.config_get(dir, "vcs.test").await.expect("get"),
        Some("yes".to_string())
    );
    assert_eq!(
        git.config_get(dir, "vcs.unset-key").await.expect("get"),
        None
    );

    // Remotes: add, then re-point.
    git.remote_add(dir, "up", "https://example.com/a.git")
        .await
        .expect("remote add");
    git.remote_set_url(dir, "up", "https://example.com/b.git")
        .await
        .expect("set-url");
    assert_eq!(
        git.remote_url(dir, "up").await.expect("url"),
        "https://example.com/b.git"
    );
}

// blame maps lines to the commits that introduced them; cherry-pick and revert
// transplant/undo a commit.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn blame_cherry_pick_and_revert_cycle() {
    let tmp = TempDir::new("blame");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir); // pins core.autocrlf=false — cherry-pick/revert re-check files out
    std::fs::write(dir.join("f.txt"), "one\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "first").await.expect("commit");
    let first = git.rev_parse(dir, "HEAD").await.expect("rev");
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "second").await.expect("commit");
    let second = git.rev_parse(dir, "HEAD").await.expect("rev");

    let blame = git.blame(dir, "f.txt", None).await.expect("blame");
    assert_eq!(blame.len(), 2);
    assert_eq!(blame[0].commit, first, "line 1 from the first commit");
    assert_eq!(blame[1].commit, second, "line 2 from the second commit");
    assert_eq!(blame[0].author, "Test");
    assert!(blame[0].author_time > 1_500_000_000, "sane epoch");
    assert_eq!(blame[1].content, "two");

    // Transplant "second" onto a branch cut at "first".
    git.create_branch(dir, "side").await.expect("branch");
    git.checkout(dir, "side").await.expect("checkout");
    git.reset_hard(dir, &first).await.expect("reset");
    git.cherry_pick(dir, &second).await.expect("cherry-pick");
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).expect("read"),
        "one\ntwo\n"
    );
    // And revert it again.
    git.revert(dir, "HEAD").await.expect("revert");
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).expect("read"),
        "one\n"
    );
}

// rebase_skip: only the `apply` backend refuses an emptied patch ("nothing to
// commit … skip this patch") — the default merge backend auto-drops it.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn rebase_skip_finishes_an_emptied_patch() {
    let tmp = TempDir::new("skip");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);
    vcs_testkit::git(dir, &["config", "rebase.backend", "apply"]);

    std::fs::write(dir.join("f.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    let main = git.current_branch(dir).await.expect("branch");
    // A stack commit whose content the base branch then also adopts.
    git.create_branch(dir, "stack").await.expect("branch");
    git.checkout(dir, "stack").await.expect("checkout");
    std::fs::write(dir.join("f.txt"), "same change\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "stack change").await.expect("commit");
    git.checkout(dir, &main).await.expect("checkout");
    std::fs::write(dir.join("f.txt"), "upstream version\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "upstream change").await.expect("commit");
    git.checkout(dir, "stack").await.expect("checkout");

    // The rebase conflicts; resolving to EXACTLY the upstream content empties
    // the patch, so --continue refuses and --skip is the way out.
    assert!(git.rebase(dir, &main).await.is_err(), "conflict expected");
    std::fs::write(dir.join("f.txt"), "upstream version\n").expect("resolve");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    assert!(
        git.rebase_continue(dir).await.is_err(),
        "apply backend refuses the emptied patch"
    );
    git.rebase_skip(dir).await.expect("rebase --skip");
    assert!(
        !git.is_rebase_in_progress(dir).await.expect("state"),
        "rebase finished after the skip"
    );
}

// capabilities round-trips against the real binary on PATH.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn capabilities_probe_real_binary() {
    let caps = Git::new().capabilities().await.expect("capabilities");
    assert!(caps.is_supported(), "got {:?}", caps.version);
    caps.ensure_supported().expect("supported");
}

// The hardened profile must suppress repo-local hooks (the code-execution
// vector when driving an untrusted checkout) while a plain client runs them.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn hardened_client_suppresses_repo_hooks() {
    let tmp = TempDir::new("harden");
    let dir = tmp.path();
    let plain = Git::new();
    plain.init(dir).await.expect("init");
    configure(dir);

    // A pre-commit hook that drops a marker file when it runs.
    let hooks = dir.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    std::fs::write(
        hooks.join("pre-commit"),
        "#!/bin/sh\necho ran >> hook-marker.txt\n",
    )
    .expect("write hook");

    // Plain client: the hook fires.
    std::fs::write(dir.join("f.txt"), "one\n").expect("write");
    plain
        .add(dir, &[PathBuf::from("f.txt")])
        .await
        .expect("add");
    plain.commit(dir, "one").await.expect("commit");
    assert!(dir.join("hook-marker.txt").exists(), "hook ran unhardened");
    let runs_before = std::fs::read_to_string(dir.join("hook-marker.txt"))
        .expect("read")
        .lines()
        .count();

    // Hardened client: the hook must NOT fire.
    let hardened = Git::hardened();
    std::fs::write(dir.join("f.txt"), "two\n").expect("write");
    hardened
        .add(dir, &[PathBuf::from("f.txt")])
        .await
        .expect("add");
    hardened.commit(dir, "two").await.expect("commit");
    let runs_after = std::fs::read_to_string(dir.join("hook-marker.txt"))
        .expect("read")
        .lines()
        .count();
    assert_eq!(runs_after, runs_before, "hook suppressed under harden()");
}

// The typed conflict model round-trips a REAL conflicted file: parse →
// resolve(Theirs) → write back → stage → the conflict is gone.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn conflict_model_resolves_a_real_conflict() {
    use vcs_git::conflict::{ResolutionSide, parse_conflicts, render, resolve};

    let tmp = TempDir::new("conflict-model");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    git.create_branch(dir, "other").await.expect("branch");
    std::fs::write(dir.join("a.txt"), "ours\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "ours").await.expect("commit");
    git.checkout(dir, "other").await.expect("checkout");
    std::fs::write(dir.join("a.txt"), "theirs\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "theirs").await.expect("commit");
    let main = "-"; // previous branch
    let _ = main;
    assert!(
        git.merge_commit(dir, "@{-1}", false, None).await.is_err(),
        "conflict expected"
    );

    let content = std::fs::read_to_string(dir.join("a.txt")).expect("read");
    let segments = parse_conflicts(&content).expect("parse real markers");
    assert_eq!(render(&segments), content, "byte-exact roundtrip");
    let resolved = resolve(&segments, ResolutionSide::Theirs).expect("resolve");
    assert!(!resolved.contains("<<<<<<<"), "markers gone");
    std::fs::write(dir.join("a.txt"), &resolved).expect("write resolved");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    assert!(
        git.conflicted_files(dir)
            .await
            .expect("conflicted")
            .is_empty(),
        "conflict cleared after writing the resolution"
    );
}
