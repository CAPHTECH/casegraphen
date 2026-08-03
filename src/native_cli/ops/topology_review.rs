use super::{
    append_validated_morphism, io::timestamp, require_current_revision, NativeTopologyReviewOptions,
};
use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
    native_model::ReviewAction,
    native_review::{
        canonical_review, check_operation_gate, execution_topology_review_morphism,
        ExecutionTopologyReviewRequest, ExecutionTopologyReviewTarget, NativeOperationGate,
        NativeReviewTargetKind,
    },
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus};
use serde_json::{json, Value};
use std::path::Path;

use super::super::{path_helpers::path_segment, reporting::report, NativeCliError};

pub(in crate::native_cli) fn topology_review_apply(
    store: &Path,
    case_space_id: &Id,
    options: NativeTopologyReviewOptions<'_>,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, options.base_revision_id)?;

    let bytes = std::fs::read(options.topology_input).map_err(|source| NativeCliError::Io {
        path: options.topology_input.to_path_buf(),
        source,
    })?;
    let topology: ExecutionTopology = serde_json::from_slice(&bytes)?;
    if topology.case_space_id != case_space_id.as_str() {
        return Err(NativeCliError::invalid(
            "execution topology input belongs to a different case space",
        ));
    }
    let topology_content_hash = execution_topology_content_hash(&topology)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    let artifact_hash = crate::native_hash::sha256_hex(&bytes);
    let artifact_id = Id::new(format!("artifact:sha256-{artifact_hash}"))?;
    let claim = replay
        .case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == *options.claim_cell_id)
        .ok_or_else(|| NativeCliError::invalid("execution topology claim does not exist"))?;
    let expansion_proposal_id = claim
        .metadata
        .get("expansion_proposal_id")
        .and_then(Value::as_str)
        .map(Id::new)
        .transpose()?;
    let target = ExecutionTopologyReviewTarget {
        topology_id: Id::new(topology.topology_id.clone())?,
        topology_content_hash,
        case_space_id: case_space_id.clone(),
        observed_base_revision_id: options.base_revision_id.clone(),
        claim_cell_id: options.claim_cell_id.clone(),
        artifact_id,
        expansion_proposal_id,
    };
    let gate = NativeOperationGate {
        actor_id: options
            .gate_options
            .actor_id
            .clone()
            .expect("required topology-review gate actor"),
        operation: "review".to_owned(),
        operation_scope_id: options
            .gate_options
            .operation_scope_id
            .clone()
            .expect("required topology-review scope"),
        audience: options
            .gate_options
            .audience
            .expect("required topology-review audience"),
        capability_ids: options.gate_options.capability_ids.clone(),
        source_boundary_id: options
            .gate_options
            .source_boundary_id
            .clone()
            .expect("required topology-review source boundary"),
    };
    check_operation_gate(&replay.case_space, &gate, "review")?;
    let target_revision_id = Id::new(format!(
        "revision:execution-topology-review:{}:{}",
        path_segment(options.claim_cell_id),
        replay.case_space.morphism_log.len() + 1
    ))?;
    let mut morphism = execution_topology_review_morphism(
        &replay.case_space,
        ExecutionTopologyReviewRequest {
            target,
            action: options.action,
            reviewer_id: options.reviewer_id.clone(),
            reviewed_at: timestamp(),
            reason: options.reason.to_owned(),
            evidence_ids: Vec::new(),
            source_ids: vec![options.claim_cell_id.clone()],
            target_revision_id,
        },
        &bytes,
    )?;
    morphism
        .metadata
        .insert("operation_gate".to_owned(), serde_json::to_value(&gate)?);
    append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(gate.actor_id.clone()),
        match options.action {
            ReviewAction::Accept => "casegraphen topology-review accept",
            ReviewAction::Reject => "casegraphen topology-review reject",
            ReviewAction::Reopen => "casegraphen topology-review reopen",
            ReviewAction::Waive | ReviewAction::Defer | ReviewAction::Supersede => {
                unreachable!("topology review parser does not admit this action")
            }
        },
    )
}

pub(in crate::native_cli) fn topology_review_inspect(
    store: &Path,
    case_space_id: &Id,
    claim_cell_id: &Id,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let reviews = replay
        .case_space
        .morphism_log
        .iter()
        .filter_map(|entry| {
            let review = canonical_review(&entry.morphism)?;
            (review.target_kind == NativeReviewTargetKind::ExecutionTopology
                && review.target_id == *claim_cell_id)
                .then(|| {
                    json!({
                        "review_id": entry.morphism.metadata.get("review_id"),
                        "action": review.action,
                        "outcome_review_status": review.outcome,
                        "binding": review.execution_topology,
                        "target_revision_id": entry.target_revision_id,
                    })
                })
        })
        .collect::<Vec<_>>();
    let current_status = reviews
        .last()
        .and_then(|value| value.get("outcome_review_status"))
        .cloned()
        .unwrap_or_else(|| json!(ReviewStatus::Unreviewed));
    Ok(report(
        "casegraphen topology-review inspect",
        json!({
            "claim_cell_id": claim_cell_id,
            "current_status": current_status,
            "reviews": reviews,
        }),
    ))
}
