use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use vcs_testkit::{GitSandbox, JjSandbox};

fn agent(args: impl IntoIterator<Item = OsString>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vcs-agent"));
    command
        .args(args)
        // Keep the real Jujutsu scenario independent of ambient identity.
        .env("JJ_USER", "Test")
        .env("JJ_EMAIL", "test@example.com");
    command.output().expect("run vcs-agent")
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn json(output: &Output) -> Value {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
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
        panic!("actual commit envelope must satisfy the committed schema: {error}");
    }
    value
}

fn capture(program: &str, cwd: &Path, args: &[&str]) -> String {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .env("JJ_USER", "Test")
        .env("JJ_EMAIL", "test@example.com")
        .output()
        .unwrap_or_else(|error| panic!("run {program} {args:?}: {error}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_is_exact_and_preserves_mixed_unrelated_state_when_detached() {
    let sandbox = GitSandbox::init("agent-checked-git");
    sandbox.commit_file("selected.txt", "selected before\n", "seed selected");
    sandbox.commit_file("unrelated.txt", "unrelated before\n", "seed unrelated");
    sandbox.git(&["checkout", "--detach", "-q"]);

    sandbox.write("selected.txt", "selected after\n");
    sandbox.write("unrelated.txt", "unrelated staged\n");
    sandbox.git(&["add", "--", "unrelated.txt"]);
    sandbox.write("unrelated.txt", "unrelated unstaged\n");
    sandbox.write("untracked.txt", "untracked\n");

    let before = sandbox.rev_parse("HEAD");
    let unrelated_head_before = sandbox.rev_parse("HEAD:unrelated.txt");
    let unrelated_index_before = sandbox.rev_parse(":unrelated.txt");
    let invocation = args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit selected",
        "--path",
        "selected.txt",
        "--include-machine-paths",
    ]);
    let output = agent(invocation.clone());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    let after = sandbox.rev_parse("HEAD");
    assert_ne!(after, before);
    assert_eq!(result["data"]["before"]["revision"], before);
    assert_eq!(result["data"]["after"]["revision"], after);
    assert_eq!(result["data"]["included_paths"][0]["value"], "selected.txt");
    assert_eq!(result["data"]["unrelated_changes_preserved"], true);
    assert_eq!(
        capture("git", sandbox.path(), &["show", "HEAD:selected.txt"]),
        "selected after"
    );
    assert_eq!(
        sandbox.rev_parse("HEAD:unrelated.txt"),
        unrelated_head_before
    );
    assert_eq!(sandbox.rev_parse(":unrelated.txt"), unrelated_index_before);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("unrelated.txt")).unwrap(),
        "unrelated unstaged\n"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("untracked.txt")).unwrap(),
        "untracked\n"
    );
    let status = capture("git", sandbox.path(), &["status", "--short"]);
    assert!(status.lines().any(|line| line.ends_with("unrelated.txt")));
    assert!(status.lines().any(|line| line == "?? untracked.txt"));

    let retry = agent(invocation);
    assert_eq!(retry.status.code(), Some(20));
    let retry = json(&retry);
    assert_eq!(retry["error"]["code"], "stale_expected_revision");
    assert_eq!(sandbox.rev_parse("HEAD"), after);
}

