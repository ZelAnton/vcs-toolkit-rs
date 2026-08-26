use super::*;
use processkit::testing::{Reply, ScriptedRunner};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use vcs_core::vcs_git::Git;
use vcs_core::vcs_jj::Jj;

/// A git-backed server over a scripted runner — no real binary, no forge.
fn git_server(runner: ScriptedRunner, writes: WriteGate) -> VcsMcpServer {
    let repo: Arc<dyn VcsRepo> =
        Arc::new(Repo::from_git("/repo", "/repo", Git::with_runner(runner)));
    VcsMcpServer::from_handles(repo, None, writes)
}

/// A jj-backed server over a scripted runner — no real binary, no forge.
fn jj_server(runner: ScriptedRunner, writes: WriteGate) -> VcsMcpServer {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_jj("/repo", "/repo", Jj::with_runner(runner)));
    VcsMcpServer::from_handles(repo, None, writes)
}

/// The JSON of a successful tool result (serialised wire form).
fn result_json(r: &CallToolResult) -> String {
    serde_json::to_string(r).expect("CallToolResult serialises")
}

// A read tool calls the facade and returns its DTO as JSON.
#[tokio::test]
async fn read_tool_returns_dto_json() {
    let server = git_server(
        ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")),
        WriteGate::None,
    );
    let out = server.repo_current_branch().await.expect("tool ok");
    assert!(result_json(&out).contains("main"), "{}", result_json(&out));
}

// R1: `begin_repo_write` checks the gate and, when allowed, *holds* the per-repo
// write lock for the caller's duration — so concurrent repo mutations serialize.
// A disabled write returns the gate error without taking the lock.
#[tokio::test]
async fn begin_repo_write_gates_then_holds_the_lock() {
    let server = git_server(ScriptedRunner::new(), WriteGate::All);
    let guard = server
        .begin_repo_write("repo_commit")
        .await
        .expect("allowed → guard");
    assert!(
        server.write_lock.try_lock().is_err(),
        "the write lock is held while a guard is outstanding"
    );
    drop(guard);
    assert!(
        server.write_lock.try_lock().is_ok(),
        "the lock is released once the guard drops"
    );

    // Read-only server: the gate rejects before any lock is taken.
    let ro = git_server(ScriptedRunner::new(), WriteGate::None);
    assert!(
        ro.begin_repo_write("repo_commit").await.is_err(),
        "a gated write is rejected"
    );
    assert!(
        ro.write_lock.try_lock().is_ok(),
        "no lock is taken on the rejected path"
    );
}

// Read tools work even when writes are disabled (the default).
#[tokio::test]
async fn read_tool_works_in_readonly_mode() {
    let server = git_server(
        ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0")),
        WriteGate::None,
    );
    let out = server.repo_status().await.expect("status ok");
    assert!(result_json(&out).contains("a.rs"));
}

// `repo_log` is a read tool (no write gate) that surfaces the facade's
// unified `Commit` DTO as JSON, author/date included on git.
#[tokio::test]
async fn repo_log_returns_commit_json() {
    let server = git_server(
        ScriptedRunner::new().on(
            ["git", "log"],
            Reply::ok("deadbeef\ndead\nJane\n2026-05-31T10:00:00+00:00\nFix bug\0"),
        ),
        WriteGate::None,
    );
    let out = server
        .repo_log(Parameters(LogParams {
            revspec_or_revset: "HEAD".into(),
            max: 10,
        }))
        .await
        .expect("repo_log ok");
    let json = result_json(&out);
    assert!(json.contains("deadbeef"), "{json}");
    assert!(json.contains("Fix bug"), "{json}");
    assert!(json.contains("Jane"), "{json}");
}

#[tokio::test]
async fn repo_op_log_is_ungated_readonly_and_returns_typed_json() {
    assert!(!WRITE_TOOLS.contains(&"repo_op_log"));
    let server = jj_server(
        ScriptedRunner::new().on(
            ["jj", "op", "log"],
            Reply::ok("abc\t\"agent@host\"\t2026-07-29T10:00:00+02:00\t\"describe commit\"\n"),
        ),
        WriteGate::None,
    );
    let out = server
        .repo_op_log(Parameters(OpLogParams { max: 5 }))
        .await
        .expect("repo_op_log ok");
    let json = result_json(&out);
    assert!(json.contains("abc"), "{json}");
    assert!(json.contains("agent@host"), "{json}");
    assert!(json.contains("describe commit"), "{json}");

    let git = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = git
        .repo_op_log(Parameters(OpLogParams { max: 5 }))
        .await
        .expect_err("Git has no faithful operation-log equivalent");
    assert!(format!("{err:?}").contains("unsupported"), "{err:?}");
}

// `repo_annotate` is an ungated read tool that serializes the facade's
// unified line-attribution DTO, including git's asymmetric author/date.
#[tokio::test]
async fn repo_annotate_returns_content_json() {
    let sha = "a".repeat(40);
    let server = git_server(
        ScriptedRunner::new().on(
            ["git", "blame"],
            Reply::ok(format!(
                "{sha} 2 5 1\nauthor Jane\nauthor-time 1717700000\nauthor-tz +0200\n\tlet x = 1;\n"
            )),
        ),
        WriteGate::None,
    );
    let out = server
        .repo_annotate(Parameters(AnnotateParams {
            path: "src/lib.rs".into(),
            rev: Some("HEAD~1".into()),
        }))
        .await
        .expect("repo_annotate ok");
    let json = result_json(&out);
    assert!(json.contains(&sha), "{json}");
    assert!(json.contains("let x = 1;"), "{json}");
    assert!(json.contains("Jane"), "{json}");
    assert!(json.contains("1717700000"), "{json}");
}

// `repo_show_file` is a read tool (no write gate) that surfaces the facade's
// file content verbatim.
#[tokio::test]
async fn repo_show_file_returns_content() {
    let server = git_server(
        ScriptedRunner::new().on(["git", "show"], Reply::ok("fn main() {}\n")),
        WriteGate::None,
    );
    let out = server
        .repo_show_file(Parameters(ShowFileParams {
            rev: "HEAD".into(),
            path: "src/main.rs".into(),
        }))
        .await
        .expect("repo_show_file ok");
    let json = result_json(&out);
    assert!(json.contains("fn main"), "{json}");
}

// T-049: the MCP server INHERITS the output budget of the client its `Repo` was
// built over — a `repo_show_file` whose content exceeds the budget surfaces as a
// tool error (the wrapped `OutputTooLarge`), never a silently truncated file. A
// budget below the ceiling returns the content in full.
// T-130: unaffected by processkit 3.0's raw-pipe-byte accounting — a content tool
// reads RAW stdout, whose byte accounting 3.0 left untouched, and the fixture is
// ~3x the ceiling under either unit. The exact boundary (and the line-pumped
// stream that DID shift) is pinned in `vcs_cli_support`'s `content_budget_*` tests.
#[tokio::test]
async fn repo_show_file_honours_inherited_output_budget() {
    let big = "x".repeat(200_000);
    // Over budget → the tool errors instead of returning a clipped file.
    let budgeted = Git::with_runner(ScriptedRunner::new().on(["git", "show"], Reply::ok(&big)))
        .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", budgeted));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let err = server
        .repo_show_file(Parameters(ShowFileParams {
            rev: "HEAD".into(),
            path: "big.bin".into(),
        }))
        .await
        .expect_err("over-budget show_file must error, not truncate");
    assert!(
        format!("{err:?}").to_lowercase().contains("ceiling")
            || format!("{err:?}").to_lowercase().contains("too large")
            || format!("{err:?}").to_lowercase().contains("exceeded"),
        "error should name the output ceiling: {err:?}"
    );

    // Under the same budget a small file still reads in full.
    let small =
        Git::with_runner(ScriptedRunner::new().on(["git", "show"], Reply::ok("fn main() {}\n")))
            .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", small));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let out = server
        .repo_show_file(Parameters(ShowFileParams {
            rev: "HEAD".into(),
            path: "src/main.rs".into(),
        }))
        .await
        .expect("under-budget show_file ok");
    assert!(result_json(&out).contains("fn main"));
}

// `repo_diff` is a read tool (no write gate) that surfaces the facade's full
// parsed working-copy diff as JSON.
#[tokio::test]
async fn repo_diff_returns_parsed_diff() {
    let out_text = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
    let server = git_server(
        ScriptedRunner::new()
            .on(["git", "rev-parse"], Reply::ok("deadbeef\n")) // HEAD resolves
            .on(["git", "diff"], Reply::ok(out_text)),
        WriteGate::None,
    );
    let out = server.repo_diff().await.expect("repo_diff ok");
    let json = result_json(&out);
    assert!(json.contains("\\\"m\\\""), "{json}");
    assert!(json.contains("Modified"), "{json}");
}

// T-049/T-068: `repo_diff` INHERITS the output budget of the client its `Repo`
// was built over, exactly like `repo_show_file` — an over-budget diff surfaces
// as a tool error (the wrapped `OutputTooLarge`), never a silently truncated
// diff. A budget below the ceiling returns the diff in full.
// T-130: unaffected by processkit 3.0's raw-pipe-byte accounting (raw-stdout
// capture, unchanged unit; fixture ~4x the ceiling) — see the note on
// `repo_show_file_honours_inherited_output_budget`.
#[tokio::test]
async fn repo_diff_honours_inherited_output_budget() {
    let big = "diff --git a/m b/m\n".to_string() + &"+x\n".repeat(100_000);
    // Over budget → the tool errors instead of returning a clipped diff.
    let budgeted = Git::with_runner(
        ScriptedRunner::new()
            .on(["git", "rev-parse"], Reply::ok("deadbeef\n"))
            .on(["git", "diff"], Reply::ok(&big)),
    )
    .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", budgeted));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let err = server
        .repo_diff()
        .await
        .expect_err("over-budget diff must error, not truncate");
    assert!(
        format!("{err:?}").to_lowercase().contains("ceiling")
            || format!("{err:?}").to_lowercase().contains("too large")
            || format!("{err:?}").to_lowercase().contains("exceeded"),
        "error should name the output ceiling: {err:?}"
    );

    // Under the same budget a small diff still reads in full.
    let small_text = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
    let small = Git::with_runner(
        ScriptedRunner::new()
            .on(["git", "rev-parse"], Reply::ok("deadbeef\n"))
            .on(["git", "diff"], Reply::ok(small_text)),
    )
    .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", small));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let out = server.repo_diff().await.expect("under-budget diff ok");
    assert!(result_json(&out).contains("Modified"));
}

// `repo_info` is a plain UTF-8 round trip in the ordinary case: `backend`,
// `root`, `cwd`, `forge` all surface as JSON strings (the regression below
// covers the non-UTF-8 fail-closed case).
#[tokio::test]
async fn repo_info_returns_utf8_paths() {
    // `/repo` and `/repo/sub` are Unix-absolute but Windows-drive-relative;
    // `Repo::from_git` absolutises `root`/`cwd` at construction (T-114), so
    // `repo_info` reports the absolutised forms (drive-qualified on Windows).
    let root = std::path::absolute("/repo").unwrap();
    let cwd = std::path::absolute("/repo/sub").unwrap();
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        root.clone(),
        cwd.clone(),
        Git::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let out = server.repo_info().await.expect("repo_info ok");
    // Parse the tool's own JSON body rather than substring-matching the escaped
    // outer wire form: a Windows path's backslashes are JSON-escaped (and doubly
    // so through the `CallToolResult` envelope), which a raw `contains` on the
    // path can't reliably match. Parsing un-escapes both, keeping the check
    // portable.
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("repo_info JSON");
    assert_eq!(value["backend"], "git", "{text}");
    assert_eq!(value["root"].as_str(), root.to_str(), "{text}");
    assert_eq!(value["cwd"].as_str(), cwd.to_str(), "{text}");
    assert!(value.get("forge").is_some(), "forge field present: {text}");
}

// T-062: `repo_info`'s `root`/`cwd` used to serialise through
// `to_string_lossy`, silently emitting `U+FFFD` for a non-UTF-8 root/cwd
// (legal on Unix). They now go through the same fail-closed path as every
// other path-bearing DTO in this crate (see `ok_json`'s doc comment): a
// non-UTF-8 root/cwd must fail the call instead of returning corrupted JSON.
#[cfg(unix)]
#[tokio::test]
async fn repo_info_rejects_non_utf8_root_instead_of_lossy_substituting() {
    let bad = std::path::PathBuf::from(vcs_testkit::non_utf8_filename());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        bad.clone(),
        bad,
        Git::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
    let err = server
        .repo_info()
        .await
        .expect_err("a non-UTF-8 root/cwd must be refused, not lossy-substituted");
    assert!(
        format!("{err:?}").to_lowercase().contains("utf-8"),
        "error should name the UTF-8 refusal: {err:?}"
    );
}

// A mutation tool is gated when writes are disabled — it errors WITHOUT
// reaching the runner. The scripted runner has NO `checkout` rule, so if the
// gate failed and the tool spawned, the call would error differently than the
// gate's `--allow-write` message.
#[tokio::test]
async fn mutation_is_gated_without_allow_write() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_checkout(Parameters(CheckoutParams {
            reference: "feat".into(),
        }))
        .await
        .expect_err("gated");
    assert!(
        format!("{err:?}").contains("allow-write"),
        "error should mention --allow-write: {err:?}"
    );
}

