use super::{
    ops::{NativeCloseGateOptions, NativeRunGateOptions},
    options::{required_segment, NativeOptions, OperationGateRequirement},
    NativeCliCommand, NativeCliError, NativeReasonSection,
};
use crate::native_model::ReviewAction;
use std::ffi::OsString;

impl NativeCliCommand {
    pub fn parse(
        namespace: &str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let mut args = args.into_iter();
        match namespace {
            "space" => Self::parse_space(required_segment(&mut args, "space operation")?, args),
            "lift" => Self::parse_lift(required_segment(&mut args, "lift adapter")?, args),
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
            "binding" => {
                Self::parse_binding(required_segment(&mut args, "binding operation")?, args)
            }
            "run" => Self::parse_run(args),
            "review" => Self::parse_review(required_segment(&mut args, "review operation")?, args),
            "evidence" => {
                Self::parse_evidence(required_segment(&mut args, "evidence operation")?, args)
            }
            "cell" => Self::parse_cell(required_segment(&mut args, "cell operation")?, args),
            _ => Err(NativeCliError::usage("unsupported native namespace")),
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
        let options = if operation == "reason" {
            NativeOptions::parse_reason(args)?
        } else {
            NativeOptions::parse(args)?
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
            "reason" => Self::parse_reason(options, NativeReasonSection::Reason),
            "frontier" => Self::parse_reason(options, NativeReasonSection::Frontier),
            "evidence" => Self::parse_reason(options, NativeReasonSection::Evidence),
            "project" => Self::parse_reason(options, NativeReasonSection::Project),
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
        let options = NativeOptions::parse(args)?;
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
                NativeOptions::parse_with_strict(args)?,
                NativeReasonSection::Obstructions,
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
                NativeOptions::parse(args)?,
                NativeReasonSection::Completions,
            ),
            Some(_) | None => Err(NativeCliError::usage("unsupported completion command")),
        }
    }

    fn parse_projection(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse(args)?;
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
        let options = NativeOptions::parse(args)?;
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
        let options = NativeOptions::parse_with_strict(args)?;
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

    fn parse_reason(
        options: NativeOptions,
        section: NativeReasonSection,
    ) -> Result<Self, NativeCliError> {
        Ok(Self::CaseReason {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            section,
            strict: options.strict,
            format: options.format,
            output: options.output,
        })
    }

    fn parse_morphism(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let operation = operation
            .to_str()
            .ok_or_else(|| NativeCliError::usage("morphism operation must be UTF-8"))?;
        let options = NativeOptions::parse(args)?;
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
                        operation: "morphism-apply",
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
                        operation: "morphism-reject",
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
        let options = NativeOptions::parse(args)?;
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
                        operation: "plan-review",
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
        let options = NativeOptions::parse(args)?;
        match operation.to_str() {
            Some("register") => Ok(Self::BindingRegister {
                store: options.require_store()?,
                input: options.require_path("--input")?,
                output: options.output,
            }),
            Some(_) | None => Err(NativeCliError::usage("unsupported native binding command")),
        }
    }

    fn parse_run(args: impl IntoIterator<Item = OsString>) -> Result<Self, NativeCliError> {
        let options = NativeOptions::parse_with_strict(args)?;
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
        let resolved_gate =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: mode,
                operation: "dispatch",
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
        if options.run_step {
            Ok(Self::RunStep {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                plan_id: options.require_id("--plan-id")?,
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                actor_id: actor_id.clone(),
                enabled_worker_kinds: options.enabled_worker_kinds,
                retry_step_id: options.retry_step_ids.last().cloned(),
                gate_options,
                strict: options.strict,
                output: options.output,
            })
        } else {
            let max_parallel = options.max_parallel.unwrap_or(4);
            if max_parallel == 0 {
                return Err(NativeCliError::usage("--max-parallel must be at least 1"));
            }
            Ok(Self::RunFrontier {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                plan_id: options.require_id("--plan-id")?,
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                actor_id: actor_id.clone(),
                enabled_worker_kinds: options.enabled_worker_kinds,
                retry_step_ids: options.retry_step_ids,
                max_parallel,
                gate_options,
                strict: options.strict,
                output: options.output,
            })
        }
    }

    fn parse_review(
        operation: OsString,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, NativeCliError> {
        let action = match operation.to_str() {
            Some("accept") => ReviewAction::Accept,
            Some("reject") => ReviewAction::Reject,
            Some("reopen") => ReviewAction::Reopen,
            Some("waive") => ReviewAction::Defer,
            Some(_) | None => {
                return Err(NativeCliError::usage("unsupported native review command"))
            }
        };
        let options = NativeOptions::parse(args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "review",
                operation: "review",
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
        let options = NativeOptions::parse(args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "evidence attach",
                operation: "evidence-attach",
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
        let options = NativeOptions::parse(args)?;
        let gate_options =
            options.resolve_operation_gate_options(OperationGateRequirement::Required {
                command: "cell transition",
                operation: "cell-transition",
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
}
