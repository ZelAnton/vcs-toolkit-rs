//! End-to-end tests for the typed `vcs-git` client against a real temporary
//! repository. Ignored by default (require the `git` binary); run with
//! `cargo test -p vcs-git -- --ignored`.

use std::path::{Path, PathBuf};

// Scaffolding from vcs-testkit; most tests here drive `git.init()` themselves
// (initialisation IS the subject), so they use `TempDir` + `configure_identity`
// rather than `GitSandbox::init`. Note `configure_identity` also pins
// `core.autocrlf=false`, keeping byte-exact content assertions valid on Windows.
use vcs_git::{
    AnnotatedTag, CheckoutTarget, CommitPaths, Git, GitApi, MergeCheck, MergeCommit, RefName,
    RevSpec, WorktreeAdd, WorktreeRemove,
};
use vcs_testkit::{BareRemote, TempDir, configure_identity as configure};

// Terse constructors for the validated newtypes in test call sites; the literals
// here are always valid, so `unwrap` is fine in tests.
fn rn(s: &str) -> RefName {
    RefName::new(s).unwrap()
}
fn rv(s: &str) -> RevSpec {
    RevSpec::new(s).unwrap()
}
fn ct(s: &str) -> CheckoutTarget {
    if s == "-" {
        CheckoutTarget::Previous
    } else {
        CheckoutTarget::Ref(rv(s))
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
    assert_eq!(status[0].path, std::path::Path::new("file.txt"));

    // Stage + commit, then status is clean.
    git.add(dir, &[PathBuf::from("file.txt")])
        .await
        .expect("add");
    git.commit(dir, "initial commit").await.expect("commit");
    assert!(git.status(dir).await.expect("status").is_empty());

    // Log reflects the commit, with the enriched fields.
    let log = git.log(dir, &rv("HEAD"), 10).await.expect("log");
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
    let branch = git
        .current_branch(dir)
        .await
        .expect("current_branch")
        .expect("on a branch");
    assert!(!branch.is_empty());
    git.create_branch(dir, &rn("feature"))
        .await
        .expect("create_branch");
    git.checkout(dir, &ct("feature")).await.expect("checkout");
    assert_eq!(
        git.current_branch(dir).await.expect("branch").as_deref(),
        Some("feature")
    );
    let branches = git.branches(dir).await.expect("branches");
    assert!(branches.iter().any(|b| b.name == "feature"));

    // rev_parse resolves HEAD to the commit hash.
    assert_eq!(
        git.rev_parse(dir, &rv("HEAD")).await.expect("rev-parse"),
        log[0].hash
    );
}

// A freshly `init`'d repo (no commits) is on an **unborn** branch. `current_branch`
// must return that branch name, not error — the old `rev-parse --abbrev-ref HEAD`
// exited 128 here; `symbolic-ref --quiet --short HEAD` returns it cleanly.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn current_branch_on_unborn_repo_returns_the_branch() {
    let tmp = TempDir::new("unborn");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);
    let branch = git
        .current_branch(dir)
        .await
        .expect("current_branch must not error on an unborn repo");
    // git's default initial branch is "main" or "master" depending on config;
    // either way it must be a non-empty Some, never None or an error.
    let branch = branch.expect("an unborn repo is still on a (named) branch");
    assert!(!branch.is_empty(), "unborn branch name should be non-empty");
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
// rename entry carrying both the new path and the original (`old_path`).
#[tokio::test]
#[ignore = "requires the git binary"]
async fn status_reports_rename_with_old_path() {
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
    assert_eq!(renamed.path, std::path::Path::new("new.txt"), "new path");
    assert_eq!(
        renamed.old_path.as_deref(),
        Some(std::path::Path::new("old.txt")),
        "original path"
    );
}

