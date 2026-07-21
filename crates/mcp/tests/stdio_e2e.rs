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
// schemas + annotations), a real read-tool round trip, then a mutating tool
// call refused by the SERVER (not the client) for lacking --allow-write.
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
    assert_eq!(info.server_info.name, "vcs-mcp");
    assert!(
        !info.server_info.version.is_empty(),
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

    // A genuinely mutating tool, annotated destructiveHint, whose JSON-schema
    // parameters (`paths`, `message`) made it across the wire intact.
    let mutating = tools
        .iter()
        .find(|t| t.name == "repo_commit")
        .expect("repo_commit is in the catalogue");
    let mutating_annotations = mutating
        .annotations
        .as_ref()
        .expect("repo_commit carries MCP annotations");
    assert_eq!(
        mutating_annotations.destructive_hint,
        Some(true),
        "repo_commit must be annotated destructiveHint"
    );
    let schema = serde_json::to_value(&mutating.input_schema).expect("schema serializes");
    let props = schema
        .get("properties")
        .expect("repo_commit schema declares properties");
    assert!(props.get("paths").is_some(), "{props}");
    assert!(props.get("message").is_some(), "{props}");

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

    // 4. A mutating tool call is refused by the SERVER (not the client) —
    //    the write gate rejects it before it ever reaches git, and the
    //    refusal surfaces as a protocol-level error naming the missing flag.
    let mut args = serde_json::Map::new();
    args.insert("paths".into(), serde_json::json!(["seed.txt"]));
    args.insert("message".into(), serde_json::json!("should be refused"));
    let err = client
        .call_tool(CallToolRequestParams::new("repo_commit").with_arguments(args))
        .await
        .expect_err("a mutating tool must be refused without --allow-write");
    assert!(
        format!("{err:?}").contains("allow-write"),
        "the refusal should name the missing flag: {err:?}"
    );

    let _ = client.cancel().await;
}
