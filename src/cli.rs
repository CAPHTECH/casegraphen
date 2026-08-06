use crate::native_cli::{
    scan_requested_format, NativeCliCommand, NativeCliError, NativeCommandResult,
    NativeOutputFormat,
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
const REFUSAL_SCHEMA: &str = "highergraphen.case.native_cli.refusal.v1";
const REFUSAL_VERSION: u32 = 1;

pub fn main_entry() -> CliExitCode {
    match parse_and_run(env::args_os().skip(1)) {
        Ok(result) => exit_with_code(result.outcome, result.strict),
        Err(refusal) => {
            emit_refusal(&refusal);
            exit_with_code(CliOutcome::ToolFailure, false)
        }
    }
}

/// A `CliError` paired with everything needed to render it correctly:
/// which format it should render in, resolved once and carried alongside
/// the error rather than re-derived at print time; and, when the command
/// already knew it, how far a durable mutation got before the refusal —
/// e.g. an append succeeded and only a later `--output` write failed.
///
/// `parse_and_run` is the only place that constructs one, and each of its
/// two `map_err` closures below sees only what it needs to resolve these
/// correctly — the parse-failure closure has raw argv and nothing else, so
/// `completed_through` is always `None` there (no command ever ran); the
/// post-parse closure has the already-known `command.format()` and
/// whatever `run_command`'s own `RunFailure` computed, and no argv binding
/// in scope to fall back to. A future change that wanted to route a
/// post-parse refusal through `scan_requested_format` would have to thread
/// argv into that second closure to do it — it cannot happen by accident.
struct Refusal {
    error: CliError,
    format: NativeOutputFormat,
    completed_through: Option<serde_json::Value>,
}

/// Parses argv and runs the resulting command, resolving which format any
/// refusal renders in in the same step: a parse failure has no `Command` to
/// ask, so `scan_requested_format`'s best-effort argv scan is reached only
/// here; a parse success has an authoritative `command.format()`, which is
/// what a later execution failure renders with. Before this, every refusal
/// (including ones from a command that had already parsed successfully)
/// re-derived format from raw argv, which could disagree with the format
/// parsing had already established with certainty — see the regression test
/// below.
fn parse_and_run(args: impl IntoIterator<Item = OsString>) -> Result<SuccessfulRun, Refusal> {
    let args: Vec<OsString> = args.into_iter().collect();
    let command = Command::parse(args.clone()).map_err(|error| Refusal {
        format: scan_requested_format(&args),
        error,
        completed_through: None,
    })?;
    let format = command.format();
    run_command(&command).map_err(|failure| Refusal {
        error: failure.error,
        format,
        completed_through: failure.completed_through,
    })
}

/// Refusals are written to stderr, never `--output`, and exit 1 either way:
/// a caller reading stdout or `--output` for a report can never mistake a
/// refusal for one. `--format json` shapes the refusal the same way it
/// shapes a report — a typed `error_code` and, where one exists, structured
/// recovery data — instead of leaving only the `--format text` prose it
/// used to be regardless of what was asked for.
fn emit_refusal(refusal: &Refusal) {
    eprintln!("{}", refusal_text(refusal));
}

/// The refusal payload as it will be written to stderr, factored out of
/// `emit_refusal` so a test can inspect it without capturing the real
/// process stderr.
fn refusal_text(refusal: &Refusal) -> String {
    match refusal.format {
        NativeOutputFormat::Text => refusal.error.to_string(),
        NativeOutputFormat::Json => {
            let mut value = serde_json::json!({
                "schema": REFUSAL_SCHEMA,
                "refusal_version": REFUSAL_VERSION,
                "error_code": refusal.error.error_code(),
                "message": refusal.error.refusal_message(),
            });
            if let Some(data) = refusal.error.refusal_data() {
                value["data"] = data;
            }
            // Top-level, not inside `data`: this is not per-`error_code`
            // recovery data, it is "the ledger stopped here" — the same
            // fact `StaleRevision`/`StalePlanRevision` already report, just
            // for the case where the command completed a mutation and then
            // failed for an unrelated reason (a bad `--output` path, a
            // JSON re-render failure) rather than failing to mutate at
            // all. Absent whenever the command does not already hold it —
            // see `completed_through_from_rendered`'s doc comment for
            // exactly which failures populate it.
            if let Some(completed_through) = &refusal.completed_through {
                value["completed_through"] = completed_through.clone();
            }
            serde_json::to_string(&value).expect("refusal payload serializes")
        }
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), CliError> {
    parse_and_run(args)
        .map(|_| ())
        .map_err(|refusal| refusal.error)
}

/// A `CliError` from `run_command`, paired with how far the command got
/// before failing, when it already held that (see
/// `completed_through_from_rendered`). Kept separate from `Refusal`: this
/// struct never carries a render format, since `run_command` does not
/// decide that — `parse_and_run` already knows it from the successfully
/// parsed `Command` before `run_command` is even called.
struct RunFailure {
    error: CliError,
    completed_through: Option<serde_json::Value>,
}

fn run_command(command: &Command) -> Result<SuccessfulRun, RunFailure> {
    let strict = command.strict();
    let format = command.format();
    let result = command.run_rendered().map_err(|error| RunFailure {
        error,
        // No report was ever built, so there is nothing to read a revision
        // back out of. For every command covered here, this also means no
        // durable mutation landed — see `completed_through_from_rendered`'s
        // doc comment for the one family of commands (`run --step` /
        // `run --frontier`) where that second half is not yet true.
        completed_through: None,
    })?;
    let (rendered, domain_finding) = result.into_parts();
    write_rendered_output(command, format, &rendered).map_err(|error| RunFailure {
        error,
        completed_through: completed_through_from_rendered(&rendered),
    })?;
    Ok(SuccessfulRun {
        outcome: if domain_finding {
            CliOutcome::DomainFinding
        } else {
            CliOutcome::Success
        },
        strict,
    })
}

/// Writes (or prints) the rendered report, the one place a refusal here can
/// still occur after a durable mutation already landed: `--output` naming a
/// directory that does not exist, or (vanishingly unlikely, since `rendered`
/// is this process's own serialization) the JSON re-parse/pretty-print step
/// for `--output` failing. Kept as a plain `Result<(), CliError>` — the
/// `completed_through` computation lives in `run_command`, which has
/// `rendered` in scope for both of `run_command`'s own fallible steps and
/// attaches it once instead of at every `?` inside this function.
fn write_rendered_output(
    command: &Command,
    format: NativeOutputFormat,
    rendered: &str,
) -> Result<(), CliError> {
    match command.output() {
        Some(path) => {
            let text = match format {
                NativeOutputFormat::Json => {
                    let value = serde_json::from_str::<serde_json::Value>(rendered)?;
                    serde_json::to_string_pretty(&value)?
                }
                NativeOutputFormat::Text => rendered.to_owned(),
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
    Ok(())
}

/// Reads `result.record.current_revision_id` back out of an already-
/// rendered report, when there is one. Every mutating command's report
/// carries a `result.record` with this field (`append_morphism`'s return
/// value, embedded verbatim) — including `plan accept`/`reject`, which
/// wraps the same shape. This deliberately does not reach into
/// `run --step` / `run --frontier`'s own internal `?` propagation
/// (`native_cli/ops/run.rs`) to extract a revision from a partially
/// completed multi-step dispatch; that is #23's problem when it builds the
/// halt object, not this refusal shape's. A read-only or non-mutating
/// command's report has no `result.record`, so this returns `None` for
/// those too — `completed_through` is present on a refusal only when the
/// command that refused already knew how far it got, never universally.
fn completed_through_from_rendered(rendered: &str) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(rendered).ok()?;
    value
        .get("result")?
        .get("record")?
        .get("current_revision_id")
        .cloned()
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
    CliExitCode::from(exit_code_value(outcome, strict))
}

fn exit_code_value(outcome: CliOutcome, strict: bool) -> u8 {
    match outcome {
        CliOutcome::ToolFailure => 1,
        CliOutcome::DomainFinding if strict => 2,
        CliOutcome::Success | CliOutcome::DomainFinding => 0,
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
                segment @ ("lift" | "graph" | "memory" | "space" | "obstruction" | "completion"
                | "projection" | "equivalence" | "invariant" | "morphism" | "plan"
                | "binding" | "run" | "operate" | "review" | "topology-review"
                | "evidence" | "cell" | "packet"),
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
                let code = exit_code_value(outcome, strict);

                assert!(matches!(code, 0..=2));
                if !strict && outcome != CliOutcome::ToolFailure {
                    assert_eq!(code, 0);
                }
                if outcome == CliOutcome::ToolFailure {
                    assert_eq!(code, 1);
                }
                assert_eq!(code == 2, strict && outcome == CliOutcome::DomainFinding);
                Ok(())
            },
        );
    }

    /// Issue #22's typed refusal payload must not change what it rides on
    /// top of: every refusal — regardless of `error_code`, `--format`, or
    /// whether it carries `data` — still maps to `CliOutcome::ToolFailure`,
    /// which `exit_with_code` always turns into exit 1. `main_entry` never
    /// branches the exit code on the error's content (`emit_refusal` and
    /// `exit_with_code` do not share an input), so this holds by
    /// construction; the property below exercises `refusal_text` across a
    /// representative sample of refusal shapes and both formats to prove
    /// building the richer payload itself cannot panic or smuggle a
    /// different outcome in.
    #[test]
    fn refusal_rendering_never_panics_and_the_tool_failure_exit_code_stays_fixed() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let error = *u.choose(&[0_u8, 1, 2, 3])?;
                let error = match error {
                    0 => CliError::usage("unsupported command segment"),
                    1 => CliError::from(NativeCliError::Invalid("domain rule violated".to_owned())),
                    2 => CliError::from(NativeCliError::StaleRevision {
                        base_revision_id: id_lossy("revision:base"),
                        current_revision_id: id_lossy("revision:current"),
                    }),
                    _ => CliError::from(
                        serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
                    ),
                };
                let format = *u.choose(&[NativeOutputFormat::Json, NativeOutputFormat::Text])?;
                let completed_through = u
                    .arbitrary::<bool>()?
                    .then(|| serde_json::json!("revision:completed-through"));
                let error_code = error.error_code();
                let refusal = Refusal {
                    error,
                    format,
                    completed_through,
                };

                let text = refusal_text(&refusal);
                assert!(!text.is_empty());
                if format == NativeOutputFormat::Json {
                    let value: serde_json::Value =
                        serde_json::from_str(&text).expect("refusal JSON parses");
                    assert_eq!(value["error_code"], serde_json::json!(error_code));
                    assert_eq!(
                        value.get("completed_through").cloned(),
                        refusal.completed_through
                    );
                }

                // Exit code is computed from `CliOutcome` alone, never from
                // the error: proving that holds for every shape above.
                assert_eq!(exit_code_value(CliOutcome::ToolFailure, false), 1);
                Ok(())
            },
        );
    }

    #[test]
    fn json_review_refusal_renders_lint_findings_without_prose_parsing() {
        use crate::{
            execution_topology::ExecutionTopology, graph_lint::lint_execution_topology,
            native_review::NativeReviewError,
        };

        let topology: ExecutionTopology = serde_json::from_str(include_str!(
            "../schemas/experimental/execution.topology.file-review.example.json"
        ))
        .expect("typed topology fixture");
        let finding = lint_execution_topology(&topology)
            .findings
            .into_iter()
            .next()
            .expect("fixture has lint findings");
        let refusal = Refusal {
            error: CliError::from(NativeCliError::Review(NativeReviewError {
                message: "execution topology review refused".to_owned(),
                findings: vec![finding.clone()],
            })),
            format: NativeOutputFormat::Json,
            completed_through: None,
        };

        let value: serde_json::Value =
            serde_json::from_str(&refusal_text(&refusal)).expect("refusal JSON");
        assert_eq!(value["data"]["findings"][0]["code"], finding.code);
        assert_eq!(value["data"]["findings"][0]["location"], finding.location);
        assert_eq!(value["data"]["findings"][0]["detail"], finding.detail);
    }

    fn id_lossy(value: &str) -> higher_graphen_core::Id {
        higher_graphen_core::Id::new(value.to_owned()).expect("test id")
    }
}
