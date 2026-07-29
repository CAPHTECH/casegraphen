use super::CliError;
use crate::topology::TopologyReportOptions;
use higher_graphen_structure::space::Dimension;
use std::{ffi::OsString, path::PathBuf};

#[derive(Default)]
pub(super) struct Options {
    pub(super) input: Option<PathBuf>,
    pub(super) projection: Option<PathBuf>,
    pub(super) left: Option<PathBuf>,
    pub(super) right: Option<PathBuf>,
    pub(super) output: Option<PathBuf>,
    pub(super) higher_order: bool,
    pub(super) max_dimension: Option<Dimension>,
    pub(super) min_persistence_stages: usize,
}

impl Options {
    pub(super) fn parse(args: impl Iterator<Item = OsString>) -> Result<Self, CliError> {
        let mut options = Self::default();
        let mut format_seen = false;
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--format") => {
                    require_json_format(&mut args)?;
                    format_seen = true;
                }
                Some("--input") => options.input = Some(require_path(&mut args, "--input")?),
                Some("--projection") => {
                    options.projection = Some(require_path(&mut args, "--projection")?);
                }
                Some("--left") => options.left = Some(require_path(&mut args, "--left")?),
                Some("--right") => options.right = Some(require_path(&mut args, "--right")?),
                Some("--output") => options.output = Some(require_path(&mut args, "--output")?),
                Some("--higher-order") => {
                    options.higher_order = true;
                }
                Some("--max-dimension") => {
                    options.max_dimension = Some(require_dimension(&mut args, "--max-dimension")?)
                }
                Some("--min-persistence") | Some("--min-persistence-stages") => {
                    options.min_persistence_stages = require_usize(&mut args, "--min-persistence")?;
                }
                Some(_) | None => {
                    return Err(CliError::usage(format!("unsupported argument {arg:?}")));
                }
            }
        }
        require_format_seen(format_seen)?;
        Ok(options)
    }

    pub(super) fn topology_options(&self) -> TopologyReportOptions {
        if self.higher_order {
            TopologyReportOptions::higher_order(self.max_dimension, self.min_persistence_stages)
        } else {
            TopologyReportOptions::baseline()
        }
    }
}

fn require_json_format(args: &mut impl Iterator<Item = OsString>) -> Result<(), CliError> {
    match args.next() {
        Some(arg) if arg == "json" => Ok(()),
        Some(arg) => Err(CliError::usage(format!(
            "unsupported format {arg:?}; only json is supported"
        ))),
        None => Err(CliError::usage("missing value for --format")),
    }
}

fn require_format_seen(format_seen: bool) -> Result<(), CliError> {
    if format_seen {
        Ok(())
    } else {
        Err(CliError::usage("--format json is required"))
    }
}

fn require_path(
    args: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<PathBuf, CliError> {
    match args.next() {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        Some(_) => Err(CliError::usage(format!("empty path for {option}"))),
        None => Err(CliError::usage(format!("missing value for {option}"))),
    }
}

fn require_string(
    args: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<String, CliError> {
    match args.next() {
        Some(value) if !value.is_empty() => value
            .into_string()
            .map_err(|value| CliError::usage(format!("non-utf8 value for {option}: {value:?}"))),
        Some(_) => Err(CliError::usage(format!("empty value for {option}"))),
        None => Err(CliError::usage(format!("missing value for {option}"))),
    }
}

fn require_dimension(
    args: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<Dimension, CliError> {
    require_string(args, option)?
        .parse::<Dimension>()
        .map_err(|_| CliError::usage(format!("invalid integer for {option}")))
}

fn require_usize(
    args: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<usize, CliError> {
    require_string(args, option)?
        .parse::<usize>()
        .map_err(|_| CliError::usage(format!("invalid integer for {option}")))
}
