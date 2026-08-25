use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::contract::{AgentError, AgentResult, ErrorKind, Fallback};

pub(crate) const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub(crate) const MIN_MAX_OUTPUT_BYTES: usize = 1024;
pub(crate) const MAX_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub(crate) const DEFAULT_CONTENT_MAX_BYTES: usize = 256 * 1024;

pub(crate) const USAGE: &str = "\
vcs-agent — bounded, outcome-oriented repository operations for agents.\n\
\n\
USAGE:\n\
    vcs-agent probe [OPTIONS]\n\
    vcs-agent inspect --repo <PATH> [OPTIONS]\n\
    vcs-agent changes --repo <PATH> [--mode summary|full] [OPTIONS]\n\
\n\
OUTCOMES:\n\
    probe       Report contract, compatibility, capabilities, and exit semantics.\n\
    inspect     Inspect repository, working-copy, remote, forge, auth, and capability facts.\n\
    changes     Report changed paths and counts; full mode also returns structured hunks.\n\
\n\
    The v1 taxonomy also reserves commit, publish, ci status, and ci wait. Until\n\
    implemented, they fail with a structured unsupported result; vcs-agent never\n\
    falls through to a raw VCS or forge command.\n\
\n\
OPTIONS:\n\
    --repo <PATH>             Repository path. Required by inspect and changes.\n\
    --mode <summary|full>     Changes detail (default: summary).\n\
    --content-max-bytes <n>   Fail if captured diff content exceeds n bytes\n\
                              (1024..=1048576; default: 262144).\n\
    --max-output-bytes <n>    Fail before emitting a result larger than n bytes\n\
                              (1024..=1048576; default: 65536).\n\
    --include-machine-paths   Include operation-required local paths. Without it,\n\
                              path values are replaced by redacted path objects.\n\
    -h, --help                Print this help.\n\
    -V, --version             Print the binary version.\n\
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
        matches!(self, Self::Probe | Self::Inspect | Self::Changes)
    }

    fn parse_leading(args: &[OsString]) -> Option<(Self, usize)> {
        let value = args.first()?.to_str()?;
        match value {
            "probe" => Some((Self::Probe, 1)),
            "inspect" => Some((Self::Inspect, 1)),
            "changes" => Some((Self::Changes, 1)),
            "commit" => Some((Self::Commit, 1)),
            "publish" => Some((Self::Publish, 1)),
            "ci" if args.get(1).and_then(|value| value.to_str()) == Some("status") => {
                Some((Self::CiStatus, 2))
            }
            "ci" if args.get(1).and_then(|value| value.to_str()) == Some("wait") => {
                Some((Self::CiWait, 2))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangesMode {
    Summary,
    Full,
}

#[derive(Clone, Debug)]
pub(crate) struct Invocation {
    pub(crate) operation: Operation,
    pub(crate) repository: Option<PathBuf>,
    pub(crate) changes_mode: ChangesMode,
    pub(crate) content_max_bytes: usize,
    pub(crate) max_output_bytes: usize,
    pub(crate) include_machine_paths: bool,
}

pub(crate) enum ParseResult {
    Help,
    Version,
    Run(Invocation),
}

pub(crate) fn parse(args: impl Iterator<Item = OsString>) -> AgentResult<ParseResult> {
    let args = args.collect::<Vec<_>>();
    if args.len() == 1 && matches!(args[0].to_str(), Some("-h" | "--help")) {
        return Ok(ParseResult::Help);
    }
    if args.len() == 1 && matches!(args[0].to_str(), Some("-V" | "--version")) {
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

    let mut repository = None;
    let mut changes_mode = ChangesMode::Summary;
    let mut mode_seen = false;
    let mut content_max_bytes = DEFAULT_CONTENT_MAX_BYTES;
    let mut content_budget_seen = false;
    let mut max_output_bytes = DEFAULT_MAX_OUTPUT_BYTES;
    let mut machine_budget_seen = false;
    let mut include_machine_paths = false;
    let mut index = command_len;
    while index < args.len() {
        let Some(option) = args[index].to_str() else {
            return Err(Box::new(AgentError::invalid_input_for(
                operation.name(),
                "option_not_utf8",
            )));
        };
        match option {
            "--repo" | "--repository" if operation != Operation::Probe => {
                if repository.is_some() {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "repository_option_repeated",
                    )));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "repository_value_required",
                    )));
                };
                if value.as_os_str() == OsStr::new("") {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "repository_path_empty",
                    )));
                }
                repository = Some(PathBuf::from(value));
                index += 2;
            }
            "--mode" if operation == Operation::Changes => {
                if mode_seen {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "mode_option_repeated",
                    )));
                }
                let value = utf8_value(&args, index + 1, operation, "mode_value_required")?;
                changes_mode = match value {
                    "summary" => ChangesMode::Summary,
                    "full" => ChangesMode::Full,
                    _ => {
                        return Err(Box::new(AgentError::invalid_input_for(
                            operation.name(),
                            "mode_must_be_summary_or_full",
                        )));
                    }
                };
                mode_seen = true;
                index += 2;
            }
            "--content-max-bytes" if operation == Operation::Changes => {
                if content_budget_seen {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "content_max_bytes_option_repeated",
                    )));
                }
                content_max_bytes = parse_budget(&args, index + 1, operation, "content_max_bytes")?;
                content_budget_seen = true;
                index += 2;
            }
            "--max-output-bytes" => {
                if machine_budget_seen {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "max_output_bytes_option_repeated",
                    )));
                }
                max_output_bytes = parse_budget(&args, index + 1, operation, "max_output_bytes")?;
                machine_budget_seen = true;
                index += 2;
            }
            "--include-machine-paths" => {
                if include_machine_paths {
                    return Err(Box::new(AgentError::invalid_input_for(
                        operation.name(),
                        "include_machine_paths_option_repeated",
                    )));
                }
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

    if matches!(operation, Operation::Inspect | Operation::Changes) && repository.is_none() {
        return Err(Box::new(AgentError::invalid_input_for(
            operation.name(),
            "repository_required",
        )));
    }

    Ok(ParseResult::Run(Invocation {
        operation,
        repository,
        changes_mode,
        content_max_bytes,
        max_output_bytes,
        include_machine_paths,
    }))
}

