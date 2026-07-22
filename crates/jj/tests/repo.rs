//! End-to-end tests for the typed `vcs-jj` client against a real temporary
//! repository. Ignored by default (require the `jj` binary); run with
//! `cargo test -p vcs-jj -- --ignored`.

// Scaffolding from vcs-testkit: `JjSandbox` owns the throwaway workspace and
// raw scenario steps; the typed client under test does the rest.
use vcs_jj::{BookmarkName, GitClone, Jj, JjApi, RevsetExpr, WorkspaceAdd};
use vcs_testkit::{BareRemote, JjSandbox, TempDir, jj as jj_raw};

// Terse constructors for the validated newtypes in test call sites; the literals
// here are always valid, so `unwrap` is fine in tests.
fn rv(s: &str) -> RevsetExpr {
    RevsetExpr::new(s).unwrap()
}
fn bn(s: &str) -> BookmarkName {
    BookmarkName::new(s).unwrap()
}

#[tokio::test]
#[ignore = "requires the jj binary"]
async fn describe_new_and_log_cycle() {
    let sandbox = JjSandbox::init("cycle");
    let dir = sandbox.path();
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
    let log = jj.log(dir, &rv("::@"), 10).await.expect("log");
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
    let sandbox = JjSandbox::init("bookmarks");
    let dir = sandbox.path();
    let jj = Jj::new();

    jj.describe(dir, "rooted").await.expect("describe");
    jj_raw(dir, &["bookmark", "create", "mark", "-r", "@"]);
    // Move it via the typed API.
    jj.bookmark_set(dir, &bn("mark"), &rv("@"))
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
    let sandbox = JjSandbox::init("ws-main");
    let dir = sandbox.path();
    let jj = Jj::new();

    // root() resolves to a real path.
    assert!(jj.root(dir).await.expect("root").exists());

    // A workspace path that doesn't exist yet, outside the repo.
    let ws_parent = TempDir::new("ws-linked");
    let ws_path = ws_parent.path().join("ws1");

    jj.workspace_add(dir, WorkspaceAdd::new("ws1", rv("@"), ws_path.clone()))
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
    let sandbox = JjSandbox::init("reachable");
    let dir = sandbox.path();
    let jj = Jj::new();

    jj.describe(dir, "base").await.expect("describe");
    jj.bookmark_create(dir, &bn("feature"), &rv("@"))
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
        jj.resolve_list(dir, &rv("@"))
            .await
            .expect("resolve_list")
            .is_empty()
    );

    // Build a real conflict: two children of base that edit the same file,
    // merged. `resolve_list` must return the actual conflicted path (this is the
    // case the format parser has to get right).
    std::fs::write(dir.join("c.txt"), "base\n").expect("write base");
    jj_raw(dir, &["new", "root()", "-m", "side-a"]);
    std::fs::write(dir.join("c.txt"), "aaa\n").expect("write a");
    let a = jj.current_change(dir).await.expect("a").change_id;
    jj_raw(dir, &["new", "root()", "-m", "side-b"]);
    std::fs::write(dir.join("c.txt"), "bbb\n").expect("write b");
    let b = jj.current_change(dir).await.expect("b").change_id;
    jj_raw(dir, &["new", &a, &b, "-m", "merge"]);

    let conflicts = jj.resolve_list(dir, &rv("@")).await.expect("resolve_list");
    assert_eq!(
        conflicts,
        [std::path::PathBuf::from("c.txt")],
        "got {conflicts:?}"
    );
}

// A renamed tracked file: jj `diff --summary` renders `R {old => new}`; status()
// must expose the real new path (and the old path), not the raw brace expression.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn status_exposes_rename_paths() {
    let sandbox = JjSandbox::init("rename");
    let dir = sandbox.path();
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
    assert_eq!(renamed.path, std::path::Path::new("new.rs"));
    assert_eq!(
        renamed.old_path.as_deref(),
        Some(std::path::Path::new("old.rs"))
    );
}

// `description` reads back exactly what `describe` wrote (single revision,
// multiline body preserved, trailing whitespace trimmed).
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn description_round_trips_describe() {
    let sandbox = JjSandbox::init("description");
    let dir = sandbox.path();
    let jj = Jj::new();

    // An undescribed change reads as empty.
    assert_eq!(
        jj.description(dir, &rv("@")).await.expect("description"),
        ""
    );

    let message = "feat: parser\n\nlonger body line";
    jj.describe(dir, message).await.expect("describe");
    assert_eq!(
        jj.description(dir, &rv("@")).await.expect("description"),
        message
    );

    // A multi-commit revset yields only the newest commit's description.
    jj.new_change(dir, "second").await.expect("new");
    assert_eq!(
        jj.description(dir, &rv("::@")).await.expect("description"),
        "second"
    );
}

