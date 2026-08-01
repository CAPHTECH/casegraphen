use crate::native_cli::{
    NativeCliCommand, NativeCliError, NativeCommandResult, NativeOutputFormat,
};
#[path = "cli_error.rs"]
mod cli_error;
#[path = "cli_required.rs"]
mod cli_required;

use crate::exec::worker::CliExitCode;
use cli_error::CliError;
use cli_required::required_segment;
use std::{env, ffi::OsString, fs, path::PathBuf};

const USAGE: &str = include_str!("cli_usage.txt");

pub fn main_entry() -> CliExitCode {
    match run_with_outcome(env::args_os().skip(1)) {
        Ok(result) => exit_with_code(result.outcome, result.strict),
        Err(error) => {
            eprintln!("{error}");
            exit_with_code(CliOutcome::ToolFailure, false)
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    run_with_outcome(args).map(|_| ())
}

fn run_with_outcome(args: impl IntoIterator<Item = OsString>) -> Result<SuccessfulRun, CliError> {
    let command = Command::parse(args)?;
    let strict = command.strict();
    let format = command.format();
    let result = command.run_rendered()?;
    let (rendered, domain_finding) = result.into_parts();
    match command.output() {
        Some(path) => {
            let text = match format {
                NativeOutputFormat::Json => {
                    let value = serde_json::from_str::<serde_json::Value>(&rendered)?;
                    serde_json::to_string_pretty(&value)?
                }
                NativeOutputFormat::Text => rendered,
            };
            fs::write(path, format!("{text}\n")).map_err(|source| {
                CliError::from(NativeCliError::Io {
                    path: path.clone(),
                    source,
                })
            })?;
        }
        None => {
            println!("{rendered}");
        }
    }
    Ok(SuccessfulRun {
        outcome: if domain_finding {
            CliOutcome::DomainFinding
        } else {
            CliOutcome::Success
        },
        strict,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CliOutcome {
    Success,
    DomainFinding,
    ToolFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SuccessfulRun {
    outcome: CliOutcome,
    strict: bool,
}

pub(crate) fn exit_with_code(outcome: CliOutcome, strict: bool) -> CliExitCode {
    match outcome {
        CliOutcome::ToolFailure => CliExitCode::FAILURE,
        CliOutcome::DomainFinding if strict => CliExitCode::from(2_u8),
        CliOutcome::Success | CliOutcome::DomainFinding => CliExitCode::SUCCESS,
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Version,
    Native(Box<NativeCliCommand>),
}

impl Command {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args.into_iter();
        match required_segment(&mut args, "command")?.to_str() {
            Some("version") | Some("--version") | Some("-V") => Ok(Self::Version),
            Some(
                segment @ ("lift" | "space" | "obstruction" | "completion" | "projection"
                | "equivalence" | "invariant" | "morphism" | "plan" | "binding" | "run"
                | "review" | "evidence" | "cell" | "packet"),
            ) => NativeCliCommand::parse(segment, args)
                .map(|command| Self::Native(Box::new(command)))
                .map_err(CliError::from),
            Some("workflow") | Some("cg") => Err(CliError::usage(
                "the workflow evaluator surface was removed (ADR 0003); lift the graph with \
                 `lift workflow` and use the native derived commands",
            )),
            Some(_) | None => Err(CliError::usage("unsupported command segment")),
        }
    }

    fn output(&self) -> Option<&PathBuf> {
        match self {
            Self::Version => None,
            Self::Native(command) => command.output(),
        }
    }

    fn strict(&self) -> bool {
        match self {
            Self::Version => false,
            Self::Native(command) => command.strict(),
        }
    }

    fn run_rendered(&self) -> Result<NativeCommandResult<String>, CliError> {
        match self {
            Self::Version => Ok(NativeCommandResult::success(format!(
                "casegraphen {}",
                env!("CARGO_PKG_VERSION")
            ))),
            Self::Native(command) => command.run_rendered().map_err(CliError::from),
        }
    }

    fn format(&self) -> NativeOutputFormat {
        match self {
            Self::Version => NativeOutputFormat::Text,
            Self::Native(command) => command.format(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbtest::arbitrary::Arbitrary;

    #[test]
    fn strict_exit_mapping_satisfies_the_fsl_invariants() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let outcome = *u.choose(&[
                    CliOutcome::Success,
                    CliOutcome::DomainFinding,
                    CliOutcome::ToolFailure,
                ])?;
                let strict = bool::arbitrary(u)?;
                let code = exit_with_code(outcome, strict);

                assert!(
                    matches!(code, CliExitCode::SUCCESS | CliExitCode::FAILURE)
                        || code == CliExitCode::from(2_u8)
                );
                if !strict && outcome != CliOutcome::ToolFailure {
                    assert_eq!(code, CliExitCode::SUCCESS);
                }
                if outcome == CliOutcome::ToolFailure {
                    assert_eq!(code, CliExitCode::FAILURE);
                }
                assert_eq!(
                    code == CliExitCode::from(2_u8),
                    strict && outcome == CliOutcome::DomainFinding
                );
                Ok(())
            },
        );
    }
}
