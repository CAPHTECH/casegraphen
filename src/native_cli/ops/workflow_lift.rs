//! Materializes a workflow case graph into native cells and relations.
//!
//! This is the substance of `lift workflow` (ADR 0003): work items become case
//! cells, workflow relations become case relations, evidence records become
//! evidence cells whose boundaries pass through the shared trust
//! normalization. Everything the mapping cannot carry — readiness rules,
//! histories, profiles, collapsed state distinctions — is declared as
//! information loss instead of silently dropped.

use super::NativeCliError;
use crate::evidence_trust::EvidenceTrustBoundary;
use crate::native_model::{
    CaseCell, CaseCellLifecycle, CaseCellType, CaseRelation, CaseRelationType, RelationStrength,
};
use crate::workflow_model::{
    EvidenceRecord, WorkItem, WorkItemState, WorkItemType, WorkflowCaseGraph, WorkflowProvenance,
    WorkflowRelationType,
};
use higher_graphen_core::{Id, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::str::FromStr;

pub(super) struct WorkflowMaterialization {
    pub(super) cells: Vec<CaseCell>,
    pub(super) relations: Vec<CaseRelation>,
    pub(super) information_loss: Vec<Value>,
}

pub(super) fn materialize_workflow_graph(
    graph: &WorkflowCaseGraph,
) -> Result<WorkflowMaterialization, NativeCliError> {
    let mut cells = Vec::with_capacity(graph.work_items.len() + graph.evidence_records.len());
    for item in &graph.work_items {
        cells.push(work_item_cell(item)?);
    }
    for record in &graph.evidence_records {
        cells.push(evidence_cell(&graph.space_id, record)?);
    }

    let mut relations = Vec::new();
    let mut covered = BTreeSet::new();
    for relation in &graph.workflow_relations {
        covered.insert(edge_key(
            &relation.from_id,
            relation.relation_type,
            &relation.to_id,
        ));
        let mut metadata = Map::new();
        if let Some(at) = &relation.provenance.recorded_at {
            metadata.insert("workflow_recorded_at".to_owned(), json!(at));
        }
        relations.push(CaseRelation {
            id: relation.id.clone(),
            relation_type: relation_type(relation.relation_type),
            relation_strength: relation_strength(relation.relation_type),
            from_id: relation.from_id.clone(),
            to_id: relation.to_id.clone(),
            evidence_ids: relation.evidence_ids.clone(),
            source_ids: relation.source_ids.clone(),
            provenance: provenance(&relation.provenance)?,
            metadata,
        });
    }

    // Work-item requirement fields are an alternate spelling of the same
    // edges; the workflow evaluator read the union of fields and relations,
    // so the lift synthesizes relations for field entries no explicit
    // relation covers.
    for item in &graph.work_items {
        for (targets, workflow_type) in [
            (&item.hard_dependency_ids, WorkflowRelationType::DependsOn),
            (&item.external_wait_ids, WorkflowRelationType::WaitsFor),
            (
                &item.evidence_requirement_ids,
                WorkflowRelationType::RequiresEvidence,
            ),
            (
                &item.proof_requirement_ids,
                WorkflowRelationType::RequiresProof,
            ),
        ] {
            for target in targets {
                if !covered.insert(edge_key(&item.id, workflow_type, target)) {
                    continue;
                }
                relations.push(synthesized_relation(
                    &item.id,
                    workflow_type,
                    target,
                    &item.source_ids,
                    &item.provenance,
                    "work_item_requirement_field",
                )?);
            }
        }
    }

    // The workflow contract lets requirement targets name evidence, proof, or
    // external waits that do not exist yet — an unsatisfied requirement by
    // reference. The native reducer refuses relations with missing endpoints,
    // so those references become explicit placeholder cells: proposed,
    // unreviewed, and boundary-less, which is exactly the combination that
    // cannot satisfy a hard requirement until real evidence arrives.
    let mut known_ids: BTreeSet<Id> = cells.iter().map(|cell| cell.id.clone()).collect();
    let mut placeholder_ids = Vec::new();
    for relation in &relations {
        if known_ids.contains(&relation.to_id) {
            continue;
        }
        let placeholder_type = match relation.relation_type {
            CaseRelationType::RequiresEvidence => CaseCellType::Evidence,
            CaseRelationType::RequiresProof => CaseCellType::Proof,
            CaseRelationType::WaitsFor => CaseCellType::ExternalRef,
            _ => continue,
        };
        known_ids.insert(relation.to_id.clone());
        placeholder_ids.push(relation.to_id.clone());
        let mut placeholder_provenance = relation.provenance.clone();
        placeholder_provenance.review_status = ReviewStatus::Unreviewed;
        let mut metadata = Map::new();
        metadata.insert(
            "lifted_from".to_owned(),
            json!("unresolved_requirement_target"),
        );
        cells.push(CaseCell {
            id: relation.to_id.clone(),
            cell_type: placeholder_type,
            space_id: graph.space_id.clone(),
            title: format!("Required: {}", relation.to_id),
            summary: None,
            lifecycle: CaseCellLifecycle::Proposed,
            source_ids: relation.source_ids.clone(),
            structure_ids: Vec::new(),
            provenance: placeholder_provenance,
            metadata,
        });
    }

    // Evidence support/contradiction edges are synthesized only toward
    // materialized cells; a reference to a non-materialized record family
    // (correspondences, derived completion candidates) is declared as loss
    // rather than becoming a dangling relation.
    let mut skipped_evidence_targets = Vec::new();
    for record in &graph.evidence_records {
        for (targets, workflow_type) in [
            (&record.supports_ids, WorkflowRelationType::Verifies),
            (&record.contradicts_ids, WorkflowRelationType::Contradicts),
        ] {
            for target in targets {
                if !known_ids.contains(target) {
                    skipped_evidence_targets.push(target.clone());
                    continue;
                }
                if !covered.insert(edge_key(&record.id, workflow_type, target)) {
                    continue;
                }
                let mut relation = synthesized_relation(
                    &record.id,
                    workflow_type,
                    target,
                    &record.source_ids,
                    &record.provenance,
                    "evidence_record_field",
                )?;
                relation.evidence_ids = vec![record.id.clone()];
                relations.push(relation);
            }
        }
    }

    let mut information_loss = information_loss(graph);
    if !placeholder_ids.is_empty() {
        information_loss.push(json!({
            "description": "Requirement targets that named no declared record were materialized \
                            as unreviewed placeholder cells; they cannot satisfy a hard \
                            requirement until real evidence is attached and promoted.",
            "omitted_ids": placeholder_ids,
        }));
    }
    if !skipped_evidence_targets.is_empty() {
        information_loss.push(json!({
            "description": "Evidence supports/contradicts references to non-materialized record \
                            families were not lifted as relations.",
            "omitted_ids": skipped_evidence_targets,
        }));
    }

    // ADR 0003 §4 states that a lifted space contains no capability cells.
    // That currently holds because the type mapping is exhaustive over a closed
    // enum, which is an incidental property of a match arm rather than a
    // guarantee. Assert it on the materialized result so the trust root does
    // not depend on that accident.
    if let Some(cell) = cells
        .iter()
        .find(|cell| cell.cell_type == CaseCellType::Custom("capability".to_owned()))
    {
        return Err(NativeCliError::invalid(format!(
            "workflow lift cannot materialize capability cell {}: capability cells are \
             administered only in a native genesis inside the declared source boundary",
            cell.id
        )));
    }

    Ok(WorkflowMaterialization {
        cells,
        relations,
        information_loss,
    })
}

/// Metadata keys the native evaluator and reducer consult as trust inputs. A
/// workflow work item's `metadata` is caller-declared, so these are stripped
/// rather than copied: an evidence-typed item must get its boundary from the
/// normalization below, and content/trace identity is only ever computed.
const CALLER_DECLARED_TRUST_KEYS: [&str; 4] = [
    "evidence_boundary",
    "content_hash",
    "trace_id",
    "worker_report_id",
];

fn work_item_cell(item: &WorkItem) -> Result<CaseCell, NativeCliError> {
    let mut metadata = item.metadata.clone();
    for key in CALLER_DECLARED_TRUST_KEYS {
        metadata.remove(key);
    }
    metadata.insert("workflow_state".to_owned(), json!(item.state));
    metadata.insert("workflow_item_type".to_owned(), json!(item.item_type));
    if let Some(at) = &item.provenance.recorded_at {
        metadata.insert("workflow_recorded_at".to_owned(), json!(at));
    }
    let cell_type = cell_type(&item.item_type);
    let mut provenance = provenance(&item.provenance)?;
    if cell_type == CaseCellType::Evidence {
        // An evidence-typed work item is evidence, so it enters on the same
        // terms as an evidence record: an untrusted boundary and an unreviewed
        // status. The workflow vocabulary has no boundary field here, so there
        // is nothing to normalize — only a claim to refuse.
        metadata.insert(
            "evidence_boundary".to_owned(),
            json!(EvidenceTrustBoundary::Inferred.metadata_value()),
        );
        provenance.review_status = ReviewStatus::Unreviewed;
    }
    Ok(CaseCell {
        id: item.id.clone(),
        cell_type,
        space_id: item.space_id.clone(),
        title: item.title.clone(),
        summary: None,
        lifecycle: lifecycle(item.state),
        source_ids: item.source_ids.clone(),
        // `case_ids` deliberately does not become `structure_ids`: the
        // evaluator reads `structure_ids` on an evidence cell as "this evidence
        // covers that requirement", and `case_ids` is caller-declared and
        // unvalidated, so mapping it would let one item declare itself the
        // evidence for every requirement it names.
        structure_ids: Vec::new(),
        provenance,
        metadata: {
            if !item.case_ids.is_empty() {
                metadata.insert("workflow_case_ids".to_owned(), json!(item.case_ids));
            }
            metadata
        },
    })
}

fn evidence_cell(space_id: &Id, record: &EvidenceRecord) -> Result<CaseCell, NativeCliError> {
    let boundary: EvidenceTrustBoundary = record.evidence_boundary.into();
    let mut metadata = Map::new();
    metadata.insert(
        "evidence_boundary".to_owned(),
        json!(boundary.metadata_value()),
    );
    metadata.insert(
        "workflow_evidence_type".to_owned(),
        json!(record.evidence_type),
    );
    if let Some(at) = &record.provenance.recorded_at {
        metadata.insert("workflow_recorded_at".to_owned(), json!(at));
    }
    // The boundary label alone is not the barrier: the shared trust rule
    // accepts a review-promoted boundary when the cell's own review status is
    // accepted, and that status is caller-declared here. Force it unreviewed so
    // promotion has to come from a gated review morphism. Genuinely
    // source-backed evidence is unaffected — that boundary does not consult
    // review status.
    let mut provenance = provenance(&record.provenance)?;
    provenance.review_status = ReviewStatus::Unreviewed;
    Ok(CaseCell {
        id: record.id.clone(),
        cell_type: CaseCellType::Evidence,
        space_id: space_id.clone(),
        title: record.summary.clone(),
        summary: None,
        lifecycle: CaseCellLifecycle::Active,
        source_ids: record.source_ids.clone(),
        structure_ids: Vec::new(),
        provenance,
        metadata,
    })
}

fn synthesized_relation(
    from: &Id,
    workflow_type: WorkflowRelationType,
    to: &Id,
    source_ids: &[Id],
    workflow_provenance: &WorkflowProvenance,
    lifted_from: &str,
) -> Result<CaseRelation, NativeCliError> {
    let mut metadata = Map::new();
    metadata.insert("lifted_from".to_owned(), json!(lifted_from));
    Ok(CaseRelation {
        id: Id::new(format!(
            "relation:lift:{}:{from}->{to}",
            workflow_type_name(workflow_type)
        ))?,
        relation_type: relation_type(workflow_type),
        relation_strength: relation_strength(workflow_type),
        from_id: from.clone(),
        to_id: to.clone(),
        evidence_ids: Vec::new(),
        source_ids: source_ids.to_vec(),
        provenance: provenance(workflow_provenance)?,
        metadata,
    })
}

fn edge_key(from: &Id, workflow_type: WorkflowRelationType, to: &Id) -> String {
    format!(
        "{from}\u{1f}{}\u{1f}{to}",
        workflow_type_name(workflow_type)
    )
}

fn cell_type(item_type: &WorkItemType) -> CaseCellType {
    match item_type {
        WorkItemType::Task => CaseCellType::Work,
        WorkItemType::Goal => CaseCellType::Goal,
        WorkItemType::Decision => CaseCellType::Decision,
        WorkItemType::Event => CaseCellType::Event,
        WorkItemType::Evidence => CaseCellType::Evidence,
        WorkItemType::Proof => CaseCellType::Proof,
        WorkItemType::ExternalWait => CaseCellType::ExternalRef,
        WorkItemType::ReviewAction => CaseCellType::Review,
        WorkItemType::Case => CaseCellType::Case,
        WorkItemType::Milestone => CaseCellType::Custom("milestone".to_owned()),
    }
}

/// Stored workflow state maps onto the lifecycle table; stored blockedness is
/// deliberately discarded because readiness is derived, never stored — if the
/// graph's relations justify the block the evaluator re-derives it, and if
/// they do not, the stored flag was an unsupported claim.
fn lifecycle(state: WorkItemState) -> CaseCellLifecycle {
    match state {
        WorkItemState::Proposed => CaseCellLifecycle::Proposed,
        WorkItemState::Todo | WorkItemState::Doing | WorkItemState::Blocked => {
            CaseCellLifecycle::Active
        }
        WorkItemState::Waiting => CaseCellLifecycle::Waiting,
        WorkItemState::Done => CaseCellLifecycle::Resolved,
        WorkItemState::Cancelled | WorkItemState::Failed => CaseCellLifecycle::Retired,
        WorkItemState::Accepted => CaseCellLifecycle::Accepted,
        WorkItemState::Rejected => CaseCellLifecycle::Rejected,
    }
}

fn relation_type(workflow_type: WorkflowRelationType) -> CaseRelationType {
    match workflow_type {
        WorkflowRelationType::DependsOn => CaseRelationType::DependsOn,
        WorkflowRelationType::WaitsFor => CaseRelationType::WaitsFor,
        WorkflowRelationType::RequiresEvidence => CaseRelationType::RequiresEvidence,
        WorkflowRelationType::RequiresProof => CaseRelationType::RequiresProof,
        WorkflowRelationType::Verifies => CaseRelationType::Verifies,
        WorkflowRelationType::Blocks => CaseRelationType::Blocks,
        WorkflowRelationType::Contradicts => CaseRelationType::Contradicts,
        WorkflowRelationType::Completes => CaseRelationType::Completes,
        WorkflowRelationType::DerivesFrom => CaseRelationType::DerivesFrom,
        WorkflowRelationType::TransitionsTo => CaseRelationType::TransitionsTo,
        WorkflowRelationType::ProjectsTo => CaseRelationType::ProjectsTo,
        WorkflowRelationType::CorrespondsTo => CaseRelationType::CorrespondsTo,
        WorkflowRelationType::Supersedes => CaseRelationType::Supersedes,
        WorkflowRelationType::RelatesTo => CaseRelationType::Custom("relates_to".to_owned()),
    }
}

/// Workflow relations carry no strength. Readiness-bearing types lift as
/// hard so the native evaluator enforces them; annotative types lift as
/// diagnostic. The defaulting is declared in the lift's information loss.
fn relation_strength(workflow_type: WorkflowRelationType) -> RelationStrength {
    match workflow_type {
        WorkflowRelationType::DependsOn
        | WorkflowRelationType::WaitsFor
        | WorkflowRelationType::RequiresEvidence
        | WorkflowRelationType::RequiresProof
        | WorkflowRelationType::Blocks
        | WorkflowRelationType::Contradicts => RelationStrength::Hard,
        WorkflowRelationType::Verifies
        | WorkflowRelationType::Completes
        | WorkflowRelationType::DerivesFrom
        | WorkflowRelationType::TransitionsTo
        | WorkflowRelationType::ProjectsTo
        | WorkflowRelationType::CorrespondsTo
        | WorkflowRelationType::Supersedes
        | WorkflowRelationType::RelatesTo => RelationStrength::Diagnostic,
    }
}

fn workflow_type_name(workflow_type: WorkflowRelationType) -> String {
    serde_json::to_value(workflow_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{workflow_type:?}"))
}

fn provenance(workflow: &WorkflowProvenance) -> Result<Provenance, NativeCliError> {
    let kind = SourceKind::from_str(&workflow.source.kind)
        .or_else(|_| SourceKind::custom(workflow.source.kind.clone()))
        .map_err(|error| {
            NativeCliError::invalid(format!(
                "workflow source kind {:?} cannot be represented: {error}",
                workflow.source.kind
            ))
        })?;
    let mut source = SourceRef::new(kind);
    source.uri = workflow.source.uri.clone();
    source.title = workflow.source.title.clone();
    source.captured_at = workflow.source.captured_at.clone();
    source.source_local_id = workflow.source.source_local_id.clone();
    let mut provenance =
        Provenance::new(source, workflow.confidence).with_review_status(workflow.review_status);
    provenance.extraction_method = workflow.extraction_method.clone();
    provenance.extractor_id = workflow.actor_id.as_ref().map(ToString::to_string);
    Ok(provenance)
}

fn information_loss(graph: &WorkflowCaseGraph) -> Vec<Value> {
    let mut loss = vec![
        json!({
            "description": "Workflow states todo, doing, and blocked collapse to the active \
                            lifecycle; blockedness is re-derived from relations instead of \
                            stored. The original state is kept as metadata.workflow_state.",
        }),
        json!({
            "description": "Workflow relations carry no strength; readiness-bearing types were \
                            lifted as hard, annotative types as diagnostic.",
        }),
    ];
    for (name, ids) in [
        (
            "readiness_rules are replaced by the native evaluator's derived rules",
            graph
                .readiness_rules
                .iter()
                .map(|rule| rule.id.clone())
                .collect::<Vec<_>>(),
        ),
        (
            "transition_records are history, not state, and were not materialized",
            graph
                .transition_records
                .iter()
                .map(|record| record.id.clone())
                .collect(),
        ),
        (
            "completion_reviews were not materialized; native review morphisms are the \
             promotion path",
            graph
                .completion_reviews
                .iter()
                .map(|record| record.id.clone())
                .collect(),
        ),
        (
            "projection_profiles were not materialized; native projections are derived",
            graph
                .projection_profiles
                .iter()
                .map(|profile| profile.id.clone())
                .collect(),
        ),
        (
            "correspondence_records were not materialized",
            graph
                .correspondence_records
                .iter()
                .map(|record| record.id.clone())
                .collect(),
        ),
    ] {
        if !ids.is_empty() {
            loss.push(json!({ "description": name, "omitted_ids": ids }));
        }
    }
    loss
}