// `transaction` rolls the op log back on Err and keeps the work on Ok.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn transaction_rolls_back_on_error_and_keeps_success() {
    let sandbox = JjSandbox::init("transaction");
    let dir = sandbox.path();
    let jj = Jj::new();

    jj.describe(dir, "before").await.expect("describe");

    // Failing transaction: the inner describe is rolled back.
    let res = jj
        .transaction(dir, |tx| async move {
            tx.describe("inside").await?;
            tx.edit(&rv("zzz-no-such-revset")).await // forces the rollback
        })
        .await;
    assert!(res.is_err(), "the closure error must surface");
    assert_eq!(
        jj.description(dir, &rv("@")).await.expect("description"),
        "before",
        "the describe inside the failed transaction must be rolled back"
    );

    // Successful transaction: the mutation sticks.
    jj.transaction(dir, |tx| async move { tx.describe("after").await })
        .await
        .expect("transaction");
    assert_eq!(
        jj.description(dir, &rv("@")).await.expect("description"),
        "after"
    );
}

// git_clone from a local bare fixture, plain and colocated.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn git_clone_from_local_bare_remote() {
    let remote = BareRemote::seeded("clone");
    let tmp = TempDir::new("clone-dest");
    let jj = Jj::new();

    let plain = tmp.path().join("plain");
    jj.git_clone(remote.url().as_str(), &plain, GitClone::separate())
        .await
        .expect("clone");
    assert!(plain.join(".jj").is_dir(), "jj repo materialised");
    assert!(
        !plain.join(".git").exists(),
        "explicit --no-colocate wins over any version/config default"
    );
    assert!(plain.join("seed.txt").exists(), "worktree materialised");

    let colocated = tmp.path().join("colocated");
    jj.git_clone(remote.url().as_str(), &colocated, GitClone::colocated())
        .await
        .expect("clone --colocate");
    assert!(colocated.join(".jj").is_dir());
    assert!(colocated.join(".git").exists(), "colocated keeps .git");
}

// `git remote list` has no machine template. Exercise the real CLI's output
// framing and all five typed remote operations without a network dependency.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn remote_management_round_trip() {
    let sandbox = JjSandbox::init("remotes");
    let dir = sandbox.path();
    let jj = Jj::new();

    // `jj git remote add` percent-encodes non-ASCII URL bytes, so use ASCII
    // here; the set-url phase below verifies literal non-ASCII preservation.
    jj.remote_add(dir, "t097-origin", "https://example.com/repo.git")
        .await
        .expect("add remote");
    assert!(
        jj.remote_list(dir)
            .await
            .expect("list after add")
            .iter()
            .any(|remote| {
                remote.name == "t097-origin" && remote.url == "https://example.com/repo.git"
            }),
        "added remote should round-trip through list"
    );

    jj.remote_rename(dir, "t097-origin", "t097-upstream")
        .await
        .expect("rename remote");
    jj.remote_set_url(dir, "t097-upstream", "ssh://example.com/новый.git")
        .await
        .expect("set remote URL");
    assert!(
        jj.remote_list(dir)
            .await
            .expect("list after rename/set-url")
            .iter()
            .any(|remote| {
                remote.name == "t097-upstream" && remote.url == "ssh://example.com/новый.git"
            }),
        "renamed remote and non-ASCII URL should round-trip through list"
    );

    jj.remote_remove(dir, "t097-upstream")
        .await
        .expect("remove remote");
    assert!(
        !jj.remote_list(dir)
            .await
            .expect("list after remove")
            .iter()
            .any(|remote| remote.name == "t097-upstream"),
        "removed remote should be absent"
    );
}

