use super::{
    append_validated_morphism,
    binding::read_registered_worker_binding,
    io::{read_json, timestamp, write_json},
    require_current_revision, NativePlanGateOptions, NativePlanReviewOptions,
};
use crate::{
    exec::{
        binding::worker_binding_content_hash, execution_plan_content_hash, validate_execution_plan,
        ExecutionPlan,
    },
    native_eval::evaluate_native_case,
    native_model::{CaseSpace, ReviewAction},
    native_review::{
        accept_review_morphism, check_operation_gate, reject_review_morphism, NativeOperationGate,
        NativeReviewRequest, NativeReviewTargetKind,
    },
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::super::{
    path_helpers::{path_segment, relative_store_path},
    reporting::report,
    NativeCliError,
};

const PLAN_DIRECTORY: &str = "plans";

pub(in crate::native_cli) fn plan_propose(
    store: &Path,
    case_space_id: &Id,
    input: &Path,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let mut plan = read_plan_file(input)?;
    validate_plan_for_current_case_space(&plan, case_space_id, &replay.case_space)?;
    record_worker_binding_hashes(store, &mut plan)?;
    let path = plan_path(store, &plan.plan_id);
    if path.exists() {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} already exists at {}",
            plan.plan_id,
            path.display()
        )));
    }
    let content_hash = plan_content_hash(&plan)?;
    write_plan_file(&path, &plan)?;

    Ok(report(
        "casegraphen plan propose",
        json!({
            "plan_status": "proposed",
            "plan_path": relative_store_path(store, &path),
            "plan_content_hash": content_hash,
            "plan": plan,
        }),
    ))
}

pub(in crate::native_cli) fn plan_check(
    store: &Path,
    case_space_id: &Id,
    plan_id: &Id,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let path = plan_path(store, plan_id);
    let plan = read_stored_plan(&path, plan_id)?;
    validate_plan_for_current_case_space(&plan, case_space_id, &replay.case_space)?;
    let evaluation = evaluate_native_case(&replay.case_space)?;
    let frontier = evaluation.frontier_cell_ids.iter().collect::<BTreeSet<_>>();
    let step_readiness = plan
        .steps
        .iter()
        .map(|step| {
            json!({
                "step_id": step.step_id,
                "work_cell_id": step.work_cell_id,
                "on_readiness_frontier": frontier.contains(&step.work_cell_id),
            })
        })
        .collect::<Vec<_>>();

    Ok(report(
        "casegraphen plan check",
        json!({
            "plan_status": "checked",
            "plan_path": relative_store_path(store, &path),
            "plan_content_hash": plan_content_hash(&plan)?,
            "plan": plan,
            "frontier_cell_ids": evaluation.frontier_cell_ids,
            "step_readiness": step_readiness,
        }),
    ))
}

pub(in crate::native_cli) fn plan_review(
    store: &Path,
    case_space_id: &Id,
    options: NativePlanReviewOptions<'_>,
) -> Result<Value, NativeCliError> {
    if !matches!(options.action, ReviewAction::Accept | ReviewAction::Reject) {
        return Err(NativeCliError::invalid(
            "plan review action must be accept or reject",
        ));
    }
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, options.base_revision_id)?;
    let path = plan_path(store, options.plan_id);
    let mut plan = read_stored_plan(&path, options.plan_id)?;
    validate_plan_for_current_case_space(&plan, case_space_id, &replay.case_space)?;
    let content_hash = plan_content_hash(&plan)?;
    let operation_gate = if options.action == ReviewAction::Accept {
        let gate = resolve_plan_review_gate(options.gate_options)?;
        check_operation_gate(&replay.case_space, &gate, "plan-review")
            .map_err(|error| NativeCliError::invalid(error.to_string()))?;
        Some(gate)
    } else {
        None
    };
    let request = NativeReviewRequest {
        target_kind: NativeReviewTargetKind::Plan,
        target_id: plan.plan_id.clone(),
        action: options.action,
        reviewer_id: options.reviewer_id.clone(),
        reviewed_at: timestamp(),
        reason: options.reason.to_owned(),
        evidence_ids: Vec::new(),
        source_ids: vec![plan.plan_id.clone()],
        target_revision_id: Id::new(format!(
            "revision:plan-review:{}:{}",
            path_segment(&plan.plan_id),
            replay.case_space.morphism_log.len() + 1
        ))?,
    };
    let mut morphism = match options.action {
        ReviewAction::Accept => accept_review_morphism(&replay.case_space, request)?,
        ReviewAction::Reject => reject_review_morphism(&replay.case_space, request)?,
        _ => unreachable!("plan action checked above"),
    };
    morphism.metadata.insert(
        "plan_content_hash".to_owned(),
        Value::String(content_hash.clone()),
    );
    let actor_id = operation_gate
        .as_ref()
        .map(|gate| gate.actor_id.clone())
        .unwrap_or_else(|| options.reviewer_id.clone());
    let command = if options.action == ReviewAction::Accept {
        "casegraphen plan accept"
    } else {
        "casegraphen plan reject"
    };
    let mut review_report = append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(actor_id),
        command,
    )?;

    plan.review_status = if options.action == ReviewAction::Accept {
        ReviewStatus::Accepted
    } else {
        ReviewStatus::Rejected
    };
    write_plan_file(&path, &plan)?;
    review_report["result"]["plan"] = json!(plan);
    review_report["result"]["plan_path"] = json!(relative_store_path(store, &path));
    review_report["result"]["plan_content_hash"] = json!(content_hash);
    if let Some(gate) = operation_gate {
        review_report["result"]["operation_gate"] = json!(gate);
    }
    Ok(review_report)
}

