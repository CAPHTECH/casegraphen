use super::{
    path_helpers::{id_lossy, path_segment, relative_store_path},
    reporting::report,
    NativeCliError, NativeReasonSection,
};
use crate::{
    core_extension_bridge::{
        native_close_check_extensions, native_close_check_result, native_morphism_check_extensions,
        native_morphism_check_result,
    },
    math_diagnostics::{native_close_temporal_diagnostics, native_morphism_temporal_diagnostics},
    native_eval::evaluate_native_case,
    native_model::{
        apply_morphism, write_genesis_materialization, CaseCell, CaseCellLifecycle, CaseCellType,
        CaseMorphism, CaseMorphismType, CaseSpace, MorphismLogEntry, ProjectionAudience,
        ReviewAction, Revision, NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION,
        NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
    },
    native_review::{
        check_native_close, check_operation_gate, declared_source_boundary_id,
        NativeCloseCheckRequest, NativeOperationGate,
    },
    native_store::NativeCaseStore,
    topology::TopologyReportOptions,
};
use higher_graphen_core::{Id, Provenance, ReviewStatus, SourceKind};
use serde_json::{json, Map, Value};
use std::path::Path;

mod binding;
mod io;
mod lift;
mod mutations;
mod plan;
mod run;
mod workflow_lift;
pub(super) use binding::binding_register;
use io::{
    case_space_checksum, known_ids, proposal_path, proposal_value, provenance, read_morphism,
    read_proposal, timestamp, write_json,
};
pub(super) use lift::{case_import, case_new, lift_structured_source};
pub(super) use mutations::{cell_transition, evidence_attach, review_apply};
pub(super) use plan::{plan_check, plan_propose, plan_review};
pub(super) use run::run_step;

pub(super) struct NativeReviewApplyOptions<'a> {
    pub(super) action: ReviewAction,
    pub(super) target_id: &'a Id,
    pub(super) reviewer_id: &'a Id,
    pub(super) reason: &'a str,
    pub(super) base_revision_id: &'a Id,
    pub(super) evidence_ids: &'a [Id],
    pub(super) gate_options: &'a NativeMutationGateOptions,
}

pub(super) struct NativePlanReviewOptions<'a> {
    pub(super) plan_id: &'a Id,
    pub(super) action: ReviewAction,
    pub(super) reviewer_id: &'a Id,
    pub(super) reason: &'a str,
    pub(super) base_revision_id: &'a Id,
    pub(super) gate_options: &'a NativePlanGateOptions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeRunGateOptions {
    pub(super) actor_id: Id,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Id,
    pub(super) audience: ProjectionAudience,
    pub(super) source_boundary_id: Id,
}

pub(super) struct NativeRunStepOptions<'a> {
    pub(super) case_space_id: &'a Id,
    pub(super) plan_id: &'a Id,
    pub(super) base_revision_id: &'a Id,
    pub(super) actor_id: &'a Id,
    pub(super) enabled_worker_kinds: &'a [String],
    pub(super) retry_step_id: Option<&'a Id>,
    pub(super) gate_options: &'a NativeRunGateOptions,
}

pub(super) fn case_reason(
    store: &Path,
    case_space_id: &Id,
    section: NativeReasonSection,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let evaluation = evaluate_native_case(&replay.case_space)?;
    let (command, result) = match section {
        NativeReasonSection::Reason => (
            "casegraphen space reason",
            json!({ "evaluation": evaluation }),
        ),
        NativeReasonSection::Frontier => (
            "casegraphen space frontier",
            json!({ "frontier_cell_ids": evaluation.frontier_cell_ids }),
        ),
        NativeReasonSection::Obstructions => (
            "casegraphen obstruction list",
            json!({ "obstructions": evaluation.obstructions }),
        ),
        NativeReasonSection::Completions => (
            "casegraphen completion candidates",
            json!({ "completion_candidates": evaluation.completion_candidates }),
        ),
        NativeReasonSection::Evidence => (
            "casegraphen space evidence",
            json!({ "evidence_findings": evaluation.evidence_findings }),
        ),
        NativeReasonSection::Project => (
            "casegraphen space project",
            json!({
                "projections": replay.case_space.projections,
                "projection_loss": evaluation.projection_loss,
            }),
        ),
    };
    Ok(report(command, result))
}

