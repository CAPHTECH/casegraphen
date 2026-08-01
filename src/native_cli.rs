use crate::native_eval::evaluate_native_case;
use crate::native_model::{ProjectionAudience, ReviewAction};
use crate::native_store::{NativeCaseStore, NativeStoreError};
use crate::topology::TopologyReportOptions;
use higher_graphen_core::Id;
use serde_json::{json, Value};
use std::{
    fmt,
    path::{Path, PathBuf},
};

mod ops;
#[path = "native_cli_options.rs"]
mod options;
pub(crate) use options::NativeOutputFormat;
pub use options::OPERATION_GATE_PROFILES_SCHEMA;
mod parser;
#[path = "native_cli_path.rs"]
mod path_helpers;
#[path = "native_cli_reporting.rs"]
mod reporting;
#[path = "native_cli_text.rs"]
mod text;
pub use ops::EVIDENCE_PACKET_SCHEMA;
use ops::{
    binding_register, case_close_check, case_import, case_new, case_reason, case_topology,
    case_topology_diff, cell_transition, evidence_attach, lift_structured_source, morphism_apply,
    morphism_check, morphism_propose, morphism_reject, packet_apply, packet_resume, plan_check,
    plan_propose, plan_review, projection_apply, review_apply, run_frontier, run_step,
    NativeCloseGateOptions, NativeMutationGateOptions, NativePlanGateOptions,
    NativePlanReviewOptions, NativeReviewApplyOptions, NativeRunFrontierOptions,
    NativeRunGateOptions, NativeRunStepOptions,
};
use reporting::report;
use text::render_native_case_evaluation;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeEvidenceAttachment {
    input: PathBuf,
    satisfies_ids: Vec<Id>,
    artifact_paths: Vec<PathBuf>,
}

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(crate) struct NativeCommandResult<T> {
    output: T,
    domain_finding: bool,
}

impl<T> NativeCommandResult<T> {
    pub(super) fn success(output: T) -> Self {
        Self {
            output,
            domain_finding: false,
        }
    }

    pub(super) fn with_domain_finding(output: T, domain_finding: bool) -> Self {
        Self {
            output,
            domain_finding,
        }
    }

    pub(crate) fn into_parts(self) -> (T, bool) {
        (self.output, self.domain_finding)
    }

    fn map<U>(self, map: impl FnOnce(T) -> U) -> NativeCommandResult<U> {
        NativeCommandResult {
            output: map(self.output),
            domain_finding: self.domain_finding,
        }
    }
}