// absorb folds an edit into the change that introduced the lines; split carves
// named paths into their own commit; duplicate copies a commit.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn absorb_split_and_duplicate_cycle() {
    let sandbox = JjSandbox::init("absorb");
    let dir = sandbox.path();
    let jj = Jj::new();

    // Base change introduces two files.
    std::fs::write(dir.join("a.txt"), "alpha\n").expect("write");
    std::fs::write(dir.join("b.txt"), "beta\n").expect("write");
    jj.describe(dir, "base").await.expect("describe");
    jj.new_change(dir, "wip").await.expect("new");

    // Absorb: an edit to a.txt belongs to "base" and must fold back into it.
    std::fs::write(dir.join("a.txt"), "alpha edited\n").expect("edit");
    jj.absorb(dir, None, &[]).await.expect("absorb");
    assert!(
        jj.current_change(dir).await.expect("change").empty,
        "the edit was absorbed out of the working copy"
    );
    assert_eq!(
        jj.file_show(dir, &rv("@-"), "a.txt").await.expect("show"),
        "alpha edited\n",
        "the base change now carries the edit"
    );

    // Split operates on @ — put a fresh edit into @ across two files, then
    // carve one of them out into its own described commit.
    assert_eq!(
        jj.description(dir, &rv("@-")).await.expect("description"),
        "base"
    );
    std::fs::write(dir.join("c.txt"), "gamma\n").expect("write");
    std::fs::write(dir.join("d.txt"), "delta\n").expect("write");
    jj.split_paths(dir, &[vcs_jj::JjFileset::path("c.txt")], "carve c")
        .await
        .expect("split");
    assert_eq!(
        jj.description(dir, &rv("@-")).await.expect("description"),
        "carve c",
        "the named fileset landed in its own commit"
    );
    assert_eq!(
        jj.file_show(dir, &rv("@-"), "c.txt").await.expect("show"),
        "gamma\n"
    );

    // Duplicate: copying @- adds a commit without moving @.
    let before = jj.commit_count(dir, &rv("all()")).await.expect("count");
    jj.duplicate(dir, &rv("@-")).await.expect("duplicate");
    let after = jj.commit_count(dir, &rv("all()")).await.expect("count");
    assert_eq!(after, before + 1, "one duplicated commit");
}

// op_log lists recent operations; evolog tracks a change's rewrites; annotate
// maps lines to the changes that introduced them.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn op_log_evolog_and_annotate_cycle() {
    let sandbox = JjSandbox::init("oplog");
    let dir = sandbox.path();
    let jj = Jj::new();

    std::fs::write(dir.join("f.txt"), "one\n").expect("write");
    jj.describe(dir, "first words").await.expect("describe");
    jj.describe(dir, "better words").await.expect("re-describe");

    let ops = jj.op_log(dir, 5).await.expect("op_log");
    assert!(ops.len() >= 3, "init + snapshots/describes, got {ops:?}");
    assert!(ops.iter().all(|op| !op.id.is_empty()));
    // A10: each op time is RFC-3339 with a colon offset — the HH:MM:SS time has two
    // colons and the `%:z` offset adds a third (a `%z` `+0200` offset would have none).
    assert!(
        ops.iter().all(|op| op.time.matches(':').count() >= 3),
        "op time carries an RFC-3339 colon offset: {ops:?}"
    );
    assert!(
        ops.iter().any(|op| op.description.contains("describe")),
        "a describe op is listed: {ops:?}"
    );
    // The newest op id matches op_head.
    assert_eq!(ops[0].id, jj.op_head(dir).await.expect("op_head"));

    // evolog: the re-described change has at least two recorded versions.
    let evolution = jj.evolog(dir, &rv("@"), 10).await.expect("evolog");
    assert!(evolution.len() >= 2, "got {evolution:?}");
    assert_eq!(evolution[0].description, "better words", "newest first");
    assert!(
        evolution
            .iter()
            .any(|c| c.description == "first words" || c.description.is_empty()),
        "an earlier version is recorded: {evolution:?}"
    );

    // annotate: both lines map to the changes that introduced them.
    jj.new_change(dir, "second line").await.expect("new");
    std::fs::write(dir.join("f.txt"), "one\ntwo\n").expect("edit");
    let lines = jj
        .file_annotate(dir, "f.txt", None)
        .await
        .expect("annotate");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].line, 1);
    assert_eq!(lines[1].line, 2);
    assert_ne!(
        lines[0].change_id, lines[1].change_id,
        "lines came from different changes"
    );
    assert_eq!(lines[1].content, "two");
}

// capabilities round-trips against the real binary on PATH.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn capabilities_probe_real_binary() {
    let caps = Jj::new().capabilities().await.expect("capabilities");
    assert!(caps.is_supported(), "got {:?}", caps.version);
    caps.ensure_supported().expect("supported");
}