// `repo_try_merge` is write-gated: it spawns a real trial merge that
// materializes working-tree content (which on an untrusted repo can run
// repo-local filter/textconv drivers), so it must NOT be callable in the default
// read-only mode — unlike the genuinely read-only tools.
#[tokio::test]
async fn try_merge_is_write_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_try_merge(Parameters(TryMergeParams {
            source: "feat".into(),
        }))
        .await
        .expect_err("try_merge must be gated in read-only mode");
    assert!(
        format!("{err:?}").contains("allow-write"),
        "error should mention --allow-write: {err:?}"
    );
}

// With writes enabled, the same tool reaches the runner and returns success.
#[tokio::test]
async fn mutation_reaches_runner_with_allow_write() {
    let server = git_server(
        ScriptedRunner::new().on(["git", "checkout"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_checkout(Parameters(CheckoutParams {
            reference: "feat".into(),
        }))
        .await
        .expect("checkout ok");
    assert!(result_json(&out).contains("feat"));
}

// repo_push is a gated mutation: blocked read-only, and with writes enabled
// it drives the facade's `push -u origin <branch>` (only ["push"] is
// scripted, so a different argv shape would error).
#[tokio::test]
async fn repo_push_is_gated_and_pushes_branch() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_push(Parameters(PushParams {
            branch: "feature".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "push"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_push(Parameters(PushParams {
            branch: "feature".into(),
        }))
        .await
        .expect("push ok");
    assert!(result_json(&out).contains("feature"));
}

// A Set gate admits exactly the named mutations: the listed tool runs, an
// unlisted one is rejected (naming itself), and read tools stay available.
#[tokio::test]
async fn allow_tools_set_gates_per_tool() {
    let gate = WriteGate::Set(
        ["repo_checkout".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
    );
    let server = git_server(
        ScriptedRunner::new()
            .on(["git", "checkout"], Reply::ok(""))
            .on(["git", "symbolic-ref"], Reply::ok("main\n")),
        gate,
    );

    // Listed mutation runs.
    server
        .repo_checkout(Parameters(CheckoutParams {
            reference: "feat".into(),
        }))
        .await
        .expect("listed tool allowed");

    // Unlisted mutation is rejected, naming the tool.
    let err = server.repo_fetch().await.expect_err("unlisted gated");
    assert!(format!("{err:?}").contains("repo_fetch"), "{err:?}");

    // Read tools are unaffected by the allowlist.
    server.repo_current_branch().await.expect("read tool ok");
}

// The facade's refused-input errors (here: an empty `paths` set, which the
// facade rejects up front) surface as INVALID_PARAMS — the client's mistake
// to fix — not as an internal server error.
#[tokio::test]
async fn refused_input_surfaces_as_invalid_params() {
    let server = git_server(ScriptedRunner::new(), WriteGate::All);
    let err = server
        .repo_commit(Parameters(CommitParams {
            paths: vec![],
            message: "msg".into(),
        }))
        .await
        .expect_err("empty paths refused");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        err.message.contains("at least one path"),
        "unexpected message: {}",
        err.message
    );
}

// A flag-like ref/revision tool parameter is rejected the moment the facade
// converts it into the validated newtype (`RefName`/`RevSpec`) — surfacing as
// INVALID_PARAMS (a classifiable client mistake) *before* any git process
// spawns, rather than an opaque internal error. The runner has no `git log`
// scripted, so had the value NOT been refused pre-spawn the command would have
// surfaced as an internal error instead — the INVALID_PARAMS code is the proof
// the rejection happened at the boundary.
#[tokio::test]
async fn flag_like_revspec_surfaces_as_invalid_params() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_log(Parameters(LogParams {
            revspec_or_revset: "--upload-pack=/bin/evil".into(),
            max: 10,
        }))
        .await
        .expect_err("a flag-like revspec must be refused");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// Forge tools report a clear error when no forge was configured.
#[tokio::test]
async fn forge_tools_error_without_a_forge() {
    let server = git_server(ScriptedRunner::new(), WriteGate::All);
    let err = server
        .forge_pr_list(Parameters(PrListParams::default()))
        .await
        .expect_err("no forge");
    assert!(
        format!("{err:?}").contains("no forge"),
        "should mention no forge: {err:?}"
    );
    let err = server
        .forge_pr_for_branch(Parameters(PrForBranchParams {
            source_branch: "feat/x".into(),
        }))
        .await
        .expect_err("no forge");
    assert!(format!("{err:?}").contains("no forge"), "{err:?}");
}

// Source-branch lookup is an ungated read that returns any-state PRs through
// the forge facade.
#[tokio::test]
async fn forge_pr_for_branch_routes_without_write_access() {
    let json = r#"[{"number":3,"title":"Bug","state":"MERGED","isDraft":false,"headRefName":"feat/x","baseRefName":"main","url":"u"}]"#;
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let out = server
        .forge_pr_for_branch(Parameters(PrForBranchParams {
            source_branch: "feat/x".into(),
        }))
        .await
        .expect("branch lookup");
    assert!(
        result_json(&out).contains("Merged"),
        "{}",
        result_json(&out)
    );
}

// The forge issue tools route to the forge handle: the read tool works in
// read-only mode and returns the unified DTO JSON; the create tool is gated.
#[tokio::test]
async fn forge_issue_tools_route_and_gate() {
    let json = r#"[{"number":3,"title":"Bug","state":"OPEN"}]"#;
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "issue", "list"], Reply::ok(json)),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let out = server
        .forge_issue_list(Parameters(IssueListParams::default()))
        .await
        .expect("issue list ok");
    assert!(result_json(&out).contains("Bug"));

    let err = server
        .forge_issue_create(Parameters(IssueCreateParams {
            title: "t".into(),
            body: "b".into(),
            labels: Vec::new(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
}

#[tokio::test]
async fn forge_list_tools_forward_optional_state_and_limit() {
    let runner = ScriptedRunner::new()
        .on(
            ["gh", "pr", "list", "--state", "merged", "--limit", "7"],
            Reply::ok("[]"),
        )
        .on(
            ["gh", "issue", "list", "--state", "closed", "--limit", "9"],
            Reply::ok("[]"),
        );
    let gh = vcs_forge::vcs_github::GitHub::with_runner(runner);
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    server
        .forge_pr_list(Parameters(PrListParams {
            state: Some(PrListStateArg::Merged),
            limit: Some(7),
        }))
        .await
        .expect("filtered PR list");
    server
        .forge_issue_list(Parameters(IssueListParams {
            state: Some(IssueListStateArg::Closed),
            limit: Some(9),
        }))
        .await
        .expect("filtered issue list");
}

// The three issue-lifecycle mutations are write-gated: refused under
// `WriteGate::None`, and (when allowed) routed to the right `gh` verb — `issue
// close`/`issue reopen`/`issue comment` (the runner rule matches only the leading
// tokens, so reaching the reply proves the routing).
#[tokio::test]
async fn forge_issue_close_reopen_comment_gate_and_route() {
    // Gated under WriteGate::None (no spawn needed).
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    for err in [
        server
            .forge_issue_close(Parameters(IssueNumberParams { number: 7 }))
            .await
            .expect_err("close gated"),
        server
            .forge_issue_reopen(Parameters(IssueNumberParams { number: 7 }))
            .await
            .expect_err("reopen gated"),
        server
            .forge_issue_comment(Parameters(IssueCommentParams {
                number: 7,
                body: "ping".into(),
            }))
            .await
            .expect_err("comment gated"),
    ] {
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
    }

    // Allowed: each routes to its `gh issue <verb>` command and reports the result.
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(["gh", "issue", "close"], Reply::ok(""))
            .on(["gh", "issue", "reopen"], Reply::ok(""))
            .on(["gh", "issue", "comment"], Reply::ok("https://gh/i/7#c1\n")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let out = server
        .forge_issue_close(Parameters(IssueNumberParams { number: 7 }))
        .await
        .expect("close ok");
    assert!(
        result_json(&out).contains("closed"),
        "{}",
        result_json(&out)
    );

    let out = server
        .forge_issue_reopen(Parameters(IssueNumberParams { number: 7 }))
        .await
        .expect("reopen ok");
    assert!(
        result_json(&out).contains("reopened"),
        "{}",
        result_json(&out)
    );

    let out = server
        .forge_issue_comment(Parameters(IssueCommentParams {
            number: 7,
            body: "ping".into(),
        }))
        .await
        .expect("comment ok");
    assert!(
        result_json(&out).contains("gh/i/7"),
        "{}",
        result_json(&out)
    );
}

// `forge_issue_comment` rejects an empty body up front (invalid_params) — the
// facade's empty-body guard surfaced through the tool, before any spawn.
#[tokio::test]
async fn forge_issue_comment_empty_body_is_invalid_params() {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let err = server
        .forge_issue_comment(Parameters(IssueCommentParams {
            number: 7,
            body: "   ".into(),
        }))
        .await
        .expect_err("empty body rejected");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// `forge_pr_diff` is read-only (works with no write access) and returns the
// parsed per-file diff as JSON.
#[tokio::test]
async fn forge_pr_diff_routes_and_returns_parsed_diff() {
    let diff = "diff --git a/notes.txt b/notes.txt\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1 @@\n-a\n+b\n";
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(diff)),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let out = server
        .forge_pr_diff(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect("pr_diff ok");
    // `result_json` serialises the whole `CallToolResult`, so the tool's own
    // JSON text comes back escaped inside it — match unquoted substrings.
    let json = result_json(&out);
    assert!(json.contains("notes.txt"), "{json}");
    assert!(json.contains("Modified"), "{json}");
}

// T-049: `forge_pr_diff` inherits the output budget of the forge client the
// server was built over — an over-budget PR diff surfaces as a tool error
// (the wrapped `OutputTooLarge`), never a truncated diff.
// T-130: unaffected by processkit 3.0's raw-pipe-byte accounting (raw-stdout
// capture, unchanged unit; fixture ~2x the ceiling) — see the note on
// `repo_show_file_honours_inherited_output_budget`.
#[tokio::test]
async fn forge_pr_diff_honours_inherited_output_budget() {
    let big = "diff --git a/m b/m\n".to_string() + &"+line\n".repeat(20_000);
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(&big)),
    )
    .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_pr_diff(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect_err("over-budget pr_diff must error, not truncate");
    assert!(
        format!("{err:?}").to_lowercase().contains("ceiling")
            || format!("{err:?}").to_lowercase().contains("too large")
            || format!("{err:?}").to_lowercase().contains("exceeded"),
        "error should name the output ceiling: {err:?}"
    );
}

// A forge op the backend can't do (tea has no single-release view) surfaces
// as INVALID_PARAMS — the client's "this forge can't do that" — without
// spawning anything (the runner has no rules, so a spawn would error
// differently).
#[tokio::test]
async fn forge_release_view_unsupported_maps_to_invalid_params() {
    let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let err = server
        .forge_release_view(Parameters(ReleaseTagParams { tag: "v1".into() }))
        .await
        .expect_err("unsupported on gitea");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("release_view"), "{}", err.message);
}

// Same treatment for `forge_pr_diff` (tea has no diff command).
#[tokio::test]
async fn forge_pr_diff_unsupported_maps_to_invalid_params() {
    let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let err = server
        .forge_pr_diff(Parameters(PrNumberParams { number: 1 }))
        .await
        .expect_err("unsupported on gitea");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("pr_diff"), "{}", err.message);
}

// The two new mutating tools (`forge_pr_comment`, `forge_pr_edit`) are
// gated like the existing `forge_pr_create` / `forge_pr_close`: the
// runner has no `pr comment` / `pr edit` rule, so a leak-through would
// error differently than the gate's `--allow-write` message.
#[tokio::test]
async fn forge_pr_comment_is_gated() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let err = server
        .forge_pr_comment(Parameters(PrCommentParams {
            number: 7,
            body: "hi".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
}

#[tokio::test]
async fn forge_pr_edit_is_gated() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let err = server
        .forge_pr_edit(Parameters(PrEditParams {
            number: 7,
            title: Some("T".into()),
            body: None,
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
}

#[tokio::test]
async fn forge_pr_mark_ready_is_gated() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let err = server
        .forge_pr_mark_ready(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
}

// `forge_release_create` is write-gated: refused under `WriteGate::None`, routed to
// `gh release create` when allowed (the runner rule matches only
// `["gh","release","create"]`, so reaching the reply proves the routing) and
// returns the CLI's output.
#[tokio::test]
async fn forge_release_create_gates_and_routes() {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_release_create(Parameters(ReleaseCreateParams {
            tag: "v1".into(),
            title: Some("One".into()),
            notes: Some("N".into()),
            draft: false,
            prerelease: false,
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "release", "create"], Reply::ok("https://gh/r/v1\n")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = server
        .forge_release_create(Parameters(ReleaseCreateParams {
            tag: "v1".into(),
            title: Some("One".into()),
            notes: Some("N".into()),
            draft: true,
            prerelease: false,
        }))
        .await
        .expect("release_create ok");
    assert!(
        result_json(&out).contains("https://gh/r/v1"),
        "{}",
        result_json(&out)
    );
}

// On GitLab, `draft`/`prerelease` are unsupported — the facade surfaces
// `Unsupported`, which the MCP layer maps to INVALID_PARAMS, without spawning.
#[tokio::test]
async fn forge_release_create_draft_unsupported_on_gitlab_maps_to_invalid_params() {
    let glab = vcs_forge::vcs_gitlab::GitLab::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", glab));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let err = server
        .forge_release_create(Parameters(ReleaseCreateParams {
            tag: "v1".into(),
            title: None,
            notes: None,
            draft: true,
            prerelease: false,
        }))
        .await
        .expect_err("draft unsupported on gitlab");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("release_create"), "{}", err.message);
}

// `forge_release_delete` is write-gated: refused under `WriteGate::None`, routed to
// `gh release delete` when allowed.
#[tokio::test]
async fn forge_release_delete_gates_and_routes() {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_release_delete(Parameters(ReleaseTagParams { tag: "v1".into() }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "release", "delete"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = server
        .forge_release_delete(Parameters(ReleaseTagParams { tag: "v1".into() }))
        .await
        .expect("release_delete ok");
    assert!(
        result_json(&out).contains("deleted"),
        "{}",
        result_json(&out)
    );
}

// `forge_pr_approve` is write-gated: refused under `WriteGate::None`, routed to
// `gh pr review --approve` when allowed (the runner rule matches only
// `["gh","pr","review"]`, so reaching the reply proves the routing).
#[tokio::test]
async fn forge_pr_approve_gates_and_routes() {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_pr_approve(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "review"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = server
        .forge_pr_approve(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect("approve ok");
    assert!(
        result_json(&out).contains("approved"),
        "{}",
        result_json(&out)
    );
}

// `forge_pr_request_changes` is write-gated and routes to `gh pr review
// --request-changes`; on GitLab it maps to the facade's `Unsupported`
// (invalid_params), and an empty body is rejected up front — both without a spawn.
#[tokio::test]
async fn forge_pr_request_changes_gates_routes_and_unsupported_on_gitlab() {
    // Gated under WriteGate::None.
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_pr_request_changes(Parameters(PrRequestChangesParams {
            number: 7,
            body: "please fix".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    // Allowed on GitHub: routes to `gh pr review`.
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "review"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = server
        .forge_pr_request_changes(Parameters(PrRequestChangesParams {
            number: 7,
            body: "please fix".into(),
        }))
        .await
        .expect("request-changes ok");
    assert!(
        result_json(&out).contains("requested_changes"),
        "{}",
        result_json(&out)
    );

    // GitLab: Unsupported → invalid_params, without spawning (no runner rule).
    let glab = vcs_forge::vcs_gitlab::GitLab::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", glab));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let err = server
        .forge_pr_request_changes(Parameters(PrRequestChangesParams {
            number: 7,
            body: "please fix".into(),
        }))
        .await
        .expect_err("unsupported on gitlab");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        err.message.contains("pr_request_changes"),
        "{}",
        err.message
    );

    // An empty body is rejected up front (invalid_params), also without a spawn.
    let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let err = server
        .forge_pr_request_changes(Parameters(PrRequestChangesParams {
            number: 7,
            body: "   ".into(),
        }))
        .await
        .expect_err("empty body rejected");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// `forge_pr_checkout` is write-gated like the other forge mutations: refused
// under `WriteGate::None`, but routed to `gh pr checkout <n>` when allowed.
#[tokio::test]
async fn forge_pr_checkout_gates_and_routes() {
    // Gated: refused before any spawn.
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
        "/repo",
        vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
    ));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
    let err = server
        .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    // Allowed: routes to `gh pr checkout` and reports the checked-out number.
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "checkout"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = server
        .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect("checkout ok");
    assert!(
        result_json(&out).contains("checked_out"),
        "{}",
        result_json(&out)
    );
}

// `forge_pr_merge` is write-gated; when allowed it maps the strategy plus the
// GitHub-only `auto`/`delete_branch` params onto gh's own flags. The runner
// rule matches only `["gh", "pr", "merge"]`, so reaching the reply proves the
// whole spec was routed to the wrapper.
#[tokio::test]
async fn forge_pr_merge_routes_strategy_and_github_options() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "merge"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let out = server
        .forge_pr_merge(Parameters(PrMergeParams {
            number: 7,
            strategy: MergeStrategyArg::Squash,
            auto: true,
            delete_branch: true,
        }))
        .await
        .expect("merge ok");
    assert!(
        result_json(&out).contains("merged"),
        "{}",
        result_json(&out)
    );
}

// The GitHub-only `auto`/`delete_branch` merge options are rejected as
// `invalid_params` on GitLab/Gitea — the facade's `Unsupported` (bubbled from
// the wrapper) is a client-fixable request, not an internal error — and nothing
// spawns (the runner has no rule).
#[tokio::test]
async fn forge_pr_merge_unsupported_options_map_to_invalid_params() {
    let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let err = server
        .forge_pr_merge(Parameters(PrMergeParams {
            number: 7,
            strategy: MergeStrategyArg::Merge,
            auto: true,
            delete_branch: false,
        }))
        .await
        .expect_err("auto is unsupported on gitea");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
}

// T-112: a mutating tool refused by the facade's pre-spawn **version gate** (the
// installed CLI is confirmed below the crate's floor) maps to `invalid_params` —
// the caller can fix it by upgrading the CLI, so it's a client-facing request
// error, not an internal one. Here `gh --version` reports 1.14.0 (< the 2.0
// floor), so `forge_pr_create` is version-gated before `gh pr create` spawns.
#[tokio::test]
async fn forge_version_gated_mutation_maps_to_invalid_params() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "--version"], Reply::ok("gh version 1.14.0\n")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let err = server
        .forge_pr_create(Parameters(PrCreateParams {
            title: "T".into(),
            body: "B".into(),
            source: None,
            target: None,
            labels: Vec::new(),
        }))
        .await
        .expect_err("an old gh must version-gate the mutation");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    // The message names the operation and the version mismatch (client-actionable).
    assert!(err.message.contains("pr_create"), "{}", err.message);
}

// T-058/T-133: `forge_pr_checkout`, `forge_pr_merge`, and `forge_pr_close` locally mutate the working
// copy (checkout/switch), so — unlike the other forge tools — they must go
// through `begin_repo_write` and actually hold the same per-repo `write_lock`
// as `repo_*` mutations, not just call the gate-only `require_write`. Prove it
// by holding the lock ourselves first: the tool call must then block (time out)
// rather than run past the lock acquisition, and must succeed once the lock is
// released.
#[tokio::test]
async fn forge_pr_checkout_merge_and_close_hold_the_repo_write_lock() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(["gh", "pr", "checkout"], Reply::ok(""))
            .on(["gh", "pr", "merge"], Reply::ok(""))
            .on(["gh", "pr", "close"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    // Hold the write lock ourselves (simulating a concurrent repo_* mutation
    // in flight), then attempt both forge tools — both must block on the same
    // lock rather than run through immediately.
    let outer_guard = server
        .write_lock
        .clone()
        .try_lock_owned()
        .expect("uncontended at test start");

    let checkout_timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        server.forge_pr_checkout(Parameters(PrNumberParams { number: 7 })),
    )
    .await
    .is_err();
    assert!(
        checkout_timed_out,
        "forge_pr_checkout must block while the repo write lock is held elsewhere"
    );

    let merge_timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        server.forge_pr_merge(Parameters(PrMergeParams {
            number: 7,
            strategy: MergeStrategyArg::Merge,
            auto: false,
            delete_branch: false,
        })),
    )
    .await
    .is_err();
    assert!(
        merge_timed_out,
        "forge_pr_merge must block while the repo write lock is held elsewhere"
    );

    let close_timed_out = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        server.forge_pr_close(Parameters(PrCloseParams {
            number: 7,
            delete_branch: true,
        })),
    )
    .await
    .is_err();
    assert!(
        close_timed_out,
        "forge_pr_close must block while the repo write lock is held elsewhere"
    );

    // Release the lock: both calls now go through and route to the wrapper.
    drop(outer_guard);

    let out = server
        .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
        .await
        .expect("checkout ok once the lock is free");
    assert!(
        result_json(&out).contains("checked_out"),
        "{}",
        result_json(&out)
    );

    let out = server
        .forge_pr_merge(Parameters(PrMergeParams {
            number: 7,
            strategy: MergeStrategyArg::Merge,
            auto: false,
            delete_branch: false,
        }))
        .await
        .expect("merge ok once the lock is free");
    assert!(
        result_json(&out).contains("merged"),
        "{}",
        result_json(&out)
    );

    let out = server
        .forge_pr_close(Parameters(PrCloseParams {
            number: 7,
            delete_branch: true,
        }))
        .await
        .expect("close ok once the lock is free");
    assert!(
        result_json(&out).contains("closed"),
        "{}",
        result_json(&out)
    );
}

// T-013: on GitHub a `body` that begins with `-` is a legitimate Markdown
// value (a `- item` bullet list, or a `---` rule), not a flag — `gh pr comment
// --body <body>` puts it in a flag-VALUE slot. The MCP layer must NOT reject it
// (the old blanket `guard_argv_field` did). The runner rule matches only
// `["gh", "pr", "comment"]`, so reaching the reply proves the body was passed
// through to the wrapper rather than refused up front.
#[tokio::test]
async fn forge_pr_comment_github_allows_leading_dash_body() {
    for body in ["- item one\n- item two", "---"] {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "comment"], Reply::ok("https://gh/pr/7#c1")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let out = server
            .forge_pr_comment(Parameters(PrCommentParams {
                number: 7,
                body: body.into(),
            }))
            .await
            .unwrap_or_else(|e| panic!("leading-`-` body {body:?} must pass on GitHub: {e:?}"));
        assert!(
            result_json(&out).contains("https://gh/pr/7#c1"),
            "{}",
            result_json(&out)
        );
    }
}

// T-013: the same on GitLab — `glab mr note <id> -m <body>` is a flag-VALUE
// slot, so a leading `-` is safe and must pass.
#[tokio::test]
async fn forge_pr_comment_gitlab_allows_leading_dash_body() {
    let gl = vcs_forge::vcs_gitlab::GitLab::with_runner(
        ScriptedRunner::new().on(["glab", "mr", "note"], Reply::ok("https://gl/mr/7#note1")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", gl));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let out = server
        .forge_pr_comment(Parameters(PrCommentParams {
            number: 7,
            body: "- a bullet".into(),
        }))
        .await
        .expect("leading-`-` body must pass on GitLab");
    assert!(
        result_json(&out).contains("https://gl/mr/7#note1"),
        "{}",
        result_json(&out)
    );
}

// T-013 regression: Gitea's `tea comment <n> <body>` takes the body as a bare
// POSITIONAL, so a flag-like body IS dangerous there and stays rejected — by
// the Gitea wrapper's own `reject_flag_like`, reached through the MCP tool. The
// runner has a `["tea", "comment"]` rule, so a leak-through would SUCCEED
// (returning the reply) instead of erroring — this pins that it does not.
#[tokio::test]
async fn forge_pr_comment_gitea_rejects_flag_like_body() {
    let tea = vcs_forge::vcs_gitea::Gitea::with_runner(
        ScriptedRunner::new().on(["tea", "comment"], Reply::ok("https://gitea/pr/7#c1")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let err = server
        .forge_pr_comment(Parameters(PrCommentParams {
            number: 7,
            body: "-evil".into(),
        }))
        .await
        .expect_err("flag-like body must stay rejected on Gitea's positional slot");
    assert!(err.message.contains("flag"), "{}", err.message);
}

// T-013: `forge_pr_edit` also passes leading-`-` `title`/`body` through — both
// ride in flag-VALUE slots (`gh pr edit --title <t> --body <b>`), so a Markdown
// bullet title or a `---` body is legitimate and must not be refused.
#[tokio::test]
async fn forge_pr_edit_allows_leading_dash_title_and_body() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "edit"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let out = server
        .forge_pr_edit(Parameters(PrEditParams {
            number: 7,
            title: Some("- a bullet title".into()),
            body: Some("---".into()),
        }))
        .await
        .expect("leading-`-` title/body must pass on GitHub");
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["edited"], 7, "{text}");
}

// `forge_pr_edit` rejects both-`None` with an invalid-params error BEFORE
// reaching the wrapper — the facade's `InvalidInput` shape surfaces as
// `invalid_params` (per the updated `forge_err` mapping).
#[tokio::test]
async fn forge_pr_edit_both_none_is_invalid_params() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let err = server
        .forge_pr_edit(Parameters(PrEditParams {
            number: 7,
            title: None,
            body: None,
        }))
        .await
        .expect_err("both-None rejected");
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("title"), "{}", err.message);
}

// `Some("")` is a real value (clears the field). The MCP tool passes it
// through to the wrapper, and the wrapper's argv carries `--title ""`
// literally. This test pins the round-trip end to end: the
// `ScriptedRunner::on(["pr", "edit"], …)` rule matches **only** an argv
// whose first two elements are exactly `["pr", "edit"]` (a different
// command, or a different argv shape, would fall through and the call
// would error). Combined with the response shape check, the round-trip
// is fully verified.
#[tokio::test]
async fn forge_pr_edit_some_empty_string_passes_through() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new().on(["gh", "pr", "edit"], Reply::ok("")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

    let out = server
        .forge_pr_edit(Parameters(PrEditParams {
            number: 7,
            title: Some("".into()),
            body: None,
        }))
        .await
        .expect("empty title accepted");
    // `ok_json` uses `to_string_pretty`; pull the inner text and check
    // the `edited` field is present (number == 7).
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["edited"], 7, "{text}");
}

// `forge_info` is read-only: a no-forge server errors with the same
// "no forge is configured" message every other forge tool uses (per the
// Q6 override).
#[tokio::test]
async fn forge_info_without_a_forge_errors() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server.forge_info().await.expect_err("no forge");
    assert!(format!("{err:?}").contains("no forge"), "{err:?}");
}

// `forge_info` returns the kind string + capability map for an authed
// GitHub handle on a modern `gh`. `capabilities()` probes the CLI version
// (`gh --version`, scripted to a modern banner above the 2.0 floor) and auth
// (`auth status`, exit 0); every static cap is `true` post-fork, and the map
// now also carries `version`/`supported`. The `auth` block rides alongside it
// (its own `auth status` read plus, since a session exists, a `repo view`
// visibility probe) — asserted in full by the tests below.
#[tokio::test]
async fn forge_info_with_authed_github_reports_all_true() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(
                ["gh", "--version"],
                Reply::ok("gh version 2.40.1 (2024-01-05)\n"),
            )
            .on(["gh", "auth", "status"], Reply::ok(""))
            .on(["gh", "repo", "view"], Reply::ok(r#"{"name":"r"}"#)),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let out = server.forge_info().await.expect("forge_info ok");
    // Extract the inner text content (the JSON value) — `result_json`
    // re-serialises the whole `CallToolResult` with the `content`
    // envelope, so assertions on the inner JSON need the inner text.
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(value["kind"], "github");
    assert_eq!(value["capabilities"]["authed"], true);
    assert_eq!(value["capabilities"]["supported"], true);
    // `version` serialises as the structured `{major,minor,patch}` shape of
    // `vcs_diff::Version` (its derived `Serialize`).
    assert_eq!(
        value["capabilities"]["version"],
        serde_json::json!({ "major": 2, "minor": 40, "patch": 1 })
    );
    assert_eq!(value["capabilities"]["pr_create"], true);
    assert_eq!(value["capabilities"]["pr_comment"], true);
    assert_eq!(value["capabilities"]["pr_edit"], true);
    assert_eq!(value["capabilities"]["pr_checks"], true);
    assert_eq!(value["capabilities"]["pr_merge"], true);
    assert_eq!(value["capabilities"]["issue_create"], true);
    assert_eq!(value["capabilities"]["issue_close"], true);
    assert_eq!(value["capabilities"]["issue_reopen"], true);
    assert_eq!(value["capabilities"]["issue_comment"], true);
}

/// A `forge_info`-ready server whose `gh` is scripted with `auth_status` for the
/// account report and `repo_view` for the visibility probe.
fn github_info_server(auth: Reply, repo_view: Reply) -> VcsMcpServer {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(
                ["gh", "--version"],
                Reply::ok("gh version 2.40.1 (2024-01-05)\n"),
            )
            .on(["gh", "auth", "status"], auth)
            .on(["gh", "repo", "view"], repo_view),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None)
}

/// The inner JSON a tool result carries (`ok_json` wraps it in a text content
/// block).
fn tool_json(out: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    serde_json::from_str(&text).expect("valid JSON")
}

// The honest-auth block, in the shape the tool's description promises: with two
// logins on one host and a repository the ACTIVE one cannot see, `forge_info`
// reports `authed: true` (a session exists) next to `repo_visible: false` — the
// pair that explains a "Could not resolve to a Repository" failure before it
// happens — and names every logged-in account.
#[tokio::test]
async fn forge_info_reports_active_account_and_invisible_repo() {
    let report = "github.com\n  \u{2713} Logged in to github.com account work-acct (keyring)\n  \
                  - Active account: false\n\n  \
                  \u{2713} Logged in to github.com account personal (keyring)\n  \
                  - Active account: true\n";
    let server = github_info_server(
        Reply::ok(report),
        Reply::fail(
            1,
            "Could not resolve to a Repository with the name 'acme/x'.",
        ),
    );
    let value = tool_json(&server.forge_info().await.expect("forge_info ok"));

    // The pre-existing shape is untouched — the new block is purely additive.
    assert_eq!(value["kind"], "github");
    assert_eq!(value["capabilities"]["authed"], true);

    assert_eq!(value["auth"]["authed"], true, "{value}");
    assert_eq!(value["auth"]["repo_visible"], false, "{value}");
    assert_eq!(value["auth"]["active_account"], "personal", "{value}");
    assert_eq!(
        value["auth"]["accounts"],
        serde_json::json!([
            { "host": "github.com", "login": "work-acct", "active": false },
            { "host": "github.com", "login": "personal", "active": true },
        ]),
        "{value}"
    );
}

// End to end, the misattribution guard: a *failed* entry (a broken `GH_TOKEN`)
// printed AFTER the working login must not touch that login's active flag. gh
// exits non-zero as soon as any entry fails — so `authed` is honestly `false`
// while the report still names which account it does work as, and no visibility
// probe is spawned. Getting this wrong reports `active_account: null` plus an
// `active: false` account, i.e. a confident *negative* about the identity gh
// actually uses.
#[tokio::test]
async fn forge_info_keeps_the_active_account_when_a_failed_entry_follows_it() {
    let report = "github.com\n  \u{2713} Logged in to github.com account personal (keyring)\n  \
                  - Active account: true\n  - Git operations protocol: ssh\n\n  \
                  X Failed to log in to github.com account work (keyring)\n  \
                  - Active account: false\n  - The token in keyring is invalid.\n";
    let server = github_info_server(Reply::fail(1, report), Reply::ok(r#"{"name":"r"}"#));
    let value = tool_json(&server.forge_info().await.expect("forge_info ok"));

    assert_eq!(
        value["auth"]["authed"], false,
        "gh exits non-zero when any entry fails: {value}"
    );
    assert_eq!(value["auth"]["active_account"], "personal", "{value}");
    assert_eq!(
        value["auth"]["accounts"],
        serde_json::json!([
            { "host": "github.com", "login": "personal", "active": true },
        ]),
        "the rejected login is not a logged-in account, and its marker is its own: {value}"
    );
    assert_eq!(
        value["auth"]["repo_visible"],
        serde_json::Value::Null,
        "no session, so nothing to probe visibility with: {value}"
    );
}

// `forge_auth_status` keeps its fixed boolean shape — the richer data went into
// `forge_info` instead, so no existing consumer of this tool has to change.
#[tokio::test]
async fn forge_auth_status_stays_a_bare_boolean() {
    let server = github_info_server(Reply::ok("github.com\n"), Reply::ok(r#"{"name":"r"}"#));
    let value = tool_json(&server.forge_auth_status().await.expect("auth_status ok"));
    assert_eq!(value, serde_json::json!(true), "{value}");
}

// Fail-soft, end to end: a `gh auth status` format the wrapper doesn't model
// leaves the identity fields null/empty in the JSON — `forge_info` still answers
// instead of failing, so an upgraded `gh` degrades the report rather than
// breaking the tool.
#[tokio::test]
async fn forge_info_unknown_auth_format_reports_nulls_not_an_error() {
    let server = github_info_server(
        Reply::ok(r#"{"hosts":{"github.com":{"user":"octocat"}}}"#),
        Reply::ok(r#"{"name":"r"}"#),
    );
    let value = tool_json(&server.forge_info().await.expect("forge_info ok"));
    assert_eq!(value["auth"]["authed"], true, "{value}");
    assert_eq!(value["auth"]["active_account"], serde_json::Value::Null);
    assert_eq!(value["auth"]["accounts"], serde_json::json!([]), "{value}");
    assert_eq!(value["auth"]["repo_visible"], true, "{value}");
}

/// An **external** `ForgeApi` implementation — the public extension point
/// (`VcsMcpServer::from_handles` takes `Arc<dyn ForgeApi>`) — that overrides
/// nothing optional. It therefore inherits the trait's defaulted `capabilities`
/// (all-`false`) *and* its defaulted `auth_info` (`Unsupported`), which is the
/// shape `forge_info` has to keep answering for.
struct BareForge;

/// The answer every required method of [`BareForge`] gives: this stub exists to
/// exercise the *defaulted* methods, not to model a forge.
fn not_implemented(operation: &'static str) -> vcs_forge::Error {
    vcs_forge::Error::unsupported(vcs_forge::ForgeKind::Unknown, operation)
}

#[async_trait::async_trait]
impl ForgeApi for BareForge {
    fn kind(&self) -> vcs_forge::ForgeKind {
        vcs_forge::ForgeKind::Unknown
    }
    fn cwd(&self) -> &std::path::Path {
        std::path::Path::new("/repo")
    }
    async fn auth_status(&self) -> vcs_forge::Result<bool> {
        Ok(false)
    }
    async fn repo_view(&self) -> vcs_forge::Result<vcs_forge::ForgeRepo> {
        Err(not_implemented("repo_view"))
    }
    async fn pr_list(&self) -> vcs_forge::Result<Vec<vcs_forge::ForgePr>> {
        Err(not_implemented("pr_list"))
    }
    async fn pr_view(&self, _number: u64) -> vcs_forge::Result<vcs_forge::ForgePr> {
        Err(not_implemented("pr_view"))
    }
    async fn pr_create(&self, _spec: vcs_forge::PrCreate) -> vcs_forge::Result<String> {
        Err(not_implemented("pr_create"))
    }
    async fn pr_merge(&self, _number: u64, _merge: vcs_forge::PrMerge) -> vcs_forge::Result<()> {
        Err(not_implemented("pr_merge"))
    }
    async fn pr_mark_ready(&self, _number: u64) -> vcs_forge::Result<()> {
        Err(not_implemented("pr_mark_ready"))
    }
    async fn pr_close(&self, _spec: vcs_forge::PrClose) -> vcs_forge::Result<()> {
        Err(not_implemented("pr_close"))
    }
    async fn pr_checks(&self, _number: u64) -> vcs_forge::Result<vcs_forge::CiStatus> {
        Err(not_implemented("pr_checks"))
    }
    async fn pr_diff(&self, _number: u64) -> vcs_forge::Result<Vec<vcs_forge::FileDiff>> {
        Err(not_implemented("pr_diff"))
    }
    async fn issue_list(&self) -> vcs_forge::Result<Vec<vcs_forge::ForgeIssue>> {
        Err(not_implemented("issue_list"))
    }
    async fn issue_view(&self, _number: u64) -> vcs_forge::Result<vcs_forge::ForgeIssue> {
        Err(not_implemented("issue_view"))
    }
    async fn issue_create(&self, _spec: vcs_forge::IssueCreate) -> vcs_forge::Result<String> {
        Err(not_implemented("issue_create"))
    }
    async fn release_list(&self) -> vcs_forge::Result<Vec<vcs_forge::ForgeRelease>> {
        Err(not_implemented("release_list"))
    }
    async fn release_view(&self, _tag: &str) -> vcs_forge::Result<vcs_forge::ForgeRelease> {
        Err(not_implemented("release_view"))
    }
}

// A read-only introspection tool must answer, not refuse. An external `ForgeApi`
// inherits `auth_info`'s defaulted `Unsupported` — the very case the default
// exists for — and that must read as "unknown" (the shape the tool documents for
// a backend without an identity probe), exactly as the same trait's defaulted
// `capabilities` degrades to all-`false` instead of erroring.
#[tokio::test]
async fn forge_info_reports_unknown_auth_for_a_backend_without_the_probe() {
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(BareForge);
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let value = tool_json(
        &server
            .forge_info()
            .await
            .expect("a defaulted `auth_info` must not fail the whole tool"),
    );
    // The pre-existing half still answers, from the trait's own defaults.
    assert_eq!(value["kind"], "unknown", "{value}");
    assert_eq!(value["capabilities"]["authed"], false, "{value}");
    // …and the new block is honestly unknown rather than a negative answer.
    assert_eq!(value["auth"]["authed"], serde_json::Value::Null, "{value}");
    assert_eq!(
        value["auth"]["active_account"],
        serde_json::Value::Null,
        "{value}"
    );
    assert_eq!(value["auth"]["accounts"], serde_json::json!([]), "{value}");
    assert_eq!(
        value["auth"]["repo_visible"],
        serde_json::Value::Null,
        "{value}"
    );
}

// --- the "repository unavailable to this account" diagnostic ------------

/// `gh`'s GraphQL refusal for a repository the active account cannot resolve —
/// the failure the diagnostic below explains.
const REPO_REFUSAL: &str =
    "GraphQL: Could not resolve to a Repository with the name 'acme/private-app'. (repository)";

/// A token-shaped fixture (not a real credential) planted in the failing
/// command's captured **stdout**, to pin that no captured stream reaches the
/// client's message.
const FIXTURE_SECRET: &str = "ghp_fixtureNotARealTokenJustAStandIn0123456";

/// Two logins for one host, the second active — a machine with a personal and a
/// work account. Carries the `Token:`/`Token scopes:`/`keyring` detail lines gh
/// prints (masked, since `--show-token` is never passed) so a test can prove the
/// hint is built from the *parsed* accounts, not from this report's text.
const TWO_LOGIN_REPORT: &str = "github.com\n  \
     \u{2713} Logged in to github.com account work-acct (keyring)\n  \
     - Active account: false\n  - Git operations protocol: https\n  \
     - Token: gho_************************************\n  \
     - Token scopes: 'gist', 'read:org', 'repo'\n\n  \
     \u{2713} Logged in to github.com account personal (keyring)\n  \
     - Active account: true\n  \
     - Token: gho_************************************\n";

/// A GitHub-backed server over a recorded, scripted `gh`: `pr list` answers with
/// `pr_list`, the identity probe (`auth status`) with `auth`, and the visibility
/// probe (`repo view`) with `repo_view`. The recorder comes back so a test can
/// assert which of them actually ran.
fn github_forge_server(
    pr_list: Reply,
    auth: Reply,
    repo_view: Reply,
) -> (
    VcsMcpServer,
    Arc<processkit::testing::RecordingRunner<ScriptedRunner>>,
) {
    let runner = Arc::new(processkit::testing::RecordingRunner::new(
        ScriptedRunner::new()
            .on(["gh", "pr", "list"], pr_list)
            .on(["gh", "auth", "status"], auth)
            .on(["gh", "repo", "view"], repo_view),
    ));
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let gh = vcs_forge::vcs_github::GitHub::with_runner(runner.clone());
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
    (
        VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None),
        runner,
    )
}

/// The message of the error `forge_pr_list` failed with.
async fn pr_list_error(server: &VcsMcpServer) -> String {
    server
        .forge_pr_list(Parameters(PrListParams {
            state: None,
            limit: None,
        }))
        .await
        .expect_err("the scripted `gh pr list` fails")
        .message
        .to_string()
}

// The failure this diagnostic exists for: two logins on one host, and the
// **active** one cannot see the repository. Raw, the client only learns that
// GitHub wouldn't resolve a name — which is indistinguishable from a typo or a
// deleted repo. With the diagnostic it learns which identity ran the call, that
// the repository is invisible to it, which other login is available, and the
// flag that picks one.
#[tokio::test]
async fn an_invisible_repository_names_the_account_the_others_and_the_flags() {
    let (server, runner) = github_forge_server(
        Reply::fail(1, REPO_REFUSAL),
        Reply::ok(TWO_LOGIN_REPORT),
        // The visibility probe agrees with the classifier: not visible.
        Reply::fail(1, REPO_REFUSAL),
    );
    let message = pr_list_error(&server).await;

    // The CLI's own diagnostic still reaches the client — the hint is appended,
    // never substituted.
    assert!(
        message.contains("Could not resolve to a Repository"),
        "{message}"
    );
    // The whole composed clause, in order: who ran it (with its host, since a
    // machine can hold logins on several), what the probe found, who else is
    // available, and how to pick one. Pinned as one string so a reworded or
    // reordered clause has to be looked at rather than silently drift.
    assert!(
        message.contains(
            "the `gh` account in use is `personal` (github.com) and this repository is not \
             visible to it; other logins here: `work-acct` (github.com); choose the identity \
             explicitly by restarting the server with `--gh-account <login>` or \
             `--gh-token-env <VAR>`"
        ),
        "{message}"
    );

    // Three spawns: the failing call, then the identity probe and its visibility
    // half — `Forge::auth_info` reused, not a third probe invented here.
    let spawned: Vec<Vec<String>> = runner.calls().iter().map(|c| c.args_str()).collect();
    assert_eq!(spawned.len(), 3, "{spawned:?}");
    assert_eq!(spawned[1][..2], ["auth", "status"], "{spawned:?}");
    assert_eq!(spawned[2][..2], ["repo", "view"], "{spawned:?}");
}

// The secret-safety half. The hint is composed from the identity probe's parsed
// fields (logins, hosts) plus fixed text — never from a captured stream — so a
// failing command's stdout cannot ride out inside it, and neither can `gh auth
// status`'s report text. The failing call's stdout here is both large and
// secret-bearing: a message that dumped captures would carry the token, and
// would also blow the size bound.
#[tokio::test]
async fn the_diagnostic_carries_no_captured_output_and_no_secret() {
    let leaky_stdout = format!(
        "{{\"token\":\"{FIXTURE_SECRET}\",\"padding\":\"{}\"}}",
        "x".repeat(4096)
    );
    let (server, _runner) = github_forge_server(
        Reply::fail(1, REPO_REFUSAL).with_stdout(leaky_stdout.clone()),
        Reply::ok(TWO_LOGIN_REPORT),
        Reply::fail(1, REPO_REFUSAL),
    );
    let message = pr_list_error(&server).await;

    // The hint is there (so this is not vacuously passing)…
    assert!(message.contains("--gh-account"), "{message}");
    // …and none of the captured stdout came with it.
    assert!(
        !message.contains(FIXTURE_SECRET),
        "a secret in the failing command's stdout must not reach the client: {message}"
    );
    assert!(!message.contains(&leaky_stdout), "{message}");
    assert!(!message.contains("xxxxxxxx"), "{message}");
    // Nor any text from the identity report — the accounts are read from parsed
    // fields, so gh's own `Token:`/`keyring` lines have no path into the message.
    for report_text in ["Token scopes", "keyring", "gho_", "Git operations protocol"] {
        assert!(
            !message.contains(report_text),
            "{report_text:?} is report text, not a parsed field: {message}"
        );
    }
    // Bounded: the CLI's own one-line diagnostic plus the composed clause, not a
    // dump of either stream (4 KiB of stdout alone would blow this).
    assert!(message.len() < 1_000, "{} bytes: {message}", message.len());
}

// The classifier is deliberately wide (an endpoint can 404 inside a repository
// the account sees perfectly well), so the probe gets the last word: when it
// answers "visible", no account-selection hint is attached — sending that caller
// to switch accounts would be sending them the wrong way.
#[tokio::test]
async fn a_visible_repository_gets_no_account_hint() {
    let (server, _runner) = github_forge_server(
        Reply::fail(1, "gh: Not Found (HTTP 404)"),
        Reply::ok(TWO_LOGIN_REPORT),
        Reply::ok(r#"{"name":"private-app"}"#),
    );
    let message = pr_list_error(&server).await;
    assert!(message.contains("Not Found"), "{message}");
    assert!(
        !message.contains("--gh-account"),
        "the probe contradicts the guess: {message}"
    );
    assert!(!message.contains("`personal`"), "{message}");
}

// A failure outside the class is mapped exactly as before — and costs exactly
// nothing extra: no identity probe is spawned at all. `Could not resolve to a
// PullRequest` is the near miss the classifier is keyed against (gh opens the
// same GraphQL sentence for a missing PR inside a repository the account sees).
#[tokio::test]
async fn an_unrelated_failure_is_mapped_as_before_without_probing() {
    let (server, runner) = github_forge_server(
        Reply::fail(
            1,
            "GraphQL: Could not resolve to a PullRequest with the number of 9999.",
        ),
        Reply::ok(TWO_LOGIN_REPORT),
        Reply::ok(r#"{"name":"private-app"}"#),
    );
    let message = pr_list_error(&server).await;
    assert!(message.contains("PullRequest"), "{message}");
    assert!(!message.contains("--gh-account"), "{message}");
    assert!(!message.contains("`personal`"), "{message}");
    assert_eq!(
        runner.calls().len(),
        1,
        "only the failing call itself: {:?}",
        runner.calls()
    );
}

// gh's exit code 4 is "authentication required": there is no session at all, so
// the hint must say that rather than name an account it doesn't have — and still
// point at the flags (plus the login the operator may actually want).
#[tokio::test]
async fn an_unauthenticated_gh_reports_no_session_and_how_to_choose_one() {
    let (server, runner) = github_forge_server(
        Reply::fail(
            4,
            "To get started with GitHub CLI, please run:  gh auth login",
        ),
        Reply::fail(
            1,
            "You are not logged into any GitHub hosts. To log in, run: gh auth login",
        ),
        Reply::ok(r#"{"name":"private-app"}"#),
    );
    let message = pr_list_error(&server).await;
    assert!(
        message.contains("no logged-in account"),
        "the honest state: {message}"
    );
    assert!(
        !message.contains("not visible to it"),
        "nothing was probed for visibility, so nothing is claimed: {message}"
    );
    assert!(
        message.contains("--gh-account") && message.contains("--gh-token-env"),
        "{message}"
    );
    assert!(message.contains("or log in with"), "{message}");
    // No session → the visibility half of the probe is skipped: two spawns, not
    // three (the gate `Forge::auth_info` already applies).
    assert_eq!(runner.calls().len(), 2, "{:?}", runner.calls());
}

// GitHub only. `glab` answers a hidden project with a plain `404 Not Found`,
// which would match the classifier's marker — but the classifier reads gh's
// semantics and the hint names gh-only flags, so a GitLab failure is mapped
// plainly instead of being pointed at a flag that could not help it.
#[tokio::test]
async fn a_gitlab_failure_is_never_pointed_at_the_gh_flags() {
    let glab = vcs_forge::vcs_gitlab::GitLab::with_runner(
        ScriptedRunner::new().on(["glab", "mr", "list"], Reply::fail(1, "404 Not Found")),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", glab));
    let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

    let message = pr_list_error(&server).await;
    assert!(message.contains("404"), "{message}");
    assert!(!message.contains("--gh-account"), "{message}");
    assert!(!message.contains("--gh-token-env"), "{message}");
}

// A diagnostic must never replace the failure it was trying to explain: when the
// identity probe itself fails, the original error is returned unchanged.
#[tokio::test]
async fn a_failing_probe_leaves_the_original_error_untouched() {
    let (server, _runner) = github_forge_server(
        Reply::fail(1, REPO_REFUSAL),
        Reply::timeout(),
        Reply::ok(r#"{"name":"private-app"}"#),
    );
    let message = pr_list_error(&server).await;
    assert!(
        message.contains("Could not resolve to a Repository"),
        "{message}"
    );
    assert!(!message.contains("--gh-account"), "{message}");
}

// The `forge_info` tool is read-only — its annotation is `readOnlyHint`,
// not `destructiveHint`. Pinned here alongside the existing
// `tool_annotations_mark_read_vs_destructive` test.
#[test]
fn tool_annotations_mark_forge_info_as_read_only() {
    let tool = VcsMcpServer::forge_info_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.read_only_hint, Some(true));
    assert_eq!(a.destructive_hint, None);

    let tool = VcsMcpServer::forge_pr_comment_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.destructive_hint, Some(true));
    assert_eq!(a.read_only_hint, None);

    let tool = VcsMcpServer::forge_pr_edit_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.destructive_hint, Some(true));
    assert_eq!(a.read_only_hint, None);

    // The review-action tools are destructive (they change a PR/MR's review state).
    let tool = VcsMcpServer::forge_pr_approve_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.destructive_hint, Some(true));
    assert_eq!(a.read_only_hint, None);

    let tool = VcsMcpServer::forge_pr_request_changes_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.destructive_hint, Some(true));
    assert_eq!(a.read_only_hint, None);

    // `forge_pr_checkout` mutates the working copy — destructive, not read-only.
    let tool = VcsMcpServer::forge_pr_checkout_tool_attr();
    let a = tool.annotations.expect("annotations present");
    assert_eq!(a.destructive_hint, Some(true));
    assert_eq!(a.read_only_hint, None);

    // The three issue-lifecycle mutations are real forge mutations (close/reopen an
    // issue, post a comment) — destructive, not read-only (K-017: the
    // snapshot-side-effect idempotent pattern is for jj-backed *reads*, not these).
    for tool in [
        VcsMcpServer::forge_issue_close_tool_attr(),
        VcsMcpServer::forge_issue_reopen_tool_attr(),
        VcsMcpServer::forge_issue_comment_tool_attr(),
    ] {
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);
    }
}

// The macro-generated tool definitions carry the right MCP annotations: a
// genuinely read-only tool (`repo_info` — no backend spawn) is read-only, a
// mutation tool is destructive. (`repo_snapshot` used to be the read example
// here, but T-068 reclassified it — it snapshots the jj working copy — so the
// read example is now `repo_info`, the one repo_* read that never spawns.)
#[test]
fn tool_annotations_mark_read_vs_destructive() {
    let read = VcsMcpServer::repo_info_tool_attr();
    assert_eq!(read.annotations.unwrap().read_only_hint, Some(true));
    let write = VcsMcpServer::repo_commit_tool_attr();
    assert_eq!(write.annotations.unwrap().destructive_hint, Some(true));
}

// T-068 (variant C — strict MCP compliance). Every `repo_*` read tool that, on
// the jj backend, dispatches to a plain (working-copy-**snapshotting**) jj
// command records an op-log operation — so it must NOT assert `readOnlyHint`
// ("does not modify its environment"), which would break the MCP contract. The
// honest, backend-agnostic classification is non-destructive + idempotent (the
// op-log snapshot is append-only/recoverable and changes no tracked content,
// refs, or bookmarks; on git these tools are read-only, a strict subset). This
// list is the *verified* set (checked against `vcs-jj`'s command construction and
// `jj_backend.rs`), which is broader than the ticket's initial sketch: `repo_log`,
// `repo_show_file`, and `repo_conflicts` snapshot too (`jj log` / `jj file show` /
// `jj resolve --list` are all default-snapshotting), and are included here for
// consistency. `repo_worktrees` snapshots via its top-level `jj workspace list`
// (its per-workspace `workspace root` fan-out is already `--ignore-working-copy`).
// Pinning all three annotation fields makes an accidental re-classification (or a
// silent `read_only_hint = true` creeping back) fail the build.
#[test]
fn jj_snapshotting_read_tools_are_not_read_only_but_non_destructive() {
    let tools = [
        ("repo_snapshot", VcsMcpServer::repo_snapshot_tool_attr()),
        ("repo_status", VcsMcpServer::repo_status_tool_attr()),
        ("repo_diff_stat", VcsMcpServer::repo_diff_stat_tool_attr()),
        ("repo_diff", VcsMcpServer::repo_diff_tool_attr()),
        ("repo_log", VcsMcpServer::repo_log_tool_attr()),
        ("repo_show_file", VcsMcpServer::repo_show_file_tool_attr()),
        ("repo_branches", VcsMcpServer::repo_branches_tool_attr()),
        ("repo_remotes", VcsMcpServer::repo_remotes_tool_attr()),
        ("repo_annotate", VcsMcpServer::repo_annotate_tool_attr()),
        (
            "repo_current_branch",
            VcsMcpServer::repo_current_branch_tool_attr(),
        ),
        ("repo_conflicts", VcsMcpServer::repo_conflicts_tool_attr()),
        ("repo_worktrees", VcsMcpServer::repo_worktrees_tool_attr()),
    ];
    for (name, tool) in tools {
        let a = tool
            .annotations
            .unwrap_or_else(|| panic!("{name} must carry annotations"));
        assert_eq!(
            a.read_only_hint, None,
            "{name} must NOT assert readOnlyHint — on jj it snapshots the working \
                 copy (records an op-log operation), so the read-only claim is false"
        );
        assert_eq!(
            a.destructive_hint,
            Some(false),
            "{name} is non-destructive (the jj op-log snapshot is append-only and \
                 recoverable; no tracked content/refs/bookmarks change)"
        );
        assert_eq!(
            a.idempotent_hint,
            Some(true),
            "{name} is idempotent (a re-run with no interim filesystem edit records \
                 no further op-log operation)"
        );
    }
}

// T-068: the complement. The genuinely backend-agnostic read-only tools KEEP
// `readOnlyHint = true`. `repo_info` makes no backend spawn at all (cached
// kind/root/cwd + forge kind); every `forge_*` read tool drives the forge CLI, not
// the jj working copy — so neither can snapshot, and the read-only claim holds on
// both backends. This is the consistency half of the fix: only the tools that
// *actually* reach a snapshotting jj command were reclassified, not the whole read
// surface.
#[test]
fn truly_read_only_tools_keep_read_only_hint() {
    let tools = [
        ("repo_info", VcsMcpServer::repo_info_tool_attr()),
        ("repo_op_log", VcsMcpServer::repo_op_log_tool_attr()),
        (
            "forge_auth_status",
            VcsMcpServer::forge_auth_status_tool_attr(),
        ),
        ("forge_repo_view", VcsMcpServer::forge_repo_view_tool_attr()),
        ("forge_pr_list", VcsMcpServer::forge_pr_list_tool_attr()),
        ("forge_pr_view", VcsMcpServer::forge_pr_view_tool_attr()),
        ("forge_pr_checks", VcsMcpServer::forge_pr_checks_tool_attr()),
        ("forge_pr_diff", VcsMcpServer::forge_pr_diff_tool_attr()),
        (
            "forge_issue_list",
            VcsMcpServer::forge_issue_list_tool_attr(),
        ),
        (
            "forge_issue_view",
            VcsMcpServer::forge_issue_view_tool_attr(),
        ),
        (
            "forge_release_list",
            VcsMcpServer::forge_release_list_tool_attr(),
        ),
        (
            "forge_release_view",
            VcsMcpServer::forge_release_view_tool_attr(),
        ),
        ("forge_info", VcsMcpServer::forge_info_tool_attr()),
    ];
    for (name, tool) in tools {
        let a = tool
            .annotations
            .unwrap_or_else(|| panic!("{name} must carry annotations"));
        assert_eq!(
            a.read_only_hint,
            Some(true),
            "{name} is genuinely read-only on both backends and must keep readOnlyHint"
        );
    }
}

// This query calls a forge CLI without touching the jj working copy, so it is
// genuinely read-only and must advertise `readOnlyHint`.
#[test]
fn forge_pr_for_branch_annotation_is_read_only() {
    let tool = VcsMcpServer::forge_pr_for_branch_tool_attr();
    let annotations = tool.annotations.expect("annotations present");
    assert_eq!(annotations.read_only_hint, Some(true));
    assert_eq!(annotations.destructive_hint, None);
    assert_eq!(annotations.idempotent_hint, None);
}

// T-068: reclassifying the jj-snapshotting reads must NOT change their
// availability — they stay ordinary read tools, callable in the default
// read-only mode. An op-log snapshot mutates neither tracked content nor refs, so
// (unlike `repo_try_merge`, which materializes working-tree content that can run
// untrusted filter/textconv drivers) it needs no `--allow-write`; none of these
// names may leak into `WRITE_TOOLS`. Two of them are also exercised end-to-end
// under `WriteGate::None` to prove they run without a gate.
#[tokio::test]
async fn reclassified_reads_stay_ungated_and_callable() {
    for name in [
        "repo_snapshot",
        "repo_status",
        "repo_diff_stat",
        "repo_diff",
        "repo_log",
        "repo_show_file",
        "repo_annotate",
        "repo_branches",
        "repo_remotes",
        "repo_current_branch",
        "repo_conflicts",
        "repo_worktrees",
    ] {
        assert!(
            !WRITE_TOOLS.contains(&name),
            "{name} is a read tool — it must not be write-gated"
        );
    }

    // End-to-end: they run under the default read-only gate (no --allow-write).
    let server = git_server(
        ScriptedRunner::new()
            .on(["git", "status"], Reply::ok(" M a.rs\0"))
            .on(["git", "symbolic-ref"], Reply::ok("main\n")),
        WriteGate::None,
    );
    server.repo_status().await.expect("repo_status ungated");
    server
        .repo_current_branch()
        .await
        .expect("repo_current_branch ungated");
}

// The server identifies itself as `vcs-mcp` on the wire, not rmcp's default
// build-env identity (which would say "rmcp").
#[test]
fn server_info_identifies_as_vcs_mcp() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let info = server.get_info();
    assert_eq!(info.server_info.name, "vcs-mcp");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    let first = &info.instructions.expect("instructions")[..SERVER_INSTRUCTIONS.len().min(512)];
    for required in ["typed outcome_*", "preflight", "write gate", "raw CLI"] {
        assert!(
            first.contains(required),
            "first 512 chars omit {required}: {first}"
        );
    }
}

#[test]
fn advertised_capabilities_follow_backend_forge_and_write_gate() {
    let readonly = git_server(ScriptedRunner::new(), WriteGate::None);
    let names = readonly
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect::<Vec<_>>();
    assert!(names.contains(&"outcome_inspect".to_string()), "{names:?}");
    assert!(names.contains(&"outcome_changes".to_string()), "{names:?}");
    assert!(
        !names.iter().any(|name| name.starts_with("forge_")),
        "{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| WRITE_TOOLS.contains(&name.as_str())),
        "{names:?}"
    );
    assert!(!names.contains(&"repo_op_log".to_string()), "{names:?}");
    assert!(!names.contains(&"repo_undo".to_string()), "{names:?}");

    let allow = WriteGate::Set(std::collections::HashSet::from([
        "outcome_commit".to_string()
    ]));
    let selective = git_server(ScriptedRunner::new(), allow);
    let names = selective
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect::<Vec<_>>();
    assert!(names.contains(&"outcome_commit".to_string()), "{names:?}");
    assert!(!names.contains(&"repo_commit".to_string()), "{names:?}");

    let jj = jj_server(ScriptedRunner::new(), WriteGate::None);
    let names = jj
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect::<Vec<_>>();
    assert!(names.contains(&"repo_op_log".to_string()), "{names:?}");
}

/// A no-op MCP client handler for the in-process round-trip.
#[derive(Clone, Default)]
struct TestClient;
impl rmcp::ClientHandler for TestClient {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default()
    }
}

// End-to-end through rmcp: an in-process client lists the tools and calls a
// read tool over an in-memory transport — proving the #[tool_router]/
// #[tool_handler] wiring routes calls, not just that the methods compile.
#[tokio::test]
async fn in_process_client_lists_and_calls_tools() {
    use rmcp::ServiceExt;
    use rmcp::model::CallToolRequestParams;

    let server = git_server(
        ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")),
        WriteGate::None,
    );
    let (server_t, client_t) = tokio::io::duplex(4096);
    let server_handle = tokio::spawn(async move {
        if let Ok(running) = server.serve(server_t).await {
            let _ = running.waiting().await;
        }
    });

    let client = TestClient.serve(client_t).await.expect("client connects");

    let tools = client.list_all_tools().await.expect("list_tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"repo_snapshot"), "{names:?}");
    assert!(names.contains(&"outcome_inspect"), "{names:?}");
    assert!(names.contains(&"outcome_changes"), "{names:?}");
    assert!(!names.contains(&"repo_commit"), "{names:?}");
    assert!(
        !names.iter().any(|name| name.starts_with("forge_")),
        "{names:?}"
    );

    let result = client
        .call_tool(CallToolRequestParams::new("repo_current_branch"))
        .await
        .expect("call repo_current_branch");
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .expect("text content");
    assert!(text.contains("main"), "{text}");

    let _ = client.cancel().await;
    server_handle.abort();
}

#[tokio::test]
async fn repo_rebase_is_gated_and_rebases() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_rebase(Parameters(RebaseParams {
            onto: "main".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "rebase"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_rebase(Parameters(RebaseParams {
            onto: "main".into(),
        }))
        .await
        .expect("rebase ok");
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["rebased_onto"], "main", "{text}");
}

#[tokio::test]
async fn repo_undo_is_write_gated_and_uses_jj_operation_recovery() {
    assert!(WRITE_TOOLS.contains(&"repo_undo"));

    let readonly = jj_server(ScriptedRunner::new(), WriteGate::None);
    let err = readonly.repo_undo().await.expect_err("repo_undo is gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let writable = jj_server(
        ScriptedRunner::new().on(["jj", "undo"], Reply::ok("")),
        WriteGate::All,
    );
    let out = writable.repo_undo().await.expect("repo_undo ok");
    assert!(result_json(&out).contains("undone"));

    let git = git_server(ScriptedRunner::new(), WriteGate::All);
    let err = git
        .repo_undo()
        .await
        .expect_err("Git has no faithful operation undo");
    assert!(format!("{err:?}").contains("unsupported"), "{err:?}");
}

#[tokio::test]
async fn repo_abort_in_progress_is_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server.repo_abort_in_progress().await.expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "rev-parse"], Reply::ok("/repo/.git\n")),
        WriteGate::All,
    );
    let out = server.repo_abort_in_progress().await.expect("abort ok");
    assert!(result_json(&out).contains("operation_state"));
}

#[tokio::test]
async fn repo_continue_in_progress_is_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server.repo_continue_in_progress().await.expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new()
            .on(["git", "diff"], Reply::ok(""))
            .on(["git", "rev-parse"], Reply::ok("/repo/.git\n")),
        WriteGate::All,
    );
    let out = server
        .repo_continue_in_progress()
        .await
        .expect("continue ok");
    assert!(result_json(&out).contains("operation_state"));
}

#[tokio::test]
async fn repo_new_child_is_gated_and_creates() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_new_child(Parameters(NewChildParams {
            reference: "main".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "checkout"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_new_child(Parameters(NewChildParams {
            reference: "main".into(),
        }))
        .await
        .expect("new child ok");
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["new_child_of"], "main", "{text}");
}

#[tokio::test]
async fn repo_create_branch_is_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_create_branch(Parameters(CreateBranchParams {
            name: "feature".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_create_branch(Parameters(CreateBranchParams {
            name: "feature".into(),
        }))
        .await
        .expect("create branch ok");
    assert!(
        result_json(&out).contains("created_branch"),
        "{}",
        result_json(&out)
    );
}

#[tokio::test]
async fn repo_delete_branch_is_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_delete_branch(Parameters(DeleteBranchParams {
            name: "feature".into(),
            force: false,
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_delete_branch(Parameters(DeleteBranchParams {
            name: "feature".into(),
            force: true,
        }))
        .await
        .expect("delete branch ok");
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["deleted_branch"], "feature", "{text}");
    assert_eq!(value["force"], true, "{text}");
}

#[tokio::test]
async fn repo_rename_branch_is_gated() {
    let server = git_server(ScriptedRunner::new(), WriteGate::None);
    let err = server
        .repo_rename_branch(Parameters(RenameBranchParams {
            old: "old".into(),
            new: "new".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

    let server = git_server(
        ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
        WriteGate::All,
    );
    let out = server
        .repo_rename_branch(Parameters(RenameBranchParams {
            old: "old".into(),
            new: "new".into(),
        }))
        .await
        .expect("rename branch ok");
    let text = out
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .expect("text content");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["renamed"]["old"], "old", "{text}");
    assert_eq!(value["renamed"]["new"], "new", "{text}");
}

#[tokio::test]
async fn forge_label_tool_is_gated_and_forwards_flag_values() {
    for name in [
        "forge_pr_add_labels",
        "forge_pr_remove_labels",
        "forge_issue_add_labels",
        "forge_issue_remove_labels",
    ] {
        assert!(WRITE_TOOLS.contains(&name), "{name} must be write-gated");
    }

    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(["gh", "--version"], Reply::ok("gh version 2.95.0\n"))
            .on(
                [
                    "gh",
                    "pr",
                    "edit",
                    "7",
                    "--add-label",
                    "-urgent",
                    "--add-label",
                    "help wanted",
                ],
                Reply::ok(""),
            ),
    );
    let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
        "/repo",
        "/repo",
        Git::with_runner(ScriptedRunner::new()),
    ));
    let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));

    let readonly = VcsMcpServer::from_handles(repo.clone(), Some(forge.clone()), WriteGate::None);
    let params = || {
        Parameters(LabelsParams {
            number: 7,
            labels: vec!["-urgent".into(), "help wanted".into()],
        })
    };
    readonly
        .forge_pr_add_labels(params())
        .await
        .expect_err("label mutation must be gated");

    let writable = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
    let out = writable
        .forge_pr_add_labels(params())
        .await
        .expect("label mutation forwards");
    let json = result_json(&out);
    assert!(json.contains("-urgent"), "{json}");
    assert!(json.contains("help wanted"), "{json}");
}

#[test]
fn create_labels_are_optional_in_mcp_json() {
    let pr: PrCreateParams =
        serde_json::from_str(r#"{"title":"T","body":"B","source":null,"target":null}"#)
            .expect("labels omitted");
    assert!(pr.labels.is_empty());

    let issue: IssueCreateParams =
        serde_json::from_str(r#"{"title":"T","body":"B"}"#).expect("labels omitted");
    assert!(issue.labels.is_empty());
}

// --- T-152: the conflict tools ------------------------------------------------
//
// These two are the only tools that touch the filesystem directly. That is not a
// shortcut: conflict markers are materialized in the working copy and, on git,
// exist *nowhere else* — `git show HEAD:<path>` returns the clean blob and
// `git show :<path>` fails outright on an unmerged path (verified against git
// 2.55), so routing them through `repo_show_file` would report "no conflicts" for
// a file `repo_conflicts` lists as conflicted. Consequently these tests need a
// real repo root on disk rather than the `"/repo"` placeholder the rest of the
// suite uses.
mod conflict_tools {
    use super::*;
    use processkit::testing::RecordingRunner;
    use std::path::Path;
    use vcs_testkit::TempDir;

    /// A git conflict in the default 2-way `merge` style (records no base).
    const GIT_MERGE: &str =
        "line 1\n<<<<<<< HEAD\nmain line 2\n=======\nfeature line 2\n>>>>>>> feature\nline 3\n";
    /// A git conflict in `diff3` style — the one that does record a base.
    const GIT_DIFF3: &str = "line 1\n<<<<<<< HEAD\nmain line 2\n||||||| 0b025ce\nline 2\n=======\nfeature line 2\n>>>>>>> feature\nline 3\n";
    /// A jj conflict in the default `diff` style, captured verbatim from jj 0.38.
    const JJ_DIFF: &str = "line 1\n<<<<<<< conflict 1 of 1\n%%%%%%% diff from: tzrkoxkm 049614bb \"base\"\n\\\\\\\\\\\\\\        to: rzzpwvsk c52135dc \"side-a\"\n-line 2\n+side-a line 2\n+++++++ ymnnrqkr 4471c4bd \"side-b\"\nside-b line 2\n>>>>>>> conflict 1 of 1 ends\nline 3\n";
    /// A 3-sided jj conflict — the shape on which "theirs" is ambiguous.
    const JJ_THREE_SIDED: &str = "<<<<<<< conflict 1 of 1\n+++++++ a \"side-a\"\nA\n------- b \"base\"\nB\n+++++++ c \"side-b\"\nC\n------- d \"base2\"\nD\n+++++++ e \"side-c\"\nE\n>>>>>>> conflict 1 of 1 ends\n";

    /// The NUL-delimited output of both backends' conflicted-path queries
    /// (`git diff --name-only --diff-filter=U -z`, `jj file list -T …`).
    fn conflicted(path: &str) -> Reply {
        Reply::ok(format!("{path}\0"))
    }

    /// A repo root that really exists on disk, with `f.txt` holding `content`.
    fn worktree(content: &str) -> TempDir {
        let dir = TempDir::new("mcp-conflict");
        std::fs::write(dir.path().join("f.txt"), content).expect("seed the working copy");
        dir
    }

    fn git_server_at(root: &Path, runner: Arc<RecordingRunner>, writes: WriteGate) -> VcsMcpServer {
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(root, root, Git::with_runner(runner)));
        VcsMcpServer::from_handles(repo, None, writes)
    }

    fn jj_server_at(root: &Path, runner: Arc<RecordingRunner>, writes: WriteGate) -> VcsMcpServer {
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_jj(root, root, Jj::with_runner(runner)));
        VcsMcpServer::from_handles(repo, None, writes)
    }

    fn recorder(scripted: ScriptedRunner) -> Arc<RecordingRunner> {
        Arc::new(RecordingRunner::new(scripted))
    }

    /// A git runner that answers the conflicted-path check with `f.txt` and
    /// accepts the finalizing `git add`.
    fn git_conflicted_runner() -> Arc<RecordingRunner> {
        recorder(
            ScriptedRunner::new()
                .on(["git", "diff"], conflicted("f.txt"))
                .on(["git", "--literal-pathspecs", "add"], Reply::ok("")),
        )
    }

    /// A jj runner that answers the conflicted-path check with `f.txt`. There is
    /// deliberately no staging command to script — jj has no index.
    fn jj_conflicted_runner() -> Arc<RecordingRunner> {
        recorder(ScriptedRunner::new().on(["jj", "file", "list"], conflicted("f.txt")))
    }

    fn regions_params(path: &str) -> Parameters<ConflictRegionsParams> {
        Parameters(ConflictRegionsParams { path: path.into() })
    }

    fn resolve_params(
        path: &str,
        side: ConflictSideArg,
        index: Option<usize>,
    ) -> Parameters<ResolveConflictParams> {
        Parameters(ResolveConflictParams {
            path: path.into(),
            side,
            index,
        })
    }

    /// The tool's JSON *payload*, re-parsed. A tool returns its document inside a
    /// text content block, so asserting on the parsed document beats matching the
    /// escaped string that embeds it.
    fn payload(r: &CallToolResult) -> serde_json::Value {
        let wire: serde_json::Value =
            serde_json::from_str(&result_json(r)).expect("result serialises");
        let text = wire["content"][0]["text"]
            .as_str()
            .expect("a text content block")
            .to_owned();
        serde_json::from_str(&text).expect("the block holds a JSON document")
    }

    fn read_back(dir: &TempDir) -> String {
        std::fs::read_to_string(dir.path().join("f.txt")).expect("working copy readable")
    }

    /// Whether the recorder saw a `git add` — the step that clears git's unmerged
    /// index entry.
    fn staged(runner: &RecordingRunner) -> bool {
        runner
            .calls()
            .iter()
            .any(|call| call.args_str().iter().any(|arg| arg == "add"))
    }

    // The read tool surfaces every side and label the git parser carries, plus the
    // `N of M` counter git's own grammar lacks (synthesized positionally).
    #[tokio::test]
    async fn conflict_regions_expose_the_git_model() {
        let dir = worktree(GIT_DIFF3);
        let server = git_server_at(dir.path(), recorder(ScriptedRunner::new()), WriteGate::None);
        let json = payload(
            &server
                .repo_conflict_regions(regions_params("f.txt"))
                .await
                .expect("read ok"),
        );
        assert_eq!(json["backend"], "git");
        assert_eq!(json["path"], "f.txt");
        assert_eq!(json["conflict_count"], 1);
        let entry = &json["regions"][0];
        assert_eq!(entry["number"], 1);
        assert_eq!(entry["total"], 1);
        let region = &entry["region"];
        assert_eq!(region["ours_label"], "HEAD");
        assert_eq!(region["base_label"], "0b025ce");
        assert_eq!(region["theirs_label"], "feature");
        assert_eq!(region["ours"], serde_json::json!(["main line 2\n"]));
        assert_eq!(region["base"], serde_json::json!(["line 2\n"]));
        assert_eq!(region["theirs"], serde_json::json!(["feature line 2\n"]));
        assert_eq!(region["marker_len"], 7);
        assert!(
            region.get("marker_ours").is_none(),
            "the private verbatim marker lines stay off the wire: {region}"
        );
    }

    // The jj model is a genuinely different shape (n-way diff/snapshot sections
    // with their own `conflict N of M` counters) and is published as such, not
    // squeezed into git's ours/base/theirs.
    #[tokio::test]
    async fn conflict_regions_expose_the_jj_model() {
        let dir = worktree(JJ_DIFF);
        let server = jj_server_at(dir.path(), recorder(ScriptedRunner::new()), WriteGate::None);
        let json = payload(
            &server
                .repo_conflict_regions(regions_params("f.txt"))
                .await
                .expect("read ok"),
        );
        assert_eq!(json["backend"], "jj");
        assert_eq!(json["conflict_count"], 1);
        let entry = &json["regions"][0];
        assert_eq!(entry["number"], 1);
        let region = &entry["region"];
        // jj's own counters survive alongside the envelope's positional ones.
        assert_eq!(region["number"], 1);
        assert_eq!(region["total"], 1);
        let sections = &region["sections"];
        assert_eq!(sections[0]["kind"], "Diff");
        assert_eq!(sections[0]["from_label"], "tzrkoxkm 049614bb \"base\"");
        assert_eq!(sections[0]["to_label"], "rzzpwvsk c52135dc \"side-a\"");
        assert_eq!(
            sections[0]["lines"],
            serde_json::json!(["-line 2\n", "+side-a line 2\n"])
        );
        assert_eq!(sections[1]["kind"], "Snapshot");
        assert_eq!(sections[1]["label"], "ymnnrqkr 4471c4bd \"side-b\"");
    }

    // A file with no markers is an empty region list, NOT an error — symmetric
    // with `repo_conflicts`, which reports `[]` on a clean tree.
    #[tokio::test]
    async fn conflict_regions_on_a_clean_file_is_empty_not_an_error() {
        let dir = worktree("just\nplain\ntext\n");
        for server in [
            git_server_at(dir.path(), recorder(ScriptedRunner::new()), WriteGate::None),
            jj_server_at(dir.path(), recorder(ScriptedRunner::new()), WriteGate::None),
        ] {
            let json = payload(
                &server
                    .repo_conflict_regions(regions_params("f.txt"))
                    .await
                    .expect("a clean file is not an error"),
            );
            assert_eq!(json["conflict_count"], 0);
            assert_eq!(json["regions"], serde_json::json!([]));
        }
    }

    // The containment guard. These tools address the filesystem directly, so a
    // path that could escape the repository is refused before any I/O — every
    // other repo_* tool gets that confinement free from the backend subprocess.
    #[tokio::test]
    async fn conflict_paths_may_not_escape_the_repository() {
        let dir = worktree(GIT_MERGE);
        let runner = git_conflicted_runner();
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);
        for bad in ["../outside.txt", "a/../../outside.txt", "/etc/passwd", ""] {
            let err = server
                .repo_conflict_regions(regions_params(bad))
                .await
                .expect_err("escaping path refused");
            assert_eq!(
                err.code,
                rmcp::model::ErrorCode::INVALID_PARAMS,
                "{bad:?} must be an invalid-params refusal, got {err:?}"
            );
            server
                .repo_resolve_conflict(resolve_params(bad, ConflictSideArg::Ours, None))
                .await
                .expect_err("escaping path refused for the write too");
        }
        assert_eq!(read_back(&dir), GIT_MERGE, "nothing was written");
        assert!(!staged(&runner), "nothing was staged");
    }

    // T-170: lexical containment is not enough when a conflicted path is a
    // symlink. The read must refuse before exposing the outside file, and the
    // write must refuse before truncating or staging anything.
    #[cfg(unix)]
    #[tokio::test]
    async fn conflict_tools_refuse_a_symlink_to_an_outside_file() {
        let dir = worktree("in-repository placeholder\n");
        let outside = TempDir::new("mcp-conflict-outside");
        let outside_file = outside.path().join("outside.txt");
        std::fs::write(&outside_file, GIT_MERGE).expect("seed outside target");
        std::fs::remove_file(dir.path().join("f.txt")).expect("remove regular path");
        std::os::unix::fs::symlink(&outside_file, dir.path().join("f.txt"))
            .expect("create in-repository symlink");

        let runner = git_conflicted_runner();
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);
        let err = server
            .repo_conflict_regions(regions_params("f.txt"))
            .await
            .expect_err("the read must not follow an outside symlink");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("outside") || err.message.contains("symbolic link"),
            "the refusal should identify the containment problem: {}",
            err.message
        );
        assert_eq!(
            std::fs::read_to_string(&outside_file).expect("outside target readable"),
            GIT_MERGE,
            "the read did not return or alter outside content"
        );

        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, None))
            .await
            .expect_err("the write must not follow an outside symlink");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert_eq!(
            std::fs::read_to_string(&outside_file).expect("outside target readable"),
            GIT_MERGE,
            "resolution did not modify the outside target"
        );
        assert_eq!(
            std::fs::read_link(dir.path().join("f.txt")).expect("link remains in place"),
            outside_file,
            "resolution did not replace the repository symlink"
        );
        assert!(!staged(&runner), "a refused path must not be staged");
    }

    // The write gate rejects before anything is read, spawned, or written.
    #[tokio::test]
    async fn resolve_conflict_is_write_gated() {
        assert!(WRITE_TOOLS.contains(&"repo_resolve_conflict"));
        let dir = worktree(GIT_MERGE);
        let runner = recorder(ScriptedRunner::new().fallback(Reply::ok("")));
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::None);
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, None))
            .await
            .expect_err("gated off by default");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            runner.calls().is_empty(),
            "the gate rejects before spawning"
        );
        assert_eq!(read_back(&dir), GIT_MERGE, "the working copy is untouched");

        // A per-tool allowlist naming it is enough — no blanket --allow-write.
        git_server_at(
            dir.path(),
            git_conflicted_runner(),
            WriteGate::Set(["repo_resolve_conflict".to_string()].into_iter().collect()),
        )
        .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, None))
        .await
        .expect("allowlisted");
        assert_eq!(read_back(&dir), "line 1\nmain line 2\nline 3\n");
    }

    // Happy path on git: the chosen side replaces every region and the rest of the
    // file survives verbatim; the path is then STAGED — without that `git add` the
    // index keeps its unmerged stages and `repo_conflicts` still reports the file.
    #[tokio::test]
    async fn resolve_conflict_writes_the_side_and_stages_it_on_git() {
        let dir = worktree(GIT_MERGE);
        let runner = git_conflicted_runner();
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);
        let json = payload(
            &server
                .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Theirs, None))
                .await
                .expect("resolve ok"),
        );
        assert_eq!(read_back(&dir), "line 1\nfeature line 2\nline 3\n");
        assert_eq!(json["side"], "theirs");
        assert_eq!(json["resolved"], "f.txt");
        assert_eq!(json["conflicts_resolved"], 1);
        assert!(
            staged(&runner),
            "the resolved path must be staged (git add)"
        );
    }

    // Happy path on jj: the same tool over the n-way model, and NO staging step —
    // jj has no index, so `mark_resolved` spawns nothing there (verified against
    // jj 0.38: overwriting the working-copy file *is* the resolution).
    #[tokio::test]
    async fn resolve_conflict_writes_the_side_without_staging_on_jj() {
        let dir = worktree(JJ_DIFF);
        let runner = jj_conflicted_runner();
        let server = jj_server_at(dir.path(), runner.clone(), WriteGate::All);
        server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Side, Some(1)))
            .await
            .expect("resolve ok");
        assert_eq!(
            read_back(&dir),
            "line 1\nside-b line 2\nline 3\n",
            "Side(1) is the second side in file order"
        );
        assert_eq!(
            runner.calls().len(),
            1,
            "only the conflicted-path check spawns on jj; there is no staging step"
        );
    }

    // `base` reaches through both models: git's diff3 `|||||||` section and jj's
    // recorded base (here the old side of the `%%%%%%%` diff section).
    #[tokio::test]
    async fn resolve_conflict_can_keep_the_base() {
        let git_dir = worktree(GIT_DIFF3);
        git_server_at(git_dir.path(), git_conflicted_runner(), WriteGate::All)
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Base, None))
            .await
            .expect("git base");
        assert_eq!(read_back(&git_dir), "line 1\nline 2\nline 3\n");

        let jj_dir = worktree(JJ_DIFF);
        jj_server_at(jj_dir.path(), jj_conflicted_runner(), WriteGate::All)
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Base, None))
            .await
            .expect("jj base");
        assert_eq!(read_back(&jj_dir), "line 1\nline 2\nline 3\n");
    }

    // Every refusal below lands BEFORE the file is written — the property that
    // keeps a wrong `side` from destroying the other side's content anyway.
    #[tokio::test]
    async fn resolve_conflict_refuses_impossible_sides_before_writing() {
        let dir = worktree(GIT_MERGE);
        let runner = git_conflicted_runner();
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);

        // A 2-way `merge`-style git conflict records no base.
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Base, None))
            .await
            .expect_err("no base recorded");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        // `side`+`index` is jj's n-way spelling; git's three sides are named.
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Side, Some(0)))
            .await
            .expect_err("side=\"side\" is jj-only");
        assert!(err.message.contains("jj-only"), "{}", err.message);

        // An `index` alongside a named side is a contradiction, not something to
        // quietly ignore.
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, Some(2)))
            .await
            .expect_err("index without side=\"side\"");
        assert!(err.message.contains("index"), "{}", err.message);

        assert_eq!(read_back(&dir), GIT_MERGE, "nothing written");
        assert!(!staged(&runner), "nothing staged");
    }

    // "Theirs" means "the other one", which only exists for a 2-sided conflict.
    // On a 3-sided jj conflict `Side(1)` would be the MIDDLE side, so the tool
    // refuses rather than silently picking it.
    #[tokio::test]
    async fn resolve_conflict_refuses_ambiguous_theirs_on_an_n_way_jj_conflict() {
        let dir = worktree(JJ_THREE_SIDED);
        let server = jj_server_at(dir.path(), jj_conflicted_runner(), WriteGate::All);
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Theirs, None))
            .await
            .expect_err("ambiguous across 3 sides");
        assert!(err.message.contains("ambiguous"), "{}", err.message);
        assert_eq!(read_back(&dir), JJ_THREE_SIDED, "nothing written");

        // An explicit index resolves the ambiguity the tool complained about.
        server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Side, Some(2)))
            .await
            .expect("explicit index accepted");
        assert_eq!(read_back(&dir), "E\n");
    }

    // The second containment guard: only a path the backend REPORTS as conflicted
    // may be rewritten. Without it any file merely *containing* marker-like text —
    // this workspace's own conflict fixtures, a quoted diff in a doc — would be
    // "resolved" and silently lose content.
    #[tokio::test]
    async fn resolve_conflict_refuses_a_path_that_is_not_conflicted() {
        let dir = worktree(GIT_MERGE);
        let runner = recorder(
            ScriptedRunner::new()
                // A clean tree: no conflicted paths at all.
                .on(["git", "diff"], Reply::ok(""))
                .on(["git", "--literal-pathspecs", "add"], Reply::ok("")),
        );
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);
        let err = server
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, None))
            .await
            .expect_err("not conflicted → refused");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("not currently conflicted"),
            "{}",
            err.message
        );
        assert_eq!(
            read_back(&dir),
            GIT_MERGE,
            "a file that merely contains marker-like text is never rewritten"
        );
        assert!(
            !staged(&runner),
            "nothing staged when the write was refused"
        );
    }

    // R-01: the two conflict tools read a file *directly*, so no client
    // `OutputBudget` can reach them (a client budget bounds a subprocess pipe, and
    // these spawn nothing for their read) — they carry the server's own ceiling
    // instead, and it must behave exactly like the one `repo_show_file` inherits:
    // refuse naming the ceiling, never truncate. Pinned at the exact boundary
    // ([[K-073]]): a file *on* the cap is still read, one byte past it is not.
    #[tokio::test]
    async fn conflict_tools_refuse_a_file_over_the_content_ceiling() {
        let dir = worktree(GIT_MERGE);
        let size = GIT_MERGE.len();

        // Exactly on the ceiling: read in full (the budget fires strictly past it,
        // matching `OutputBudget::bytes`' documented boundary).
        let json = payload(
            &git_server_at(dir.path(), git_conflicted_runner(), WriteGate::All)
                .with_output_budget(vcs_core::OutputBudget::bytes(size))
                .repo_conflict_regions(regions_params("f.txt"))
                .await
                .expect("a file sitting exactly on the ceiling is still read"),
        );
        assert_eq!(json["conflict_count"], 1);

        // One byte past it: refused. The ungated read tool is the one that must
        // not be able to buffer an arbitrary working-copy file into the server.
        let runner = git_conflicted_runner();
        let tight = git_server_at(dir.path(), runner.clone(), WriteGate::All)
            .with_output_budget(vcs_core::OutputBudget::bytes(size - 1));
        let err = tight
            .repo_conflict_regions(regions_params("f.txt"))
            .await
            .expect_err("over the ceiling → refused, not truncated");
        assert_eq!(
            err.code,
            rmcp::model::ErrorCode::INTERNAL_ERROR,
            "the same mapping repo_show_file's OutputTooLarge gets: {err:?}"
        );
        assert!(
            err.message.contains("ceiling") && err.message.contains("--max-output-bytes"),
            "the refusal must name the operator's knob: {}",
            err.message
        );

        // The mutating tool reads through the same bounded path, so the ceiling
        // stops it BEFORE it writes or stages anything.
        let err = tight
            .repo_resolve_conflict(resolve_params("f.txt", ConflictSideArg::Ours, None))
            .await
            .expect_err("over the ceiling → refused");
        assert!(err.message.contains("ceiling"), "{}", err.message);
        assert_eq!(read_back(&dir), GIT_MERGE, "nothing was written");
        assert!(!staged(&runner), "nothing was staged");
    }

    // `--max-output-bytes 0` (and the library default) mean *unlimited*, exactly as
    // for the subprocess-backed content tools — a big conflicted file still reads.
    #[tokio::test]
    async fn conflict_tools_are_unbounded_when_the_ceiling_is_disabled() {
        let big = format!("{}{GIT_MERGE}", "filler line\n".repeat(20_000));
        let dir = worktree(&big);
        for server in [
            git_server_at(dir.path(), git_conflicted_runner(), WriteGate::All)
                .with_output_budget(vcs_core::OutputBudget::unlimited()),
            // No `with_output_budget` at all — the default is unlimited too, so a
            // library embedder's server behaves like `--max-output-bytes 0`.
            git_server_at(dir.path(), git_conflicted_runner(), WriteGate::All),
        ] {
            let json = payload(
                &server
                    .repo_conflict_regions(regions_params("f.txt"))
                    .await
                    .expect("no ceiling → the whole file is read"),
            );
            assert_eq!(json["conflict_count"], 1);
        }
    }

    // R-01, the second half: on Windows a handful of legacy names resolve to
    // DEVICES in every directory (`<repo>\CON` is the console), and reading one can
    // block forever — the direct filesystem I/O these two tools do is the only
    // place in the server that isn't behind a subprocess `--timeout`, so the
    // component guard has to reject them outright.
    #[cfg(windows)]
    #[tokio::test]
    async fn conflict_tools_refuse_a_windows_device_name() {
        let dir = worktree(GIT_MERGE);
        let runner = git_conflicted_runner();
        let server = git_server_at(dir.path(), runner.clone(), WriteGate::All);
        for bad in ["CON", "nul", "COM1.txt", "sub/LPT1"] {
            let err = server
                .repo_conflict_regions(regions_params(bad))
                .await
                .expect_err("a device name is not a repository file");
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
            assert!(err.message.contains("device"), "{}", err.message);
            server
                .repo_resolve_conflict(resolve_params(bad, ConflictSideArg::Ours, None))
                .await
                .expect_err("refused for the write too");
        }
        assert_eq!(read_back(&dir), GIT_MERGE, "nothing was written");
        assert!(!staged(&runner), "nothing was staged");
    }

    // The device-name predicate itself, checked on every platform (only Windows
    // *applies* it): Win32 matches the stem before the first `.`, ignores case and
    // trailing spaces/dots, and reserves only `COM1`–`COM9` / `LPT1`–`LPT9`.
    #[test]
    fn reserved_device_names_are_recognised_by_win32_rules() {
        for name in [
            "CON", "con", "NUL", "nul.txt", "AUX", "PRN", "CONIN$", "COM1", "lpt9", "com1.log",
            "CON. ", "CON ",
        ] {
            assert!(
                crate::conflicts::is_reserved_device_name(name),
                "{name} is a Windows device"
            );
        }
        for name in [
            "console",
            "context.rs",
            "conflict.rs",
            "COM0",
            "COM10",
            "LPT",
            "nulls",
            "a.con",
            "auxiliary",
        ] {
            assert!(
                !crate::conflicts::is_reserved_device_name(name),
                "{name} is an ordinary filename"
            );
        }
    }

    // The read tool spawns NO backend command (it reads the working copy), so —
    // unlike the [K-017] family of jj-*snapshotting* reads — `readOnlyHint` is the
    // honest annotation here, exactly as for `repo_info`. The write tool is
    // destructive and gated.
    #[test]
    fn conflict_tool_annotations_match_what_the_tools_actually_do() {
        let read = VcsMcpServer::repo_conflict_regions_tool_attr();
        let a = read.annotations.expect("annotations present");
        assert_eq!(
            a.read_only_hint,
            Some(true),
            "the read tool spawns no git/jj command at all — it cannot snapshot a jj \
             working copy, so readOnlyHint holds on both backends"
        );
        assert!(!WRITE_TOOLS.contains(&"repo_conflict_regions"));

        let write = VcsMcpServer::repo_resolve_conflict_tool_attr();
        let a = write.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);
        assert!(WRITE_TOOLS.contains(&"repo_resolve_conflict"));
    }
}
