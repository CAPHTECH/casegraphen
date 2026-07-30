use crate::native_cli::{NativeCliCommand, NativeCliError};
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
    match run(env::args_os().skip(1)) {
        Ok(()) => CliExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            CliExitCode::FAILURE
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    let command = Command::parse(args)?;
    let json = command.run_json()?;
    match command.output() {
        Some(path) => {
            let value = serde_json::from_str::<serde_json::Value>(&json)?;
            let text = serde_json::to_string_pretty(&value)?;
            fs::write(path, format!("{text}\n")).map_err(|source| {
                CliError::from(NativeCliError::Io {
                    path: path.clone(),
                    source,
                })
            })
        }
        None => {
            println!("{json}");
            Ok(())
        }
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
                | "equivalence" | "invariant" | "case" | "morphism" | "plan" | "binding"
                | "run" | "review" | "evidence" | "cell"),
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

    fn run_json(&self) -> Result<String, CliError> {
        match self {
            Self::Version => Ok(format!("casegraphen {}", env!("CARGO_PKG_VERSION"))),
            Self::Native(command) => command.run_json().map_err(CliError::from),
        }
    }
}