pub(super) fn projection_apply(
    store: &Path,
    case_space_id: &Id,
    projection: &Path,
) -> Result<Value, NativeCliError> {
    let raw = std::fs::read_to_string(projection).map_err(|source| NativeCliError::Io {
        path: projection.to_path_buf(),
        source,
    })?;
    let request: Value = serde_json::from_str(&raw)?;
    let request_object = request
        .as_object()
        .ok_or_else(|| NativeCliError::invalid("projection request must be a JSON object"))?;
    let projection_id = request_object
        .get("projection_id")
        .and_then(Value::as_str)
        .ok_or_else(|| NativeCliError::invalid("projection request must contain projection_id"))?;
    let audience = request_object.get("audience").and_then(Value::as_str);
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let evaluation = evaluate_native_case(&replay.case_space)?;
    let matched_projections: Vec<_> = replay
        .case_space
        .projections
        .iter()
        .filter(|candidate| candidate.projection_id.as_str() == projection_id)
        .filter(|candidate| {
            audience.map_or(true, |value| audience_name(candidate.audience) == value)
        })
        .cloned()
        .collect();
    let projection_match_status = if matched_projections.is_empty() {
        "not_found"
    } else {
        "matched"
    };
    Ok(report(
        "casegraphen projection apply",
        json!({
            "projection_request": request,
            "matched_projections": matched_projections,
            "projection_loss": evaluation.projection_loss,
            "projection_match_status": projection_match_status,
        }),
    ))
}

fn audience_name(audience: ProjectionAudience) -> &'static str {
    match audience {
        ProjectionAudience::HumanReview => "human_review",
        ProjectionAudience::AiAgent => "ai_agent",
        ProjectionAudience::Audit => "audit",
        ProjectionAudience::System => "system",
        ProjectionAudience::Migration => "migration",
    }
}

