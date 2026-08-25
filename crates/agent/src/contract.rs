use std::collections::BTreeMap;
use std::process::ExitCode;

use processkit::{Error as ProcessError, ErrorReason};
use serde::Serialize;

use crate::redaction::{RedactionPolicy, redact_text};

pub(crate) const CONTRACT_VERSION: &str = "vcs-agent/v1";
const BINARY_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) type AgentResult<T> = Result<T, Box<AgentError>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    Unsupported,
    Denied,
    InvalidInput,
    Backend,
    Forge,
    Authentication,
    Timeout,
    Cancelled,
    OutputLimit,
    ExternalCommand,
    Internal,
}

impl ErrorKind {
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Self::InvalidInput => 2,
            Self::Unsupported => 10,
            Self::Denied => 20,
            Self::Backend => 30,
            Self::Forge => 31,
            Self::Authentication => 32,
            Self::Timeout => 40,
            Self::Cancelled => 41,
            Self::OutputLimit => 42,
            Self::ExternalCommand => 50,
            Self::Internal => 70,
        }
    }

    pub(crate) fn contract() -> Vec<ErrorDescriptor> {
        [
            Self::Unsupported,
            Self::Denied,
            Self::InvalidInput,
            Self::Backend,
            Self::Forge,
            Self::Authentication,
            Self::Timeout,
            Self::Cancelled,
            Self::OutputLimit,
            Self::ExternalCommand,
            Self::Internal,
        ]
        .into_iter()
        .map(|kind| ErrorDescriptor {
            kind,
            exit_code: kind.exit_code(),
        })
        .collect()
    }
}

#[derive(Serialize)]
pub(crate) struct ErrorDescriptor {
    kind: ErrorKind,
    exit_code: u8,
}

#[derive(Serialize)]
pub(crate) struct ExitBand {
    name: &'static str,
    first: u8,
    last: u8,
}

impl ExitBand {
    pub(crate) fn contract() -> Vec<Self> {
        vec![
            Self {
                name: "success",
                first: 0,
                last: 0,
            },
            Self {
                name: "caller",
                first: 2,
                last: 19,
            },
            Self {
                name: "policy",
                first: 20,
                last: 29,
            },
            Self {
                name: "domain",
                first: 30,
                last: 39,
            },
            Self {
                name: "lifecycle",
                first: 40,
                last: 49,
            },
            Self {
                name: "external_command",
                first: 50,
                last: 59,
            },
            Self {
                name: "internal",
                first: 70,
                last: 79,
            },
        ]
    }
}

#[allow(dead_code)] // Reserved by the v1 application boundary for T-191+ outcomes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FailureDomain {
    Backend,
    Forge,
    Authentication,
    ExternalCommand,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Fallback {
    allowed: bool,
    interface: &'static str,
    reason: &'static str,
}

