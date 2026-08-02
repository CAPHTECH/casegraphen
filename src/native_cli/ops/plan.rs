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
        return Err(stale_plan_revision(
            &plan,
            &replay.case_space.revision.revision_id,
        ));
    }
    let content_hash = plan_content_hash(&plan)?;
    let operation_gate = resolve_plan_review_gate(options.gate_options)?;
    check_operation_gate(&replay.case_space, &operation_gate, "plan-review")?;
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
        return Err(stale_plan_revision(plan, &case_space.revision.revision_id));
    }
    Ok(())
}

/// The one place that builds a stale-plan-base-revision refusal — used by
/// both `plan_review` (a plan already unreviewed, base revision moved since
/// propose) and `validate_plan_for_current_case_space` (propose/check
/// against a plan whose base revision has already moved). Kept as its own
/// `NativeCliError::StalePlanRevision` rather than reusing the mutation
/// surface's `StaleRevision` or the two hand-built `Invalid` strings this
/// replaced: see `StalePlanRevision`'s doc comment for why the subject and
/// the recovery both differ from a plain stale base revision.
fn stale_plan_revision(plan: &ExecutionPlan, current_revision_id: &Id) -> NativeCliError {
    NativeCliError::StalePlanRevision {
        plan_id: plan.plan_id.clone(),
        base_revision_id: plan.base_revision_id.clone(),
        current_revision_id: current_revision_id.clone(),
    }
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

/// Re-verifies a plan's *stored* review against the log it was recorded in
/// — tamper detection over already-recorded state, never live
/// authorization, since nothing is being asked for right now. Every
/// refusal this function returns is `NativeCliError::StoreIntegrity`, not
/// `Invalid` and never `GateViolation`: the correct response to any of
/// them is "stop and investigate", not "fix the call" or "get a different
/// actor or capability". Do not let a future check added here reach for
/// `NativeCliError::invalid` or `?` on a `NativeOperationGateError` out of
/// habit — both are the wrong classification for what this function does.
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
            return Err(NativeCliError::StoreIntegrity(format!(
                "execution plan {} stored review_status {:?} disagrees with log-derived status unreviewed; possible plan tampering",
                plan.plan_id, plan.review_status
            )));
        }
        return Ok(ReviewStatus::Unreviewed);
    };
    let metadata = &entry.morphism.metadata;
    let malformed = |field: &str| {
        NativeCliError::StoreIntegrity(format!(
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
        return Err(NativeCliError::StoreIntegrity(format!(
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
    // Re-verifying a *stored* review's recorded gate, not authorizing a
    // live request — the same shape as the store's own replay-time gate
    // re-validation, and classified the same way for the same reason: the
    // actor asked for nothing just now, so a failure here means the log
    // disagrees with itself, not that a different actor or capability is
    // needed. `?` alone would route this through `GateViolation` via the
    // blanket `From<NativeOperationGateError>` conversion, which is wrong
    // here specifically — see `NativeCliError::StoreIntegrity`'s doc
    // comment.
    check_operation_gate(case_space, &gate, "plan-review")
        .map_err(|error| NativeCliError::StoreIntegrity(error.to_string()))?;
    if entry.actor_id != gate.actor_id {
        return Err(malformed("operation_gate.actor_id"));
    }
    if plan.review_status != derived_status {
        return Err(NativeCliError::StoreIntegrity(format!(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_model::{CaseMorphism, MorphismLogEntry, NATIVE_MORPHISM_LOG_ENTRY_SCHEMA};
    use higher_graphen_core::SourceKind;
    use serde_json::Map;

    const NATIVE_EXAMPLE: &str =
        include_str!("../../../schemas/casegraphen/native.case.space.example.json");

    /// A tampered *stored* review's recorded operation gate is tamper
    /// detection over already-recorded state, not live authorization — the
    /// actor asked for nothing just now — so it must classify as
    /// `StoreIntegrity` ("stop and investigate"), never `GateViolation`
    /// ("get a different actor or capability"). This was wrong once: an
    /// earlier version of this check routed through the same `?`/
    /// `From<NativeOperationGateError>` conversion the live-authorization
    /// check at `plan_review`'s own gate uses, which produced
    /// `GateViolation` here too.
    ///
    /// Reaching this specific branch through the real store is not
    /// feasible as a regression test: every entry in the morphism log is
    /// hash-chained (`previous_entry_hash`) and the tail is additionally
    /// cross-checked against `morphism_log.head.json` on every read, so
    /// tampering *any* entry's content — the tail or an interior one — is
    /// always caught by that integrity check first, before
    /// `verified_plan_review_status` ever inspects the gate. That is
    /// correct, desired behaviour, not a gap: it means this exact failure
    /// can only be exercised in memory, against a `CaseSpace` built
    /// directly rather than replayed from a store.
    #[test]
    fn verified_plan_review_status_classifies_a_tampered_stored_gate_as_store_integrity() {
        let case_space: CaseSpace =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");

        let plan = ExecutionPlan {
            schema: "highergraphen.case.workflow.execution_plan.v1".to_owned(),
            schema_version: 1,
            plan_id: id_lossy("plan:gate-tamper-unit"),
            case_space_id: case_space.case_space_id.clone(),
            base_revision_id: case_space.revision.revision_id.clone(),
            steps: Vec::new(),
            provenance: super::super::io::provenance(SourceKind::Human, ReviewStatus::Unreviewed),
            review_status: ReviewStatus::Accepted,
            metadata: Map::new(),
        };
        let content_hash = execution_plan_content_hash(&plan).expect("plan content hash");

        // Empty `capability_ids` fails `check_operation_gate`'s own first
        // check, independent of whether any capability cell exists — the
        // simplest reliable way to fail the gate re-verification without
        // also needing a real, authorized capability grant.
        let mut gate_metadata = Map::new();
        gate_metadata.insert("actor_id".to_owned(), json!("actor:plan-review-unit"));
        gate_metadata.insert("operation".to_owned(), json!("plan-review"));
        gate_metadata.insert(
            "operation_scope_id".to_owned(),
            json!(case_space.case_space_id),
        );
        gate_metadata.insert("audience".to_owned(), json!("audit"));
        gate_metadata.insert("capability_ids".to_owned(), json!([]));
        gate_metadata.insert(
            "source_boundary_id".to_owned(),
            json!("source_boundary:native-case-management-contract"),
        );

        let mut metadata = Map::new();
        metadata.insert("native_review_schema_version".to_owned(), json!(1));
        metadata.insert("target_kind".to_owned(), json!("plan"));
        metadata.insert("target_id".to_owned(), json!(plan.plan_id));
        metadata.insert("action".to_owned(), json!("accept"));
        metadata.insert("outcome_review_status".to_owned(), json!("accepted"));
        metadata.insert("review_id".to_owned(), json!("review:plan-review-unit"));
        metadata.insert("reviewer_id".to_owned(), json!("reviewer:plan-review-unit"));
        metadata.insert("reviewed_at".to_owned(), json!("unix:0"));
        metadata.insert("reason".to_owned(), json!("unit test review"));
        metadata.insert("plan_content_hash".to_owned(), json!(content_hash));
        metadata.insert("operation_gate".to_owned(), Value::Object(gate_metadata));

        let review_morphism = CaseMorphism {
            morphism_id: id_lossy("morphism:review:plan-gate-tamper-unit"),
            morphism_type: CaseMorphismType::Review,
            source_revision_id: Some(case_space.revision.revision_id.clone()),
            target_revision_id: id_lossy("revision:plan-review:plan-gate-tamper-unit"),
            added_ids: Vec::new(),
            updated_ids: Vec::new(),
            retired_ids: Vec::new(),
            preserved_ids: Vec::new(),
            violated_invariant_ids: Vec::new(),
            review_status: ReviewStatus::Accepted,
            evidence_ids: Vec::new(),
            source_ids: Vec::new(),
            metadata,
        };
        let review_entry = MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: 1,
            case_space_id: case_space.case_space_id.clone(),
            sequence: case_space.morphism_log.len() as u64 + 1,
            entry_id: id_lossy("morphism_log_entry:review:plan-gate-tamper-unit"),
            morphism_id: review_morphism.morphism_id.clone(),
            source_revision_id: review_morphism.source_revision_id.clone(),
            target_revision_id: review_morphism.target_revision_id.clone(),
            actor_id: id_lossy("actor:plan-review-unit"),
            recorded_at: "unix:0".to_owned(),
            provenance: super::super::io::provenance(SourceKind::Human, ReviewStatus::Accepted),
            source_ids: Vec::new(),
            previous_entry_hash: None,
            replay_checksum: String::new(),
            morphism: review_morphism,
        };

        let mut tampered_case_space = case_space;
        tampered_case_space.morphism_log.push(review_entry);

        let error = verified_plan_review_status(&plan, &tampered_case_space)
            .expect_err("empty capability_ids must fail the stored gate re-verification");
        assert!(
            matches!(error, NativeCliError::StoreIntegrity(_)),
            "expected StoreIntegrity, got {error:?}"
        );
    }

    fn id_lossy(value: &str) -> Id {
        Id::new(value.to_owned()).expect("test id")
    }
}