pub(super) fn case_close_check(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    validation_evidence_ids: &[Id],
    gate_options: NativeCloseGateOptions,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let operation_gate = close_operation_gate(&replay.case_space, gate_options)?;
    let check = check_native_close(
        &replay.case_space,
        NativeCloseCheckRequest {
            close_policy_id: operation_gate.close_policy_id.clone(),
            base_revision_id: base_revision_id.clone(),
            declared_projection_loss_ids: Vec::new(),
            validation_evidence_ids: validation_evidence_ids.to_vec(),
            source_ids: validation_evidence_ids.to_vec(),
            operation_gate: Some(operation_gate.gate),
        },
    )?;
    let core_extensions = native_close_check_extensions(&replay.case_space, &check);
    let mut result = native_close_check_result(check, core_extensions);
    result["mathematical_diagnostics"] = json!(native_close_temporal_diagnostics(
        &replay.case_space,
        validation_evidence_ids
    )?);
    Ok(report("casegraphen invariant close-check", result))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativeCloseGateOptions {
    pub(super) close_policy_id: Option<Id>,
    pub(super) actor_id: Option<Id>,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Option<Id>,
    pub(super) audience: Option<ProjectionAudience>,
    pub(super) source_boundary_id: Option<Id>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativePlanGateOptions {
    pub(super) actor_id: Option<Id>,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Option<Id>,
    pub(super) audience: Option<ProjectionAudience>,
    pub(super) source_boundary_id: Option<Id>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct NativeMutationGateOptions {
    pub(super) actor_id: Option<Id>,
    pub(super) capability_ids: Vec<Id>,
    pub(super) operation_scope_id: Option<Id>,
    pub(super) audience: Option<ProjectionAudience>,
    pub(super) source_boundary_id: Option<Id>,
}

struct ResolvedCloseGate {
    close_policy_id: Option<Id>,
    gate: NativeOperationGate,
}

pub(super) fn case_topology(
    store: &Path,
    case_space_id: &Id,
    topology_options: TopologyReportOptions,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let topology = crate::topology::native_case_topology_with_history(
        &replay.case_space,
        &replay.history,
        topology_options,
    )?;
    Ok(report(
        "casegraphen space topology",
        json!({ "topology": topology }),
    ))
}

pub(super) fn case_topology_diff(
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
        "casegraphen space topology diff",
        json!({
            "left_case_space_id": left_case_space_id,
            "right_case_space_id": right_case_space_id,
            "topology_diff": topology_diff
        }),
    ))
}

pub(super) fn morphism_propose(
    store: &Path,
    case_space_id: &Id,
    input: &Path,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let morphism = read_morphism(input)?;
    validate_generic_morphism_metadata(&morphism)?;
    validate_candidate_morphism(&replay.case_space, &morphism)?;
    let proposal = proposal_value(case_space_id, &morphism);
    let path = proposal_path(store, case_space_id, &morphism.morphism_id)?;
    write_json(&path, &proposal)?;
    Ok(report(
        "casegraphen morphism propose",
        json!({
            "proposal_status": "checked",
            "proposal_path": relative_store_path(store, &path),
            "morphism": morphism
        }),
    ))
}

pub(super) fn morphism_check(
    store: &Path,
    case_space_id: &Id,
    morphism_id: &Id,
) -> Result<Value, NativeCliError> {
    let replay =
        NativeCaseStore::new(store.to_path_buf()).replay_current_case_space(case_space_id)?;
    let morphism = read_proposal(store, case_space_id, morphism_id)?;
    validate_generic_morphism_metadata(&morphism)?;
    validate_candidate_morphism(&replay.case_space, &morphism)?;
    let core_extensions = native_morphism_check_extensions(&replay.case_space, &morphism);
    let mathematical_diagnostics =
        native_morphism_temporal_diagnostics(&replay.case_space, &morphism)?;
    let mut result = native_morphism_check_result(morphism, core_extensions);
    result["mathematical_diagnostics"] = json!(mathematical_diagnostics);
    Ok(report("casegraphen morphism check", result))
}

pub(super) fn morphism_apply(
    store: &Path,
    case_space_id: &Id,
    morphism_id: &Id,
    base_revision_id: &Id,
    reviewer_id: Option<&Id>,
    reason: Option<&str>,
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    let mut morphism = read_proposal(store, case_space_id, morphism_id)?;
    validate_generic_morphism_metadata(&morphism)?;
    validate_candidate_morphism(&replay.case_space, &morphism)?;
    let operation_gate = validated_mutation_gate(
        &replay.case_space,
        gate_options,
        "morphism-apply",
        "morphism apply",
    )?;
    morphism.review_status = ReviewStatus::Accepted;
    if let Some(reviewer_id) = reviewer_id {
        morphism
            .metadata
            .insert("reviewer_id".to_owned(), json!(reviewer_id));
    }
    if let Some(reason) = reason {
        if reason.trim().is_empty() {
            return Err(NativeCliError::invalid("review reason must not be empty"));
        }
        morphism
            .metadata
            .insert("review_reason".to_owned(), json!(reason.trim()));
    }
    morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(operation_gate.actor_id),
        "casegraphen morphism apply",
    )
}

pub(super) fn morphism_reject(
    store: &Path,
    case_space_id: &Id,
    morphism_id: &Id,
    reviewer_id: &Id,
    reason: &str,
    revision_id: &Id,
    gate_options: &NativeMutationGateOptions,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    let proposal = read_proposal(store, case_space_id, morphism_id)?;
    validate_generic_morphism_metadata(&proposal)?;
    validate_candidate_morphism(&replay.case_space, &proposal)?;
    let mut review = review_morphism(
        &replay.case_space.revision.revision_id,
        revision_id,
        morphism_id,
        reviewer_id,
        reason,
    )?;
    let operation_gate = validated_mutation_gate(
        &replay.case_space,
        gate_options,
        "morphism-reject",
        "morphism reject",
    )?;
    review.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::to_value(&operation_gate)?,
    );
    let mut entry = entry_for_morphism(&replay.case_space, review, Some(operation_gate.actor_id))?;
    entry.replay_checksum = checksum_after_append(&replay.case_space, &entry)?;
    let record = store_api.append_morphism(case_space_id, entry.clone())?;
    Ok(report(
        "casegraphen morphism reject",
        json!({ "record": record, "entry": entry, "rejected_morphism": proposal }),
    ))
}

