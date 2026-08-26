//! `vcs-agent`: the bounded, outcome-oriented vcs-toolkit application facade.
//!
//! The v1 surface implements `probe`, `inspect`, `changes`, and checked
//! exact-path `commit` through the typed vcs-core/vcs-forge clients.
//! [`app::ExecutionPolicy`] carries ProcessKit cancellation, deadline, and
//! fail-loud content limits into every backend. This binary never constructs
//! raw git/jj/forge child processes.

use std::ffi::OsString;
use std::io;
use std::process::ExitCode;

use vcs_agent::OutcomeServices;
use vcs_agent::app::ExecutionPolicy;
use vcs_agent::cli::{self, Invocation, ParseResult, USAGE};
use vcs_agent::contract::{AgentError, RenderedOutput, render};

fn main() -> ExitCode {
    run(
        std::env::args_os().skip(1),
        &mut io::stdout(),
        &mut io::stderr(),
    )
}

fn run(
    args: impl Iterator<Item = OsString>,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cli::parse(args) {
        Ok(ParseResult::Help) => write_text(stdout, USAGE, stderr),
        Ok(ParseResult::Version) => write_text(
            stdout,
            concat!("vcs-agent ", env!("CARGO_PKG_VERSION"), "\n"),
            stderr,
        ),
        Ok(ParseResult::Run(invocation)) => run_invocation(*invocation, stdout, stderr),
        Err(error) => write_machine(
            render(*error, cli::DEFAULT_MAX_OUTPUT_BYTES),
            stdout,
            stderr,
        ),
    }
}

fn run_invocation(
    invocation: Invocation,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    let policy = ExecutionPolicy::new(invocation.content_max_bytes).with_deadline(
        if invocation.operation == cli::Operation::CiWait {
            std::time::Duration::from_secs(invocation.wait_seconds)
        } else {
            std::time::Duration::from_secs(120)
        },
    );
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            return write_machine(
                render(
                    AgentError::internal("async_runtime_initialization_failed"),
                    invocation.max_output_bytes,
                ),
                stdout,
                stderr,
            );
        }
    };
    let output = runtime.block_on(OutcomeServices::execute(&invocation, &policy));
    write_machine(output, stdout, stderr)
}

fn write_machine(
    output: RenderedOutput,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    if stdout.write_all(&output.stdout).is_err() || stdout.flush().is_err() {
        let _ = writeln!(
            stderr,
            "vcs-agent: could not write the machine result to stdout"
        );
        return AgentError::internal("stdout_write_failed").exit_code();
    }
    if let Some(diagnostic) = output.diagnostic {
        let _ = writeln!(stderr, "vcs-agent: {diagnostic}");
    }
    output.exit_code
}

