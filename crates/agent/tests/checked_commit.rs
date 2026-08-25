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
#[ignore = "requires the jj and git binaries"]
fn jj_checked_commit_uses_exact_filesets_and_preserves_unselected_change_without_bookmark() {
    let sandbox = JjSandbox::init_non_colocated("agent-checked-jj");
    sandbox.write("selected.txt", "selected\n");
    sandbox.write("unrelated.txt", "unrelated\n");
    sandbox.jj(&["status"]);
    let before = sandbox.at_commit();
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
    assert_eq!(result["data"]["repository"]["backend"], "jujutsu");
    assert_eq!(result["data"]["before"]["revision"], before);
    assert!(result["data"]["before"]["change_id"].is_string());
    assert!(result["data"]["after"]["change_id"].is_string());
    assert_eq!(
        result["data"]["semantics"]["backend_selection"],
        "jujutsu-exact-filesets"
    );
    let remaining = sandbox.jj_capture(&["diff", "-r", "@", "--summary"]);
    assert!(remaining.contains("unrelated.txt"), "{remaining}");
    assert!(!remaining.contains("selected.txt"), "{remaining}");
    assert_eq!(
        sandbox.jj_capture(&["log", "-r", "@-", "--no-graph", "-T", "description"]),
        "commit selected"
    );

    let after = sandbox.at_commit();
    assert_eq!(result["data"]["after"]["revision"], after);
    assert_ne!(after, before);
    let retry = agent(invocation);
    assert_eq!(retry.status.code(), Some(20));
    assert_eq!(json(&retry)["error"]["code"], "stale_expected_revision");
    assert_eq!(sandbox.at_commit(), after);
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