fn new_case_space(
    case_space_id: &Id,
    space_id: &Id,
    title: &str,
    revision_id: &Id,
) -> Result<CaseSpace, NativeCliError> {
    if title.trim().is_empty() {
        return Err(NativeCliError::invalid("case title must not be empty"));
    }
    let cell_id = Id::new("case:native-root".to_owned())?;
    let source_id = Id::new("source:native-cli".to_owned())?;
    let morphism_id = Id::new(format!("morphism:create:{}", path_segment(case_space_id)))?;
    let entry_id = Id::new(format!(
        "morphism_log_entry:create:{}",
        path_segment(case_space_id)
    ))?;
    let source_boundary = source_boundary_value(
        Id::new(format!(
            "source_boundary:{}",
            path_segment(case_space_id)
        ))?,
        std::slice::from_ref(&source_id),
        &["native.case.new.v1"],
        "native CLI source fields are accepted as explicit user input; inferred fields need review before close.",
        "case new records no inferred facts beyond the requested identifiers and title.",
        Vec::new(),
    );
    let now = timestamp();
    let provenance = provenance(SourceKind::Human, ReviewStatus::Accepted);
    let entry = genesis_entry(GenesisEntryInput {
        case_space_id,
        revision_id,
        cell_id: &cell_id,
        source_id: &source_id,
        morphism_id,
        entry_id,
        recorded_at: &now,
        provenance: &provenance,
        source_boundary: source_boundary.clone(),
    })?;
    let mut metadata = Map::new();
    metadata.insert("source_boundary".to_owned(), source_boundary);
    let mut case_space = CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: case_space_id.clone(),
        space_id: space_id.clone(),
        case_cells: vec![root_case_cell(
            cell_id,
            space_id,
            title,
            &source_id,
            &provenance,
        )],
        case_relations: Vec::new(),
        morphism_log: vec![entry],
        projections: Vec::new(),
        revision: Revision {
            revision_id: revision_id.clone(),
            case_space_id: case_space_id.clone(),
            applied_entry_ids: Vec::new(),
            applied_morphism_ids: Vec::new(),
            checksum: String::new(),
            parent_revision_id: None,
            created_at: now,
            source_ids: vec![source_id],
            metadata: Map::new(),
        },
        close_policy_id: None,
        metadata,
    };
    case_space.revision.applied_entry_ids = vec![case_space.morphism_log[0].entry_id.clone()];
    case_space.revision.applied_morphism_ids = vec![case_space.morphism_log[0].morphism_id.clone()];
    write_genesis_materialization(&mut case_space)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    let checksum = case_space_checksum(&case_space)?;
    case_space.revision.checksum = checksum.clone();
    case_space.morphism_log[0].replay_checksum = checksum;
    Ok(case_space)
}

struct GenesisEntryInput<'a> {
    case_space_id: &'a Id,
    revision_id: &'a Id,
    cell_id: &'a Id,
    source_id: &'a Id,
    morphism_id: Id,
    entry_id: Id,
    recorded_at: &'a str,
    provenance: &'a Provenance,
    source_boundary: Value,
}

fn genesis_entry(input: GenesisEntryInput<'_>) -> Result<MorphismLogEntry, NativeCliError> {
    let mut metadata = Map::new();
    metadata.insert(
        "lift_semantics".to_owned(),
        json!("native_cli_request_to_case_space"),
    );
    metadata.insert(
        "source_boundary_id".to_owned(),
        json!(Id::new(format!(
            "source_boundary:{}",
            path_segment(input.case_space_id)
        ))?),
    );
    metadata.insert("source_boundary".to_owned(), input.source_boundary);
    let morphism = CaseMorphism {
        morphism_id: input.morphism_id.clone(),
        morphism_type: CaseMorphismType::Create,
        source_revision_id: None,
        target_revision_id: input.revision_id.clone(),
        added_ids: vec![input.cell_id.clone()],
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![input.source_id.clone()],
        metadata,
    };
    Ok(MorphismLogEntry {
        schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: input.case_space_id.clone(),
        sequence: 1,
        entry_id: input.entry_id,
        morphism_id: input.morphism_id,
        source_revision_id: None,
        target_revision_id: input.revision_id.clone(),
        morphism,
        actor_id: Id::new("actor:native-cli".to_owned())?,
        recorded_at: input.recorded_at.to_owned(),
        provenance: input.provenance.clone(),
        source_ids: vec![input.source_id.clone()],
        previous_entry_hash: None,
        replay_checksum: String::new(),
    })
}