#[test]
#[ignore = "requires the git binary"]
fn commit_output_budget_is_checked_before_mutation() {
    let sandbox = GitSandbox::init("agent-checked-budget");
    sandbox.commit_file("selected.txt", "before\n", "seed");
    sandbox.write("selected.txt", "after\n");
    let before = sandbox.rev_parse("HEAD");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit selected",
        "--path",
        "selected.txt",
        "--include-machine-paths",
        "--max-output-bytes",
        "1024",
    ]));
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(json(&output)["error"]["kind"], "output_limit");
    assert_eq!(sandbox.rev_parse("HEAD"), before);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("selected.txt")).unwrap(),
        "after\n"
    );
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_rejects_directory_selection_and_commits_one_leaf_only() {
    let sandbox = GitSandbox::init("agent-checked-git-leaves");
    sandbox.commit_file("tracked/a.txt", "before\n", "seed tracked leaf");
    sandbox.write("nested/a.txt", "a\n");
    sandbox.write("nested/b.txt", "b\n");
    sandbox.write("tracked/a.txt", "after\n");
    let before = sandbox.rev_parse("HEAD");

    let directory = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "must not expand directory",
        "--path",
        "nested",
    ]));
    assert_eq!(directory.status.code(), Some(20));
    assert_eq!(
        json(&directory)["error"]["code"],
        "selected_path_not_changed"
    );
    assert_eq!(sandbox.rev_parse("HEAD"), before);

    let leaf = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit one leaf",
        "--path",
        "tracked/a.txt",
        "--include-machine-paths",
    ]));
    assert!(
        leaf.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&leaf.stderr)
    );
    let result = json(&leaf);
    assert_eq!(
        result["data"]["included_paths"][0]["value"],
        "tracked/a.txt"
    );
    assert_eq!(
        capture(
            "git",
            sandbox.path(),
            &["show", "--format=", "--name-only", "HEAD"]
        ),
        "tracked/a.txt"
    );
    assert!(sandbox.path().join("nested/a.txt").exists());
    assert!(sandbox.path().join("nested/b.txt").exists());
    assert!(
        capture("git", sandbox.path(), &["status", "--short"])
            .lines()
            .any(|line| line == "?? nested/")
    );
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_proves_a_deleted_leaf() {
    let sandbox = GitSandbox::init("agent-checked-git-delete");
    sandbox.commit_file("deleted.txt", "delete me\n", "seed deletion");
    std::fs::remove_file(sandbox.path().join("deleted.txt")).expect("delete tracked leaf");
    let before = sandbox.rev_parse("HEAD");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit deletion",
        "--path",
        "deleted.txt",
        "--include-machine-paths",
    ]));
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json(&output)["data"]["included_paths"][0]["value"],
        "deleted.txt"
    );
    assert_eq!(
        capture(
            "git",
            sandbox.path(),
            &["show", "--format=", "--name-only", "HEAD"]
        ),
        "deleted.txt"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_proves_a_symlink_leaf() {
    use std::os::unix::fs::symlink;

    let sandbox = GitSandbox::init("agent-checked-git-symlink");
    sandbox.commit_file("target-a.txt", "a\n", "seed target a");
    sandbox.commit_file("target-b.txt", "b\n", "seed target b");
    symlink("target-a.txt", sandbox.path().join("link.txt")).expect("create symlink");
    sandbox.add_all();
    sandbox.commit("seed symlink");
    std::fs::remove_file(sandbox.path().join("link.txt")).expect("remove old symlink");
    symlink("target-b.txt", sandbox.path().join("link.txt")).expect("replace symlink");
    let before = sandbox.rev_parse("HEAD");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit symlink leaf",
        "--path",
        "link.txt",
        "--include-machine-paths",
    ]));
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json(&output)["data"]["included_paths"][0]["value"],
        "link.txt"
    );
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_does_not_run_pre_commit_hook_that_mutates_unrelated_worktree() {
    let sandbox = GitSandbox::init("agent-checked-git-no-pre-hook");
    sandbox.commit_file("selected.txt", "before\n", "seed selected");
    sandbox.commit_file("unrelated.txt", "clean\n", "seed unrelated");
    sandbox.write("selected.txt", "after\n");
    let hook = sandbox.path().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks dir");
    std::fs::write(&hook, "#!/bin/sh\nprintf 'mutated\\n' > unrelated.txt\n").expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("make hook executable");
    }
    let before = sandbox.rev_parse("HEAD");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit without repository hooks",
        "--path",
        "selected.txt",
    ]));
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json(&output)["data"]["semantics"]["repository_hooks_executed"],
        false
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("unrelated.txt")).unwrap(),
        "clean\n",
        "the repository pre-commit hook must not mutate unrelated content"
    );
    assert_ne!(sandbox.rev_parse("HEAD"), before);
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_does_not_run_post_commit_hook_that_creates_and_stages_path() {
    let sandbox = GitSandbox::init("agent-checked-git-no-post-hook");
    sandbox.commit_file("selected.txt", "before\n", "seed selected");
    sandbox.write("selected.txt", "after\n");
    let hook = sandbox.path().join(".git/hooks/post-commit");
    std::fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks dir");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf 'created\\n' > hook-created.txt\ngit add -- hook-created.txt\n",
    )
    .expect("write post-commit hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("make hook executable");
    }

    let before = sandbox.rev_parse("HEAD");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit without post hook",
        "--path",
        "selected.txt",
    ]));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(sandbox.rev_parse("HEAD"), before);
    assert!(
        !sandbox.path().join("hook-created.txt").exists(),
        "the repository post-commit hook must not create or stage unrelated state"
    );
    assert!(
        !capture("git", sandbox.path(), &["status", "--short"]).contains("hook-created.txt"),
        "the repository post-commit hook must not stage the sentinel"
    );
}

