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
    native_model::{CaseMorphismType, CaseSpace, ReviewAction},
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
    validate_plan_shape_and_references(&plan, case_space_id, &replay.case_space)?;
    let prior_status = verified_plan_review_status(&plan, &replay.case_space)?;
    if prior_status == ReviewStatus::Unreviewed
        && plan.base_revision_id != replay.case_space.revision.revision_id
    {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} base revision {} is stale; current revision is {}",
            plan.plan_id, plan.base_revision_id, replay.case_space.revision.revision_id
        )));
    }
    let content_hash = plan_content_hash(&plan)?;
    let operation_gate = resolve_plan_review_gate(options.gate_options)?;
    check_operation_gate(&replay.case_space, &operation_gate, "plan-review")
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
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
    morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    let actor_id = operation_gate.actor_id.clone();
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
    review_report["result"]["operation_gate"] = json!(operation_gate);
    Ok(review_report)
}

fn resolve_plan_review_gate(
    options: &NativePlanGateOptions,
) -> Result<NativeOperationGate, NativeCliError> {
    Ok(NativeOperationGate {
        actor_id: options
            .actor_id
            .clone()
            .expect("required gate resolution checked actor_id"),
        operation: "plan-review".to_owned(),
        operation_scope_id: options
            .operation_scope_id
            .clone()
            .expect("required gate resolution checked operation_scope_id"),
        audience: options
            .audience
            .expect("required gate resolution checked audience"),
        capability_ids: options.capability_ids.clone(),
        source_boundary_id: options
            .source_boundary_id
            .clone()
            .expect("required gate resolution checked source_boundary_id"),
    })
}

fn validate_plan_for_current_case_space(
    plan: &ExecutionPlan,
    case_space_id: &Id,
    case_space: &CaseSpace,
) -> Result<(), NativeCliError> {
    validate_plan_shape_and_references(plan, case_space_id, case_space)?;
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
    Ok(())
}

fn validate_plan_shape_and_references(
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
    let missing_success_requirement_ids = plan
        .steps
        .iter()
        .flat_map(|step| &step.success_evidence_requirement_ids)
        .filter(|id| !cell_ids.contains(*id))
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing_success_requirement_ids.is_empty() {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} names success_evidence_requirement_ids that are not existing case cells: {}",
            plan.plan_id,
            missing_success_requirement_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

pub(super) fn verified_plan_review_status(
    plan: &ExecutionPlan,
    case_space: &CaseSpace,
) -> Result<ReviewStatus, NativeCliError> {
    let latest = case_space.morphism_log.iter().rev().find(|entry| {
        let metadata = &entry.morphism.metadata;
        metadata.get("target_kind").and_then(Value::as_str) == Some("plan")
            && metadata.get("target_id").and_then(Value::as_str) == Some(plan.plan_id.as_str())
    });
    let Some(entry) = latest else {
        if plan.review_status != ReviewStatus::Unreviewed {
            return Err(NativeCliError::invalid(format!(
                "execution plan {} stored review_status {:?} disagrees with log-derived status unreviewed; possible plan tampering",
                plan.plan_id, plan.review_status
            )));
        }
        return Ok(ReviewStatus::Unreviewed);
    };
    let metadata = &entry.morphism.metadata;
    let malformed = |field: &str| {
        NativeCliError::invalid(format!(
            "latest plan review for {} has invalid canonical field {field}",
            plan.plan_id
        ))
    };
    if entry.morphism.morphism_type != CaseMorphismType::Review
        || entry.morphism.review_status != ReviewStatus::Accepted
    {
        return Err(malformed("morphism_type/review_status"));
    }
    if metadata
        .get("native_review_schema_version")
        .and_then(Value::as_u64)
        != Some(1)
    {
        return Err(malformed("native_review_schema_version"));
    }
    for field in ["review_id", "reviewer_id"] {
        let value = metadata
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| Id::is_valid_value(value))
            .ok_or_else(|| malformed(field))?;
        if value.trim().is_empty() {
            return Err(malformed(field));
        }
    }
    for field in ["reviewed_at", "reason"] {
        if !metadata
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(malformed(field));
        }
    }
    let action: ReviewAction = serde_json::from_value(
        metadata
            .get("action")
            .cloned()
            .ok_or_else(|| malformed("action"))?,
    )
    .map_err(|_| malformed("action"))?;
    let derived_status = match action {
        ReviewAction::Accept => ReviewStatus::Accepted,
        ReviewAction::Reject => ReviewStatus::Rejected,
        _ => return Err(malformed("action")),
    };
    let outcome: ReviewStatus = serde_json::from_value(
        metadata
            .get("outcome_review_status")
            .cloned()
            .ok_or_else(|| malformed("outcome_review_status"))?,
    )
    .map_err(|_| malformed("outcome_review_status"))?;
    if outcome != derived_status {
        return Err(malformed("outcome_review_status"));
    }
    let recorded_hash = metadata
        .get("plan_content_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("plan_content_hash"))?;
    if !crate::exec::accepted_plan_content_hash_matches(plan, recorded_hash)? {
        return Err(NativeCliError::invalid(format!(
            "latest plan review for {} has plan_content_hash {recorded_hash}, but the stored plan content no longer matches",
            plan.plan_id
        )));
    }
    let gate: NativeOperationGate = serde_json::from_value(
        metadata
            .get("operation_gate")
            .cloned()
            .ok_or_else(|| malformed("operation_gate"))?,
    )
    .map_err(|_| malformed("operation_gate"))?;
    check_operation_gate(case_space, &gate, "plan-review")
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    if entry.actor_id != gate.actor_id {
        return Err(malformed("operation_gate.actor_id"));
    }
    if plan.review_status != derived_status {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} stored review_status {:?} disagrees with log-derived status {:?}; possible plan tampering",
            plan.plan_id, plan.review_status, derived_status
        )));
    }
    Ok(derived_status)
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