fn utf8_value<'a>(
    args: &'a [OsString],
    index: usize,
    operation: Operation,
    missing_code: &'static str,
) -> AgentResult<&'a str> {
    let Some(value) = args.get(index) else {
        return Err(Box::new(AgentError::invalid_input_for(
            operation.name(),
            missing_code,
        )));
    };
    value.to_str().ok_or_else(|| {
        Box::new(AgentError::invalid_input_for(
            operation.name(),
            "option_value_not_utf8",
        ))
    })
}

fn parse_budget(
    args: &[OsString],
    index: usize,
    operation: Operation,
    label: &'static str,
) -> AgentResult<usize> {
    let value = utf8_value(
        args,
        index,
        operation,
        if label == "max_output_bytes" {
            "max_output_bytes_value_required"
        } else {
            "content_max_bytes_value_required"
        },
    )?;
    let value = value.parse::<usize>().map_err(|_| {
        AgentError::invalid_input_for(
            operation.name(),
            if label == "max_output_bytes" {
                "max_output_bytes_must_be_integer"
            } else {
                "content_max_bytes_must_be_integer"
            },
        )
    })?;
    if !(MIN_MAX_OUTPUT_BYTES..=MAX_MAX_OUTPUT_BYTES).contains(&value) {
        return Err(Box::new(AgentError::invalid_input_for(
            operation.name(),
            if label == "max_output_bytes" {
                "max_output_bytes_out_of_range"
            } else {
                "content_max_bytes_out_of_range"
            },
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strings(args: &[&str]) -> AgentResult<ParseResult> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn parser_accepts_each_implemented_outcome() {
        let ParseResult::Run(probe) = parse_strings(&[
            "probe",
            "--max-output-bytes",
            "4096",
            "--include-machine-paths",
        ])
        .expect("valid probe") else {
            panic!("expected runnable invocation");
        };
        assert_eq!(probe.operation, Operation::Probe);
        assert_eq!(probe.max_output_bytes, 4096);

        let ParseResult::Run(inspect) =
            parse_strings(&["inspect", "--repo", "repo"]).expect("valid inspect")
        else {
            panic!("expected runnable invocation");
        };
        assert_eq!(inspect.repository, Some(PathBuf::from("repo")));

        let ParseResult::Run(changes) = parse_strings(&[
            "changes",
            "--repository",
            "repo",
            "--mode",
            "full",
            "--content-max-bytes",
            "8192",
        ])
        .expect("valid changes") else {
            panic!("expected runnable invocation");
        };
        assert_eq!(changes.changes_mode, ChangesMode::Full);
        assert_eq!(changes.content_max_bytes, 8192);
    }

    #[test]
    fn parser_rejects_missing_duplicate_and_out_of_range_values() {
        for args in [
            vec!["inspect"],
            vec!["changes", "--repo"],
            vec!["changes", "--repo", "repo", "--mode", "compact"],
            vec!["probe", "--max-output-bytes", "0"],
            vec!["probe", "--max-output-bytes", "1048577"],
            vec![
                "probe",
                "--max-output-bytes",
                "4096",
                "--max-output-bytes",
                "4096",
            ],
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
        let status = [OsString::from("ci"), OsString::from("status")];
        let wait = [OsString::from("ci"), OsString::from("wait")];
        assert_eq!(
            Operation::parse_leading(&status),
            Some((Operation::CiStatus, 2))
        );
        assert_eq!(
            Operation::parse_leading(&wait),
            Some((Operation::CiWait, 2))
        );
    }

    #[cfg(unix)]
    #[test]
    fn repository_argument_preserves_non_utf8_os_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let raw = b"/tmp/repo-\xff".to_vec();
        let ParseResult::Run(invocation) = parse(
            [
                OsString::from("inspect"),
                OsString::from("--repo"),
                OsString::from_vec(raw.clone()),
            ]
            .into_iter(),
        )
        .expect("non-UTF-8 repository path is a valid OS path") else {
            panic!("expected runnable invocation");
        };
        assert_eq!(invocation.repository.unwrap().as_os_str().as_bytes(), raw);
    }

    #[cfg(windows)]
    #[test]
    fn repository_argument_preserves_non_unicode_utf16_units() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let raw = [0x0043, 0x003a, 0x005c, 0xd800];
        let ParseResult::Run(invocation) = parse(
            [
                OsString::from("inspect"),
                OsString::from("--repo"),
                OsString::from_wide(&raw),
            ]
            .into_iter(),
        )
        .expect("non-Unicode repository path is a valid Windows OS path") else {
            panic!("expected runnable invocation");
        };
        assert_eq!(
            invocation
                .repository
                .unwrap()
                .as_os_str()
                .encode_wide()
                .collect::<Vec<_>>(),
            raw
        );
    }
}