fn resolve_plan_review_gate(
    options: &NativePlanGateOptions,
) -> Result<NativeOperationGate, NativeCliError> {
    Ok(NativeOperationGate {
        actor_id: options
            .actor_id
            .clone()
            .ok_or_else(|| NativeCliError::usage("--actor-id <id> is required for plan accept"))?,
        operation: "plan-review".to_owned(),
        operation_scope_id: options.operation_scope_id.clone().ok_or_else(|| {
            NativeCliError::usage("--operation-scope-id <id> is required for plan accept")
        })?,
        audience: options.audience.ok_or_else(|| {
            NativeCliError::usage("--audience audit|system is required for plan accept")
        })?,
        capability_ids: options.capability_ids.clone(),
        source_boundary_id: options.source_boundary_id.clone().ok_or_else(|| {
            NativeCliError::usage("--source-boundary-id <id> is required for plan accept")
        })?,
    })
}

fn validate_plan_for_current_case_space(
    plan: &ExecutionPlan,
    case_space_id: &Id,
    case_space: &CaseSpace,
) -> Result<(), NativeCliError> {
    validate_execution_plan(plan).map_err(|error| NativeCliError::invalid(error.to_string()))?;
    if plan.case_space_id != *case_space_id {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} belongs to case space {}, expected {}",
            plan.plan_id, plan.case_space_id, case_space_id
        )));
    }
    if plan.review_status != ReviewStatus::Unreviewed {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} review_status must be unreviewed",
            plan.plan_id
        )));
    }
    if plan.base_revision_id != case_space.revision.revision_id {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} base revision {} is stale; current revision is {}",
            plan.plan_id, plan.base_revision_id, case_space.revision.revision_id
        )));
    }
    let cell_ids = case_space
        .case_cells
        .iter()
        .map(|cell| &cell.id)
        .collect::<BTreeSet<_>>();
    let missing_work_cell_ids = plan
        .steps
        .iter()
        .map(|step| &step.work_cell_id)
        .filter(|id| !cell_ids.contains(*id))
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing_work_cell_ids.is_empty() {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} names missing work_cell_id values: {}",
            plan.plan_id,
            missing_work_cell_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

pub(super) fn read_stored_plan(
    path: &Path,
    expected_plan_id: &Id,
) -> Result<ExecutionPlan, NativeCliError> {
    let plan = read_plan_file(path)?;
    if plan.plan_id != *expected_plan_id {
        return Err(NativeCliError::invalid(format!(
            "{}: execution plan id {} does not match requested {}",
            path.display(),
            plan.plan_id,
            expected_plan_id
        )));
    }
    Ok(plan)
}

fn read_plan_file(path: &Path) -> Result<ExecutionPlan, NativeCliError> {
    serde_json::from_value(read_json(path)?).map_err(NativeCliError::from)
}

fn write_plan_file(path: &Path, plan: &ExecutionPlan) -> Result<(), NativeCliError> {
    write_json(path, &serde_json::to_value(plan)?)
}

fn plan_content_hash(plan: &ExecutionPlan) -> Result<String, NativeCliError> {
    execution_plan_content_hash(plan).map_err(NativeCliError::from)
}

pub(super) fn plan_path(store: &Path, plan_id: &Id) -> PathBuf {
    store
        .join(PLAN_DIRECTORY)
        .join(format!("{}.execution.plan.json", path_segment(plan_id)))
}

fn record_worker_binding_hashes(
    store: &Path,
    plan: &mut ExecutionPlan,
) -> Result<(), NativeCliError> {
    let mut hashes = serde_json::Map::new();
    for binding_id in plan
        .steps
        .iter()
        .map(|step| &step.worker_binding_id)
        .collect::<BTreeSet<_>>()
    {
        let binding = read_registered_worker_binding(store, binding_id)?;
        hashes.insert(
            binding_id.to_string(),
            Value::String(worker_binding_content_hash(&binding)?),
        );
    }
    plan.metadata
        .insert("worker_binding_hashes".to_owned(), Value::Object(hashes));
    Ok(())
}
