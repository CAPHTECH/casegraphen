use super::{
    ops::{
        MemoryIndexMode, MemoryReadMode, MemorySourceMode, NativeCloseGateOptions,
        NativeRunGateOptions,
    },
    options::{require_id, required_segment, NativeOptions, OperationGateRequirement},
    NativeCliCommand, NativeCliError, NativeOutputFormat, NativeReasonSection,
};
use crate::native_model::ReviewAction;
use higher_graphen_core::Id;
use std::ffi::OsString;

/// `run --frontier`'s and `operate`'s shared dispatch gate: `parse_operate`
/// takes exactly `parse_run`'s frontier-branch gate flags, because ADR 0016
/// decision 3 makes `operate` repeat that same selection round after round.
/// The `--gate-actor-id` alias check is authorization-adjacent (it asserts
/// the compatibility flag names the same actor the gate resolved to), so it
/// stayed in exactly one place rather than two bodies that could disagree.
struct RunFamilyGate {
    actor_id: Id,
    gate_options: NativeRunGateOptions,
}

fn resolve_run_family_gate(
    options: &NativeOptions,
    command: &'static str,
) -> Result<RunFamilyGate, NativeCliError> {
    let resolved_gate =
        options.resolve_operation_gate_options(OperationGateRequirement::Required {
            command,
            actor_command: None,
        })?;
    let actor_id = resolved_gate
        .actor_id
        .clone()
        .expect("required gate resolution checked actor_id");
    if let Some(gate_actor_id) = &options.gate_actor_id {
        if gate_actor_id != &actor_id {
            return Err(NativeCliError::usage(
                "--gate-actor-id is a compatibility alias and must equal --actor-id",
            ));
        }
    }
    let gate_options = NativeRunGateOptions {
        capability_ids: resolved_gate.capability_ids,
        operation_scope_id: resolved_gate
            .operation_scope_id
            .expect("required gate resolution checked operation_scope_id"),
        audience: resolved_gate
            .audience
            .expect("required gate resolution checked audience"),
        source_boundary_id: resolved_gate
            .source_boundary_id
            .expect("required gate resolution checked source_boundary_id"),
    };
    Ok(RunFamilyGate {
        actor_id,
        gate_options,
    })
}

fn resolve_max_parallel(options: &NativeOptions) -> Result<usize, NativeCliError> {
    let max_parallel = options.max_parallel.unwrap_or(4);
    if max_parallel == 0 {
        return Err(NativeCliError::usage("--max-parallel must be at least 1"));
    }
    Ok(max_parallel)
}

impl NativeCliCommand {
    pub fn parse(
        namespace: &str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let mut args = args.into_iter();
        match namespace {
            "space" => Self::parse_space(required_segment(&mut args, "space operation")?, args),
            "lift" => Self::parse_lift(required_segment(&mut args, "lift adapter")?, args),
            "graph" => Self::parse_graph(required_segment(&mut args, "graph operation")?, args),
            "schema" => Self::parse_schema(required_segment(&mut args, "schema operation")?, args),
            "memory" => Self::parse_memory(required_segment(&mut args, "memory operation")?, args),
            "github" => Self::parse_github(required_segment(&mut args, "github operation")?, args),
            "obstruction" => {
                Self::parse_obstruction(required_segment(&mut args, "obstruction operation")?, args)
            }
            "completion" => {
                Self::parse_completion(required_segment(&mut args, "completion operation")?, args)
            }
            "projection" => {
                Self::parse_projection(required_segment(&mut args, "projection operation")?, args)
            }
            "equivalence" => {
                Self::parse_equivalence(required_segment(&mut args, "equivalence operation")?, args)
            }
            "invariant" => {
                Self::parse_invariant(required_segment(&mut args, "invariant operation")?, args)
            }
            "morphism" => {
                Self::parse_morphism(required_segment(&mut args, "morphism operation")?, args)
            }
            "plan" => Self::parse_plan(required_segment(&mut args, "plan operation")?, args),
            "topology-review" => Self::parse_topology_review(
                required_segment(&mut args, "topology-review operation")?,
                args,
            ),
            "binding" => {
                Self::parse_binding(required_segment(&mut args, "binding operation")?, args)
            }
            "run" => Self::parse_run(args),
            "operate" => Self::parse_operate(args),
            "review" => Self::parse_review(required_segment(&mut args, "review operation")?, args),
            "evidence" => {
                Self::parse_evidence(required_segment(&mut args, "evidence operation")?, args)
            }
            "cell" => Self::parse_cell(required_segment(&mut args, "cell operation")?, args),
            "packet" => Self::parse_packet(required_segment(&mut args, "packet operation")?, args),
            _ => Err(NativeCliError::usage("unsupported native namespace")),
        }
    }

