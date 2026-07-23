use super::*;
use processkit::testing::{Reply, ScriptedRunner};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use vcs_core::vcs_git::Git;

/// A git-backed server over a scripted runner — no real binary, no forge.
fn git_server(runner: ScriptedRunner, writes: WriteGate) -> VcsMcpServer {
    let repo: Arc<dyn VcsRepo> =
        Arc::new(Repo::from_git("/repo", "/repo", Git::with_runner(runner)));
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
            Reply::ok("deadbeef\u{1f}dead\u{1f}Jane\u{1f}2026-05-31T10:00:00+00:00\u{1f}Fix bug\0"),
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
    let err = server.forge_pr_list().await.expect_err("no forge");
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

    let out = server.forge_issue_list().await.expect("issue list ok");
    assert!(result_json(&out).contains("Bug"));

    let err = server
        .forge_issue_create(Parameters(IssueCreateParams {
            title: "t".into(),
            body: "b".into(),
        }))
        .await
        .expect_err("gated");
    assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
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

// T-058: `forge_pr_checkout` and `forge_pr_merge` locally mutate the working
// copy (checkout/switch), so — unlike the other forge tools — they must go
// through `begin_repo_write` and actually hold the same per-repo `write_lock`
// as `repo_*` mutations, not just call the gate-only `require_write`. Prove it
// by holding the lock ourselves first: the tool call must then block (time out)
// rather than run past the lock acquisition, and must succeed once the lock is
// released.
#[tokio::test]
async fn forge_pr_checkout_and_forge_pr_merge_hold_the_repo_write_lock() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(["gh", "pr", "checkout"], Reply::ok(""))
            .on(["gh", "pr", "merge"], Reply::ok("")),
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
// now also carries `version`/`supported`.
#[tokio::test]
async fn forge_info_with_authed_github_reports_all_true() {
    let gh = vcs_forge::vcs_github::GitHub::with_runner(
        ScriptedRunner::new()
            .on(
                ["gh", "--version"],
                Reply::ok("gh version 2.40.1 (2024-01-05)\n"),
            )
            .on(["gh", "auth", "status"], Reply::ok("")),
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
    assert!(names.contains(&"repo_commit"), "{names:?}");
    assert!(names.contains(&"forge_pr_list"), "{names:?}");
    assert!(names.contains(&"forge_pr_for_branch"), "{names:?}");
    assert!(names.contains(&"forge_pr_comment"), "{names:?}");
    assert!(names.contains(&"forge_pr_edit"), "{names:?}");
    assert!(names.contains(&"forge_pr_approve"), "{names:?}");
    assert!(names.contains(&"forge_pr_request_changes"), "{names:?}");
    assert!(names.contains(&"forge_pr_checkout"), "{names:?}");
    assert!(names.contains(&"forge_issue_close"), "{names:?}");
    assert!(names.contains(&"forge_issue_reopen"), "{names:?}");
    assert!(names.contains(&"forge_issue_comment"), "{names:?}");
    assert!(names.contains(&"forge_info"), "{names:?}");

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