// T-050: a filename whose bytes are NOT valid UTF-8 (legal on Unix) survives a
// full `status → add / commit_paths` round trip. The path from `status` is a
// `PathBuf` carrying the exact bytes; fed straight back into the mutating API it
// addresses the SAME file — not a `U+FFFD`-mangled neighbour, as the old
// `String::from_utf8_lossy` decode would have produced.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the git binary"]
async fn non_utf8_path_round_trips_status_to_commit() {
    use std::os::unix::ffi::OsStrExt;

    let tmp = TempDir::new("nonutf8");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);

    let name = vcs_testkit::non_utf8_filename();
    std::fs::write(dir.join(&name), "hi\n").expect("write non-utf8 file");

    // `status` carries the exact bytes (no U+FFFD substitution).
    let status = git.status(dir).await.expect("status");
    let entry = status
        .iter()
        .find(|e| e.code == "??")
        .expect("the untracked non-UTF-8 file must appear in status");
    assert_eq!(
        entry.path.as_os_str().as_bytes(),
        name.as_bytes(),
        "the status path must carry the exact non-UTF-8 bytes, not U+FFFD"
    );

    // Feed the status path straight back into BOTH mutating APIs.
    let staged = entry.path.clone();
    git.add(dir, std::slice::from_ref(&staged))
        .await
        .expect("add must accept the non-UTF-8 path");
    git.commit_paths(dir, CommitPaths::new([staged.clone()], "add non-utf8 file"))
        .await
        .expect("commit_paths must accept the non-UTF-8 path");

    // Committed → the working tree is clean, so the SAME file was addressed (a
    // mangled path would have left the real file untracked and committed nothing,
    // or errored on a non-existent pathspec).
    let after = git.status(dir).await.expect("status after commit");
    assert!(
        after.is_empty(),
        "the non-UTF-8 path must be committed (same file); still dirty: {after:?}"
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
    let cur = git
        .current_branch(dir)
        .await
        .expect("current_branch")
        .expect("on a branch");
    assert!(
        git.is_merged(dir, MergeCheck::branch(rn(&cur)).into_base(rv(&cur)))
            .await
            .expect("is_merged")
    );
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
        WorktreeAdd::create_branch(wt.clone(), rn("feature"), rv("HEAD")),
    )
    .await
    .expect("worktree add");
    assert!(
        git.branch_exists(dir, &rn("feature"))
            .await
            .expect("exists")
    );

    let list = git.worktree_list(dir).await.expect("list");
    assert!(
        list.iter().any(|w| w.branch.as_deref() == Some("feature")),
        "new worktree should be listed, got {list:?}"
    );

    git.worktree_remove(dir, WorktreeRemove::new(&wt).force())
        .await
        .expect("remove");
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
    let full = git.rev_parse(dir, &rv("HEAD")).await.expect("rev_parse");
    let short = bound.rev_parse_short(&rv("HEAD")).await.expect("short");
    assert!(
        !short.is_empty() && full.starts_with(&short),
        "{short} vs {full}"
    );
}

