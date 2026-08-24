//! Hermetic Phase 0 contract checks for the versioned agent-interface corpus.
//!
//! These tests only read committed JSON fixtures and run the standard-library
//! validator/recorder locally.  They never contact a model, forge, MCP host, or
//! network service, so the normal Cargo test gate stays deterministic.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const CORPUS: &str = include_str!("../../../docs/agent-interface/corpus.v1.json");
const RESULT_SCHEMA: &str = include_str!("../../../docs/agent-interface/result-schema.v1.json");
const BASELINE: &str = include_str!("../../../docs/agent-interface/baseline-mcp.v1.json");

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn corpus_is_versioned_and_covers_the_routing_matrix() {
    let corpus: Value = serde_json::from_str(CORPUS).expect("valid corpus JSON");
    assert_eq!(corpus["schema_version"], "agent-interface.corpus.v1");
    assert_eq!(corpus["corpus_version"], "1.0.0");
    assert_eq!(
        corpus["selection_policy"]["preferred_interface"],
        "vcs-agent"
    );

    let cases = corpus["cases"].as_array().expect("cases array");
    let mut ids = BTreeSet::new();
    let scenarios: BTreeSet<&str> = cases
        .iter()
        .map(|case| {
            let id = case["case_id"].as_str().expect("case id");
            assert!(ids.insert(id), "duplicate case id: {id}");
            case["scenario"].as_str().expect("scenario")
        })
        .collect();
    for required in [
        "inspect_status",
        "changes_diff",
        "exact_path_commit",
        "publish_pr",
        "wait_ci",
        "conflict",
        "ordinary_file_search",
        "unsupported_low_level",
        "preferred_unavailable",
    ] {
        assert!(scenarios.contains(required), "missing scenario {required}");
    }
    assert!(cases.iter().any(|case| case["request"]["backend"] == "git"));
    assert!(cases.iter().any(|case| case["request"]["backend"] == "jj"));
    for forge in ["github", "gitlab", "gitea"] {
        assert!(
            cases.iter().any(|case| case["request"]["forge"] == forge),
            "missing forge variant {forge}"
        );
    }
}

#[test]
fn result_schema_and_no_data_baseline_are_explicit() {
    let schema: Value = serde_json::from_str(RESULT_SCHEMA).expect("valid result schema");
    assert_eq!(
        schema["properties"]["schema_version"]["const"],
        "agent-interface.result.v1"
    );
    for field in ["selection", "calls", "workspace", "revision"] {
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item == field)
        );
    }

    let baseline: Value = serde_json::from_str(BASELINE).expect("valid baseline JSON");
    assert_eq!(baseline["schema_version"], "agent-interface.baseline.v1");
    assert_eq!(baseline["status"], "no_data");
    assert!(
        baseline["metrics"].is_null(),
        "no_data must not become zero metrics"
    );
    assert_eq!(baseline["harness"]["availability"], "unavailable");
}

fn python_command() -> Command {
    for candidate in ["python", "python3"] {
        let available = Command::new(candidate).arg("--version").output();
        if available.is_ok_and(|output| output.status.success()) {
            let mut command = Command::new(candidate);
            command.env("PYTHONDONTWRITEBYTECODE", "1");
            return command;
        }
    }
    panic!("Python 3 is required to run the deterministic agent-interface script tests");
}

fn run_python(args: &[&str]) -> std::process::Output {
    python_command()
        .current_dir(root())
        .args(args)
        .output()
        .expect("spawn Python")
}

#[test]
fn validator_and_recorder_are_repeatable_without_network_state() {
    let repo = root();
    let corpus = repo.join("docs/agent-interface/corpus.v1.json");
    let results = repo.join("docs/agent-interface/fixtures/results.v1.json");
    let baseline = repo.join("docs/agent-interface/baseline-mcp.v1.json");
    let validator = repo.join("scripts/agent-interface/validate.py");
    let recorder = repo.join("scripts/agent-interface/record.py");
    let corpus = corpus.to_str().expect("corpus path");
    let results = results.to_str().expect("results path");
    let baseline = baseline.to_str().expect("baseline path");
    let validator = validator.to_str().expect("validator path");
    let recorder = recorder.to_str().expect("recorder path");

    let validation = run_python(&[
        validator,
        "--corpus",
        corpus,
        "--results",
        results,
        "--baseline",
        baseline,
    ]);
    assert!(
        validation.status.success(),
        "validator failed: {}{}",
        String::from_utf8_lossy(&validation.stdout),
        String::from_utf8_lossy(&validation.stderr)
    );

    let temp = std::env::temp_dir().join(format!("vcs-agent-interface-{}", std::process::id()));
    fs::create_dir_all(&temp).expect("create isolated recorder directory");
    let first = temp.join("first.json");
    let second = temp.join("second.json");
    for output in [&first, &second] {
        let output = output.to_str().expect("recording path");
        let recorded = run_python(&[
            recorder,
            "--corpus",
            corpus,
            "--results",
            results,
            "--baseline",
            baseline,
            "--output",
            output,
        ]);
        assert!(
            recorded.status.success(),
            "recorder failed: {}{}",
            String::from_utf8_lossy(&recorded.stdout),
            String::from_utf8_lossy(&recorded.stderr)
        );
    }
    assert_eq!(
        fs::read(&first).expect("first recording"),
        fs::read(&second).expect("second recording")
    );
    fs::remove_dir_all(temp).expect("remove isolated recorder directory");
}
