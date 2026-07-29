use crate::{
    native_cli::NativeCliCommand,
    store::write_report,
    topology::TopologyReportOptions,
    workflow_eval::cli_reports::{
        workflow_completions_json, workflow_correspond_json, workflow_evidence_json,
        workflow_evolution_json, workflow_obstructions_json, workflow_project_json,
        workflow_readiness_json, workflow_reason_json, workflow_topology_diff_json,
        workflow_topology_json, workflow_validate_json,
    },
    workflow_workspace::cli_bridge::CgWorkflowBridgeCommand,
};
#[path = "cli_error.rs"]
mod cli_error;
#[path = "cli_required.rs"]
mod cli_required;
mod options;

use cli_error::CliError;
use cli_required::required_segment;
use options::Options;
use std::{env, ffi::OsString, path::PathBuf, process::ExitCode};

const USAGE: &str = include_str!("cli_usage.txt");

pub fn main_entry() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    let command = Command::parse(args)?;
    let json = command.run_json()?;
    match command.output() {
        Some(path) => write_report(path, &serde_json::from_str::<serde_json::Value>(&json)?)
            .map_err(CliError::from),
        None => {
            println!("{json}");
            Ok(())
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Version,
    WorkflowReason {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowValidate {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowReadiness {
        input: PathBuf,
        projection: Option<PathBuf>,
        output: Option<PathBuf>,
    },
    WorkflowObstructions {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowCompletions {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowEvidence {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowHistoryTopology {
        input: PathBuf,
        topology_options: TopologyReportOptions,
        output: Option<PathBuf>,
    },
    WorkflowHistoryTopologyDiff {
        left: PathBuf,
        right: PathBuf,
        topology_options: TopologyReportOptions,
        output: Option<PathBuf>,
    },
    WorkflowProject {
        input: PathBuf,
        projection: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowCorrespond {
        left: PathBuf,
        right: PathBuf,
        output: Option<PathBuf>,
    },
    WorkflowEvolution {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    CgWorkflowBridge(CgWorkflowBridgeCommand),
    Native(NativeCliCommand),
}

impl Command {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args.into_iter();
        match required_segment(&mut args, "command")?.to_str() {
            Some("version") | Some("--version") | Some("-V") => Ok(Self::Version),
            Some("workflow") => Self::parse_workflow(args),
            Some("lift") => NativeCliCommand::parse("lift", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("space") => NativeCliCommand::parse("space", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("obstruction") => NativeCliCommand::parse("obstruction", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("completion") => NativeCliCommand::parse("completion", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("projection") => NativeCliCommand::parse("projection", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("equivalence") => NativeCliCommand::parse("equivalence", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("invariant") => NativeCliCommand::parse("invariant", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("case") => NativeCliCommand::parse("case", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("morphism") => NativeCliCommand::parse("morphism", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("review") => NativeCliCommand::parse("review", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("evidence") => NativeCliCommand::parse("evidence", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("cell") => NativeCliCommand::parse("cell", args)
                .map(Self::Native)
                .map_err(CliError::from),
            Some("cg") => CgWorkflowBridgeCommand::parse(args)
                .map(Self::CgWorkflowBridge)
                .map_err(CliError::usage),
            Some(_) | None => Err(CliError::usage("unsupported command segment")),
        }
    }

    fn parse_one_input(
        args: impl Iterator<Item = OsString>,
        constructor: impl FnOnce(PathBuf, Option<PathBuf>) -> Self,
    ) -> Result<Self, CliError> {
        let options = Options::parse(args)?;
        Ok(constructor(
            options
                .input
                .ok_or_else(|| CliError::usage("--input <path> is required"))?,
            options.output,
        ))
    }

    fn parse_topology(
        args: impl Iterator<Item = OsString>,
        constructor: impl FnOnce(PathBuf, TopologyReportOptions, Option<PathBuf>) -> Self,
        diff_constructor: impl FnOnce(PathBuf, PathBuf, TopologyReportOptions, Option<PathBuf>) -> Self,
    ) -> Result<Self, CliError> {
        let mut args = args.peekable();
        if matches!(args.peek().and_then(|arg| arg.to_str()), Some("diff")) {
            args.next();
            return Self::parse_topology_diff(args, diff_constructor);
        }

        let options = Options::parse(args)?;
        let topology_options = options.topology_options();
        Ok(constructor(
            options
                .input
                .ok_or_else(|| CliError::usage("--input <path> is required"))?,
            topology_options,
            options.output,
        ))
    }

    fn parse_topology_diff(
        args: impl Iterator<Item = OsString>,
        constructor: impl FnOnce(PathBuf, PathBuf, TopologyReportOptions, Option<PathBuf>) -> Self,
    ) -> Result<Self, CliError> {
        let options = Options::parse(args)?;
        let topology_options = options.topology_options();
        Ok(constructor(
            options
                .left
                .ok_or_else(|| CliError::usage("--left <path> is required"))?,
            options
                .right
                .ok_or_else(|| CliError::usage("--right <path> is required"))?,
            topology_options,
            options.output,
        ))
    }

    fn parse_workflow(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args;
        match required_segment(&mut args, "workflow operation")?.to_str() {
            Some("reason") => {
                Self::parse_one_input(args, |input, output| Self::WorkflowReason { input, output })
            }
            Some("validate") => Self::parse_one_input(args, |input, output| {
                Self::WorkflowValidate { input, output }
            }),
            Some("readiness") => Self::parse_workflow_readiness(args),
            Some("obstructions") => Self::parse_one_input(args, |input, output| {
                Self::WorkflowObstructions { input, output }
            }),
            Some("completions") => Self::parse_one_input(args, |input, output| {
                Self::WorkflowCompletions { input, output }
            }),
            Some("evidence") => Self::parse_one_input(args, |input, output| {
                Self::WorkflowEvidence { input, output }
            }),
            Some("history") => Self::parse_workflow_history(args),
            Some("project") => Self::parse_workflow_project(args),
            Some("correspond") => Self::parse_workflow_correspond(args),
            Some("evolution") => Self::parse_one_input(args, |input, output| {
                Self::WorkflowEvolution { input, output }
            }),
            Some(_) | None => Err(CliError::usage("unsupported workflow command segment")),
        }
    }

    fn parse_workflow_history(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let mut args = args;
        match required_segment(&mut args, "workflow history operation")?.to_str() {
            Some("topology") => Self::parse_topology(
                args,
                |input, topology_options, output| Self::WorkflowHistoryTopology {
                    input,
                    topology_options,
                    output,
                },
                |left, right, topology_options, output| Self::WorkflowHistoryTopologyDiff {
                    left,
                    right,
                    topology_options,
                    output,
                },
            ),
            Some(_) | None => Err(CliError::usage(
                "unsupported workflow history command segment",
            )),
        }
    }

    fn parse_workflow_readiness(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let options = Options::parse(args)?;
        Ok(Self::WorkflowReadiness {
            input: options
                .input
                .ok_or_else(|| CliError::usage("--input <path> is required"))?,
            projection: options.projection,
            output: options.output,
        })
    }

    fn parse_workflow_project(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let options = Options::parse(args)?;
        Ok(Self::WorkflowProject {
            input: options
                .input
                .ok_or_else(|| CliError::usage("--input <path> is required"))?,
            projection: options
                .projection
                .ok_or_else(|| CliError::usage("--projection <path> is required"))?,
            output: options.output,
        })
    }

    fn parse_workflow_correspond(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let options = Options::parse(args)?;
        Ok(Self::WorkflowCorrespond {
            left: options
                .left
                .ok_or_else(|| CliError::usage("--left <path> is required"))?,
            right: options
                .right
                .ok_or_else(|| CliError::usage("--right <path> is required"))?,
            output: options.output,
        })
    }

    fn output(&self) -> Option<&PathBuf> {
        match self {
            Self::Version => None,
            Self::WorkflowReason { output, .. }
            | Self::WorkflowValidate { output, .. }
            | Self::WorkflowReadiness { output, .. }
            | Self::WorkflowObstructions { output, .. }
            | Self::WorkflowCompletions { output, .. }
            | Self::WorkflowEvidence { output, .. }
            | Self::WorkflowHistoryTopology { output, .. }
            | Self::WorkflowHistoryTopologyDiff { output, .. }
            | Self::WorkflowProject { output, .. }
            | Self::WorkflowCorrespond { output, .. }
            | Self::WorkflowEvolution { output, .. } => output.as_ref(),
            Self::CgWorkflowBridge(command) => command.output(),
            Self::Native(command) => command.output(),
        }
    }

    fn run_json(&self) -> Result<String, CliError> {
        match self {
            Self::Version => Ok(format!("casegraphen {}", env!("CARGO_PKG_VERSION"))),
            Self::WorkflowReason { input, .. } => {
                workflow_reason_json(input).map_err(CliError::from)
            }
            Self::WorkflowValidate { input, .. } => {
                workflow_validate_json(input).map_err(CliError::from)
            }
            Self::WorkflowReadiness {
                input, projection, ..
            } => workflow_readiness_json(input, projection.as_deref()).map_err(CliError::from),
            Self::WorkflowObstructions { input, .. } => {
                workflow_obstructions_json(input).map_err(CliError::from)
            }
            Self::WorkflowCompletions { input, .. } => {
                workflow_completions_json(input).map_err(CliError::from)
            }
            Self::WorkflowEvidence { input, .. } => {
                workflow_evidence_json(input).map_err(CliError::from)
            }
            Self::WorkflowHistoryTopology {
                input,
                topology_options,
                ..
            } => workflow_topology_json(input, *topology_options).map_err(CliError::from),
            Self::WorkflowHistoryTopologyDiff {
                left,
                right,
                topology_options,
                ..
            } => {
                workflow_topology_diff_json(left, right, *topology_options).map_err(CliError::from)
            }
            Self::WorkflowProject {
                input, projection, ..
            } => workflow_project_json(input, projection).map_err(CliError::from),
            Self::WorkflowCorrespond { left, right, .. } => {
                workflow_correspond_json(left, right).map_err(CliError::from)
            }
            Self::WorkflowEvolution { input, .. } => {
                workflow_evolution_json(input).map_err(CliError::from)
            }
            Self::CgWorkflowBridge(command) => command.run_json().map_err(CliError::from),
            Self::Native(command) => command.run_json().map_err(CliError::from),
        }
    }
}
