use std::ffi::OsString;

use crate::contract::{AgentError, AgentResult, ErrorKind, Fallback};

pub(crate) const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub(crate) const MIN_MAX_OUTPUT_BYTES: usize = 1024;
pub(crate) const MAX_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

pub(crate) const USAGE: &str = "\
vcs-agent — bounded, outcome-oriented repository operations for agents.\n\
\n\
USAGE:\n\
    vcs-agent probe [OPTIONS]\n\
    vcs-agent <OUTCOME> [OPTIONS]\n\
\n\
OUTCOMES:\n\
    probe       Report contract, compatibility, capabilities, and exit semantics.\n\
\n\
    The v1 taxonomy also reserves inspect, changes, commit, publish, ci status,\n\
    and ci wait. Until implemented, they fail with a structured unsupported\n\
    result; vcs-agent never falls through to a raw VCS/forge command.\n\
\n\
OPTIONS:\n\
    --max-output-bytes <n>   Fail loudly before emitting a result larger than n\n\
                             bytes (1024..=1048576; default: 65536).\n\
    --include-machine-paths  Permit operation-required local paths in results.\n\
                             Probe currently emits none.\n\
    -h, --help               Print this help.\n\
    -V, --version            Print the binary version.\n\
\n\
Machine results are complete JSON documents on stdout. Diagnostics are written\n\
only to stderr. Content is refused, never truncated into valid-looking JSON.\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Probe,
    Inspect,
    Changes,
    Commit,
    Publish,
    CiStatus,
    CiWait,
}

impl Operation {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Inspect => "inspect",
            Self::Changes => "changes",
            Self::Commit => "commit",
            Self::Publish => "publish",
            Self::CiStatus => "ci_status",
            Self::CiWait => "ci_wait",
        }
    }

    const fn implemented(self) -> bool {
        matches!(self, Self::Probe)
    }

    fn parse_leading(args: &[String]) -> Option<(Self, usize)> {
        let value = args.first()?.as_str();
        match value {
            "probe" => Some((Self::Probe, 1)),
            "inspect" => Some((Self::Inspect, 1)),
            "changes" => Some((Self::Changes, 1)),
            "commit" => Some((Self::Commit, 1)),
            "publish" => Some((Self::Publish, 1)),
            "ci" if args.get(1).is_some_and(|value| value == "status") => Some((Self::CiStatus, 2)),
            "ci" if args.get(1).is_some_and(|value| value == "wait") => Some((Self::CiWait, 2)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Invocation {
    pub(crate) operation: Operation,
    pub(crate) max_output_bytes: usize,
    pub(crate) include_machine_paths: bool,
}

pub(crate) enum ParseResult {
    Help,
    Version,
    Run(Invocation),
}

pub(crate) fn parse(args: impl Iterator<Item = OsString>) -> AgentResult<ParseResult> {
    let args = args
        .map(|arg| {
            arg.into_string()
                .map_err(|_| Box::new(AgentError::invalid_input("argument_not_utf8")))
        })
        .collect::<AgentResult<Vec<_>>>()?;

    if args.len() == 1 && matches!(args[0].as_str(), "-h" | "--help") {
        return Ok(ParseResult::Help);
    }
    if args.len() == 1 && matches!(args[0].as_str(), "-V" | "--version") {
        return Ok(ParseResult::Version);
    }

    if args.is_empty() {
        return Err(Box::new(AgentError::invalid_input("operation_required")));
    }
    let Some((operation, command_len)) = Operation::parse_leading(&args) else {
        return Err(Box::new(
            AgentError::new(
                "unknown",
                ErrorKind::Unsupported,
                "unsupported_outcome",
                false,
            )
            .with_fallback(Fallback::raw_cli("unknown_outcome")),
        ));
    };
    if !operation.implemented() {
        if args.len() != command_len {
            return Err(Box::new(AgentError::invalid_input_for(
                operation.name(),
                "unsupported_outcome_has_arguments",
            )));
        }
        return Err(Box::new(
            AgentError::new(
                operation.name(),
                ErrorKind::Unsupported,
                "outcome_not_implemented",
                false,
            )
            .with_fallback(Fallback::raw_cli("outcome_not_implemented")),
        ));
    }

    let mut max_output_bytes = DEFAULT_MAX_OUTPUT_BYTES;
    let mut include_machine_paths = false;
    let mut index = command_len;
    while index < args.len() {
        match args[index].as_str() {
            "--max-output-bytes" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "max_output_bytes_value_required",
                    )));
                };
                max_output_bytes = value.parse::<usize>().map_err(|_| {
                    AgentError::invalid_input_for(
                        operation.name(),
                        "max_output_bytes_must_be_integer",
                    )
                })?;
                if !(MIN_MAX_OUTPUT_BYTES..=MAX_MAX_OUTPUT_BYTES).contains(&max_output_bytes) {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "max_output_bytes_out_of_range",
                    )));
                }
                index += 2;
            }
            "--include-machine-paths" => {
                include_machine_paths = true;
                index += 1;
            }
            "-h" | "--help" => return Ok(ParseResult::Help),
            _ => {
                return Err(Box::new(AgentError::invalid_input_for(
                    operation.name(),
                    "unknown_option",
                )));
            }
        }
    }

    Ok(ParseResult::Run(Invocation {
        operation,
        max_output_bytes,
        include_machine_paths,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strings(args: &[&str]) -> AgentResult<ParseResult> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn parser_accepts_probe_options_in_a_deterministic_shape() {
        let ParseResult::Run(invocation) = parse_strings(&[
            "probe",
            "--max-output-bytes",
            "4096",
            "--include-machine-paths",
        ])
        .expect("valid invocation") else {
            panic!("expected runnable invocation");
        };
        assert_eq!(invocation.operation, Operation::Probe);
        assert_eq!(invocation.max_output_bytes, 4096);
        assert!(invocation.include_machine_paths);
    }

    #[test]
    fn parser_rejects_missing_duplicate_and_out_of_range_values() {
        for args in [
            vec!["probe", "--max-output-bytes"],
            vec!["probe", "--max-output-bytes", "0"],
            vec!["probe", "--max-output-bytes", "1048577"],
            vec!["probe", "extra"],
        ] {
            let err = match parse_strings(&args) {
                Err(err) => err,
                Ok(_) => panic!("{args:?} must fail"),
            };
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn nested_ci_taxonomy_matches_the_committed_roadmap() {
        let status = ["ci".to_owned(), "status".to_owned()];
        let wait = ["ci".to_owned(), "wait".to_owned()];
        assert_eq!(
            Operation::parse_leading(&status),
            Some((Operation::CiStatus, 2))
        );
        assert_eq!(
            Operation::parse_leading(&wait),
            Some((Operation::CiWait, 2))
        );
        assert_eq!(Operation::CiStatus.name(), "ci_status");
        assert_eq!(Operation::CiWait.name(), "ci_wait");
    }
}