    fn parse_memory(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("memory operation must be UTF-8"))?;
        let mut args = args.into_iter();
        let nested = if matches!(operation, "source" | "index") {
            Some(required_segment(&mut args, "memory sub-operation")?)
        } else {
            None
        };
        let nested = nested.as_ref().and_then(|value| value.to_str());
        let options = NativeOptions::parse("memory", args)?;
        let read = |mode, target_required: bool| -> Result<Self, NativeCliError> {
            let target_id = if target_required {
                Some(options.require_id("--target-id")?)
            } else {
                options.target_id.clone()
            };
            Ok(Self::MemoryRead {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                input: options.require_path("--input")?,
                policy: options.require_path("--policy")?,
                mode,
                target_id,
                output: options.output.clone(),
            })
        };
        match operation {
            "query" => read(MemoryReadMode::Query, false),
            "explain" => read(MemoryReadMode::Explain, true),
            "history" => read(MemoryReadMode::History, true),
            "conflicts" => read(MemoryReadMode::Conflicts, false),
            "candidates" => read(MemoryReadMode::Candidates, false),
            "sources" => read(MemoryReadMode::Sources, true),
            "source" => match nested {
                Some("attach") => Ok(Self::MemorySource {
                    source_record: options.require_path("--source-record")?,
                    source_artifact: options.require_path("--source-artifact")?,
                    mode: MemorySourceMode::Attach,
                    output: options.output,
                }),
                Some("inspect") => Ok(Self::MemorySource {
                    source_record: options.require_path("--source-record")?,
                    source_artifact: options.require_path("--source-artifact")?,
                    mode: MemorySourceMode::Inspect,
                    output: options.output,
                }),
                _ => Err(NativeCliError::usage("unsupported memory source command")),
            },
            "check" => Ok(Self::MemoryCheck {
                input: options.require_path("--input")?,
                source_record: options.require_path("--source-record")?,
                source_artifact: options.require_path("--source-artifact")?,
                policy: options.require_path("--policy")?,
                output: options.output,
            }),
            "propose" => Ok(Self::MemoryPropose {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                input: options.require_path("--input")?,
                source_record: options.require_path("--source-record")?,
                source_artifact: options.require_path("--source-artifact")?,
                policy: options.require_path("--policy")?,
                output: options.output,
            }),
            "index" => match nested {
                Some("rebuild") => Ok(Self::MemoryIndex {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    input: options.require_path("--input")?,
                    policy: options.require_path("--policy")?,
                    mode: MemoryIndexMode::Rebuild,
                    index: None,
                    output: options.output,
                }),
                Some("validate") => Ok(Self::MemoryIndex {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    input: options.require_path("--input")?,
                    policy: options.require_path("--policy")?,
                    mode: MemoryIndexMode::Validate,
                    index: Some(options.require_path("--index")?),
                    output: options.output,
                }),
                _ => Err(NativeCliError::usage("unsupported memory index command")),
            },
            _ => Err(NativeCliError::usage("unsupported memory command")),
        }
    }

    /// `github observe|refresh|project` (design doc §9). Shaped like
    /// `parse_memory`: one `NativeOptions::parse` per operation, `--format
    /// json` required (no `--format text` — these are read-only reports,
    /// not `space reason`'s halt rendering), no `--store`.
    ///
    /// `--strict` is accepted on all three (S6): each can carry a domain
    /// finding (`observe`'s cross-repository exclusions, `refresh`'s
    /// `stale_head`, `project`'s blocking findings), and `--strict` is this
    /// CLI's one existing mechanism for turning that into exit 2 rather
    /// than an obstruction a caller must remember to check for in the JSON
    /// body — the same convention `space reason --strict` already uses.
    ///
    /// `--require-independent-review` is accepted **only** on `project`
    /// (S5) — it is the one operation that reads `options
    /// .require_independent_review`; `observe`/`refresh` do not, so
    /// `NativeOptions::parse_with_strict` (which refuses this flag) is used
    /// for them instead of the combined parser, or the flag would parse
    /// successfully on those two and then be silently dropped instead of
    /// ever reaching `evaluate_independence`.
    fn parse_github(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("github operation must be UTF-8"))?;
        let options = if operation == "project" {
            NativeOptions::parse_with_strict_and_require_independent_review("github", args)?
        } else {
            NativeOptions::parse_with_strict("github", args)?
        };
        match operation {
            "observe" => Ok(Self::GithubObserve {
                manifest: options.require_path("--manifest")?,
                capture_dir: options.require_path("--capture-dir")?,
                strict: options.strict,
                output: options.output,
            }),
            "refresh" => Ok(Self::GithubRefresh {
                manifest: options.require_path("--manifest")?,
                capture_dir: options.require_path("--capture-dir")?,
                previous_manifest: options.require_path("--previous-manifest")?,
                previous_capture_dir: options.require_path("--previous-capture-dir")?,
                previous_observation: options.previous_observation.clone(),
                strict: options.strict,
                output: options.output,
            }),
            "project" => Ok(Self::GithubProject {
                manifest: options.require_path("--manifest")?,
                capture_dir: options.require_path("--capture-dir")?,
                require_independent_review: options.require_independent_review,
                strict: options.strict,
                output: options.output,
            }),
            _ => Err(NativeCliError::usage("unsupported github command")),
        }
    }

    fn parse_graph(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse_text_only("graph", args)?;
        match operation.to_str() {
            Some("lint") => Ok(Self::GraphLint {
                input: options.require_path("--input")?,
                format: options.format,
                output: options.output,
            }),
            Some(_) | None => Err(NativeCliError::usage("unsupported graph command")),
        }
    }

    /// `casegraphen schema list|get`: read-only lookups against the compiled-in
    /// catalog (`schema_catalog.rs`), so — unlike every other native
    /// namespace — neither operation touches `--store`. `list` takes no
    /// selector; `get` takes exactly one of `--id`/`--file`, checked here
    /// rather than left for `ops/schema.rs` to refuse, so the usage error
    /// names the flag before any catalog lookup runs.
    fn parse_schema(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse("schema", args)?;
        match operation.to_str() {
            Some("list") => Ok(Self::SchemaList {
                output: options.output,
            }),
            Some("get") => match (&options.schema_id, &options.schema_file) {
                (Some(_), Some(_)) => Err(NativeCliError::usage(
                    "schema get accepts only one of --id or --file",
                )),
                (None, None) => Err(NativeCliError::usage("schema get requires --id or --file")),
                _ => Ok(Self::SchemaGet {
                    id: options.schema_id,
                    file: options.schema_file,
                    output: options.output,
                }),
            },
            Some(_) | None => Err(NativeCliError::usage("unsupported schema command")),
        }
    }

    fn parse_space(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("space operation must be UTF-8"))?;
        let mut args = args.into_iter().collect::<Vec<_>>();
        let topology = operation == "topology";
        let topology_diff = topology
            && args
                .first()
                .and_then(|argument| argument.to_str())
                .is_some_and(|argument| argument == "diff");
        if topology_diff {
            args.remove(0);
        }
        // `--since-revision` belongs only to `space reason --format text`'s
        // "Changed since" section, and is pulled out of argv here rather
        // than accepted through the shared, `text_allowed`-gated argument
        // loop `space history` also goes through: `text_allowed` answers
        // "is `--format text` legal here", a question several operations
        // share an answer to, and `--since-revision`'s validity is a
        // different question that only one of them has an answer to. Gating
        // it on `text_allowed` was the two-literal trap `parse_reason`
        // below used to guard against by hand; scanning for the flag only
        // when `operation == "reason"` makes this the single place its
        // validity is decided. If `run`/`operate` later switch from
        // `parse_with_strict` to `parse_reason` to gain `--format text` (the
        // approved follow-up), they inherit `text_allowed` but never reach
        // this branch, so `--since-revision` stays exactly as unrecognized
        // for them as any other unsupported argument — never parsed, so
        // never silently dropped.
        let since_revision_id = if operation == "reason" {
            Self::extract_since_revision(&mut args)?
        } else {
            None
        };
        let options = if operation == "reason" {
            NativeOptions::parse_reason("space", args)?
        } else if operation == "history" {
            NativeOptions::parse_text_only("space", args)?
        } else {
            NativeOptions::parse("space", args)?
        };
        match operation {
            "new" => Ok(Self::CaseNew {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                space_id: options.require_id("--space-id")?,
                title: options.require_string("--title")?,
                revision_id: options.require_id("--revision-id")?,
                output: options.output,
            }),
            "list" => Ok(Self::CaseList {
                store: options.require_store()?,
                output: options.output,
            }),
            "inspect" => Ok(Self::CaseInspect {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                output: options.output,
            }),
            "history" => Ok(Self::CaseHistory {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                format: options.format,
                output: options.output,
            }),
            "replay" => Ok(Self::CaseReplay {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                output: options.output,
            }),
            "rebuild" => Ok(Self::CaseRebuild {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                adopt_existing_log: options.adopt_existing_log,
                output: options.output,
            }),
            "validate" => Ok(Self::CaseValidate {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                output: options.output,
            }),
            "reason" => Self::parse_reason(options, NativeReasonSection::Reason, since_revision_id),
            "frontier" => Self::parse_reason(options, NativeReasonSection::Frontier, None),
            "evidence" => Self::parse_reason(options, NativeReasonSection::Evidence, None),
            "project" => Self::parse_reason(options, NativeReasonSection::Project, None),
            "topology" if topology_diff => Ok(Self::CaseTopologyDiff {
                left_store: options.require_path("--left-store")?,
                left_case_space_id: options.require_id("--left-case-space-id")?,
                right_store: options.require_path("--right-store")?,
                right_case_space_id: options.require_id("--right-case-space-id")?,
                topology_options: options.topology_options(),
                output: options.output,
            }),
            "topology" => Ok(Self::CaseTopology {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                topology_options: options.topology_options(),
                output: options.output,
            }),
            _ => Err(NativeCliError::usage("unsupported native space command")),
        }
    }

    fn parse_lift(
        adapter: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let adapter = adapter
            .to_str()
            .ok_or_else(|| NativeCliError::usage("lift adapter must be UTF-8"))?;
        let options = NativeOptions::parse("lift", args)?;
        // Every lift adapter derives its identity from the input — `lift
        // native` the case space id from the imported case space's own id,
        // the structured-source adapters both the space id and the case
        // space id from the source graph/snapshot id — so a caller-supplied
        // `--case-space-id` or `--space-id` is never load-bearing. Issue
        // #130: `--case-space-id` used to be accepted and silently dropped,
        // which is worse than an unknown flag because nothing downstream
        // ever reads it; `--space-id` has the same shape and the same
        // false-naming risk. Refuse both before dispatching on the adapter,
        // rather than duplicating the adapter match here just to reorder
        // this ahead of "unsupported lift adapter" — that would answer
        // "which lift adapters exist?" in two places for one flag check.
        if options.case_space_id.is_some() {
            return Err(NativeCliError::usage(
                "--case-space-id is not accepted by lift: the case space id is always derived \
                 from the input, never from the command line",
            ));
        }
        if options.space_id.is_some() {
            return Err(NativeCliError::usage(
                "--space-id is not accepted by lift: the space id is always derived from the \
                 input, never from the command line",
            ));
        }
        match adapter {
            "native" => Ok(Self::CaseImport {
                store: options.require_store()?,
                input: options.require_path("--input")?,
                revision_id: options.require_id("--revision-id")?,
                output: options.output,
            }),
            "workflow" | "case-graph" | "github-issues" => Ok(Self::LiftStructuredSource {
                store: options.require_store()?,
                input: options.require_path("--input")?,
                revision_id: options.require_id("--revision-id")?,
                adapter: adapter.to_owned(),
                output: options.output,
            }),
            _ => Err(NativeCliError::usage("unsupported lift adapter")),
        }
    }

    fn parse_obstruction(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        match operation.to_str() {
            Some("list") => Self::parse_reason(
                NativeOptions::parse_with_strict("obstruction", args)?,
                NativeReasonSection::Obstructions,
                None,
            ),
            Some(_) | None => Err(NativeCliError::usage("unsupported obstruction command")),
        }
    }

    fn parse_completion(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        match operation.to_str() {
            Some("candidates") => Self::parse_reason(
                NativeOptions::parse("completion", args)?,
                NativeReasonSection::Completions,
                None,
            ),
            Some(_) | None => Err(NativeCliError::usage("unsupported completion command")),
        }
    }

    fn parse_projection(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse("projection", args)?;
        match operation.to_str() {
            Some("apply") => {
                let projection = options.require_path("--projection")?;
                Ok(Self::ProjectionApply {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    projection,
                    output: options.output,
                })
            }
            Some(_) | None => Err(NativeCliError::usage("unsupported projection command")),
        }
    }

    fn parse_equivalence(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse("equivalence", args)?;
        match operation.to_str() {
            Some("check") => Ok(Self::EquivalenceCheck {
                left_store: options.require_path("--left-store")?,
                left_case_space_id: options.require_id("--left-case-space-id")?,
                right_store: options.require_path("--right-store")?,
                right_case_space_id: options.require_id("--right-case-space-id")?,
                topology_options: options.topology_options(),
                output: options.output,
            }),
            Some(_) | None => Err(NativeCliError::usage("unsupported equivalence command")),
        }
    }

    fn parse_invariant(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse_with_strict("invariant", args)?;
        match operation.to_str() {
            Some("check") => Ok(Self::InvariantCheck {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                strict: options.strict,
                output: options.output,
            }),
            Some("close-check") => Self::parse_close_check(options),
            Some(_) | None => Err(NativeCliError::usage("unsupported invariant command")),
        }
    }

    fn parse_close_check(options: NativeOptions) -> Result<Self, NativeCliError> {
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Optional)?;
        Ok(Self::CaseCloseCheck {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .or(options.revision_id.clone())
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            validation_evidence_ids: options.validation_evidence_ids,
            gate_options: NativeCloseGateOptions {
                close_policy_id: options.close_policy_id,
                gate_options,
            },
            strict: options.strict,
            output: options.output,
        })
    }

    /// `since_revision_id` is `Some` only when the caller is `parse_space`'s
    /// `"reason"` arm (the only place `extract_since_revision` runs) — every
    /// other call site below passes `None` because its operation never
    /// scanned argv for the flag in the first place. So the one thing left
    /// to check here is `--format text`: the same case space's own
    /// argv could still name `--format json` alongside a `--since-revision`
    /// `extract_since_revision` already accepted syntactically, and that
    /// combination is refused rather than rendering a report with the
    /// section silently missing.
    fn parse_reason(
        options: NativeOptions,
        section: NativeReasonSection,
        since_revision_id: Option<Id>,
    ) -> Result<Self, NativeCliError> {
        if since_revision_id.is_some() && options.format != NativeOutputFormat::Text {
            return Err(NativeCliError::usage(
                "--since-revision is only valid on space reason --format text",
            ));
        }
        Ok(Self::CaseReason {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            section,
            strict: options.strict,
            format: options.format,
            since_revision_id,
            output: options.output,
        })
    }

    /// The one place `--since-revision <id>` is recognized as a token at
    /// all — see the call site in `parse_space` for why that is
    /// deliberate. Removes the flag and its value from `args` before the
    /// rest is handed to `NativeOptions::parse_reason`, the same
    /// pre-scan-then-strip style `parse_space` already uses for
    /// `topology diff`'s positional token.
    fn extract_since_revision(args: &mut Vec<OsString>) -> Result<Option<Id>, NativeCliError> {
        let Some(index) = args
            .iter()
            .position(|arg| arg.to_str() == Some("--since-revision"))
        else {
            return Ok(None);
        };
        args.remove(index);
        if index >= args.len() {
            return Err(NativeCliError::usage("--since-revision <id> is required"));
        }
        let value = args.remove(index);
        require_id(&mut std::iter::once(value), "--since-revision").map(Some)
    }

    fn parse_morphism(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("morphism operation must be UTF-8"))?;
        let options = NativeOptions::parse("morphism", args)?;
        match operation {
            "propose" => Ok(Self::MorphismPropose {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                input: options.require_path("--input")?,
                output: options.output,
            }),
            "check" => Ok(Self::MorphismCheck {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                morphism_id: options.require_id("--morphism-id")?,
                output: options.output,
            }),
            "apply" => {
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "morphism apply",
                        actor_command: Some("morphism apply"),
                    })?;
                Ok(Self::MorphismApply {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    morphism_id: options.require_id("--morphism-id")?,
                    base_revision_id: options
                        .base_revision_id
                        .clone()
                        .or(options.revision_id.clone())
                        .ok_or_else(|| {
                            NativeCliError::usage("--base-revision-id <id> is required")
                        })?,
                    reviewer_id: Some(options.require_id("--reviewer-id")?),
                    reason: Some(options.require_string("--reason")?),
                    gate_options,
                    output: options.output,
                })
            }
            "reject" => {
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "morphism reject",
                        actor_command: Some("morphism reject"),
                    })?;
                Ok(Self::MorphismReject {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    morphism_id: options.require_id("--morphism-id")?,
                    reviewer_id: options.require_id("--reviewer-id")?,
                    reason: options.require_string("--reason")?,
                    revision_id: options.require_id("--revision-id")?,
                    gate_options,
                    output: options.output,
                })
            }
            _ => Err(NativeCliError::usage("unsupported native morphism command")),
        }
    }

    fn parse_plan(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("plan operation must be UTF-8"))?;
        let options = NativeOptions::parse("plan", args)?;
        match operation {
            "propose" => Ok(Self::PlanPropose {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                input: options.require_path("--input")?,
                output: options.output,
            }),
            "check" => Ok(Self::PlanCheck {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                plan_id: options.require_id("--plan-id")?,
                output: options.output,
            }),
            "accept" | "reject" => {
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "plan review",
                        actor_command: Some("plan review"),
                    })?;
                Ok(Self::PlanReview {
                    action: if operation == "accept" {
                        ReviewAction::Accept
                    } else {
                        ReviewAction::Reject
                    },
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    plan_id: options.require_id("--plan-id")?,
                    reviewer_id: options.require_id("--reviewer-id")?,
                    reason: options.require_string("--reason")?,
                    base_revision_id: options.base_revision_id.clone().ok_or_else(|| {
                        NativeCliError::usage("--base-revision-id <id> is required")
                    })?,
                    gate_options,
                    output: options.output,
                })
            }
            _ => Err(NativeCliError::usage("unsupported native plan command")),
        }
    }

    fn parse_binding(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse("binding", args)?;
        match operation.to_str() {
            Some("register") => Ok(Self::BindingRegister {
                store: options.require_store()?,
                input: options.require_path("--input")?,
                output: options.output,
            }),
            Some(_) | None => Err(NativeCliError::usage("unsupported native binding command")),
        }
    }

    fn parse_topology_review(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("topology-review operation must be UTF-8"))?;
        let options = NativeOptions::parse("topology-review", args)?;
        match operation {
            "inspect" => Ok(Self::TopologyReviewInspect {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                claim_cell_id: options.require_id("--target-id")?,
                output: options.output,
            }),
            // Which topology-review operations exist beyond "inspect" is
            // answered once, by this inner match — the outer arm no longer
            // repeats "accept" | "reject" | "reopen" just to justify
            // reaching this branch, so it cannot drift from the action
            // conversion below the way it did before (adding a fourth
            // action here without also adding it above used to compile and
            // then panic on `_ => unreachable!()`).
            _ => {
                let action = match operation {
                    "accept" => ReviewAction::Accept,
                    "reject" => ReviewAction::Reject,
                    "reopen" => ReviewAction::Reopen,
                    _ => {
                        return Err(NativeCliError::usage(
                            "unsupported native topology-review command",
                        ))
                    }
                };
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "execution topology review",
                        actor_command: Some("execution topology review"),
                    })?;
                Ok(Self::TopologyReview {
                    action,
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    claim_cell_id: options.require_id("--target-id")?,
                    topology_input: options.require_path("--input")?,
                    policy_manifest_input: options.policy_manifest.clone().ok_or_else(|| {
                        NativeCliError::usage("--policy-manifest <path> is required")
                    })?,
                    reviewer_id: options.require_id("--reviewer-id")?,
                    reason: options.require_string("--reason")?,
                    base_revision_id: options.base_revision_id.clone().ok_or_else(|| {
                        NativeCliError::usage("--base-revision-id <id> is required")
                    })?,
                    gate_options,
                    output: options.output,
                })
            }
        }
    }

    /// `NativeOptions::parse_reason` rather than `parse_with_strict`, to
    /// gain `--format text` for issue #35's halt rendering. This does not
    /// reopen `--since-revision` for `run`: that flag is recognized nowhere
    /// in the shared options parser (`NativeOptions::consume_arg` has no
    /// arm for it at all) — the only place it is ever read is
    /// `parse_space`'s `"reason"` arm's own `Self::extract_since_revision`
    /// pre-scan of argv, which `parse_run` never calls. An unrecognized
    /// `--since-revision` here still falls through to `consume_arg`'s
    /// generic "unsupported native argument" refusal, exactly as before this
    /// change — see `native_run_since_revision_is_still_refused` in
    /// `tests/command.rs`.
    fn parse_run(args: impl IntoIterator<Item = OsString>) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse_reason("run", args)?;
        if options.run_step == options.run_frontier {
            return Err(NativeCliError::usage(
                "run requires exactly one of --step or --frontier",
            ));
        }
        let mode = if options.run_step {
            "run --step"
        } else {
            "run --frontier"
        };
        let RunFamilyGate {
            actor_id,
            gate_options,
        } = resolve_run_family_gate(&options, mode)?;
        if options.run_step {
            Ok(Self::RunStep {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                plan_id: options.require_id("--plan-id")?,
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                actor_id,
                enabled_worker_kinds: options.enabled_worker_kinds,
                retry_step_id: options.retry_step_ids.last().cloned(),
                supersede_trace_ids: options.supersede_trace_ids,
                gate_options,
                strict: options.strict,
                format: options.format,
                output: options.output,
            })
        } else {
            let max_parallel = resolve_max_parallel(&options)?;
            Ok(Self::RunFrontier {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                plan_id: options.require_id("--plan-id")?,
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                actor_id,
                enabled_worker_kinds: options.enabled_worker_kinds,
                retry_step_ids: options.retry_step_ids,
                supersede_trace_ids: options.supersede_trace_ids,
                max_parallel,
                gate_options,
                strict: options.strict,
                format: options.format,
                output: options.output,
            })
        }
    }

    /// ADR 0016 decision 3: `operate` repeats exactly the round selection
    /// `run --frontier` performs, so it takes the same gate and dispatch
    /// flags `run --frontier` does, plus the one flag that bounds the loop.
    /// `--max-rounds` has no default, unlike `--max-parallel` — decision 6
    /// makes it the thing that keeps "one gate authorizes the invocation"
    /// from meaning "unbounded work", so a caller states the bound rather
    /// than inheriting one silently.
    /// `NativeOptions::parse_reason`, for the same reason and with the same
    /// `--since-revision` non-reopening as `parse_run` — see its doc
    /// comment.
    fn parse_operate(args: impl IntoIterator<Item = OsString>) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse_reason("operate", args)?;
        let RunFamilyGate {
            actor_id,
            gate_options,
        } = resolve_run_family_gate(&options, "operate")?;
        let max_parallel = resolve_max_parallel(&options)?;
        let max_rounds = options
            .max_rounds
            .ok_or_else(|| NativeCliError::usage("--max-rounds <n> is required for operate"))?;
        if max_rounds == 0 {
            return Err(NativeCliError::usage("--max-rounds must be at least 1"));
        }
        // Retry is an act between invocations (ADR 0002/0004), never a
        // standing flag the loop re-applies every round: a step named once
        // stays exempt from `select_steps`'s failed-trace gate for the
        // invocation's whole life, so a step that fails again after being
        // retried is retried again automatically — an auto-retry loop
        // bounded only by `--max-rounds`. Run `run --frontier --retry-step
        // <id>` explicitly, then `operate` again.
        if !options.retry_step_ids.is_empty() {
            return Err(NativeCliError::usage(
                "--retry-step is not accepted by operate; retry is an explicit act between \
                 invocations — run `run --frontier --retry-step <id>` first, then operate again",
            ));
        }
        Ok(Self::Operate {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            plan_id: options.require_id("--plan-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            actor_id,
            enabled_worker_kinds: options.enabled_worker_kinds,
            supersede_trace_ids: options.supersede_trace_ids,
            max_parallel,
            max_rounds,
            gate_options,
            strict: options.strict,
            format: options.format,
            output: options.output,
        })
    }

    fn parse_review(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let action = match operation.to_str() {
            Some("accept") => ReviewAction::Accept,
            Some("reject") => ReviewAction::Reject,
            Some("reopen") => ReviewAction::Reopen,
            Some("waive") => ReviewAction::Waive,
            Some(_) | None => {
                return Err(NativeCliError::usage("unsupported native review command"))
            }
        };
        let options = NativeOptions::parse("review", args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "review",
                actor_command: Some("review"),
            })?;
        Ok(Self::Review {
            action,
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            target_id: options.require_id("--target-id")?,
            reviewer_id: options.require_id("--reviewer-id")?,
            reason: options.require_string("--reason")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            gate_options,
            evidence_ids: options.evidence_ids,
            output: options.output,
        })
    }

    fn parse_evidence(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        if operation.to_str() != Some("attach") {
            return Err(NativeCliError::usage("unsupported native evidence command"));
        }
        let options = NativeOptions::parse("evidence", args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "evidence attach",
                actor_command: Some("evidence attach"),
            })?;
        options.require_path("--input")?;
        Ok(Self::EvidenceAttach {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            attachments: options.evidence_attachments,
            gate_options,
            output: options.output,
        })
    }

    fn parse_cell(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        if operation.to_str() != Some("transition") {
            return Err(NativeCliError::usage("unsupported native cell command"));
        }
        let options = NativeOptions::parse("cell", args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "cell transition",
                actor_command: Some("cell transition"),
            })?;
        Ok(Self::CellTransition {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            cell_id: options.require_id("--cell-id")?,
            lifecycle: options.require_string("--to")?,
            gate_options,
            reason: options.reason,
            output: options.output,
        })
    }

    /// `NativeOptions::parse_text_only`, not `parse_reason`: `packet apply`/
    /// `packet resume` have no `strict` field to carry `--strict` to (they
    /// were never `strict_allowed`), the same reason `space history` uses
    /// `parse_text_only` rather than `parse_reason` — see that method's own
    /// doc comment.
    fn parse_packet(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        match operation.to_str() {
            Some("apply") => {
                let options = NativeOptions::parse_text_only("packet", args)?;
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "packet apply",
                        actor_command: Some("packet apply"),
                    })?;
                Ok(Self::PacketApply {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    base_revision_id: options.base_revision_id.clone().ok_or_else(|| {
                        NativeCliError::usage("--base-revision-id <id> is required")
                    })?,
                    packet: options.require_path("--packet")?,
                    gate_options,
                    format: options.format,
                    output: options.output,
                })
            }
            Some("resume") => {
                let options = NativeOptions::parse_text_only("packet", args)?;
                let gate_options =
                    options.resolve_operation_gate_options(OperationGateRequirement::Required {
                        command: "packet resume",
                        actor_command: Some("packet resume"),
                    })?;
                Ok(Self::PacketResume {
                    store: options.require_store()?,
                    case_space_id: options.require_id("--case-space-id")?,
                    base_revision_id: options.base_revision_id.clone().ok_or_else(|| {
                        NativeCliError::usage("--base-revision-id <id> is required")
                    })?,
                    packet: options.require_path("--packet")?,
                    completed_through: options.require_id("--completed-through")?,
                    gate_options,
                    format: options.format,
                    output: options.output,
                })
            }
            Some(_) | None => Err(NativeCliError::usage("unsupported native packet command")),
        }
    }
}
