use std::process::{Command, Output};

use serde_json::Value;
use vcs_testkit::{GitSandbox, JjSandbox};

fn agent(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vcs-agent"))
        .args(args)
        .output()
        .expect("run vcs-agent")
}

fn json(output: &Output) -> Value {
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "machine stdout must be JSON ({error}): {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let schema: Value = serde_json::from_str(include_str!("../schema/envelope.v1.schema.json"))
        .expect("valid committed schema");
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("compile committed schema");
    if let Err(error) = validator.validate(&value) {
        panic!("actual backend envelope must satisfy the committed schema: {error}");
    }
    value
}

#[test]
#[ignore = "requires the git binary"]
fn git_inspect_and_changes_leave_ref_index_and_content_unchanged() {
    let sandbox = GitSandbox::init("agent-readonly-git");
    sandbox.commit_file("tracked.txt", "before\n", "seed");
    sandbox.write("tracked.txt", "after\n");
    let repo = sandbox.path().to_str().expect("UTF-8 fixture path");

    let head_before = sandbox.rev_parse("HEAD");
    let index_before = std::fs::read(sandbox.path().join(".git/index")).expect("read index");
    let content_before = std::fs::read(sandbox.path().join("tracked.txt")).expect("read file");

    for args in [
        vec!["inspect", "--repo", repo],
        vec!["changes", "--repo", repo, "--mode", "summary"],
        vec![
            "changes",
            "--repo",
            repo,
            "--mode",
            "full",
            "--content-max-bytes",
            "8192",
        ],
    ] {
        let output = agent(&args);
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = json(&output);
        assert_eq!(result["status"], "success");
        assert_eq!(result["data"]["repository"]["backend"], "git");
        assert_eq!(result["data"]["read_semantics"]["refs_mutated"], false);
        assert_eq!(result["data"]["read_semantics"]["index_mutated"], false);
    }

    assert_eq!(sandbox.rev_parse("HEAD"), head_before);
    assert_eq!(
        std::fs::read(sandbox.path().join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(
        std::fs::read(sandbox.path().join("tracked.txt")).unwrap(),
        content_before
    );

    sandbox.git(&["checkout", "--detach", "-q"]);
    let detached = agent(&["inspect", "--repo", repo]);
    assert!(detached.status.success());
    assert!(json(&detached)["data"]["working_copy"]["branch"].is_null());
}

#[test]
#[ignore = "requires the git binary"]
fn full_changes_refuses_an_over_budget_diff_instead_of_truncating() {
    let sandbox = GitSandbox::init("agent-output-limit");
    sandbox.commit_file("large.txt", "seed\n", "seed");
    sandbox.write("large.txt", &"changed line\n".repeat(600));
    let repo = sandbox.path().to_str().expect("UTF-8 fixture path");
    let output = agent(&[
        "changes",
        "--repo",
        repo,
        "--mode",
        "full",
        "--content-max-bytes",
        "1024",
    ]);
    assert_eq!(output.status.code(), Some(42));
    let result = json(&output);
    assert_eq!(result["status"], "error");
    assert_eq!(result["data"], Value::Null);
    assert_eq!(result["error"]["kind"], "output_limit");
}

#[test]
#[ignore = "requires the jj and git binaries"]
fn jj_changes_discloses_and_performs_the_live_working_copy_snapshot() {
    let sandbox = JjSandbox::init("agent-live-jj-snapshot");
    let path = sandbox.path().join("new.txt");
    std::fs::write(&path, "unsnapshotted\n").expect("write pending jj edit");
    let content_before = std::fs::read(&path).expect("read pending edit");
    let op_before = sandbox.op_head();
    let commit_before = sandbox.at_commit();
    let repo = sandbox.path().to_str().expect("UTF-8 fixture path");

    let output = agent(&[
        "changes",
        "--repo",
        repo,
        "--mode",
        "full",
        "--content-max-bytes",
        "8192",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    assert_eq!(result["data"]["repository"]["backend"], "jujutsu");
    assert_eq!(
        result["data"]["read_semantics"]["working_copy_snapshot"],
        "live-jj-snapshot"
    );
    assert_eq!(
        result["data"]["read_semantics"]["operation_log_may_advance"],
        true
    );
    assert_ne!(
        sandbox.op_head(),
        op_before,
        "the live read must not pretend --ignore-working-copy semantics"
    );
    assert_ne!(
        sandbox.at_commit(),
        commit_before,
        "snapshotting the pending edit rewrites @ honestly"
    );
    assert_eq!(
        std::fs::read(path).unwrap(),
        content_before,
        "snapshotting must not alter content"
    );

    let inspect = agent(&["inspect", "--repo", repo]);
    assert!(
        inspect.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect = json(&inspect);
    assert!(
        inspect["data"]["working_copy"]["branch"].is_null(),
        "fresh jj repo has no bookmark"
    );
    assert!(
        inspect["data"]["working_copy"]["change_id"]
            .as_str()
            .is_some()
    );
}