fn root_case_cell(
    cell_id: Id,
    space_id: &Id,
    title: &str,
    source_id: &Id,
    provenance: &Provenance,
) -> CaseCell {
    CaseCell {
        id: cell_id,
        cell_type: CaseCellType::Case,
        space_id: space_id.clone(),
        title: title.trim().to_owned(),
        summary: None,
        lifecycle: CaseCellLifecycle::Active,
        source_ids: vec![source_id.clone()],
        structure_ids: Vec::new(),
        provenance: provenance.clone(),
        metadata: Map::new(),
    }
}

fn source_boundary_value(
    source_boundary_id: Id,
    included_sources: &[Id],
    adapters: &[&str],
    accepted_fact_policy: &str,
    inference_policy: &str,
    information_loss: Vec<Value>,
) -> Value {
    json!({
        "id": source_boundary_id,
        "included_sources": included_sources,
        "excluded_sources": [],
        "adapters": adapters,
        "accepted_fact_policy": accepted_fact_policy,
        "inference_policy": inference_policy,
        "information_loss": information_loss
    })
}

fn close_operation_gate(
    case_space: &CaseSpace,
    options: NativeCloseGateOptions,
) -> Result<ResolvedCloseGate, NativeCliError> {
    let source_boundary_id = match options.source_boundary_id {
        Some(id) => id,
        None => declared_source_boundary_id(case_space).ok_or_else(|| {
            NativeCliError::invalid(
                "case space does not declare a source boundary id for close-check",
            )
        })?,
    };
    let actor_id = options
        .actor_id
        .unwrap_or_else(|| id_lossy("actor:casegraphen-cli"));
    let capability_ids = if options.capability_ids.is_empty() {
        vec![id_lossy("capability:casegraphen-cli:close-check")]
    } else {
        options.capability_ids
    };
    Ok(ResolvedCloseGate {
        close_policy_id: options.close_policy_id,
        gate: NativeOperationGate {
            actor_id,
            operation: "close-check".to_owned(),
            operation_scope_id: options
                .operation_scope_id
                .unwrap_or_else(|| case_space.case_space_id.clone()),
            audience: options.audience.unwrap_or(ProjectionAudience::Audit),
            capability_ids,
            source_boundary_id,
        },
    })
}

fn retarget_latest_revision(
    case_space: &mut CaseSpace,
    revision_id: &Id,
) -> Result<(), NativeCliError> {
    {
        let latest = case_space
            .morphism_log
            .last_mut()
            .ok_or_else(|| NativeCliError::invalid("case space morphism_log is empty"))?;
        latest.target_revision_id = revision_id.clone();
        latest.morphism.target_revision_id = revision_id.clone();
    }
    case_space.revision.revision_id = revision_id.clone();
    for projection in &mut case_space.projections {
        projection.revision_id = revision_id.clone();
    }
    write_genesis_materialization(case_space)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    case_space.revision.checksum.clear();
    case_space
        .morphism_log
        .last_mut()
        .expect("latest checked")
        .replay_checksum
        .clear();
    let checksum = case_space_checksum(case_space)?;
    case_space.revision.checksum = checksum.clone();
    case_space
        .morphism_log
        .last_mut()
        .expect("latest checked")
        .replay_checksum = checksum;
    Ok(())
}