impl Fallback {
    pub(crate) const fn raw_cli(reason: &'static str) -> Self {
        Self {
            allowed: true,
            interface: "raw-cli",
            reason,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct MachineError {
    kind: ErrorKind,
    exit_code: u8,
    code: &'static str,
    message: String,
    retryable: bool,
    details: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(crate) struct MachineEnvelope {
    contract_version: &'static str,
    binary_version: &'static str,
    operation: &'static str,
    status: &'static str,
    data: Option<serde_json::Value>,
    error: Option<MachineError>,
    warnings: Vec<String>,
    fallback: Option<Fallback>,
}

impl MachineEnvelope {
    pub(crate) fn success(operation: &'static str, data: impl Serialize) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            binary_version: BINARY_VERSION,
            operation,
            status: "success",
            data: Some(serde_json::to_value(data).expect("probe DTO is serializable")),
            error: None,
            warnings: Vec::new(),
            fallback: None,
        }
    }

    fn failure(error: &AgentError) -> Self {
        let redaction = RedactionPolicy {
            include_machine_paths: error.include_machine_paths,
        };
        Self {
            contract_version: CONTRACT_VERSION,
            binary_version: BINARY_VERSION,
            operation: error.operation,
            status: "error",
            data: None,
            error: Some(MachineError {
                kind: error.kind,
                exit_code: error.kind.exit_code(),
                code: error.code,
                message: redact_text(error.message, redaction),
                retryable: error.retryable,
                details: error
                    .details
                    .iter()
                    .map(|(key, value)| (key.clone(), redact_text(value, redaction)))
                    .collect(),
            }),
            warnings: error
                .warnings
                .iter()
                .map(|warning| redact_text(warning, redaction))
                .collect(),
            fallback: error.fallback.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AgentError {
    operation: &'static str,
    kind: ErrorKind,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    details: BTreeMap<String, String>,
    warnings: Vec<String>,
    fallback: Option<Fallback>,
    include_machine_paths: bool,
}

impl AgentError {
    pub(crate) fn new(
        operation: &'static str,
        kind: ErrorKind,
        code: &'static str,
        retryable: bool,
    ) -> Self {
        let message = match kind {
            ErrorKind::Unsupported => "unsupported outcome",
            ErrorKind::Denied => "operation denied",
            ErrorKind::InvalidInput => "invalid input",
            ErrorKind::Backend => "repository backend failed",
            ErrorKind::Forge => "forge operation failed",
            ErrorKind::Authentication => "authentication failed",
            ErrorKind::Timeout => "operation timed out",
            ErrorKind::Cancelled => "operation cancelled",
            ErrorKind::OutputLimit => "output limit exceeded",
            ErrorKind::ExternalCommand => "external command failed",
            ErrorKind::Internal => "internal error",
        };
        Self {
            operation,
            kind,
            code,
            message,
            retryable,
            details: BTreeMap::new(),
            warnings: Vec::new(),
            fallback: None,
            include_machine_paths: false,
        }
    }

    pub(crate) fn invalid_input(code: &'static str) -> Self {
        Self::invalid_input_for("unknown", code)
    }

    pub(crate) fn invalid_input_for(operation: &'static str, code: &'static str) -> Self {
        Self::new(operation, ErrorKind::InvalidInput, code, false)
    }

    pub(crate) fn internal(code: &'static str) -> Self {
        Self::new("unknown", ErrorKind::Internal, code, false)
    }

    pub(crate) fn output_limit(operation: &'static str, max_bytes: usize) -> Self {
        Self::new(
            operation,
            ErrorKind::OutputLimit,
            "machine_result_too_large",
            false,
        )
        .with_detail("max_bytes", max_bytes.to_string())
    }

    #[allow(dead_code)] // Contract mapping is pinned now; typed-client use starts in T-191.
    pub(crate) fn from_processkit(
        operation: &'static str,
        domain: FailureDomain,
        error: &ProcessError,
    ) -> Self {
        let kind = if error.is_timeout() {
            ErrorKind::Timeout
        } else if error.is_cancelled() {
            ErrorKind::Cancelled
        } else if matches!(error.reason(), ErrorReason::OutputTooLarge { .. }) {
            ErrorKind::OutputLimit
        } else if error.is_permission_denied() {
            ErrorKind::Denied
        } else {
            match domain {
                FailureDomain::Backend => ErrorKind::Backend,
                FailureDomain::Forge => ErrorKind::Forge,
                FailureDomain::Authentication => ErrorKind::Authentication,
                FailureDomain::ExternalCommand => ErrorKind::ExternalCommand,
            }
        };
        Self::new(operation, kind, "typed_client_failed", error.is_transient())
            .with_detail("process_error_kind", error.kind().name())
    }

    pub(crate) fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub(crate) fn with_fallback(mut self, fallback: Fallback) -> Self {
        self.fallback = Some(fallback);
        self
    }

    #[cfg(test)]
    fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    #[cfg(test)]
    fn include_machine_paths(mut self, include: bool) -> Self {
        self.include_machine_paths = include;
        self
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.kind.exit_code())
    }
}

pub(crate) trait IntoEnvelope {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<&'static str>);
}

impl IntoEnvelope for MachineEnvelope {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<&'static str>) {
        (self, ExitCode::SUCCESS, None)
    }
}

impl IntoEnvelope for AgentError {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<&'static str>) {
        let exit_code = self.exit_code();
        let diagnostic = Some(self.message);
        (MachineEnvelope::failure(&self), exit_code, diagnostic)
    }
}

pub(crate) struct RenderedOutput {
    pub(crate) stdout: Vec<u8>,
    pub(crate) diagnostic: Option<&'static str>,
    pub(crate) exit_code: ExitCode,
}

pub(crate) fn render(value: impl IntoEnvelope, max_bytes: usize) -> RenderedOutput {
    let (envelope, exit_code, diagnostic) = value.into_envelope();
    let operation = envelope.operation;
    let mut stdout = serialize(&envelope);
    if stdout.len() > max_bytes {
        let error = AgentError::output_limit(operation, max_bytes);
        let envelope = MachineEnvelope::failure(&error);
        stdout = serialize(&envelope);
        debug_assert!(
            stdout.len() <= crate::cli::MIN_MAX_OUTPUT_BYTES,
            "the bounded output-limit envelope must fit the minimum budget"
        );
        return RenderedOutput {
            stdout,
            diagnostic: Some(error.message),
            exit_code: error.exit_code(),
        };
    }
    RenderedOutput {
        stdout,
        diagnostic,
        exit_code,
    }
}

fn serialize(envelope: &MachineEnvelope) -> Vec<u8> {
    let mut json = serde_json::to_vec_pretty(envelope).expect("machine envelope is serializable");
    json.push(b'\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn exit_codes_are_stable_unique_and_inside_documented_bands() {
        let descriptors = ErrorKind::contract();
        let value = serde_json::to_value(&descriptors).expect("serializable descriptors");
        let codes: Vec<u64> = value
            .as_array()
            .expect("array")
            .iter()
            .map(|entry| entry["exit_code"].as_u64().expect("numeric code"))
            .collect();
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(codes.len(), unique.len());
        assert_eq!(ErrorKind::Unsupported.exit_code(), 10);
        assert_eq!(ErrorKind::Denied.exit_code(), 20);
        assert_eq!(ErrorKind::InvalidInput.exit_code(), 2);
        assert_eq!(ErrorKind::Backend.exit_code(), 30);
        assert_eq!(ErrorKind::Forge.exit_code(), 31);
        assert_eq!(ErrorKind::Authentication.exit_code(), 32);
        assert_eq!(ErrorKind::Timeout.exit_code(), 40);
        assert_eq!(ErrorKind::Cancelled.exit_code(), 41);
        assert_eq!(ErrorKind::OutputLimit.exit_code(), 42);
        assert_eq!(ErrorKind::ExternalCommand.exit_code(), 50);
    }

    #[test]
    fn fail_loud_output_budget_emits_only_an_output_limit_error() {
        let success = MachineEnvelope::success("probe", json!({"large": "x".repeat(4096)}));
        let output = render(success, crate::cli::MIN_MAX_OUTPUT_BYTES);
        assert_eq!(output.exit_code, ExitCode::from(42));
        let value: Value = serde_json::from_slice(&output.stdout).expect("complete JSON error");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"], Value::Null);
        assert_eq!(value["error"]["kind"], "output_limit");
        assert!(
            !String::from_utf8(output.stdout)
                .expect("UTF-8")
                .contains("xxxx")
        );
    }

    #[test]
    fn every_error_field_runs_through_redaction() {
        let error = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_detail("remote", "https://user:token@example.invalid/repo")
            .with_detail("token", "token=top-secret")
            .with_detail("path", r"C:\Users\alice\private\repo")
            .with_warning("Authorization: Bearer abc123")
            .include_machine_paths(false);
        let output = render(error, crate::cli::DEFAULT_MAX_OUTPUT_BYTES);
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        assert!(!text.contains("top-secret"));
        assert!(!text.contains("user:token"));
        assert!(!text.contains("alice"));
        assert!(!text.contains("abc123"));
        assert!(text.contains("[REDACTED]"));
        assert!(text.contains("[REDACTED_PATH]"));
    }

    #[test]
    fn machine_paths_are_included_only_when_explicitly_requested() {
        let path = r"C:\Users\alice\repo";
        let hidden =
            AgentError::new("probe", ErrorKind::Backend, "test", false).with_detail("path", path);
        let visible = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_detail("path", path)
            .include_machine_paths(true);
        let hidden = String::from_utf8(render(hidden, 4096).stdout).expect("UTF-8");
        let visible = String::from_utf8(render(visible, 4096).stdout).expect("UTF-8");
        assert!(!hidden.contains("alice"));
        assert!(visible.contains(r"C:\\Users\\alice\\repo"));
    }

    #[test]
    fn processkit_mapping_preserves_lifecycle_and_domain_classes() {
        let timeout = ProcessError::timeout("git", std::time::Duration::from_secs(1), "", "");
        assert_eq!(
            AgentError::from_processkit("inspect", FailureDomain::Backend, &timeout).kind(),
            ErrorKind::Timeout
        );
        let cancelled: ProcessError = ErrorReason::Cancelled {
            program: "git".to_owned(),
        }
        .into();
        assert_eq!(
            AgentError::from_processkit("inspect", FailureDomain::Backend, &cancelled).kind(),
            ErrorKind::Cancelled
        );
        let exit = ProcessError::exit("gh", 1, "", "failed");
        assert_eq!(
            AgentError::from_processkit("publish", FailureDomain::Forge, &exit).kind(),
            ErrorKind::Forge
        );
        assert_eq!(
            AgentError::from_processkit("publish", FailureDomain::Authentication, &exit).kind(),
            ErrorKind::Authentication
        );
        assert_eq!(
            AgentError::from_processkit("publish", FailureDomain::ExternalCommand, &exit).kind(),
            ErrorKind::ExternalCommand
        );
    }

    #[test]
    fn schema_fixture_identifies_the_same_contract() {
        let schema: Value = serde_json::from_str(include_str!("../schema/envelope.v1.schema.json"))
            .expect("valid JSON schema");
        assert_eq!(
            schema["properties"]["contract_version"]["const"],
            CONTRACT_VERSION
        );
        assert_eq!(schema["properties"]["binary_version"]["type"], "string");
        assert_eq!(
            schema["required"].as_array().expect("required array").len(),
            8
        );
    }
}