#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_does_not_run_post_index_change_hook_during_index_writes() {
    let sandbox = GitSandbox::init("agent-checked-git-no-post-index-change-hook");
    sandbox.commit_file("selected.txt", "before\n", "seed selected");
    sandbox.commit_file("unrelated.txt", "clean\n", "seed unrelated");
    sandbox.write("selected.txt", "after\n");
    sandbox.write("unrelated.txt", "staged\n");
    sandbox.git(&["add", "--", "unrelated.txt"]);
    sandbox.write("unrelated.txt", "unstaged\n");
    sandbox.write("hook-sentinel.txt", "untouched\n");

    let hook = sandbox.path().join(".git/hooks/post-index-change");
    std::fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hooks dir");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf 'executed\\n' > hook-sentinel.txt\nprintf 'hook-mutated\\n' > unrelated.txt\n",
    )
    .expect("write post-index-change hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .expect("make hook executable");
    }

    let before = sandbox.rev_parse("HEAD");
    let unrelated_index_before = sandbox.rev_parse(":unrelated.txt");
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit with every repository hook disabled",
        "--path",
        "selected.txt",
    ]));

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("hook-sentinel.txt")).unwrap(),
        "untouched\n",
        "post-index-change must not execute during checked-commit index writes"
    );
    assert_eq!(sandbox.rev_parse(":unrelated.txt"), unrelated_index_before);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("unrelated.txt")).unwrap(),
        "unstaged\n"
    );

    // Negative control: the installed hook is runnable and would mutate the
    // sentinels if hooks were not pinned off by checked commit.
    sandbox.git(&["add", "--", "unrelated.txt"]);
    assert_eq!(
        std::fs::read_to_string(sandbox.path().join("hook-sentinel.txt")).unwrap(),
        "executed\n"
    );
}

#[test]
#[ignore = "requires the jj and git binaries"]
fn jj_checked_commit_is_unsupported_without_snapshot_or_commit_mutation() {
    let sandbox = JjSandbox::init_non_colocated("agent-checked-jj");
    sandbox.write("selected.txt", "selected\n");
    sandbox.write("unrelated.txt", "unrelated\n");
    let before = sandbox.at_commit();
    let before_op = sandbox.op_head();
    let output = agent(args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit selected",
        "--path",
        "selected.txt",
        "--include-machine-paths",
    ]));

    assert_eq!(output.status.code(), Some(10));
    let result = json(&output);
    assert_eq!(result["error"]["kind"], "unsupported");
    assert_eq!(result["error"]["code"], "jujutsu_atomic_commit_unsupported");
    assert_eq!(
        sandbox.op_head(),
        before_op,
        "agent must not create a jj operation"
    );
    assert_eq!(sandbox.at_commit(), before, "agent must not rewrite @");
}

#[cfg(unix)]
#[test]
#[ignore = "requires the git binary"]
fn git_checked_commit_round_trips_a_non_utf8_path() {
    use std::os::unix::ffi::OsStringExt;
    use vcs_testkit::non_utf8_filename;

    let sandbox = GitSandbox::init("agent-checked-git-bytes");
    sandbox.commit_file("seed.txt", "seed\n", "seed");
    let filename = non_utf8_filename();
    std::fs::write(sandbox.path().join(&filename), "bytes\n").expect("write non-UTF-8 path");
    let before = sandbox.rev_parse("HEAD");
    let mut invocation = args(&[
        "commit",
        "--repo",
        sandbox.path().to_str().expect("UTF-8 sandbox path"),
        "--write-intent",
        "commit",
        "--expected-revision",
        &before,
        "--message",
        "commit byte path",
        "--path",
    ]);
    invocation.push(filename.clone());
    invocation.push(OsString::from("--include-machine-paths"));
    let output = agent(invocation);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = json(&output);
    assert_eq!(
        result["data"]["included_paths"][0]["encoding"],
        "os-bytes-hex"
    );
    let raw = filename.into_vec();
    let expected = raw
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(result["data"]["included_paths"][0]["value"], expected);
}
