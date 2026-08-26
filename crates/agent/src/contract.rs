use std::collections::BTreeMap;
use std::process::ExitCode;

use processkit::{Error as ProcessError, ErrorReason};
use serde::Serialize;

use crate::redaction::{RedactionPolicy, redact_text};

pub const CONTRACT_VERSION: &str = "vcs-agent/v1";
const BINARY_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type AgentResult<T> = Result<T, Box<AgentError>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Unsupported,
    Denied,
    InvalidInput,
    Backend,
    Forge,
    Authentication,
    Timeout,
    Cancelled,
    OutputLimit,
    OutcomeUnknown,
    ExternalCommand,
    Internal,
}

impl ErrorKind {
    pub const fn exit_code(self) -> u8 {
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
            Self::OutcomeUnknown => 43,
            Self::ExternalCommand => 50,
            Self::Internal => 70,
        }
    }

    pub fn contract() -> Vec<ErrorDescriptor> {
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
            Self::OutcomeUnknown,
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
pub struct ErrorDescriptor {
    kind: ErrorKind,
    exit_code: u8,
}

#[derive(Serialize)]
pub struct ExitBand {
    name: &'static str,
    first: u8,
    last: u8,
}

impl ExitBand {
    pub fn contract() -> Vec<Self> {
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

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum FailureDomain {
    Backend,
    Forge,
    Authentication,
    ExternalCommand,
}

#[derive(Clone, Debug, Serialize)]
pub struct Fallback {
    allowed: bool,
    interface: &'static str,
    reason: &'static str,
}

impl Fallback {
    pub const fn raw_cli(reason: &'static str) -> Self {
        Self {
            allowed: true,
            interface: "raw-cli",
            reason,
        }
    }
}

#[derive(Serialize)]
pub struct MachineError {
    kind: ErrorKind,
    exit_code: u8,
    code: &'static str,
    message: String,
    retryable: bool,
    details: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct MachineEnvelope {
    contract_version: &'static str,
    binary_version: &'static str,
    operation: &'static str,
    status: &'static str,
    data: Option<serde_json::Value>,
    error: Option<MachineError>,
    warnings: Vec<String>,
    fallback: Option<Fallback>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum DetailKey {
    MaxBytes,
    ProcessErrorKind,
    RemoteUrl,
    Token,
    RepositoryPath,
    WorkingDirectory,
    Revision,
    Remote,
    Source,
    Target,
    Forge,
    Account,
    Checkpoint,
    PushState,
    ChangeRequestState,
    Conclusion,
    RunId,
}

#[derive(Clone, Copy)]
enum DetailSensitivity {
    Text,
    Secret,
    MachinePath,
}

impl DetailKey {
    const fn name(self) -> &'static str {
        match self {
            Self::MaxBytes => "max_bytes",
            Self::ProcessErrorKind => "process_error_kind",
            Self::RemoteUrl => "remote_url",
            Self::Token => "token",
            Self::RepositoryPath => "repository_path",
            Self::WorkingDirectory => "working_directory",
            Self::Revision => "revision",
            Self::Remote => "remote",
            Self::Source => "source",
            Self::Target => "target",
            Self::Forge => "forge",
            Self::Account => "account",
            Self::Checkpoint => "checkpoint",
            Self::PushState => "push_state",
            Self::ChangeRequestState => "change_request_state",
            Self::Conclusion => "conclusion",
            Self::RunId => "run_id",
        }
    }

    const fn sensitivity(self) -> DetailSensitivity {
        match self {
            Self::Token => DetailSensitivity::Secret,
            Self::RepositoryPath | Self::WorkingDirectory => DetailSensitivity::MachinePath,
            Self::MaxBytes
            | Self::ProcessErrorKind
            | Self::RemoteUrl
            | Self::Revision
            | Self::Remote
            | Self::Source
            | Self::Target
            | Self::Forge
            | Self::Account
            | Self::Checkpoint
            | Self::PushState
            | Self::ChangeRequestState
            | Self::Conclusion
            | Self::RunId => DetailSensitivity::Text,
        }
    }
}

impl MachineEnvelope {
    pub fn success(operation: &'static str, data: impl Serialize) -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            binary_version: BINARY_VERSION,
            operation,
            status: "success",
            data: Some(serde_json::to_value(data).expect("machine DTO is serializable")),
            error: None,
            warnings: Vec::new(),
            fallback: None,
        }
    }

    fn failure(error: &AgentError) -> Self {
        let redaction = error.redaction_policy();
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
                    .map(|(key, value)| {
                        let value = match key.sensitivity() {
                            DetailSensitivity::Secret => "[REDACTED]".to_owned(),
                            DetailSensitivity::MachinePath if !redaction.include_machine_paths => {
                                "[REDACTED_PATH]".to_owned()
                            }
                            DetailSensitivity::Text | DetailSensitivity::MachinePath => {
                                redact_text(value, redaction)
                            }
                        };
                        (key.name().to_owned(), value)
                    })
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
pub struct AgentError {
    operation: &'static str,
    kind: ErrorKind,
    code: &'static str,
    message: &'static str,
    retryable: bool,
    details: BTreeMap<DetailKey, String>,
    warnings: Vec<String>,
    fallback: Option<Fallback>,
    include_machine_paths: bool,
}

impl AgentError {
    pub fn new(
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
            ErrorKind::OutcomeUnknown => "mutation outcome could not be verified",
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

    pub fn invalid_input(code: &'static str) -> Self {
        Self::invalid_input_for("unknown", code)
    }

    pub fn invalid_input_for(operation: &'static str, code: &'static str) -> Self {
        Self::new(operation, ErrorKind::InvalidInput, code, false)
    }

    pub fn internal(code: &'static str) -> Self {
        Self::new("unknown", ErrorKind::Internal, code, false)
    }

    pub fn output_limit(operation: &'static str, max_bytes: usize) -> Self {
        Self::new(
            operation,
            ErrorKind::OutputLimit,
            "machine_result_too_large",
            false,
        )
        .with_detail(DetailKey::MaxBytes, max_bytes.to_string())
    }

    pub fn from_processkit(
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
            .with_detail(DetailKey::ProcessErrorKind, error.kind().name())
    }

    pub fn with_detail(mut self, key: DetailKey, value: impl Into<String>) -> Self {
        self.details.insert(key, value.into());
        self
    }

    pub fn with_fallback(mut self, fallback: Fallback) -> Self {
        self.fallback = Some(fallback);
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    fn redaction_policy(&self) -> RedactionPolicy {
        RedactionPolicy {
            include_machine_paths: self.include_machine_paths,
        }
    }

    fn redacted_diagnostic(&self) -> String {
        redact_text(self.message, self.redaction_policy())
    }

    pub fn include_machine_paths(mut self, include: bool) -> Self {
        self.include_machine_paths = include;
        self
    }

    #[doc(hidden)]
    pub fn with_message(mut self, message: &'static str) -> Self {
        self.message = message;
        self
    }

    #[cfg(test)]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[cfg(test)]
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.kind.exit_code())
    }
}

pub trait IntoEnvelope {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<String>);
}

impl IntoEnvelope for MachineEnvelope {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<String>) {
        (self, ExitCode::SUCCESS, None)
    }
}

impl IntoEnvelope for AgentError {
    fn into_envelope(self) -> (MachineEnvelope, ExitCode, Option<String>) {
        let exit_code = self.exit_code();
        let diagnostic = Some(self.redacted_diagnostic());
        (MachineEnvelope::failure(&self), exit_code, diagnostic)
    }
}

pub struct RenderedOutput {
    pub stdout: Vec<u8>,
    pub diagnostic: Option<String>,
    pub exit_code: ExitCode,
}

pub fn render(value: impl IntoEnvelope, max_bytes: usize) -> RenderedOutput {
    let (envelope, exit_code, diagnostic) = value.into_envelope();
    let mut stdout = serialize(&envelope);
    if stdout.len() > max_bytes {
        let error = output_limit_error(&envelope, max_bytes);
        let envelope = MachineEnvelope::failure(&error);
        stdout = serialize(&envelope);
        debug_assert!(
            stdout.len() <= crate::cli::MIN_MAX_OUTPUT_BYTES,
            "the bounded output-limit envelope must fit the minimum budget"
        );
        return RenderedOutput {
            stdout,
            diagnostic: Some(error.redacted_diagnostic()),
            exit_code: error.exit_code(),
        };
    }
    RenderedOutput {
        stdout,
        diagnostic,
        exit_code,
    }
}

fn output_limit_error(envelope: &MachineEnvelope, max_bytes: usize) -> AgentError {
    let mut error = AgentError::output_limit(envelope.operation, max_bytes);
    if envelope.operation != "publish" {
        return error;
    }

    error.code = "machine_result_too_large_after_publish";
    let mut checkpoint = None;
    let mut revision = None;
    let mut push_state = None;
    let mut change_request_state = None;
    if let Some(data) = envelope.data.as_ref() {
        checkpoint = bounded_evidence(
            data.get("checkpoint").and_then(serde_json::Value::as_str),
            64,
        );
        revision = bounded_revision(
            data.get("expected_revision")
                .and_then(serde_json::Value::as_str),
        );
        push_state = bounded_state(
            data.get("push")
                .and_then(|push| push.get("state"))
                .and_then(serde_json::Value::as_str),
            &["performed", "already_satisfied", "recovered_after_error"],
        );
        change_request_state = bounded_state(
            data.get("change_request")
                .and_then(|request| request.get("state"))
                .and_then(serde_json::Value::as_str),
            &[
                "created",
                "already_satisfied",
                "discovered_after_push",
                "recovered_after_error",
            ],
        );
    } else if let Some(source) = envelope.error.as_ref() {
        checkpoint = bounded_evidence(source.details.get("checkpoint").map(String::as_str), 64);
        revision = bounded_revision(source.details.get("revision").map(String::as_str));
        push_state = bounded_state(
            source.details.get("push_state").map(String::as_str),
            &["performed", "already_satisfied", "recovered_after_error"],
        );
        change_request_state = bounded_state(
            source
                .details
                .get("change_request_state")
                .map(String::as_str),
            &[
                "created",
                "already_satisfied",
                "discovered_after_push",
                "recovered_after_error",
            ],
        );
    }

    error = error.with_detail(DetailKey::Checkpoint, checkpoint.unwrap_or("preflight"));
    if let Some(value) = revision {
        error = error.with_detail(DetailKey::Revision, value);
    }
    if let Some(value) = push_state {
        error = error.with_detail(DetailKey::PushState, value);
    }
    if let Some(value) = change_request_state {
        error = error.with_detail(DetailKey::ChangeRequestState, value);
    }
    error
}

fn bounded_evidence(value: Option<&str>, max_len: usize) -> Option<&str> {
    value.filter(|value| {
        !value.is_empty()
            && value.len() <= max_len
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn bounded_revision(value: Option<&str>) -> Option<&str> {
    value.filter(|value| {
        (40..=64).contains(&value.len())
            && value.chars().all(|character| character.is_ascii_hexdigit())
    })
}

fn bounded_state<'a>(value: Option<&'a str>, allowed: &[&str]) -> Option<&'a str> {
    value.filter(|value| allowed.contains(value))
}

fn serialize(envelope: &MachineEnvelope) -> Vec<u8> {
    let mut json = serde_json::to_vec_pretty(envelope).expect("machine envelope is serializable");
    json.push(b'\n');
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ExecutionPolicy, execute};
    use crate::cli::{Invocation, Operation};
    use serde_json::{Value, json};

    fn contract_validator() -> jsonschema::Validator {
        let schema: Value = serde_json::from_str(include_str!("../schema/envelope.v1.schema.json"))
            .expect("valid JSON schema");
        assert!(
            jsonschema::meta::is_valid(&schema),
            "the committed contract must itself satisfy its declared meta-schema"
        );
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .expect("Draft 2020-12 contract compiles")
    }

    fn rendered_json(value: impl IntoEnvelope) -> Value {
        serde_json::from_slice(&render(value, crate::cli::DEFAULT_MAX_OUTPUT_BYTES).stdout)
            .expect("rendered envelope is JSON")
    }

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
        assert_eq!(ErrorKind::OutcomeUnknown.exit_code(), 43);
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
    fn publish_output_overflow_preserves_bounded_recovery_evidence() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let success = MachineEnvelope::success(
            "publish",
            json!({
                "expected_revision": revision,
                "checkpoint": "publish_complete",
                "push": {"state": "recovered_after_error"},
                "change_request": {"state": "discovered_after_push"},
                "large": "x".repeat(4096),
            }),
        );
        let output = render(success, crate::cli::MIN_MAX_OUTPUT_BYTES);
        assert_eq!(output.exit_code, ExitCode::from(42));
        assert!(output.stdout.len() <= crate::cli::MIN_MAX_OUTPUT_BYTES);
        let value: Value = serde_json::from_slice(&output.stdout).expect("complete JSON error");
        assert_eq!(
            value["error"]["code"],
            "machine_result_too_large_after_publish"
        );
        assert_eq!(value["error"]["details"]["checkpoint"], "publish_complete");
        assert_eq!(value["error"]["details"]["revision"], revision);
        assert_eq!(
            value["error"]["details"]["push_state"],
            "recovered_after_error"
        );
        assert_eq!(
            value["error"]["details"]["change_request_state"],
            "discovered_after_push"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("xxxx"));
        assert!(contract_validator().is_valid(&value));
    }

    #[test]
    fn every_error_field_runs_through_redaction() {
        let error = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_detail(
                DetailKey::RemoteUrl,
                "HTTPS://user:token@example.invalid/repo",
            )
            .with_detail(DetailKey::Token, "ghp_top-secret")
            .with_detail(DetailKey::RepositoryPath, "/workspaces/alice/repo")
            .with_detail(DetailKey::WorkingDirectory, r"\\server\private\alice\repo")
            .with_warning("Authorization: Bearer abc123 at /workspaces/alice/cache")
            .include_machine_paths(false);
        let output = render(error, crate::cli::DEFAULT_MAX_OUTPUT_BYTES);
        let text = String::from_utf8(output.stdout).expect("UTF-8");
        assert!(!text.contains("top-secret"));
        assert!(!text.contains("user:token"));
        assert!(!text.contains("alice"));
        assert!(!text.contains("server"));
        assert!(!text.contains("workspaces"));
        assert!(!text.contains("abc123"));
        assert!(text.contains("[REDACTED]"));
        assert!(text.contains("[REDACTED_PATH]"));
    }

    #[test]
    fn machine_paths_are_included_only_when_explicitly_requested() {
        let path = "file:///workspaces/alice/repo";
        let hidden = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_detail(DetailKey::RepositoryPath, path)
            .with_message("failed at file:///workspaces/alice/repo token=hidden-secret");
        let visible = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_detail(DetailKey::RepositoryPath, path)
            .with_message("failed at file:///workspaces/alice/repo token=visible-secret")
            .include_machine_paths(true);
        let hidden = render(hidden, 4096);
        let visible = render(visible, 4096);
        let hidden_stdout = String::from_utf8(hidden.stdout).expect("UTF-8");
        let visible_stdout = String::from_utf8(visible.stdout).expect("UTF-8");
        let hidden_diagnostic = hidden.diagnostic.expect("error diagnostic");
        let visible_diagnostic = visible.diagnostic.expect("error diagnostic");

        assert!(!hidden_stdout.contains("alice"));
        assert!(!hidden_diagnostic.contains("alice"));
        assert!(visible_stdout.contains(path));
        assert!(visible_diagnostic.contains(path));
        assert!(!hidden_diagnostic.contains("hidden-secret"));
        assert!(!visible_diagnostic.contains("visible-secret"));
    }

    #[test]
    fn shared_redaction_protects_mixed_machine_stdout_and_stderr() {
        let error = || {
            AgentError::new("inspect", ErrorKind::Backend, "test", false)
                .with_message(
                    r"failed primary=https://alice:stderr-first@example.invalid/repo,mirror=ssh://git@mirror.invalid/repo,backup=custom+ssh://bob:stderr-second@backup.invalid/repo --token stderr-flag-secret github_pat_STDERR_PAT at C:\Users\stderr-user\repo",
                )
                .with_detail(
                    DetailKey::RemoteUrl,
                    r#"{"primary":"https://carol:stdout-first@example.invalid","mirror":"ssh://git@mirror.invalid","backup":"custom://dave:stdout-second@backup.invalid"}"#,
                )
                .with_warning(
                    "--password=stdout-password-secret glpat-STDOUT_PAT /workspaces/stdout-user/repo",
                )
        };

        let hidden = render(error(), crate::cli::DEFAULT_MAX_OUTPUT_BYTES);
        let visible = render(
            error().include_machine_paths(true),
            crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
        );

        for output in [&hidden, &visible] {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let diagnostic = output.diagnostic.as_deref().expect("error diagnostic");
            for leaked in [
                "alice:stderr-first",
                "bob:stderr-second",
                "stderr-flag-secret",
                "github_pat_STDERR_PAT",
                "carol:stdout-first",
                "dave:stdout-second",
                "stdout-password-secret",
                "glpat-STDOUT_PAT",
            ] {
                assert!(!stdout.contains(leaked), "stdout leaked {leaked}: {stdout}");
                assert!(
                    !diagnostic.contains(leaked),
                    "diagnostic leaked {leaked}: {diagnostic}"
                );
            }
            assert!(stdout.contains("ssh://git@mirror.invalid"), "{stdout}");
            assert!(
                diagnostic.contains("ssh://git@mirror.invalid"),
                "{diagnostic}"
            );
        }

        let hidden_stdout = String::from_utf8_lossy(&hidden.stdout);
        let hidden_diagnostic = hidden.diagnostic.as_deref().expect("error diagnostic");
        assert!(!hidden_stdout.contains("stderr-user"));
        assert!(!hidden_stdout.contains("stdout-user"));
        assert!(!hidden_diagnostic.contains("stderr-user"));

        let visible_json: Value =
            serde_json::from_slice(&visible.stdout).expect("visible envelope is JSON");
        let visible_message = visible_json["error"]["message"]
            .as_str()
            .expect("message is a string");
        let visible_warning = visible_json["warnings"][0]
            .as_str()
            .expect("warning is a string");
        let visible_diagnostic = visible.diagnostic.as_deref().expect("error diagnostic");
        assert!(visible_message.contains(r"C:\Users\stderr-user\repo"));
        assert!(visible_warning.contains("/workspaces/stdout-user/repo"));
        assert!(visible_diagnostic.contains(r"C:\Users\stderr-user\repo"));
    }

    #[test]
    fn file_uris_are_redacted_from_message_details_and_warnings() {
        let error = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_message(
                "failed at file:///workspaces/message-secret/repo token=diagnostic-secret",
            )
            .with_detail(
                DetailKey::RemoteUrl,
                "file:///workspaces/remote-secret/repository-secret",
            )
            .with_warning("retry avoided for FILE://server-secret/share-secret/warning")
            .include_machine_paths(false);
        let output = render(error, crate::cli::DEFAULT_MAX_OUTPUT_BYTES);
        let diagnostic = output.diagnostic.as_deref().expect("error diagnostic");
        let text = String::from_utf8(output.stdout).expect("UTF-8");

        for leaked in [
            "workspaces",
            "message-secret",
            "diagnostic-secret",
            "remote-secret",
            "repository-secret",
            "server-secret",
            "share-secret",
        ] {
            assert!(!text.contains(leaked), "stdout leaked {leaked}: {text}");
            assert!(
                !diagnostic.contains(leaked),
                "diagnostic leaked {leaked}: {diagnostic}"
            );
        }
        assert_eq!(text.matches("[REDACTED_PATH]").count(), 3);
        assert!(diagnostic.contains("[REDACTED_PATH]"));
        assert!(diagnostic.contains("[REDACTED]"));
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

    #[tokio::test]
    async fn emitted_envelopes_and_golden_fixtures_validate_against_draft_2020_12() {
        let validator = contract_validator();
        let invocation = Invocation {
            operation: Operation::Probe,
            repository: None,
            changes_mode: crate::cli::ChangesMode::Summary,
            content_max_bytes: crate::cli::DEFAULT_CONTENT_MAX_BYTES,
            max_output_bytes: crate::cli::DEFAULT_MAX_OUTPUT_BYTES,
            include_machine_paths: false,
            write_intent: false,
            expected_revision: None,
            message: None,
            commit_paths: Vec::new(),
            remote: None,
            source: None,
            target: None,
            expected_remote_revision: None,
            forge: None,
            expected_account: None,
            title: None,
            body: None,
            wait_seconds: crate::cli::DEFAULT_WAIT_SECONDS,
            poll_seconds: crate::cli::DEFAULT_POLL_SECONDS,
        };
        let policy = ExecutionPolicy::new(invocation.content_max_bytes);
        let emitted_probe =
            rendered_json(execute(&invocation, &policy).await.expect("probe succeeds"));
        assert!(validator.is_valid(&emitted_probe));

        for kind in [
            ErrorKind::Unsupported,
            ErrorKind::Denied,
            ErrorKind::InvalidInput,
            ErrorKind::Backend,
            ErrorKind::Forge,
            ErrorKind::Authentication,
            ErrorKind::Timeout,
            ErrorKind::Cancelled,
            ErrorKind::OutputLimit,
            ErrorKind::OutcomeUnknown,
            ErrorKind::ExternalCommand,
            ErrorKind::Internal,
        ] {
            let emitted_error =
                rendered_json(AgentError::new("probe", kind, "schema_regression", false));
            assert!(
                validator.is_valid(&emitted_error),
                "emitted {kind:?} envelope must satisfy the contract"
            );
        }

        for fixture in [
            include_str!("../tests/fixtures/probe-success.v1.json"),
            include_str!("../tests/fixtures/invalid-input.v1.json"),
            include_str!("../tests/fixtures/future-operation-success.v1.json"),
            include_str!("../tests/fixtures/inspect-success-git.v1.json"),
            include_str!("../tests/fixtures/inspect-no-remote-git.v1.json"),
            include_str!("../tests/fixtures/changes-summary-git.v1.json"),
            include_str!("../tests/fixtures/changes-full-jj.v1.json"),
            include_str!("../tests/fixtures/changes-output-limit.v1.json"),
            include_str!("../tests/fixtures/commit-success-git.v1.json"),
            include_str!("../tests/fixtures/publish-success-git.v1.json"),
            include_str!("../tests/fixtures/publish-success-retry-git.v1.json"),
            include_str!("../tests/fixtures/publish-success-discovered-git.v1.json"),
            include_str!("../tests/fixtures/publish-success-recovered-git.v1.json"),
            include_str!("../tests/fixtures/ci-status-success-github.v1.json"),
            include_str!("../tests/fixtures/ci-wait-success-github.v1.json"),
        ] {
            let fixture: Value = serde_json::from_str(fixture).expect("golden fixture is JSON");
            assert!(
                validator.is_valid(&fixture),
                "every committed fixture must satisfy the executable schema"
            );
        }
    }

    #[test]
    fn schema_rejects_broken_success_error_and_version_invariants() {
        let validator = contract_validator();
        let success: Value =
            serde_json::from_str(include_str!("../tests/fixtures/probe-success.v1.json"))
                .expect("success fixture is JSON");
        let error: Value =
            serde_json::from_str(include_str!("../tests/fixtures/invalid-input.v1.json"))
                .expect("error fixture is JSON");

        let mut success_without_data = success.clone();
        success_without_data["data"] = Value::Null;
        assert!(!validator.is_valid(&success_without_data));

        let mut success_with_error = success.clone();
        success_with_error["error"] = error["error"].clone();
        assert!(!validator.is_valid(&success_with_error));

        let mut error_with_data = error.clone();
        error_with_data["data"] = success["data"].clone();
        assert!(!validator.is_valid(&error_with_data));

        let mut error_without_error = error.clone();
        error_without_error["error"] = Value::Null;
        assert!(!validator.is_valid(&error_without_error));

        let inspect: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/inspect-success-git.v1.json"
        ))
        .expect("inspect fixture is JSON");
        let mut dishonest_snapshot = inspect.clone();
        dishonest_snapshot["data"]["read_semantics"]["operation_log_may_advance"] =
            Value::Bool(true);
        assert!(!validator.is_valid(&dishonest_snapshot));

        let mut unavailable_without_reason = inspect;
        unavailable_without_reason["data"]["forge"]["capabilities"] = json!({
            "status": "unavailable",
            "reason": null,
            "value": null
        });
        assert!(!validator.is_valid(&unavailable_without_reason));

        let commit: Value =
            serde_json::from_str(include_str!("../tests/fixtures/commit-success-git.v1.json"))
                .expect("commit fixture is JSON");
        let mut commit_without_paths = commit.clone();
        commit_without_paths["data"]["included_paths"] = json!([]);
        assert!(!validator.is_valid(&commit_without_paths));

        let mut commit_claims_push = commit.clone();
        commit_claims_push["data"]["semantics"]["push_performed"] = Value::Bool(true);
        assert!(!validator.is_valid(&commit_claims_push));

        let mut commit_hides_unrelated_loss = commit;
        commit_hides_unrelated_loss["data"]["unrelated_changes_preserved"] = Value::Bool(false);
        assert!(!validator.is_valid(&commit_hides_unrelated_loss));

        let publish: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/publish-success-git.v1.json"
        ))
        .expect("publish fixture is JSON");
        let mut publish_without_postflight = publish;
        publish_without_postflight["data"]["push"]["verified"] = json!(false);
        assert!(!validator.is_valid(&publish_without_postflight));

        let ci: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/ci-status-success-github.v1.json"
        ))
        .expect("CI fixture is JSON");
        let mut successful_but_pending = ci;
        successful_but_pending["data"]["terminal"] = json!(false);
        assert!(!validator.is_valid(&successful_but_pending));

        let mut wrong_version = success.clone();
        wrong_version["contract_version"] = json!("vcs-agent/v2");
        assert!(!validator.is_valid(&wrong_version));

        let mut wrong_field_type = error;
        wrong_field_type["warnings"] = json!("not-an-array");
        assert!(!validator.is_valid(&wrong_field_type));

        let changes: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/changes-summary-git.v1.json"
        ))
        .expect("changes fixture is JSON");
        let mut summary_with_full_diff = changes.clone();
        summary_with_full_diff["data"]["diff"] = json!([]);
        assert!(!validator.is_valid(&summary_with_full_diff));

        let inspect: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/inspect-success-git.v1.json"
        ))
        .expect("inspect fixture is JSON");
        let mut lying_read_semantics = inspect;
        lying_read_semantics["data"]["read_semantics"]["refs_mutated"] = json!(true);
        assert!(!validator.is_valid(&lying_read_semantics));

        let inspect: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/inspect-success-git.v1.json"
        ))
        .expect("inspect fixture is JSON");
        let mut missing_working_copy_field = inspect.clone();
        missing_working_copy_field["data"]["working_copy"]
            .as_object_mut()
            .expect("working_copy object")
            .remove("revision");
        assert!(!validator.is_valid(&missing_working_copy_field));

        let mut invalid_forge_capability = inspect.clone();
        invalid_forge_capability["data"]["forge"]["capabilities"]["value"]["cli_supported"] =
            json!("yes");
        assert!(!validator.is_valid(&invalid_forge_capability));

        let mut invalid_forge_auth = inspect;
        invalid_forge_auth["data"]["forge"]["auth"]["value"]["accounts"] = json!([{
            "host": 42,
            "login": "agent",
            "active": true
        }]);
        assert!(!validator.is_valid(&invalid_forge_auth));

        let changes: Value =
            serde_json::from_str(include_str!("../tests/fixtures/changes-full-jj.v1.json"))
                .expect("full changes fixture is JSON");
        let mut missing_count = changes.clone();
        missing_count["data"]["counts"]
            .as_object_mut()
            .expect("counts object")
            .remove("insertions");
        assert!(!validator.is_valid(&missing_count));

        let mut missing_changed_path_field = changes.clone();
        missing_changed_path_field["data"]["files"][0]
            .as_object_mut()
            .expect("changed path object")
            .remove("old_path");
        assert!(!validator.is_valid(&missing_changed_path_field));

        let mut missing_hunk_field = changes.clone();
        missing_hunk_field["data"]["diff"][0]["hunks"][0]
            .as_object_mut()
            .expect("hunk object")
            .remove("old_start");
        assert!(!validator.is_valid(&missing_hunk_field));

        let mut invalid_diff_line = changes;
        invalid_diff_line["data"]["diff"][0]["hunks"][0]["lines"][0]["text"] = json!(7);
        assert!(!validator.is_valid(&invalid_diff_line));
    }
}
