use super::{
    ops::{
        NativeCloseGateOptions, NativeMutationGateOptions, NativePlanGateOptions,
        NativeRunGateOptions,
    },
    options::{required_segment, NativeOptions},
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
        let options = NativeOptions::parse(args)?;
        match operation {
            "new" | "create" => Ok(Self::CaseNew {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                space_id: options.require_id("--space-id")?,
                title: options.require_string("--title")?,
                revision_id: options.require_id("--revision-id")?,
                output: options.output,
            }),
            "import" => Ok(Self::CaseImport {
                store: options.require_store()?,
                input: options.require_path("--input")?,
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
                NativeOptions::parse(args)?,
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
        let options = NativeOptions::parse(args)?;
        match operation.to_str() {
            Some("check") => Ok(Self::InvariantCheck {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                output: options.output,
            }),
            Some("close-check") => Self::parse_close_check(options),
            Some(_) | None => Err(NativeCliError::usage("unsupported invariant command")),
        }
    }

    fn parse_close_check(options: NativeOptions) -> Result<Self, NativeCliError> {
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
                actor_id: options.actor_id,
                capability_ids: options.capability_ids,
                operation_scope_id: options.operation_scope_id,
                audience: options.audience,
                source_boundary_id: options.source_boundary_id,
            },
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
            "apply" => Ok(Self::MorphismApply {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                morphism_id: options.require_id("--morphism-id")?,
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .or(options.revision_id.clone())
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                reviewer_id: Some(options.require_id("--reviewer-id")?),
                reason: Some(options.require_string("--reason")?),
                gate_options: mutation_gate_options(&options),
                output: options.output,
            }),
            "reject" => Ok(Self::MorphismReject {
                store: options.require_store()?,
                case_space_id: options.require_id("--case-space-id")?,
                morphism_id: options.require_id("--morphism-id")?,
                reviewer_id: options.require_id("--reviewer-id")?,
                reason: options.require_string("--reason")?,
                revision_id: options.require_id("--revision-id")?,
                gate_options: mutation_gate_options(&options),
                output: options.output,
            }),
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
            "accept" | "reject" => Ok(Self::PlanReview {
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
                base_revision_id: options
                    .base_revision_id
                    .clone()
                    .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
                gate_options: NativePlanGateOptions {
                    actor_id: options.actor_id,
                    capability_ids: options.capability_ids,
                    operation_scope_id: options.operation_scope_id,
                    audience: options.audience,
                    source_boundary_id: options.source_boundary_id,
                },
                output: options.output,
            }),
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
        let options = NativeOptions::parse(args)?;
        if options.run_step == options.run_frontier {
            return Err(NativeCliError::usage(
                "run requires exactly one of --step or --frontier",
            ));
        }
        let actor_id = options.require_id("--actor-id")?;
        if let Some(gate_actor_id) = &options.gate_actor_id {
            if gate_actor_id != &actor_id {
                return Err(NativeCliError::usage(
                    "--gate-actor-id is a compatibility alias and must equal --actor-id",
                ));
            }
        }
        let mode = if options.run_step {
            "run --step"
        } else {
            "run --frontier"
        };
        let gate_options = NativeRunGateOptions {
            actor_id: actor_id.clone(),
            capability_ids: options.capability_ids.clone(),
            operation_scope_id: options.operation_scope_id.clone().ok_or_else(|| {
                NativeCliError::usage(format!("--operation-scope-id <id> is required for {mode}"))
            })?,
            audience: options.audience.ok_or_else(|| {
                NativeCliError::usage(format!("--audience audit|system is required for {mode}"))
            })?,
            source_boundary_id: options.source_boundary_id.clone().ok_or_else(|| {
                NativeCliError::usage(format!("--source-boundary-id <id> is required for {mode}"))
            })?,
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
            gate_options: mutation_gate_options(&options),
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
        Ok(Self::EvidenceAttach {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            input: options.require_path("--input")?,
            gate_options: mutation_gate_options(&options),
            satisfies_ids: options.satisfies_ids,
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
        Ok(Self::CellTransition {
            store: options.require_store()?,
            case_space_id: options.require_id("--case-space-id")?,
            base_revision_id: options
                .base_revision_id
                .clone()
                .ok_or_else(|| NativeCliError::usage("--base-revision-id <id> is required"))?,
            cell_id: options.require_id("--cell-id")?,
            lifecycle: options.require_string("--to")?,
            gate_options: mutation_gate_options(&options),
            reason: options.reason,
            output: options.output,
        })
    }
}

fn mutation_gate_options(options: &NativeOptions) -> NativeMutationGateOptions {
    NativeMutationGateOptions {
        actor_id: options.actor_id.clone(),
        capability_ids: options.capability_ids.clone(),
        operation_scope_id: options.operation_scope_id.clone(),
        audience: options.audience,
        source_boundary_id: options.source_boundary_id.clone(),
    }
}