fn evaluation_carries_domain_finding(
    evaluation: &crate::native_eval::NativeCaseEvaluation,
) -> bool {
    !evaluation.obstructions.is_empty()
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NativeCliCommand {
    CaseNew {
        store: PathBuf,
        case_space_id: Id,
        space_id: Id,
        title: String,
        revision_id: Id,
        output: Option<PathBuf>,
    },
    CaseImport {
        store: PathBuf,
        input: PathBuf,
        revision_id: Id,
        output: Option<PathBuf>,
    },
    LiftStructuredSource {
        store: PathBuf,
        input: PathBuf,
        revision_id: Id,
        adapter: String,
        output: Option<PathBuf>,
    },
    CaseList {
        store: PathBuf,
        output: Option<PathBuf>,
    },
    CaseInspect {
        store: PathBuf,
        case_space_id: Id,
        output: Option<PathBuf>,
    },
    CaseHistory {
        store: PathBuf,
        case_space_id: Id,
        output: Option<PathBuf>,
    },
    CaseReplay {
        store: PathBuf,
        case_space_id: Id,
        output: Option<PathBuf>,
    },
    CaseRebuild {
        store: PathBuf,
        case_space_id: Id,
        adopt_existing_log: bool,
        output: Option<PathBuf>,
    },
    CaseValidate {
        store: PathBuf,
        case_space_id: Id,
        output: Option<PathBuf>,
    },
    InvariantCheck {
        store: PathBuf,
        case_space_id: Id,
        strict: bool,
        output: Option<PathBuf>,
    },
    CaseReason {
        store: PathBuf,
        case_space_id: Id,
        section: NativeReasonSection,
        strict: bool,
        format: NativeOutputFormat,
        output: Option<PathBuf>,
    },
    ProjectionApply {
        store: PathBuf,
        case_space_id: Id,
        projection: PathBuf,
        output: Option<PathBuf>,
    },
    CaseCloseCheck {
        store: PathBuf,
        case_space_id: Id,
        base_revision_id: Id,
        validation_evidence_ids: Vec<Id>,
        gate_options: NativeCloseGateOptions,
        strict: bool,
        output: Option<PathBuf>,
    },
    CaseTopology {
        store: PathBuf,
        case_space_id: Id,
        topology_options: TopologyReportOptions,
        output: Option<PathBuf>,
    },
    CaseTopologyDiff {
        left_store: PathBuf,
        left_case_space_id: Id,
        right_store: PathBuf,
        right_case_space_id: Id,
        topology_options: TopologyReportOptions,
        output: Option<PathBuf>,
    },
    EquivalenceCheck {
        left_store: PathBuf,
        left_case_space_id: Id,
        right_store: PathBuf,
        right_case_space_id: Id,
        topology_options: TopologyReportOptions,
        output: Option<PathBuf>,
    },
    MorphismPropose {
        store: PathBuf,
        case_space_id: Id,
        input: PathBuf,
        output: Option<PathBuf>,
    },
    MorphismCheck {
        store: PathBuf,
        case_space_id: Id,
        morphism_id: Id,
        output: Option<PathBuf>,
    },
    MorphismApply {
        store: PathBuf,
        case_space_id: Id,
        morphism_id: Id,
        base_revision_id: Id,
        reviewer_id: Option<Id>,
        reason: Option<String>,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    MorphismReject {
        store: PathBuf,
        case_space_id: Id,
        morphism_id: Id,
        reviewer_id: Id,
        reason: String,
        revision_id: Id,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    PlanPropose {
        store: PathBuf,
        case_space_id: Id,
        input: PathBuf,
        output: Option<PathBuf>,
    },
    PlanCheck {
        store: PathBuf,
        case_space_id: Id,
        plan_id: Id,
        output: Option<PathBuf>,
    },
    PlanReview {
        action: ReviewAction,
        store: PathBuf,
        case_space_id: Id,
        plan_id: Id,
        reviewer_id: Id,
        reason: String,
        base_revision_id: Id,
        gate_options: NativePlanGateOptions,
        output: Option<PathBuf>,
    },
    BindingRegister {
        store: PathBuf,
        input: PathBuf,
        output: Option<PathBuf>,
    },
    RunStep {
        store: PathBuf,
        case_space_id: Id,
        plan_id: Id,
        base_revision_id: Id,
        actor_id: Id,
        enabled_worker_kinds: Vec<String>,
        retry_step_id: Option<Id>,
        supersede_trace_ids: Vec<Id>,
        gate_options: NativeRunGateOptions,
        strict: bool,
        output: Option<PathBuf>,
    },
    RunFrontier {
        store: PathBuf,
        case_space_id: Id,
        plan_id: Id,
        base_revision_id: Id,
        actor_id: Id,
        enabled_worker_kinds: Vec<String>,
        retry_step_ids: Vec<Id>,
        supersede_trace_ids: Vec<Id>,
        max_parallel: usize,
        gate_options: NativeRunGateOptions,
        strict: bool,
        output: Option<PathBuf>,
    },
    Review {
        action: ReviewAction,
        store: PathBuf,
        case_space_id: Id,
        target_id: Id,
        reviewer_id: Id,
        reason: String,
        base_revision_id: Id,
        evidence_ids: Vec<Id>,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    EvidenceAttach {
        store: PathBuf,
        case_space_id: Id,
        base_revision_id: Id,
        attachments: Vec<NativeEvidenceAttachment>,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    CellTransition {
        store: PathBuf,
        case_space_id: Id,
        base_revision_id: Id,
        cell_id: Id,
        lifecycle: String,
        reason: Option<String>,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    PacketApply {
        store: PathBuf,
        case_space_id: Id,
        base_revision_id: Id,
        packet: PathBuf,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
    PacketResume {
        store: PathBuf,
        case_space_id: Id,
        base_revision_id: Id,
        packet: PathBuf,
        completed_through: Id,
        gate_options: NativeMutationGateOptions,
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeReasonSection {
    Reason,
    Frontier,
    Obstructions,
    Completions,
    Evidence,
    Project,
}

impl NativeCliCommand {
    pub fn output(&self) -> Option<&PathBuf> {
        match self {
            Self::CaseNew { output, .. }
            | Self::CaseImport { output, .. }
            | Self::LiftStructuredSource { output, .. }
            | Self::CaseList { output, .. }
            | Self::CaseInspect { output, .. }
            | Self::CaseHistory { output, .. }
            | Self::CaseReplay { output, .. }
            | Self::CaseRebuild { output, .. }
            | Self::CaseValidate { output, .. }
            | Self::InvariantCheck { output, .. }
            | Self::CaseReason { output, .. }
            | Self::ProjectionApply { output, .. }
            | Self::CaseCloseCheck { output, .. }
            | Self::CaseTopology { output, .. }
            | Self::CaseTopologyDiff { output, .. }
            | Self::EquivalenceCheck { output, .. }
            | Self::MorphismPropose { output, .. }
            | Self::MorphismCheck { output, .. }
            | Self::MorphismApply { output, .. }
            | Self::MorphismReject { output, .. }
            | Self::PlanPropose { output, .. }
            | Self::PlanCheck { output, .. }
            | Self::PlanReview { output, .. }
            | Self::BindingRegister { output, .. }
            | Self::RunStep { output, .. }
            | Self::RunFrontier { output, .. }
            | Self::Review { output, .. }
            | Self::EvidenceAttach { output, .. }
            | Self::CellTransition { output, .. }
            | Self::PacketApply { output, .. }
            | Self::PacketResume { output, .. } => output.as_ref(),
        }
    }

    pub fn run_json(&self) -> Result<NativeCommandResult<String>, NativeCliError> {
        let result = self.run_value()?;
        let (value, domain_finding) = result.into_parts();
        let json = serde_json::to_string(&value)?;
        Ok(NativeCommandResult::with_domain_finding(
            json,
            domain_finding,
        ))
    }

    pub fn run_rendered(&self) -> Result<NativeCommandResult<String>, NativeCliError> {
        match self {
            Self::CaseReason {
                store,
                case_space_id,
                section: NativeReasonSection::Reason,
                format: NativeOutputFormat::Text,
                ..
            } => ops::case_reason_evaluation(store, case_space_id)
                .map(|result| result.map(|evaluation| render_native_case_evaluation(&evaluation))),
            _ => self.run_json(),
        }
    }

    pub fn format(&self) -> NativeOutputFormat {
        match self {
            Self::CaseReason { format, .. } => *format,
            _ => NativeOutputFormat::Json,
        }
    }

    pub fn strict(&self) -> bool {
        match self {
            Self::InvariantCheck { strict, .. }
            | Self::CaseReason { strict, .. }
            | Self::CaseCloseCheck { strict, .. }
            | Self::RunStep { strict, .. }
            | Self::RunFrontier { strict, .. } => *strict,
            _ => false,
        }
    }

    fn run_value(&self) -> Result<NativeCommandResult<Value>, NativeCliError> {
        match self {
            Self::CaseNew { .. }
            | Self::CaseImport { .. }
            | Self::LiftStructuredSource { .. }
            | Self::CaseList { .. }
            | Self::CaseInspect { .. }
            | Self::CaseHistory { .. }
            | Self::CaseReplay { .. }
            | Self::CaseRebuild { .. }
            | Self::CaseValidate { .. }
            | Self::InvariantCheck { .. }
            | Self::CaseReason { .. }
            | Self::ProjectionApply { .. }
            | Self::CaseCloseCheck { .. }
            | Self::CaseTopology { .. }
            | Self::CaseTopologyDiff { .. }
            | Self::EquivalenceCheck { .. } => self.run_case_value(),
            Self::MorphismPropose { .. }
            | Self::MorphismCheck { .. }
            | Self::MorphismApply { .. }
            | Self::MorphismReject { .. }
            | Self::PlanPropose { .. }
            | Self::PlanCheck { .. }
            | Self::PlanReview { .. }
            | Self::BindingRegister { .. }
            | Self::RunStep { .. }
            | Self::RunFrontier { .. }
            | Self::Review { .. }
            | Self::EvidenceAttach { .. }
            | Self::CellTransition { .. }
            | Self::PacketApply { .. }
            | Self::PacketResume { .. } => self.run_morphism_value(),
        }
    }

    fn run_case_value(&self) -> Result<NativeCommandResult<Value>, NativeCliError> {
        match self {
            Self::CaseNew { .. }
            | Self::CaseImport { .. }
            | Self::LiftStructuredSource { .. }
            | Self::CaseList { .. }
            | Self::CaseInspect { .. }
            | Self::CaseHistory { .. }
            | Self::CaseReplay { .. }
            | Self::CaseRebuild { .. }
            | Self::CaseValidate { .. } => self.run_case_store_value(),
            Self::InvariantCheck { .. }
            | Self::CaseReason { .. }
            | Self::ProjectionApply { .. }
            | Self::CaseCloseCheck { .. }
            | Self::CaseTopology { .. }
            | Self::CaseTopologyDiff { .. }
            | Self::EquivalenceCheck { .. } => self.run_case_analysis_value(),
            _ => unreachable!("run_case_value called for morphism command"),
        }
    }

    fn run_case_store_value(&self) -> Result<NativeCommandResult<Value>, NativeCliError> {
        let value = match self {
            Self::CaseNew {
                store,
                case_space_id,
                space_id,
                title,
                revision_id,
                ..
            } => case_new(store, case_space_id, space_id, title, revision_id)?,
            Self::CaseImport {
                store,
                input,
                revision_id,
                ..
            } => case_import(store, input, revision_id)?,
            Self::LiftStructuredSource {
                store,
                input,
                revision_id,
                adapter,
                ..
            } => lift_structured_source(store, input, revision_id, adapter)?,
            Self::CaseList { store, .. } => case_list(store)?,
            Self::CaseInspect {
                store,
                case_space_id,
                ..
            } => case_inspect(store, case_space_id)?,
            Self::CaseHistory {
                store,
                case_space_id,
                ..
            } => case_history(store, case_space_id)?,
            Self::CaseReplay {
                store,
                case_space_id,
                ..
            } => case_replay(store, case_space_id)?,
            Self::CaseRebuild {
                store,
                case_space_id,
                adopt_existing_log,
                ..
            } => case_rebuild(store, case_space_id, *adopt_existing_log)?,
            Self::CaseValidate {
                store,
                case_space_id,
                ..
            } => case_validate(store, case_space_id)?,
            _ => unreachable!("run_case_store_value called for analysis command"),
        };
        Ok(NativeCommandResult::success(value))
    }

    fn run_case_analysis_value(&self) -> Result<NativeCommandResult<Value>, NativeCliError> {
        match self {
            Self::InvariantCheck {
                store,
                case_space_id,
                ..
            } => invariant_check(store, case_space_id),
            Self::CaseReason {
                store,
                case_space_id,
                section,
                ..
            } => case_reason(store, case_space_id, *section),
            Self::ProjectionApply {
                store,
                case_space_id,
                projection,
                ..
            } => {
                projection_apply(store, case_space_id, projection).map(NativeCommandResult::success)
            }
            Self::CaseCloseCheck {
                store,
                case_space_id,
                base_revision_id,
                validation_evidence_ids,
                gate_options,
                ..
            } => case_close_check(
                store,
                case_space_id,
                base_revision_id,
                validation_evidence_ids,
                gate_options.clone(),
            ),
            Self::CaseTopology {
                store,
                case_space_id,
                topology_options,
                ..
            } => case_topology(store, case_space_id, *topology_options)
                .map(NativeCommandResult::success),
            Self::CaseTopologyDiff {
                left_store,
                left_case_space_id,
                right_store,
                right_case_space_id,
                topology_options,
                ..
            } => case_topology_diff(
                left_store,
                left_case_space_id,
                right_store,
                right_case_space_id,
                *topology_options,
            )
            .map(NativeCommandResult::success),
            Self::EquivalenceCheck {
                left_store,
                left_case_space_id,
                right_store,
                right_case_space_id,
                topology_options,
                ..
            } => equivalence_check(
                left_store,
                left_case_space_id,
                right_store,
                right_case_space_id,
                *topology_options,
            )
            .map(NativeCommandResult::success),
            _ => unreachable!("run_case_analysis_value called for store command"),
        }
    }

    fn run_morphism_value(&self) -> Result<NativeCommandResult<Value>, NativeCliError> {
        let value = match self {
            Self::MorphismPropose {
                store,
                case_space_id,
                input,
                ..
            } => morphism_propose(store, case_space_id, input)?,
            Self::MorphismCheck {
                store,
                case_space_id,
                morphism_id,
                ..
            } => morphism_check(store, case_space_id, morphism_id)?,
            Self::MorphismApply {
                store,
                case_space_id,
                morphism_id,
                base_revision_id,
                reviewer_id,
                reason,
                gate_options,
                ..
            } => morphism_apply(
                store,
                case_space_id,
                morphism_id,
                base_revision_id,
                reviewer_id.as_ref(),
                reason.as_deref(),
                gate_options,
            )?,
            Self::MorphismReject {
                store,
                case_space_id,
                morphism_id,
                reviewer_id,
                reason,
                revision_id,
                gate_options,
                ..
            } => morphism_reject(
                store,
                case_space_id,
                morphism_id,
                reviewer_id,
                reason,
                revision_id,
                gate_options,
            )?,
            Self::PlanPropose {
                store,
                case_space_id,
                input,
                ..
            } => plan_propose(store, case_space_id, input)?,
            Self::PlanCheck {
                store,
                case_space_id,
                plan_id,
                ..
            } => plan_check(store, case_space_id, plan_id)?,
            Self::PlanReview {
                action,
                store,
                case_space_id,
                plan_id,
                reviewer_id,
                reason,
                base_revision_id,
                gate_options,
                ..
            } => plan_review(
                store,
                case_space_id,
                NativePlanReviewOptions {
                    plan_id,
                    action: *action,
                    reviewer_id,
                    reason,
                    base_revision_id,
                    gate_options,
                },
            )?,
            Self::BindingRegister { store, input, .. } => binding_register(store, input)?,
            Self::RunStep {
                store,
                case_space_id,
                plan_id,
                base_revision_id,
                actor_id,
                enabled_worker_kinds,
                retry_step_id,
                supersede_trace_ids,
                gate_options,
                ..
            } => {
                return run_step(
                    store,
                    NativeRunStepOptions {
                        case_space_id,
                        plan_id,
                        base_revision_id,
                        actor_id,
                        enabled_worker_kinds,
                        retry_step_id: retry_step_id.as_ref(),
                        supersede_trace_ids,
                        gate_options,
                    },
                )
            }
            Self::RunFrontier {
                store,
                case_space_id,
                plan_id,
                base_revision_id,
                actor_id,
                enabled_worker_kinds,
                retry_step_ids,
                supersede_trace_ids,
                max_parallel,
                gate_options,
                ..
            } => {
                return run_frontier(
                    store,
                    NativeRunFrontierOptions {
                        case_space_id,
                        plan_id,
                        base_revision_id,
                        actor_id,
                        enabled_worker_kinds,
                        retry_step_ids,
                        supersede_trace_ids,
                        max_parallel: *max_parallel,
                        gate_options,
                    },
                )
            }
            Self::Review {
                action,
                store,
                case_space_id,
                target_id,
                reviewer_id,
                reason,
                base_revision_id,
                evidence_ids,
                gate_options,
                ..
            } => review_apply(
                store,
                case_space_id,
                NativeReviewApplyOptions {
                    action: *action,
                    target_id,
                    reviewer_id,
                    reason,
                    base_revision_id,
                    evidence_ids,
                    gate_options,
                },
            )?,
            Self::EvidenceAttach {
                store,
                case_space_id,
                base_revision_id,
                attachments,
                gate_options,
                ..
            } => evidence_attach(
                store,
                case_space_id,
                base_revision_id,
                attachments,
                gate_options,
            )?,
            Self::CellTransition {
                store,
                case_space_id,
                base_revision_id,
                cell_id,
                lifecycle,
                reason,
                gate_options,
                ..
            } => cell_transition(
                store,
                case_space_id,
                base_revision_id,
                cell_id,
                lifecycle,
                reason.as_deref(),
                gate_options,
            )?,
            Self::PacketApply {
                store,
                case_space_id,
                base_revision_id,
                packet,
                gate_options,
                ..
            } => packet_apply(store, case_space_id, base_revision_id, packet, gate_options)?,
            Self::PacketResume {
                store,
                case_space_id,
                base_revision_id,
                packet,
                completed_through,
                gate_options,
                ..
            } => packet_resume(
                store,
                case_space_id,
                base_revision_id,
                packet,
                completed_through,
                gate_options,
            )?,
            _ => unreachable!("run_morphism_value called for case command"),
        };
        Ok(NativeCommandResult::success(value))
    }
}

pub(super) fn parse_projection_audience(value: &str) -> Result<ProjectionAudience, NativeCliError> {
    match value {
        "human_review" => Ok(ProjectionAudience::HumanReview),
        "ai_agent" => Ok(ProjectionAudience::AiAgent),
        "audit" => Ok(ProjectionAudience::Audit),
        "system" => Ok(ProjectionAudience::System),
        "migration" => Ok(ProjectionAudience::Migration),
        _ => Err(NativeCliError::usage(format!(
            "unsupported projection audience {value:?}"
        ))),
    }
}

fn case_list(store: &Path) -> Result<Value, NativeCliError> {
    let records = NativeCaseStore::new(store.to_path_buf()).list_case_spaces()?;
    Ok(report(
        "casegraphen space list",
        json!({ "case_spaces": records }),
    ))
}

fn case_inspect(store: &Path, case_space_id: &Id) -> Result<Value, NativeCliError> {
    let record = NativeCaseStore::new(store.to_path_buf()).inspect_case_space(case_space_id)?;
    Ok(report(
        "casegraphen space inspect",
        json!({ "record": record }),
    ))
}

fn case_history(store: &Path, case_space_id: &Id) -> Result<Value, NativeCliError> {
    let entries = NativeCaseStore::new(store.to_path_buf()).history_entries(case_space_id)?;
    Ok(report(
        "casegraphen space history",
        json!({ "entries": entries }),
    ))
}

fn case_replay(store: &Path, case_space_id: &Id) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    Ok(report(
        "casegraphen space replay",
        json!({ "replay": replay }),
    ))
}

fn case_rebuild(
    store: &Path,
    case_space_id: &Id,
    adopt_existing_log: bool,
) -> Result<Value, NativeCliError> {
    let store = NativeCaseStore::new(store.to_path_buf());
    let rebuild = if adopt_existing_log {
        store.rebuild_case_space_adopting_existing_log(case_space_id)?
    } else {
        store.rebuild_case_space(case_space_id)?
    };
    Ok(report(
        "casegraphen space rebuild",
        json!({ "rebuild": rebuild }),
    ))
}

fn case_validate(store: &Path, case_space_id: &Id) -> Result<Value, NativeCliError> {
    let validation =
        NativeCaseStore::new(store.to_path_buf()).validate_case_space(case_space_id)?;
    Ok(report(
        "casegraphen space validate",
        json!({ "validation": validation }),
    ))
}

fn invariant_check(
    store: &Path,
    case_space_id: &Id,
) -> Result<NativeCommandResult<Value>, NativeCliError> {
    let store = NativeCaseStore::new(store.to_path_buf());
    let validation = store.validate_case_space(case_space_id)?;
    let replay = store.replay_current_case_space(case_space_id)?;
    let evaluation = evaluate_native_case(&replay.case_space)?;
    let domain_finding = evaluation_carries_domain_finding(&evaluation);
    let evidence_findings = evaluation.evidence_findings.clone();
    let projection_loss = evaluation.projection_loss.clone();
    let obstructions = evaluation.obstructions.clone();
    let completion_candidates = evaluation.completion_candidates.clone();
    let review_gaps = evaluation.review_gaps.clone();
    Ok(NativeCommandResult::with_domain_finding(
        report(
            "casegraphen invariant check",
            json!({
            "validation": validation,
            "evaluation": evaluation,
            "evidence_findings": evidence_findings,
            "projection_loss": projection_loss,
            "obstructions": obstructions,
            "completion_candidates": completion_candidates,
            "review_gaps": review_gaps,
            }),
        ),
        domain_finding,
    ))
}

fn equivalence_check(
    left_store: &Path,
    left_case_space_id: &Id,
    right_store: &Path,
    right_case_space_id: &Id,
    topology_options: TopologyReportOptions,
) -> Result<Value, NativeCliError> {
    let left_replay = NativeCaseStore::new(left_store.to_path_buf())
        .replay_current_case_space(left_case_space_id)?;
    let right_replay = NativeCaseStore::new(right_store.to_path_buf())
        .replay_current_case_space(right_case_space_id)?;
    let left_topology = crate::topology::native_case_topology_with_history(
        &left_replay.case_space,
        &left_replay.history,
        topology_options,
    )?;
    let right_topology = crate::topology::native_case_topology_with_history(
        &right_replay.case_space,
        &right_replay.history,
        topology_options,
    )?;
    let topology_diff = crate::topology::topology_diff(&left_topology, &right_topology);
    Ok(report(
        "casegraphen equivalence check",
        json!({
            "left_case_space_id": left_case_space_id,
            "right_case_space_id": right_case_space_id,
            "topology_diff": topology_diff
        }),
    ))
}

#[derive(Debug)]
pub enum NativeCliError {
    Usage(String),
    Invalid(String),
    Core(higher_graphen_core::CoreError),
    Store(NativeStoreError),
    Review(crate::native_review::NativeReviewError),
    Eval(crate::native_eval::NativeEvalError),
    Worker(crate::exec::worker::WorkerError),
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json(serde_json::Error),
}

impl NativeCliError {
    fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl From<higher_graphen_core::CoreError> for NativeCliError {
    fn from(error: higher_graphen_core::CoreError) -> Self {
        Self::Core(error)
    }
}

impl From<NativeStoreError> for NativeCliError {
    fn from(error: NativeStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<crate::native_review::NativeReviewError> for NativeCliError {
    fn from(error: crate::native_review::NativeReviewError) -> Self {
        Self::Review(error)
    }
}

impl From<crate::native_eval::NativeEvalError> for NativeCliError {
    fn from(error: crate::native_eval::NativeEvalError) -> Self {
        Self::Eval(error)
    }
}

impl From<serde_json::Error> for NativeCliError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl fmt::Display for NativeCliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Invalid(message) => formatter.write_str(message),
            Self::Core(error) => write!(formatter, "{error}"),
            Self::Store(error) => write!(formatter, "{error}"),
            Self::Review(error) => write!(formatter, "{error}"),
            Self::Eval(error) => write!(formatter, "{error:?}"),
            Self::Worker(error) => write!(formatter, "{error}"),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Json(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for NativeCliError {}

impl From<crate::exec::worker::WorkerError> for NativeCliError {
    fn from(error: crate::exec::worker::WorkerError) -> Self {
        Self::Worker(error)
    }
}