fn write_text(stdout: &mut impl io::Write, text: &str, stderr: &mut impl io::Write) -> ExitCode {
    if stdout.write_all(text.as_bytes()).is_ok() && stdout.flush().is_ok() {
        ExitCode::SUCCESS
    } else {
        let _ = writeln!(stderr, "vcs-agent: could not write to stdout");
        AgentError::internal("stdout_write_failed").exit_code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use vcs_agent::contract::{DetailKey, ErrorKind};

    fn call(args: &[&str]) -> (ExitCode, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run(args.iter().map(OsString::from), &mut stdout, &mut stderr);
        (
            exit,
            String::from_utf8(stdout).expect("stdout is UTF-8"),
            String::from_utf8(stderr).expect("stderr is UTF-8"),
        )
    }

    #[test]
    fn probe_writes_only_machine_json_to_stdout() {
        let (exit, stdout, stderr) = call(&["probe"]);
        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(stderr.is_empty());
        let actual: Value = serde_json::from_str(&stdout).expect("valid result JSON");
        let expected: Value =
            serde_json::from_str(include_str!("../tests/fixtures/probe-success.v1.json"))
                .expect("valid golden JSON");
        assert_eq!(actual, expected);
    }

    #[test]
    fn invalid_input_is_structured_on_stdout_and_diagnostic_on_stderr() {
        let (exit, stdout, stderr) = call(&["probe", "--max-output-bytes", "nope"]);
        assert_eq!(exit, ExitCode::from(2));
        let actual: Value = serde_json::from_str(&stdout).expect("valid error JSON");
        let expected: Value =
            serde_json::from_str(include_str!("../tests/fixtures/invalid-input.v1.json"))
                .expect("valid golden JSON");
        assert_eq!(actual, expected);
        assert_eq!(stderr, "vcs-agent: invalid input\n");
        assert!(!stderr.contains("nope"));
    }

    #[test]
    fn help_and_version_do_not_mix_diagnostics_into_stdout() {
        let (help_exit, help, help_err) = call(&["--help"]);
        assert_eq!(help_exit, ExitCode::SUCCESS);
        assert!(help.contains("vcs-agent probe"));
        assert!(help_err.is_empty());

        let (version_exit, version, version_err) = call(&["--version"]);
        assert_eq!(version_exit, ExitCode::SUCCESS);
        assert_eq!(
            version,
            concat!("vcs-agent ", env!("CARGO_PKG_VERSION"), "\n")
        );
        assert!(version_err.is_empty());
    }

    #[test]
    fn publish_and_ci_reject_incomplete_requests_without_echoing_values() {
        for (operation, args) in [
            ("publish", &["publish", "origin", "agent-secret"][..]),
            (
                "ci_status",
                &["ci", "status", "--provider=agent-secret"][..],
            ),
            ("ci_wait", &["ci", "wait", "agent-secret"][..]),
        ] {
            let (exit, stdout, stderr) = call(args);
            assert_eq!(exit, ExitCode::from(2), "{operation}");
            assert!(!stdout.contains("agent-secret"), "{operation}: {stdout}");
            let value: Value = serde_json::from_str(&stdout).expect("valid error JSON");
            assert_eq!(value["operation"], operation);
            assert_eq!(value["error"]["kind"], "invalid_input");
            assert_eq!(value["error"]["exit_code"], 2);
            assert_ne!(value["error"]["code"], "outcome_not_implemented");
            assert!(value["fallback"].is_null());
            assert_eq!(stderr, "vcs-agent: invalid input\n");
            assert!(!stderr.contains("agent-secret"));
        }
    }

    #[test]
    fn arbitrary_unknown_command_is_not_reflected_into_output() {
        let (exit, stdout, _stderr) = call(&["https://user:secret@example.invalid"]);
        assert_eq!(exit, ExitCode::from(10));
        assert!(!stdout.contains("secret"));
        let value: Value = serde_json::from_str(&stdout).expect("valid error JSON");
        assert_eq!(value["operation"], "unknown");
    }

    #[test]
    fn write_machine_redacts_diagnostics_and_machine_stdout() {
        let error = AgentError::new("probe", ErrorKind::Backend, "test", false)
            .with_message("failed at file:///workspaces/stderr-path/repo token=stderr-secret")
            .with_detail(
                DetailKey::RemoteUrl,
                "file:///workspaces/stdout-path/repository",
            );
        let output = render(error, cli::DEFAULT_MAX_OUTPUT_BYTES);
        let diagnostic = output.diagnostic.as_deref().expect("error diagnostic");
        assert!(!diagnostic.contains("stderr-path"));
        assert!(!diagnostic.contains("stderr-secret"));

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = write_machine(output, &mut stdout, &mut stderr);
        assert_eq!(exit, ExitCode::from(30));

        let stdout = String::from_utf8(stdout).expect("stdout is UTF-8");
        let stderr = String::from_utf8(stderr).expect("stderr is UTF-8");
        for leaked in ["workspaces", "stderr-path", "stderr-secret", "stdout-path"] {
            assert!(!stdout.contains(leaked), "stdout leaked {leaked}: {stdout}");
            assert!(!stderr.contains(leaked), "stderr leaked {leaked}: {stderr}");
        }
        assert!(stdout.contains("[REDACTED_PATH]"));
        assert!(stderr.contains("[REDACTED_PATH]"));
        assert!(stderr.contains("[REDACTED]"));
    }

    #[test]
    fn production_source_has_no_raw_subprocess_constructor() {
        let sources = [
            include_str!("main.rs"),
            include_str!("app.rs"),
            include_str!("cli.rs"),
            include_str!("contract.rs"),
            include_str!("redaction.rs"),
        ];
        let qualified = ["std::process::", "Command"].concat();
        let constructor = ["Command", "::new("].concat();
        for source in sources {
            assert!(!source.contains(&qualified));
            assert!(!source.contains(&constructor));
        }
    }
}
