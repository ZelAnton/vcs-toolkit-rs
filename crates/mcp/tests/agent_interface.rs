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
const RECORDING_SCHEMA: &str =
    include_str!("../../../docs/agent-interface/recording-schema.v1.json");
const RECORDING_FIXTURE: &str =
    include_str!("../../../docs/agent-interface/fixtures/recording.v1.json");
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
    let recording_schema: Value =
        serde_json::from_str(RECORDING_SCHEMA).expect("valid recording schema");
    assert_eq!(
        recording_schema["properties"]["schema_version"]["const"],
        "agent-interface.recording.v1"
    );
    for field in ["outcome_status", "calls", "revision"] {
        assert!(
            recording_schema["properties"]["cases"]["items"]["required"]
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
    let recording: Value = serde_json::from_slice(&fs::read(&first).expect("recording JSON"))
        .expect("valid recording JSON");
    let expected_recording: Value =
        serde_json::from_str(RECORDING_FIXTURE).expect("valid recording fixture");
    assert_eq!(recording, expected_recording);
    let published = recording["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case_id"] == "publish-pr-github")
        .expect("published case");
    assert_eq!(published["outcome_status"], "success");
    assert_eq!(published["calls"]["preferred_interface"], 2);
    assert_eq!(published["calls"]["fallback_interface"], 0);
    assert_eq!(published["calls"]["raw_cli"], 0);
    assert_eq!(published["calls"]["total"], 2);
    assert_eq!(published["revision"]["before"], "a1");
    assert_eq!(published["revision"]["after"], "abc123");
    assert_eq!(published["revision"]["published"], "abc123");
    assert_eq!(published["revision"]["terminal_ci"]["revision"], "abc123");
    assert_eq!(
        published["revision"]["terminal_ci"]["conclusion"],
        "success"
    );
    fs::remove_dir_all(temp).expect("remove isolated recorder directory");
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialise mutation fixture"),
    )
    .expect("write mutation fixture");
}

fn assert_validator_rejects(validator: &str, corpus: &str, results: &Path, baseline: &str) {
    let results = results.to_str().expect("mutation result path");
    let output = run_python(&[
        validator,
        "--corpus",
        corpus,
        "--results",
        results,
        "--baseline",
        baseline,
    ]);
    assert!(
        !output.status.success(),
        "mutated result unexpectedly passed validation"
    );
}

#[test]
fn validator_rejects_contradictory_selection_and_partial_results() {
    let repo = root();
    let corpus = repo.join("docs/agent-interface/corpus.v1.json");
    let results_path = repo.join("docs/agent-interface/fixtures/results.v1.json");
    let baseline = repo.join("docs/agent-interface/baseline-mcp.v1.json");
    let validator = repo.join("scripts/agent-interface/validate.py");
    let corpus = corpus.to_str().expect("corpus path");
    let baseline = baseline.to_str().expect("baseline path");
    let validator = validator.to_str().expect("validator path");
    let original: Value =
        serde_json::from_str(&fs::read_to_string(&results_path).expect("read result fixture"))
            .expect("result fixture JSON");
    let temp = std::env::temp_dir().join(format!(
        "vcs-agent-interface-negative-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp).expect("create negative fixture directory");

    let mut false_activation = original.clone();
    false_activation["results"][0]["selection"]["false_activation"] = true.into();
    let false_activation_path = temp.join("false-activation.json");
    write_json(&false_activation_path, &false_activation);
    assert_validator_rejects(validator, corpus, &false_activation_path, baseline);

    let mut raw_bypass = original.clone();
    let raw_bypass_case = raw_bypass["results"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|result| result["case_id"] == "mcp-unavailable-fallback")
        .expect("MCP fallback fixture");
    raw_bypass_case["selection"]["raw_cli_bypass"] = true.into();
    let raw_bypass_path = temp.join("raw-bypass.json");
    write_json(&raw_bypass_path, &raw_bypass);
    assert_validator_rejects(validator, corpus, &raw_bypass_path, baseline);

    let mut partial = original;
    partial["results"].as_array_mut().unwrap().pop();
    let partial_path = temp.join("partial.json");
    write_json(&partial_path, &partial);
    let partial_output = run_python(&[
        validator,
        "--corpus",
        corpus,
        "--results",
        partial_path.to_str().expect("partial result path"),
        "--baseline",
        baseline,
    ]);
    assert!(!partial_output.status.success());
    assert!(
        String::from_utf8_lossy(&partial_output.stderr).contains("results missing case IDs"),
        "missing-case diagnostic: {}",
        String::from_utf8_lossy(&partial_output.stderr)
    );

    let output_path = temp.join("partial-recording.json");
    let recorder = repo.join("scripts/agent-interface/record.py");
    let recorded = run_python(&[
        recorder.to_str().expect("recorder path"),
        "--corpus",
        corpus,
        "--results",
        partial_path.to_str().expect("partial result path"),
        "--baseline",
        baseline,
        "--output",
        output_path.to_str().expect("partial recording path"),
    ]);
    assert!(!recorded.status.success());
    assert!(!output_path.exists(), "recorder wrote a partial recording");
    fs::remove_dir_all(temp).expect("remove negative fixture directory");
}
