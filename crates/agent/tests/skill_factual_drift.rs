use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("vcs-agent crate lives under the repository root")
        .to_path_buf()
}

fn python() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".to_owned()
        } else {
            "python3".to_owned()
        }
    })
}

fn validate(root: &Path, contract: Option<&Path>) -> Output {
    let mut command = Command::new(python());
    command
        .current_dir(root)
        .arg(root.join("scripts/agent-interface/validate_skill.py"))
        .arg("--vcs-agent")
        .arg(env!("CARGO_BIN_EXE_vcs-agent"));
    if let Some(contract) = contract {
        command.arg("--contract").arg(contract);
    }
    command.output().expect("run Skill factual-drift validator")
}

#[test]
fn skill_contract_matches_the_built_binary() {
    let root = repository_root();
    let output = validate(&root, None);
    assert!(
        output.status.success(),
        "validator failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validator_rejects_a_skill_flag_missing_from_the_binary() {
    let root = repository_root();
    let source = root.join("skills/vcs-agent/references/contract.v1.json");
    let mut contract: serde_json::Value =
        serde_json::from_slice(&fs::read(source).expect("read Skill contract"))
            .expect("parse Skill contract");
    contract["required_flags"]["inspect"] = serde_json::json!(["--not-a-real-flag"]);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let temporary = std::env::temp_dir().join(format!(
        "vcs-agent-skill-contract-{}-{nonce}.json",
        std::process::id()
    ));
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&contract).expect("serialize modified contract"),
    )
    .expect("write modified Skill contract");
    let output = validate(&root, Some(&temporary));
    fs::remove_file(&temporary).expect("remove modified Skill contract");

    assert!(!output.status.success(), "drifted contract must fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing flag --not-a-real-flag"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
