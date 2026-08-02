//! Integration tests for `vcs-mcp` against a real temporary git repository.
//! Ignored by default (require the `git` binary). Run with
//! `cargo test -p vcs-mcp -- --ignored`.
//!
//! The tool logic, gating, serialization, and the in-process MCP round-trip are
//! covered hermetically in `src/lib.rs`; this drives the tools against a real
//! repo to confirm the end-to-end path (real `git` → facade → JSON result).

use rmcp::handler::server::wrapper::Parameters;
use vcs_core::Repo;
use vcs_mcp::{
    CheckoutParams, ConflictRegionsParams, ConflictSideArg, ResolveConflictParams, VcsMcpServer,
    WriteGate,
};
use vcs_testkit::GitSandbox;

/// Parse the JSON a tool returned (the first text content of its result).
fn inner(r: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = r
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    serde_json::from_str(&text).expect("the tool returns JSON")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn read_tools_run_against_a_real_repo() {
    let sandbox = GitSandbox::init("mcp-real");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    let repo = Repo::discover(sandbox.path()).expect("open");
    let server = VcsMcpServer::new(repo, None, WriteGate::None);

    // The current branch is the seeded default (main or master).
    let branch = inner(&server.repo_current_branch().await.expect("current_branch"));
    let branch = branch.as_str().expect("a branch name");
    assert!(branch == "main" || branch == "master", "{branch}");

    // A snapshot succeeds and reports a clean tree.
    let snap = inner(&server.repo_snapshot().await.expect("snapshot"));
    assert_eq!(snap["dirty"], false);
    assert_eq!(snap["operation"], "Clear");

    // An edit shows up in repo_status as a modified seed.txt.
    sandbox.write("seed.txt", "changed\n");
    let status = inner(&server.repo_status().await.expect("status"));
    assert!(
        status
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["path"] == "seed.txt"),
        "{status}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn gated_mutation_does_not_run_against_a_real_repo() {
    let sandbox = GitSandbox::init("mcp-gate");
    sandbox.commit_file("seed.txt", "seed\n", "initial");
    sandbox.branch("feature");
    let repo = Repo::discover(sandbox.path()).expect("open");
    // Read-only server: checkout must be refused before touching git.
    let server = VcsMcpServer::new(repo, None, WriteGate::None);
    let err = server
        .repo_checkout(Parameters(CheckoutParams {
            reference: "feature".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
}

/// Seed a real, conflicting `git merge` on `path` and return the sandbox.
fn conflicted_sandbox(tag: &str, path: &str) -> GitSandbox {
    let sandbox = GitSandbox::init(tag);
    sandbox.commit_file(path, "line 1\nline 2\nline 3\n", "base");
    sandbox.branch("feature");
    sandbox.checkout("feature");
    sandbox.commit_file(path, "line 1\nfeature line 2\nline 3\n", "feature");
    sandbox.git(&["checkout", "-q", "-"]);
    sandbox.commit_file(path, "line 1\nmain line 2\nline 3\n", "main");

    // A conflicting merge exits non-zero by design, so it can't go through the
    // panic-on-failure sandbox helper.
    let merge = std::process::Command::new("git")
        .args(["merge", "feature"])
        .current_dir(sandbox.path())
        .output()
        .expect("run git merge");
    assert!(!merge.status.success(), "the merge must conflict");
    sandbox
}

// The end-to-end claim the hermetic suite can only assert in pieces: a REAL git
// merge conflict is parsed into regions, resolved to one side, and — crucially —
// actually stops being a conflict afterwards. That last step is what makes the
// `git add` in `Repo::mark_resolved` load-bearing: rewriting the working-tree
// file alone leaves git's index entry unmerged, so `repo_conflicts` would keep
// reporting the path forever.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn conflict_tools_resolve_a_real_merge_conflict() {
    let sandbox = conflicted_sandbox("mcp-conflict", "f.txt");
    let repo = Repo::discover(sandbox.path()).expect("open");
    let server = VcsMcpServer::new(repo, None, WriteGate::All);

    // git really does report the path as conflicted...
    let conflicts = inner(&server.repo_conflicts().await.expect("conflicts"));
    assert_eq!(conflicts, serde_json::json!(["f.txt"]), "{conflicts}");

    // ...and the read tool parses the markers git actually wrote into the file.
    let regions = inner(
        &server
            .repo_conflict_regions(Parameters(ConflictRegionsParams {
                path: "f.txt".into(),
            }))
            .await
            .expect("conflict_regions"),
    );
    assert_eq!(regions["backend"], "git");
    assert_eq!(regions["conflict_count"], 1);
    let region = &regions["regions"][0]["region"];
    assert_eq!(region["ours_label"], "HEAD");
    assert_eq!(region["ours"], serde_json::json!(["main line 2\n"]));
    assert_eq!(region["theirs"], serde_json::json!(["feature line 2\n"]));

    // Resolving to "theirs" writes that side and stages the path.
    let done = inner(
        &server
            .repo_resolve_conflict(Parameters(ResolveConflictParams {
                path: "f.txt".into(),
                side: ConflictSideArg::Theirs,
                index: None,
            }))
            .await
            .expect("resolve_conflict"),
    );
    assert_eq!(done["conflicts_resolved"], 1);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("f.txt")).expect("read back"),
        "line 1\nfeature line 2\nline 3\n"
    );

    // The payoff: git no longer considers the path conflicted, so the merge can
    // be committed. Without the staging step this assertion fails.
    let after = inner(&server.repo_conflicts().await.expect("conflicts after"));
    assert_eq!(after, serde_json::json!([]), "still conflicted: {after}");
    sandbox.git(&["commit", "-qm", "resolved"]);

    // A now-clean file parses to zero regions rather than erroring.
    let clean = inner(
        &server
            .repo_conflict_regions(Parameters(ConflictRegionsParams {
                path: "f.txt".into(),
            }))
            .await
            .expect("conflict_regions on a clean file"),
    );
    assert_eq!(clean["conflict_count"], 0);
}

// The same round trip with the server opened on a SUBDIRECTORY, so `Repo`'s
// `cwd` is not its `root`. Both backends report conflicted paths relative to the
// repo *root* whatever directory the query ran in, while a git pathspec resolves
// against the process directory — so staging from `cwd` would look for
// `<cwd>/<root-relative path>`, fail with "pathspec did not match any files", and
// leave the file rewritten but the conflict still open. Regression cover for
// `Repo::mark_resolved` running its git spawn at the root.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary"]
async fn conflict_tools_work_when_the_server_is_opened_on_a_subdirectory() {
    let sandbox = conflicted_sandbox("mcp-conflict-sub", "sub/f.txt");
    let repo = Repo::discover(sandbox.path().join("sub")).expect("open");
    assert_ne!(repo.cwd(), repo.root(), "the fixture needs cwd != root");
    let server = VcsMcpServer::new(repo, None, WriteGate::All);

    // The path is root-relative even though the queries run in `sub/`.
    let conflicts = inner(&server.repo_conflicts().await.expect("conflicts"));
    assert_eq!(conflicts, serde_json::json!(["sub/f.txt"]), "{conflicts}");

    let regions = inner(
        &server
            .repo_conflict_regions(Parameters(ConflictRegionsParams {
                path: "sub/f.txt".into(),
            }))
            .await
            .expect("conflict_regions"),
    );
    assert_eq!(regions["conflict_count"], 1);

    server
        .repo_resolve_conflict(Parameters(ResolveConflictParams {
            path: "sub/f.txt".into(),
            side: ConflictSideArg::Ours,
            index: None,
        }))
        .await
        .expect("resolve_conflict from a subdirectory");

    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("sub/f.txt")).expect("read back"),
        "line 1\nmain line 2\nline 3\n"
    );
    let after = inner(&server.repo_conflicts().await.expect("conflicts after"));
    assert_eq!(after, serde_json::json!([]), "still conflicted: {after}");
}
