use std::time::Duration;

use processkit::CancellationToken;
use serde::Serialize;
use vcs_cli_support::OutputBudget;

use crate::cli::{Invocation, Operation};
use crate::contract::{
    AgentError, AgentResult, CONTRACT_VERSION, ErrorDescriptor, ErrorKind, ExitBand, Fallback,
    MachineEnvelope,
};

const DEFAULT_DEADLINE: Duration = Duration::from_secs(120);

/// Policy carried across every outcome implementation.
///
/// A repository or forge outcome must project these values onto the existing
/// typed clients. Keeping the token, deadline, and fail-loud budget in one
/// boundary prevents an operation from accidentally bypassing ProcessKit.
pub(crate) struct ExecutionPolicy {
    pub(crate) cancellation: CancellationToken,
    pub(crate) deadline: Duration,
    pub(crate) content_budget: OutputBudget,
}

impl ExecutionPolicy {
    pub(crate) fn new(max_output_bytes: usize) -> Self {
        Self {
            cancellation: CancellationToken::new(),
            deadline: DEFAULT_DEADLINE,
            content_budget: OutputBudget::bytes(max_output_bytes),
        }
    }
}

#[derive(Serialize)]
struct ProbeData {
    contract: ContractCompatibility,
    commands: CommandCapabilities,
    execution: ExecutionCapabilities,
    limits: Limits,
    exit_bands: Vec<ExitBand>,
    error_kinds: Vec<ErrorDescriptor>,
}

#[derive(Serialize)]
struct ContractCompatibility {
    schema: &'static str,
    minimum_client_contract: &'static str,
    maximum_client_contract: &'static str,
    compatibility: &'static str,
}

#[derive(Serialize)]
struct CommandCapabilities {
    supported: Vec<&'static str>,
    reserved: Vec<&'static str>,
}

#[derive(Serialize)]
struct ExecutionCapabilities {
    vcs_backends: Vec<&'static str>,
    forges: Vec<&'static str>,
    subprocess_route: &'static str,
    process_tree_containment: &'static str,
    cancellation: &'static str,
    deadlines: &'static str,
    deadline_default_seconds: u64,
    processkit_cli_composition: &'static str,
    raw_command_escape_hatch: bool,
}

#[derive(Serialize)]
struct Limits {
    machine_output_default_bytes: usize,
    machine_output_max_bytes: usize,
    content_overflow: &'static str,
    machine_paths_included: bool,
}

pub(crate) fn execute(
    invocation: &Invocation,
    policy: &ExecutionPolicy,
) -> AgentResult<MachineEnvelope> {
    // Read the policy at the application boundary even though probe performs no
    // subprocess work. Later commands receive this same object and cannot omit
    // cancellation/deadline/output containment by construction.
    let _ = (&policy.cancellation, policy.deadline, policy.content_budget);

    match invocation.operation {
        Operation::Probe => Ok(MachineEnvelope::success(
            Operation::Probe.name(),
            ProbeData {
                contract: ContractCompatibility {
                    schema: "https://zelanton.github.io/vcs-toolkit-rs/vcs-agent/v1/envelope.schema.json",
                    minimum_client_contract: CONTRACT_VERSION,
                    maximum_client_contract: CONTRACT_VERSION,
                    compatibility: "same-contract-version",
                },
                commands: CommandCapabilities {
                    supported: vec!["probe"],
                    reserved: vec![
                        "inspect",
                        "changes",
                        "commit",
                        "publish",
                        "ci status",
                        "ci wait",
                    ],
                },
                execution: ExecutionCapabilities {
                    vcs_backends: vec!["git", "jujutsu"],
                    forges: vec!["github", "gitlab", "gitea"],
                    subprocess_route: "vcs-toolkit-typed-clients",
                    process_tree_containment: "processkit-rs",
                    cancellation: "processkit-cancellation-token",
                    deadlines: "per-operation",
                    deadline_default_seconds: DEFAULT_DEADLINE.as_secs(),
                    processkit_cli_composition: "external-executable",
                    raw_command_escape_hatch: false,
                },
                limits: Limits {
                    machine_output_default_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
                    machine_output_max_bytes: crate::cli::MAX_MAX_OUTPUT_BYTES,
                    content_overflow: "fail-loud-no-truncation",
                    machine_paths_included: invocation.include_machine_paths,
                },
                exit_bands: ExitBand::contract(),
                error_kinds: ErrorKind::contract(),
            },
        )),
        _ => Err(Box::new(
            AgentError::new(
                invocation.operation.name(),
                ErrorKind::Unsupported,
                "outcome_not_implemented",
                false,
            )
            .with_fallback(Fallback::raw_cli("outcome_not_implemented")),
        )),
    }
}
