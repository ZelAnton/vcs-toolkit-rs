//! End-to-end test of the real `vcs-mcp` **binary** over its actual stdio
//! transport. Every other integration test in this crate drives the server
//! in-process (`crates/mcp/tests/mcp.rs`) or through an in-memory duplex
//! transport (`src/tests.rs::in_process_client_lists_and_calls_tools`) — this
//! is the one that spawns the compiled binary
//! (`env!("CARGO_BIN_EXE_vcs-mcp")`) as a child process and drives it through
//! an `rmcp` client over a real child-process/stdio transport
//! (`TokioChildProcess`), the transport layer an actual agent harness talks
//! over. That catches a class of regression the in-process tests structurally
//! can't: a broken schema/annotation serialization on the wire, rmcp version
//! drift, or a broken argv/flag in the binary itself.
//!
//! Ignored by default (needs the real `git` binary and a built `vcs-mcp`).
//! Run with `cargo test -p vcs-mcp -- --ignored`.

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use vcs_testkit::GitSandbox;

/// A `tokio::process::Command` for the compiled `vcs-mcp` binary, serving
/// `repo` read-only (no `--allow-write`/`--allow-tools`) — the default rights
/// this whole e2e test runs under.
fn vcs_mcp_readonly_command(repo: &std::path::Path) -> tokio::process::Command {
    let repo = repo.to_path_buf();
    tokio::process::Command::new(env!("CARGO_BIN_EXE_vcs-mcp")).configure(move |cmd| {
        cmd.arg("--repo").arg(&repo);
    })
}

/// The JSON a tool call returned (the first text content of its result).
fn inner(r: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = r
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    serde_json::from_str(&text).expect("the tool returns JSON")
}

// The full stdio transport, driven end to end against the real binary: spawn
// read-only (no --allow-write), `initialize`, `tools/list` (catalogue +
// schemas + annotations), a real read-tool round trip, then proof that a
// disabled mutation is neither advertised nor routable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the git binary and a built vcs-mcp binary"]
async fn stdio_binary_e2e_initialize_tools_list_read_call_and_gated_mutation() {
    let sandbox = GitSandbox::init("mcp-stdio-e2e");
    sandbox.commit_file("seed.txt", "seed\n", "initial");

    // 1. Spawn the real binary over its real stdio transport, read-only
    //    (the default: no --allow-write, no --allow-tools). `initialize` is
    //    the handshake `.serve()` performs; `peer_info()` is the response.
    let transport =
        TokioChildProcess::new(vcs_mcp_readonly_command(sandbox.path())).expect("spawn vcs-mcp");
    let client = ().serve(transport).await.expect("initialize handshake");

    let info = client
        .peer_info()
        .expect("server_info present after a successful initialize");
    let server_info = info
        .server_info
        .as_ref()
        .expect("vcs-mcp must advertise server_info");
    assert_eq!(server_info.name, "vcs-mcp");
    assert!(
        !server_info.version.is_empty(),
        "server_info.version must be populated"
    );

    // 2. tools/list: a non-empty catalogue whose schemas and read-only/
    //    destructive annotations survived the wire round trip.
    let tools = client.list_all_tools().await.expect("tools/list");
    assert!(!tools.is_empty(), "the catalogue must not be empty");

    // A genuinely read-only tool (per crates/mcp/docs/mcp.md: `repo_info`
    // spawns no backend command at all, so it alone carries `readOnlyHint`).
    let read_only = tools
        .iter()
        .find(|t| t.name == "repo_info")
        .expect("repo_info is in the catalogue");
    let read_only_annotations = read_only
        .annotations
        .as_ref()
        .expect("repo_info carries MCP annotations");
    assert_eq!(
        read_only_annotations.read_only_hint,
        Some(true),
        "repo_info must be annotated readOnlyHint"
    );
    assert_eq!(read_only_annotations.destructive_hint, None);

    assert!(tools.iter().any(|tool| tool.name == "outcome_inspect"));
    assert!(tools.iter().any(|tool| tool.name == "outcome_changes"));
    assert!(!tools.iter().any(|tool| tool.name == "repo_commit"));
    assert!(!tools.iter().any(|tool| tool.name == "outcome_commit"));
    assert!(!tools.iter().any(|tool| tool.name.starts_with("forge_")));

    // A genuinely idempotent tool: on jj it snapshots the working copy (a
    // reversible, append-only op-log operation), so per crates/mcp/docs/mcp.md
    // it is annotated `destructiveHint = false` + `idempotentHint = true`
    // rather than `readOnlyHint` — verified here on the wire, then exercised
    // for real via the read-tool round trip in step 3 below.
    let idempotent = tools
        .iter()
        .find(|t| t.name == "repo_current_branch")
        .expect("repo_current_branch is in the catalogue");
    let idempotent_annotations = idempotent
        .annotations
        .as_ref()
        .expect("repo_current_branch carries MCP annotations");
    assert_eq!(
        idempotent_annotations.idempotent_hint,
        Some(true),
        "repo_current_branch must be annotated idempotentHint"
    );
    assert_eq!(idempotent_annotations.destructive_hint, Some(false));

    // 3. A real read-tool round trip through the full protocol.
    let branch = inner(
        &client
            .call_tool(CallToolRequestParams::new("repo_current_branch"))
            .await
            .expect("repo_current_branch call"),
    );
    let branch = branch.as_str().expect("a branch name");
    assert!(branch == "main" || branch == "master", "{branch}");

    let outcome = inner(
        &client
            .call_tool(CallToolRequestParams::new("outcome_inspect"))
            .await
            .expect("outcome_inspect call"),
    );
    assert_eq!(outcome["contract_version"], "vcs-agent/v1");
    assert_eq!(outcome["operation"], "inspect");
    assert_eq!(outcome["status"], "success");

    // 4. A client cannot bypass discovery by naming the disabled mutation.
    let mut args = serde_json::Map::new();
    args.insert("paths".into(), serde_json::json!(["seed.txt"]));
    args.insert("message".into(), serde_json::json!("should be refused"));
    let err = client
        .call_tool(CallToolRequestParams::new("repo_commit").with_arguments(args))
        .await
        .expect_err("a disabled mutation must not be routable");
    assert!(
        format!("{err:?}").to_lowercase().contains("not found"),
        "the disabled route must be absent: {err:?}"
    );

    let mut outcome_args = serde_json::Map::new();
    outcome_args.insert("expected_revision".into(), serde_json::json!("stale"));
    outcome_args.insert("message".into(), serde_json::json!("should be refused"));
    outcome_args.insert("paths".into(), serde_json::json!(["seed.txt"]));
    let err = client
        .call_tool(CallToolRequestParams::new("outcome_commit").with_arguments(outcome_args))
        .await
        .expect_err("the disabled outcome mutation must not be routable");
    assert!(
        format!("{err:?}").to_lowercase().contains("not found"),
        "the disabled outcome route must be absent: {err:?}"
    );

    let _ = client.cancel().await;
}