fn validate_candidate_morphism(
    case_space: &CaseSpace,
    morphism: &CaseMorphism,
) -> Result<(), NativeCliError> {
    if morphism.source_revision_id.as_ref() != Some(&case_space.revision.revision_id) {
        return Err(NativeCliError::invalid(format!(
            "morphism source_revision_id {:?} does not match current revision {}",
            morphism.source_revision_id, case_space.revision.revision_id
        )));
    }
    if morphism.target_revision_id == case_space.revision.revision_id {
        return Err(NativeCliError::invalid(
            "morphism target_revision_id must advance the revision",
        ));
    }
    let mut candidate = case_space.clone();
    apply_morphism(&mut candidate, morphism)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    let known = known_ids(&candidate);
    for id in morphism.preserved_ids.iter().chain(&morphism.evidence_ids) {
        if !known.contains(id) {
            return Err(NativeCliError::invalid(format!(
                "unknown referenced id {id}"
            )));
        }
    }
    Ok(())
}

fn validated_mutation_gate(
    case_space: &CaseSpace,
    options: &NativeMutationGateOptions,
    operation: &str,
    command: &str,
) -> Result<NativeOperationGate, NativeCliError> {
    let gate = NativeOperationGate {
        actor_id: options.actor_id.clone().ok_or_else(|| {
            NativeCliError::usage(format!("--actor-id <id> is required for {command}"))
        })?,
        operation: operation.to_owned(),
        operation_scope_id: options.operation_scope_id.clone().ok_or_else(|| {
            NativeCliError::usage(format!(
                "--operation-scope-id <id> is required for {command}"
            ))
        })?,
        audience: options.audience.ok_or_else(|| {
            NativeCliError::usage(format!("--audience audit|system is required for {command}"))
        })?,
        capability_ids: options.capability_ids.clone(),
        source_boundary_id: options.source_boundary_id.clone().ok_or_else(|| {
            NativeCliError::usage(format!(
                "--source-boundary-id <id> is required for {command}"
            ))
        })?,
    };
    check_operation_gate(case_space, &gate, operation)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    Ok(gate)
}

fn validate_generic_morphism_metadata(morphism: &CaseMorphism) -> Result<(), NativeCliError> {
    let reserved_review_keys = [
        "native_review_schema_version",
        "target_kind",
        "outcome_review_status",
        "operation_gate",
    ];
    if reserved_review_keys
        .iter()
        .any(|key| morphism.metadata.contains_key(*key))
    {
        return Err(NativeCliError::invalid(
            "generic morphism propose/apply cannot use reserved canonical review metadata: \
             native_review_schema_version, target_kind, outcome_review_status, and operation_gate \
             are reserved for casegraphen review and plan review commands",
        ));
    }
    Ok(())
}

fn require_current_revision(
    current_revision_id: &Id,
    base_revision_id: &Id,
) -> Result<(), NativeCliError> {
    if current_revision_id == base_revision_id {
        Ok(())
    } else {
        Err(NativeCliError::invalid(format!(
            "base revision {base_revision_id} is stale; current revision is {current_revision_id}"
        )))
    }
}

fn append_validated_morphism(
    store: &NativeCaseStore,
    case_space: &CaseSpace,
    morphism: CaseMorphism,
    actor_id: Option<Id>,
    command: &str,
) -> Result<Value, NativeCliError> {
    validate_candidate_morphism(case_space, &morphism)?;
    let mut entry = entry_for_morphism(case_space, morphism, actor_id)?;
    entry.replay_checksum = checksum_after_append(case_space, &entry)?;
    let record = store.append_morphism(&case_space.case_space_id, entry.clone())?;
    Ok(report(command, json!({ "record": record, "entry": entry })))
}

fn entry_for_morphism(
    case_space: &CaseSpace,
    morphism: CaseMorphism,
    actor_id: Option<Id>,
) -> Result<MorphismLogEntry, NativeCliError> {
    let previous_entry_hash = case_space
        .morphism_log
        .last()
        .map(crate::native_hash::morphism_log_entry_hash)
        .transpose()?;
    Ok(MorphismLogEntry {
        schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: case_space.case_space_id.clone(),
        sequence: case_space.morphism_log.len() as u64 + 1,
        entry_id: Id::new(format!(
            "morphism_log_entry:{}:{}",
            path_segment(&morphism.morphism_id),
            case_space.morphism_log.len() + 1
        ))?,
        morphism_id: morphism.morphism_id.clone(),
        source_revision_id: morphism.source_revision_id.clone(),
        target_revision_id: morphism.target_revision_id.clone(),
        actor_id: actor_id.unwrap_or_else(|| id_lossy("actor:native-cli")),
        recorded_at: timestamp(),
        provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
        source_ids: morphism.source_ids.clone(),
        previous_entry_hash,
        replay_checksum: String::new(),
        morphism,
    })
}

