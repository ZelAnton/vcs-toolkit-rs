//! Integration tests for `vcs-mcp` against a real temporary git repository.
//! Ignored by default (require the `git` binary). Run with
//! `cargo test -p vcs-mcp -- --ignored`.
//!
//! The tool logic, gating, serialization, and the in-process MCP round-trip are
//! covered hermetically in `src/lib.rs`; this drives the tools against a real
//! repo to confirm the end-to-end path (real `git` → facade → JSON result).

use rmcp::handler::server::wrapper::Parameters;
use vcs_core::Repo;
use vcs_mcp::{CheckoutParams, VcsMcpServer, WriteGate};
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
