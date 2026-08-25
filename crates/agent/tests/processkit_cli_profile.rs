use serde_json::Value;

fn compile_schema(source: &str, label: &str) -> jsonschema::Validator {
    let schema: Value =
        serde_json::from_str(source).unwrap_or_else(|error| panic!("{label} is JSON: {error}"));
    assert!(
        jsonschema::meta::is_valid(&schema),
        "{label} satisfies its declared meta-schema"
    );
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .unwrap_or_else(|error| panic!("{label} compiles: {error}"))
}

#[test]
fn committed_processkit_cli_profile_and_child_evidence_are_schema_valid() {
    let profile: Value = serde_json::from_str(include_str!(
        "../../../docs/agent-interface/processkit-cli-profile.v1.json"
    ))
    .expect("committed ProcessKit-CLI profile is JSON");
    let profile_validator = compile_schema(
        include_str!("../../../docs/agent-interface/processkit-cli-profile.v1.schema.json"),
        "ProcessKit-CLI profile schema",
    );
    if let Err(error) = profile_validator.validate(&profile) {
        panic!("committed ProcessKit-CLI profile satisfies its schema: {error}");
    }

    let envelope_validator = compile_schema(
        include_str!("../schema/envelope.v1.schema.json"),
        "vcs-agent envelope schema",
    );
    for (name, source) in [
        (
            "probe-success",
            include_str!("fixtures/probe-success.v1.json"),
        ),
        (
            "unknown-outcome",
            include_str!("fixtures/unknown-outcome.v1.json"),
        ),
        (
            "inspect-success",
            include_str!("fixtures/inspect-success-git.v1.json"),
        ),
    ] {
        let child: Value =
            serde_json::from_str(source).unwrap_or_else(|error| panic!("{name} is JSON: {error}"));
        if let Err(error) = envelope_validator.validate(&child) {
            panic!("{name} satisfies the child envelope schema: {error}");
        }
    }
}

#[test]
fn profile_schema_rejects_exact_required_surface_drift() {
    let profile: Value = serde_json::from_str(include_str!(
        "../../../docs/agent-interface/processkit-cli-profile.v1.json"
    ))
    .expect("committed ProcessKit-CLI profile is JSON");
    let profile_validator = compile_schema(
        include_str!("../../../docs/agent-interface/processkit-cli-profile.v1.schema.json"),
        "ProcessKit-CLI profile schema",
    );
    let surface = profile["preflight"]["required_surface"]
        .as_array()
        .expect("required surface array");
    let mut mutations = Vec::new();

    let mut removal = profile.clone();
    removal["preflight"]["required_surface"]
        .as_array_mut()
        .expect("required surface array")
        .remove(10);
    mutations.push(("removal", removal));

    let mut substitution = profile.clone();
    substitution["preflight"]["required_surface"][10] = Value::String("run:--unrelated".into());
    mutations.push(("substitution", substitution));

    let mut duplication = profile.clone();
    duplication["preflight"]["required_surface"]
        .as_array_mut()
        .expect("required surface array")
        .push(surface[10].clone());
    mutations.push(("duplication", duplication));

    let mut addition = profile.clone();
    addition["preflight"]["required_surface"]
        .as_array_mut()
        .expect("required surface array")
        .push(Value::String("run:--unrelated".into()));
    mutations.push(("addition", addition));

    let mut reordering = profile.clone();
    reordering["preflight"]["required_surface"]
        .as_array_mut()
        .expect("required surface array")
        .swap(9, 10);
    mutations.push(("reordering", reordering));

    for (label, mutated) in mutations {
        assert!(
            profile_validator.validate(&mutated).is_err(),
            "profile schema rejects required-surface {label}"
        );
    }
}

#[test]
fn evidence_fixture_covers_each_profile_scenario_without_membership_overclaim() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../docs/agent-interface/fixtures/processkit-cli-evidence.v1.json"
    ))
    .expect("committed ProcessKit-CLI evidence is JSON");
    let scenarios = evidence["scenarios"].as_array().expect("scenario array");
    let mut ids = scenarios
        .iter()
        .map(|scenario| scenario["id"].as_str().expect("scenario id"))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "agent-structured-failure",
            "agent-success",
            "bounded-capture",
            "control-cancel",
            "nested-containment",
            "timeout",
        ]
    );
    let nested = scenarios
        .iter()
        .find(|scenario| scenario["id"] == "nested-containment")
        .expect("nested-containment evidence");
    assert_eq!(
        nested["claim"],
        "outer-lifecycle-observed-inner-membership-not-attested"
    );
}

#[test]
fn agent_manifest_does_not_link_processkit_cli_library() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("processkit_cli") || line.starts_with("processkit-cli")
    }));
}