// The typed conflict model round-trips a REAL jj-materialized conflict (the
// default `diff` style): parse → resolve(Side(1)) → write back → snapshot →
// the conflict is gone.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn conflict_model_resolves_a_real_conflict() {
    use vcs_jj::conflict::{JjResolution, parse_conflicts, render, resolve};

    let sandbox = JjSandbox::init("conflict-model");
    let dir = sandbox.path();
    let jj = Jj::new();

    sandbox.write("c.txt", "line 1\nline 2\nline 3\n");
    sandbox.describe("base");
    jj_raw(dir, &["new", "@", "-m", "side-a"]);
    sandbox.write("c.txt", "line 1\nmain line 2\nline 3\n");
    let a = jj.current_change(dir).await.expect("a").change_id;
    jj_raw(dir, &["new", "@-", "-m", "side-b"]);
    sandbox.write("c.txt", "line 1\nfeature line 2\nline 3\n");
    let b = jj.current_change(dir).await.expect("b").change_id;
    jj_raw(dir, &["new", &a, &b, "-m", "merge"]);
    assert_eq!(
        jj.resolve_list(dir, &rv("@")).await.expect("resolve_list"),
        [std::path::PathBuf::from("c.txt")]
    );

    let content = std::fs::read_to_string(dir.join("c.txt")).expect("read");
    let segments = parse_conflicts(&content).expect("parse real jj markers");
    assert_eq!(render(&segments), content, "byte-exact roundtrip");
    let resolved = resolve(&segments, JjResolution::Side(1)).expect("resolve");
    assert_eq!(resolved, "line 1\nfeature line 2\nline 3\n");
    std::fs::write(dir.join("c.txt"), &resolved).expect("write resolved");

    // jj re-parses the materialized file on snapshot: markers gone → resolved.
    assert!(
        jj.resolve_list(dir, &rv("@"))
            .await
            .expect("resolve_list")
            .is_empty(),
        "conflict cleared after writing the resolution"
    );
}

// T-040: `status`/`diff_summary` must report the same, root-relative paths
// regardless of the directory the client is bound to — the workspace root or
// a nested subdirectory. Backslash separators never leak into the paths (on
// Windows jj's `--summary` uses the OS-native separator), and a caller bound
// to a nested dir must never see a `..`-prefixed, cwd-relative path.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn status_paths_are_root_relative_from_a_nested_directory() {
    let sandbox = JjSandbox::init("nested-status");
    let dir = sandbox.path();
    let jj = Jj::new();

    let nested = dir.join("sub").join("deep");
    std::fs::create_dir_all(&nested).expect("mkdir nested");
    std::fs::write(dir.join("top.rs"), "top\n").expect("write top");
    std::fs::write(nested.join("bottom.rs"), "bottom\n").expect("write nested");

    let from_root = jj.status(dir).await.expect("status from root");
    let from_nested = jj.status(&nested).await.expect("status from nested dir");

    assert_eq!(
        from_root, from_nested,
        "status must be identical regardless of the bound working directory"
    );
    let paths: Vec<&str> = from_root.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert!(paths.contains(&"top.rs"), "got {paths:?}");
    assert!(paths.contains(&"sub/deep/bottom.rs"), "got {paths:?}");
    assert!(
        paths.iter().all(|p| !p.contains('\\') && !p.contains("..")),
        "paths must be forward-slash, workspace-root-relative: {paths:?}"
    );
}

// `diff_summary` (an explicit `from..to` revset range) must show the same
// root-relative-path contract as `status`, independent of the bound directory.
#[tokio::test]
#[ignore = "requires the jj binary"]
async fn diff_summary_paths_are_root_relative_from_a_nested_directory() {
    let sandbox = JjSandbox::init("nested-diff-summary");
    let dir = sandbox.path();
    let jj = Jj::new();

    std::fs::create_dir_all(dir.join("pkg")).expect("mkdir pkg");
    std::fs::write(dir.join("root.rs"), "root\n").expect("write root");
    std::fs::write(dir.join("pkg").join("mod.rs"), "mod\n").expect("write pkg/mod.rs");

    let from_root = jj
        .diff_summary(dir, &rv("root()"), &rv("@"))
        .await
        .expect("diff_summary from root");
    let from_nested = jj
        .diff_summary(&dir.join("pkg"), &rv("root()"), &rv("@"))
        .await
        .expect("diff_summary from nested dir");

    assert_eq!(
        from_root, from_nested,
        "diff_summary must be identical regardless of the bound working directory"
    );
    let paths: Vec<&str> = from_root.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert!(paths.contains(&"root.rs"), "got {paths:?}");
    assert!(paths.contains(&"pkg/mod.rs"), "got {paths:?}");
    assert!(
        paths.iter().all(|p| !p.contains('\\') && !p.contains("..")),
        "paths must be forward-slash, workspace-root-relative: {paths:?}"
    );
}