// Whether the installed git can create a SHA-256 repository. `--object-format=
// sha256` is rejected by git built without the (historically experimental)
// SHA-256 support, so the SHA-256 unborn-diff test skips rather than fails there.
// A bare `std::process::Command` (not the panic-on-failure `vcs_testkit::git`) so
// an unsupported git is a clean `false`, not a panic.
fn git_supports_sha256() -> bool {
    let probe = TempDir::new("sha256-probe");
    std::process::Command::new("git")
        .args(["init", "--object-format=sha256"])
        .arg(probe.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// Shared body for the unborn working-tree diff/stat tests. `dir` must already be
// an initialised, still-unborn repo of the object format under test. On an unborn
// repo `HEAD` is unresolvable, so the working-tree diff/stat must fall back to the
// repo's *format-correct* empty tree (`empty_tree_oid`, not the SHA-1 constant)
// and report the staged addition instead of erroring. `expected_oid_len` pins the
// hash width so a SHA-256 repo isn't silently handed a 40-hex SHA-1 id.
async fn unborn_working_tree_diff_and_stat(dir: &Path, expected_oid_len: usize) {
    let git = Git::new();
    configure(dir);
    std::fs::write(dir.join("f.txt"), "hello\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    assert!(git.is_unborn(dir).await.expect("is_unborn"));

    // The empty-tree id tracks the repo's active object format.
    let oid = git.empty_tree_oid(dir).await.expect("empty_tree_oid");
    assert_eq!(
        oid.len(),
        expected_oid_len,
        "empty-tree oid width must match the repo's object format, got {oid}"
    );

    // Working-tree diff shows the addition instead of erroring on the unborn HEAD.
    let diff = git
        .diff_text(dir, vcs_git::DiffSpec::WorkingTree)
        .await
        .expect("diff_text must not error on unborn repo");
    assert!(diff.contains("f.txt"), "expected the new file in: {diff}");

    // Stat against the format-correct empty tree counts the added file.
    let stat = git.diff_stat(dir, &rv(&oid)).await.expect("diff_stat");
    assert_eq!(stat.files_changed, 1, "one added file expected: {stat:?}");
    assert!(stat.insertions >= 1, "expected an insertion: {stat:?}");
}

// SHA-1 (git's default): the unborn working-tree diff/stat resolves the 40-hex
// empty tree and reports the addition.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn diff_text_and_stat_work_on_unborn_sha1_repo() {
    let tmp = TempDir::new("unborn-sha1");
    let dir = tmp.path();
    Git::new().init(dir).await.expect("init");
    unborn_working_tree_diff_and_stat(dir, 40).await;
}

// SHA-256: the SHA-1 empty-tree id doesn't exist here, so the code must resolve
// the 64-hex empty tree from git. Skips when the installed git lacks SHA-256
// support (see `git_supports_sha256`).
#[tokio::test]
#[ignore = "requires the git binary"]
async fn diff_text_and_stat_work_on_unborn_sha256_repo() {
    if !git_supports_sha256() {
        eprintln!("skipping: installed git lacks --object-format=sha256 support");
        return;
    }
    let tmp = TempDir::new("unborn-sha256");
    let dir = tmp.path();
    // `git.init()` only makes a SHA-1 repo; init the SHA-256 one via raw git.
    vcs_testkit::git(dir, &["init", "--object-format=sha256"]);
    unborn_working_tree_diff_and_stat(dir, 64).await;
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
    let main = git
        .current_branch(dir)
        .await
        .expect("branch")
        .expect("on a branch");

    // Diverge: edit a.txt on both sides.
    git.create_branch(dir, &rn("other")).await.expect("branch");
    std::fs::write(dir.join("a.txt"), "main change\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "main edit").await.expect("commit");
    git.checkout(dir, &ct("other")).await.expect("checkout");
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
    assert!(
        git.merge_commit(dir, MergeCommit::branch(rv(&main)))
            .await
            .is_err()
    );
    assert_eq!(
        git.conflicted_files(dir).await.expect("conflicted_files"),
        [std::path::PathBuf::from("a.txt")]
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
    let main = git
        .current_branch(dir)
        .await
        .expect("branch")
        .expect("on a branch");

    // A feature branch one commit ahead; main does not move, so a plain merge
    // would fast-forward.
    git.create_branch(dir, &rn("feature"))
        .await
        .expect("branch");
    git.checkout(dir, &ct("feature")).await.expect("checkout");
    std::fs::write(dir.join("b.txt"), "feature\n").expect("write");
    git.add(dir, &[PathBuf::from("b.txt")]).await.expect("add");
    git.commit(dir, "feature work").await.expect("commit");
    git.checkout(dir, &ct(&main)).await.expect("checkout");

    git.merge_commit(
        dir,
        MergeCommit::branch(rv("feature"))
            .no_ff()
            .message("merge feature"),
    )
    .await
    .expect("merge_commit");

    // A 2-parent merge commit resolves `HEAD^2`; a fast-forward would not.
    assert!(
        git.resolve_commit(dir, &rv("HEAD^2")).await.is_ok(),
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
    let main = git
        .current_branch(dir)
        .await
        .expect("branch")
        .expect("on a branch");

    // `done` branches off base and is merged back into main.
    git.create_branch(dir, &rn("done")).await.expect("branch");
    git.checkout(dir, &ct("done")).await.expect("checkout");
    std::fs::write(dir.join("b.txt"), "done\n").expect("write");
    git.add(dir, &[PathBuf::from("b.txt")]).await.expect("add");
    git.commit(dir, "done work").await.expect("commit");
    git.checkout(dir, &ct(&main)).await.expect("checkout");
    git.merge_commit(
        dir,
        MergeCommit::branch(rv("done"))
            .no_ff()
            .message("merge done"),
    )
    .await
    .expect("merge_commit");

    // `pending` has a commit that was never merged into main.
    git.create_branch(dir, &rn("pending"))
        .await
        .expect("branch");
    git.checkout(dir, &ct("pending")).await.expect("checkout");
    std::fs::write(dir.join("c.txt"), "pending\n").expect("write");
    git.add(dir, &[PathBuf::from("c.txt")]).await.expect("add");
    git.commit(dir, "pending work").await.expect("commit");
    git.checkout(dir, &ct(&main)).await.expect("checkout");

    assert!(
        git.is_merged(dir, MergeCheck::branch(rn("done")).into_base(rv(&main)))
            .await
            .expect("is_merged done"),
        "`done` was merged into main"
    );
    assert!(
        !git.is_merged(dir, MergeCheck::branch(rn("pending")).into_base(rv(&main)))
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
    git.create_branch(dir, &rn("feature"))
        .await
        .expect("branch");

    // Dirty tree: a tracked edit and an untracked file both travel.
    std::fs::write(dir.join("a.txt"), "edited\n").expect("write");
    std::fs::write(dir.join("new.txt"), "untracked\n").expect("write");
    git.switch_with_stash(dir, &ct("feature"))
        .await
        .expect("switch");
    assert_eq!(
        git.current_branch(dir).await.expect("branch").as_deref(),
        Some("feature")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("read"),
        "edited\n"
    );
    assert!(dir.join("new.txt").exists(), "untracked file must travel");

    // A failing checkout restores the dirty state where it was.
    assert!(
        git.switch_with_stash(dir, &ct("no-such-branch"))
            .await
            .is_err(),
        "checkout of a missing branch must fail"
    );
    assert_eq!(
        git.current_branch(dir).await.expect("branch").as_deref(),
        Some("feature")
    );
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
    let log = git.log(&dest, &rv("HEAD"), 10).await.expect("log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "seed");
    assert_eq!(
        git.current_branch(&dest).await.expect("branch").as_deref(),
        Some("main")
    );
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
    git.tag_create(dir, &rn("v1"), None).await.expect("tag");
    git.tag_create_annotated(dir, AnnotatedTag::new(rn("v1.1"), "first release"))
        .await
        .expect("tag -a");
    assert_eq!(git.tag_list(dir).await.expect("list"), ["v1", "v1.1"]);
    git.tag_delete(dir, &rn("v1")).await.expect("delete");
    assert_eq!(git.tag_list(dir).await.expect("list"), ["v1.1"]);

    // show_file resolves a subdir path. The backslash form is the Windows trap
    // (normalised internally there); on Unix a backslash is a legal filename
    // byte and passes through verbatim, so query with the native `/` instead.
    #[cfg(windows)]
    let sub_path = r"sub\f.txt";
    #[cfg(not(windows))]
    let sub_path = "sub/f.txt";
    assert_eq!(
        git.show_file(dir, &rv("HEAD"), sub_path)
            .await
            .expect("show"),
        "v1\n"
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
    let first = git.rev_parse(dir, &rv("HEAD")).await.expect("rev");
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "second").await.expect("commit");
    let second = git.rev_parse(dir, &rv("HEAD")).await.expect("rev");

    let blame = git.blame(dir, "f.txt", None).await.expect("blame");
    assert_eq!(blame.len(), 2);
    assert_eq!(blame[0].commit, first, "line 1 from the first commit");
    assert_eq!(blame[1].commit, second, "line 2 from the second commit");
    assert_eq!(blame[0].author, "Test");
    assert!(blame[0].author_time > 1_500_000_000, "sane epoch");
    assert_eq!(blame[1].content, "two");

    // Transplant "second" onto a branch cut at "first".
    git.create_branch(dir, &rn("side")).await.expect("branch");
    git.checkout(dir, &ct("side")).await.expect("checkout");
    git.reset_hard(dir, &rv(&first)).await.expect("reset");
    git.cherry_pick(dir, &rv(&second))
        .await
        .expect("cherry-pick");
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).expect("read"),
        "one\ntwo\n"
    );
    // And revert it again.
    git.revert(dir, &rv("HEAD")).await.expect("revert");
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
    let main = git
        .current_branch(dir)
        .await
        .expect("branch")
        .expect("on a branch");
    // A stack commit whose content the base branch then also adopts.
    git.create_branch(dir, &rn("stack")).await.expect("branch");
    git.checkout(dir, &ct("stack")).await.expect("checkout");
    std::fs::write(dir.join("f.txt"), "same change\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "stack change").await.expect("commit");
    git.checkout(dir, &ct(&main)).await.expect("checkout");
    std::fs::write(dir.join("f.txt"), "upstream version\n").expect("write");
    git.add(dir, &[PathBuf::from("f.txt")]).await.expect("add");
    git.commit(dir, "upstream change").await.expect("commit");
    git.checkout(dir, &ct("stack")).await.expect("checkout");

    // The rebase conflicts; resolving to EXACTLY the upstream content empties
    // the patch, so --continue refuses and --skip is the way out.
    assert!(
        git.rebase(dir, &rv(&main)).await.is_err(),
        "conflict expected"
    );
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
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho ran >> hook-marker.txt\n").expect("write hook");
    // Unix git silently ignores a non-executable hook ("hook was ignored because
    // it's not set as executable"), and `fs::write` creates 0644 — without the
    // exec bit the plain-client half of this test never fires. Windows git runs
    // hooks through sh regardless, so no equivalent is needed there.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("make hook executable");
    }

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

// The vcs-cli-support classifiers branch on the REAL CLI's failure output; the
// hermetic tests in cli-support feed canned strings, so these three tests pin
// the classifiers against what live git actually prints. They run in the CI
// integration lane across git/jj versions, so any message drift that breaks a
// classifier gets caught here rather than at a consumer.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn classifier_matches_real_merge_conflict() {
    let tmp = TempDir::new("cls-conflict");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "base\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "base").await.expect("commit");
    let main = git
        .current_branch(dir)
        .await
        .expect("branch")
        .expect("on a branch");

    // Branch A edits the line; main then edits the SAME line — a merge can't
    // auto-resolve, so it fails on a content conflict.
    git.create_branch(dir, &rn("a")).await.expect("branch");
    git.checkout(dir, &ct("a")).await.expect("checkout");
    std::fs::write(dir.join("a.txt"), "a change\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "a edit").await.expect("commit");
    git.checkout(dir, &ct(&main)).await.expect("checkout");
    std::fs::write(dir.join("a.txt"), "main change\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "main edit").await.expect("commit");

    let err = git
        .merge_commit(dir, MergeCommit::branch(rv("a")))
        .await
        .expect_err("conflicting merge must fail");
    assert!(
        vcs_git::is_merge_conflict(&err),
        "real merge-conflict output must classify, got {err:?}"
    );
}

// A `commit` on a clean tree fails with git's "nothing to commit" — the
// classifier must recognise the real wording.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn classifier_matches_real_nothing_to_commit() {
    let tmp = TempDir::new("cls-nothing");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);
    std::fs::write(dir.join("a.txt"), "x\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "seed").await.expect("commit");

    // Tree is clean now: a second commit has nothing to record.
    let err = git
        .commit(dir, "empty")
        .await
        .expect_err("commit on a clean tree must fail");
    assert!(
        vcs_git::is_nothing_to_commit(&err),
        "real nothing-to-commit output must classify, got {err:?}"
    );
}

// A fetch from an unreachable remote is a transient network failure — the
// classifier must recognise the real connection error. `fetch_from` retries
// transient errors (3 attempts x 500ms backoff), so this takes ~1-1.5s; the
// connection-refused on a closed port is immediate per attempt.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn classifier_matches_real_transient_fetch() {
    let tmp = TempDir::new("cls-fetch");
    let dir = tmp.path();
    let git = Git::new();

    git.init(dir).await.expect("init");
    configure(dir);

    // Port 1 is reserved and never listening, so git's connection is refused
    // immediately ("Connection refused"/"failed to connect" — both in the
    // transient-marker list) rather than waiting on a DNS/connect timeout.
    git.remote_add(dir, "dead", "http://127.0.0.1:1/x.git")
        .await
        .expect("remote add");
    let err = git
        .fetch_from(dir, "dead")
        .await
        .expect_err("fetch from an unreachable remote must fail");
    assert!(
        vcs_git::is_transient_fetch_error(&err),
        "real connection-refused output must classify as transient, got {err:?}"
    );
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
    git.create_branch(dir, &rn("other")).await.expect("branch");
    std::fs::write(dir.join("a.txt"), "ours\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "ours").await.expect("commit");
    git.checkout(dir, &ct("other")).await.expect("checkout");
    std::fs::write(dir.join("a.txt"), "theirs\n").expect("write");
    git.add(dir, &[PathBuf::from("a.txt")]).await.expect("add");
    git.commit(dir, "theirs").await.expect("commit");
    let main = "-"; // previous branch
    let _ = main;
    assert!(
        git.merge_commit(dir, MergeCommit::branch(rv("@{-1}")))
            .await
            .is_err(),
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

// T-052: `add` and `commit_paths` must accept a path set whose combined length
// is definitely longer than Windows' `CreateProcess` argv ceiling (~32,767
// UTF-16 code units — building it as one plain `add -- <paths>`/`commit --only
// -- <paths>` argv used to fail there with `OS error 206`). Both route through
// the NUL-safe `--pathspec-from-file=-` transport instead once the path set
// crosses this crate's own (much smaller) internal budget, so neither ever
// builds an oversized argv in the first place.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn add_and_commit_paths_survive_an_oversized_argv() {
    let tmp = TempDir::new("huge-pathspec");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);

    // ~5,000 files of ~15 characters each — comfortably past the 32,767-char
    // Windows argv ceiling if ever built as one plain-argv pathspec list.
    let count = 5_000usize;
    let mut paths = Vec::with_capacity(count);
    let mut total_pathspec_len = 0usize;
    for i in 0..count {
        let name = format!("f_{i:05}.txt");
        std::fs::write(dir.join(&name), "x").expect("write file");
        total_pathspec_len += name.len() + 1;
        paths.push(PathBuf::from(name));
    }
    assert!(
        total_pathspec_len > 32_767,
        "test paths must exceed the Windows argv ceiling, got {total_pathspec_len}"
    );

    git.add(dir, &paths)
        .await
        .expect("add must not exceed the OS argv limit");
    let status = git.status(dir).await.expect("status");
    assert_eq!(status.len(), count, "every file must be staged");
    assert!(
        status.iter().all(|e| e.code == "A "),
        "all staged as new files"
    );

    git.commit_paths(dir, CommitPaths::new(paths, "huge commit"))
        .await
        .expect("commit_paths must not exceed the OS argv limit");
    assert!(
        git.status(dir).await.expect("status").is_empty(),
        "commit_paths must leave a clean tree"
    );
    let log = git.log(dir, &rv("HEAD"), 1).await.expect("log");
    assert_eq!(log[0].subject, "huge commit");
}

// T-052/R-04: `log_paths`'s large-path-set fallback resolves the (here
// symbolic) `revspec` via a real `git rev-parse` exactly once, before any of
// the several chunk calls and the commit-order oracle call it then makes —
// exercised end-to-end against the real binary (the hermetic tests in
// `src/lib.rs` script this same sequence, but can't confirm the real
// `git rev-parse HEAD` output is actually consumable by the subsequent real
// `git log <resolved> ...` calls the way the scripted tests assume). Two real
// commits, each touching a different half of a path set too large for one
// `git log` call, must still come back newest-first — proving the chunk
// merge + oracle reorder + one-time revspec resolution all agree with what a
// single unchunked call would have produced.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn log_paths_large_path_set_resolves_head_and_reorders_across_real_commits() {
    let tmp = TempDir::new("log-paths-chunked");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);

    // Each half is already comfortably past `log_paths`'s internal
    // (much-smaller-than-Windows'-32,767) argv budget on its own, so querying
    // both together below forces `log_paths` down its chunked path.
    let count = 500usize;
    let make_chunk = |dir: &Path, prefix: &str| -> Vec<String> {
        (0..count)
            .map(|i| {
                let name = format!("{prefix}_{i:05}.txt");
                std::fs::write(dir.join(&name), prefix).expect("write file");
                name
            })
            .collect()
    };

    let paths_a = make_chunk(dir, "chunk_a");
    git.add(
        dir,
        &paths_a
            .iter()
            .map(|s| PathBuf::from(s.clone()))
            .collect::<Vec<_>>(),
    )
    .await
    .expect("add chunk a");
    git.commit(dir, "commit a").await.expect("commit a");

    let paths_b = make_chunk(dir, "chunk_b");
    git.add(
        dir,
        &paths_b
            .iter()
            .map(|s| PathBuf::from(s.clone()))
            .collect::<Vec<_>>(),
    )
    .await
    .expect("add chunk b");
    git.commit(dir, "commit b").await.expect("commit b");

    let mut all_paths = paths_a;
    all_paths.extend(paths_b);
    let total_len: usize = all_paths.iter().map(|p| p.len() + 1).sum();
    assert!(
        total_len > 12_000,
        "path set must be large enough to force multiple `log_paths` chunks, \
         got {total_len} bytes"
    );

    let commits = git
        .log_paths(dir, &rv("HEAD"), 10, &all_paths)
        .await
        .expect("log_paths");
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        ["commit b", "commit a"],
        "log_paths must report both real commits, newest first, once the \
         chunked calls (over the resolved `HEAD`) are merged and reordered \
         by the commit-order oracle"
    );
}

// R-01/R-02: `add`, `commit_paths`, and `log_paths` must treat a pathspec
// glob-magic character (`[]`) literally, not as glob magic — even for a small
// path set that never goes anywhere near the T-052 chunking/stdin transport.
// `[` and `]` (unlike `*`/`?`) are valid on a Windows filesystem, so
// `file[1].txt` is a portable literal filename; as an *unquoted* git pathspec
// it would otherwise glob-match the unrelated `file1.txt`.
#[tokio::test]
#[ignore = "requires the git binary"]
async fn add_commit_paths_and_log_paths_treat_glob_characters_literally() {
    let tmp = TempDir::new("literal-pathspec");
    let dir = tmp.path();
    let git = Git::new();
    git.init(dir).await.expect("init");
    configure(dir);

    let literal_name = "file[1].txt";
    let glob_target_name = "file1.txt"; // what `file[1].txt` would glob-match
    std::fs::write(dir.join(literal_name), "literal").expect("write literal file");
    std::fs::write(dir.join(glob_target_name), "glob target").expect("write glob-target file");

    git.add(dir, &[PathBuf::from(literal_name)])
        .await
        .expect("add");
    let status = git.status(dir).await.expect("status");
    let literal_entry = status
        .iter()
        .find(|e| e.path == std::path::Path::new(literal_name))
        .expect("literal file must be present in status");
    assert_eq!(literal_entry.code, "A ", "the literal path must be staged");
    let glob_entry = status
        .iter()
        .find(|e| e.path == std::path::Path::new(glob_target_name))
        .expect("glob-target file must be present in status");
    assert_eq!(
        glob_entry.code, "??",
        "the glob-target path must remain untracked — `add` must not have \
         matched it via glob expansion (R-01)"
    );

    git.commit_paths(
        dir,
        CommitPaths::new([PathBuf::from(literal_name)], "literal commit"),
    )
    .await
    .expect("commit_paths");
    let status_after_commit = git.status(dir).await.expect("status");
    assert!(
        status_after_commit
            .iter()
            .all(|e| e.path != std::path::Path::new(literal_name)),
        "the committed literal path must no longer show as changed"
    );
    let glob_entry_after = status_after_commit
        .iter()
        .find(|e| e.path == std::path::Path::new(glob_target_name))
        .expect("glob-target file must still be present in status");
    assert_eq!(
        glob_entry_after.code, "??",
        "the glob-target path must remain untracked after commit_paths — must \
         not have been matched via glob expansion (R-01)"
    );

    let log_literal = git
        .log_paths(dir, &rv("HEAD"), 5, &[literal_name.to_string()])
        .await
        .expect("log_paths");
    assert_eq!(
        log_literal.len(),
        1,
        "log_paths must find the commit that touched the literal path"
    );
    assert_eq!(log_literal[0].subject, "literal commit");

    let log_glob = git
        .log_paths(dir, &rv("HEAD"), 5, &[glob_target_name.to_string()])
        .await
        .expect("log_paths");
    assert!(
        log_glob.is_empty(),
        "log_paths must not match the glob-target path, which was never \
         committed — a glob-expanding query would incorrectly find the \
         literal commit here (R-02)"
    );
}