fn review_morphism(
    source_revision_id: &Id,
    target_revision_id: &Id,
    rejected_morphism_id: &Id,
    reviewer_id: &Id,
    reason: &str,
) -> Result<CaseMorphism, NativeCliError> {
    if target_revision_id == source_revision_id {
        return Err(NativeCliError::invalid(
            "review target_revision_id must advance the revision",
        ));
    }
    if reason.trim().is_empty() {
        return Err(NativeCliError::invalid("review reason must not be empty"));
    }
    let morphism_id = Id::new(format!(
        "morphism:review-reject:{}:{}",
        path_segment(rejected_morphism_id),
        path_segment(target_revision_id)
    ))?;
    let mut metadata = Map::new();
    metadata.insert("target_kind".to_owned(), json!("morphism"));
    metadata.insert("target_id".to_owned(), json!(rejected_morphism_id));
    metadata.insert("action".to_owned(), json!(ReviewAction::Reject));
    metadata.insert(
        "outcome_review_status".to_owned(),
        json!(ReviewStatus::Rejected),
    );
    metadata.insert("reviewer_id".to_owned(), json!(reviewer_id));
    metadata.insert("reason".to_owned(), json!(reason.trim()));
    Ok(CaseMorphism {
        morphism_id,
        morphism_type: CaseMorphismType::Review,
        source_revision_id: Some(source_revision_id.clone()),
        target_revision_id: target_revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![rejected_morphism_id.clone()],
        metadata,
    })
}

fn checksum_after_append(
    case_space: &CaseSpace,
    entry: &MorphismLogEntry,
) -> Result<String, NativeCliError> {
    let mut next = case_space.clone();
    apply_morphism(&mut next, &entry.morphism)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    next.morphism_log.push(entry.clone());
    next.revision = Revision {
        revision_id: entry.target_revision_id.clone(),
        case_space_id: case_space.case_space_id.clone(),
        applied_entry_ids: vec![entry.entry_id.clone()],
        applied_morphism_ids: vec![entry.morphism_id.clone()],
        checksum: String::new(),
        parent_revision_id: entry.source_revision_id.clone(),
        created_at: entry.recorded_at.clone(),
        source_ids: entry.source_ids.clone(),
        metadata: Map::new(),
    };
    case_space_checksum(&next)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE_EXAMPLE: &str =
        include_str!("../../schemas/casegraphen/native.case.space.example.json");

    #[test]
    fn non_genesis_entry_hashes_its_predecessor() {
        let case_space: CaseSpace =
            serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");
        let morphism = CaseMorphism {
            morphism_id: id_lossy("morphism:entry-hash-test"),
            morphism_type: CaseMorphismType::Review,
            source_revision_id: Some(case_space.revision.revision_id.clone()),
            target_revision_id: id_lossy("revision:entry-hash-test"),
            added_ids: Vec::new(),
            updated_ids: Vec::new(),
            retired_ids: Vec::new(),
            preserved_ids: Vec::new(),
            violated_invariant_ids: Vec::new(),
            review_status: ReviewStatus::Accepted,
            evidence_ids: Vec::new(),
            source_ids: Vec::new(),
            metadata: Map::new(),
        };

        let entry =
            entry_for_morphism(&case_space, morphism, None).expect("build morphism log entry");
        let expected = crate::native_hash::morphism_log_entry_hash(
            case_space
                .morphism_log
                .last()
                .expect("genesis morphism log entry"),
        )
        .expect("predecessor hash");

        assert_eq!(entry.previous_entry_hash, Some(expected));
    }
}
