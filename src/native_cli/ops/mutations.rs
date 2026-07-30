use super::{
    append_validated_morphism, io::timestamp, require_current_revision, NativeReviewApplyOptions,
};
use crate::{
    native_model::{
        CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType, CaseRelation,
        CaseRelationType, CaseSpace, MorphismPayload, RelationStrength, ReviewAction,
    },
    native_review::{
        accept_review_morphism, defer_review_morphism, reject_review_morphism,
        reopen_review_morphism, NativeReviewRequest, NativeReviewTargetKind,
    },
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus};
use serde_json::{json, Map, Value};
use std::{fs, path::Path};

use super::super::{path_helpers::path_segment, NativeCliError};

pub(in crate::native_cli) fn review_apply(
    store: &Path,
    case_space_id: &Id,
    options: NativeReviewApplyOptions<'_>,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, options.base_revision_id)?;
    let target_kind = review_target_kind(&replay.case_space, options.target_id)?;
    let request = NativeReviewRequest {
        target_kind,
        target_id: options.target_id.clone(),
        action: options.action,
        reviewer_id: options.reviewer_id.clone(),
        reviewed_at: timestamp(),
        reason: options.reason.to_owned(),
        evidence_ids: options.evidence_ids.to_vec(),
        source_ids: vec![options.target_id.clone()],
        target_revision_id: generated_revision_id(&replay.case_space, "review", options.target_id)?,
    };
    let morphism = match options.action {
        ReviewAction::Accept => accept_review_morphism(&replay.case_space, request)?,
        ReviewAction::Reject => reject_review_morphism(&replay.case_space, request)?,
        ReviewAction::Reopen => reopen_review_morphism(&replay.case_space, request)?,
        ReviewAction::Defer => defer_review_morphism(&replay.case_space, request)?,
        ReviewAction::Waive | ReviewAction::Supersede => {
            return Err(NativeCliError::invalid("unsupported CLI review action"))
        }
    };
    let command = match options.action {
        ReviewAction::Accept => "casegraphen review accept",
        ReviewAction::Reject => "casegraphen review reject",
        ReviewAction::Reopen => "casegraphen review reopen",
        ReviewAction::Defer => "casegraphen review waive",
        ReviewAction::Waive | ReviewAction::Supersede => unreachable!("action checked above"),
    };
    append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(options.reviewer_id.clone()),
        command,
    )
}

pub(in crate::native_cli) fn evidence_attach(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    input: &Path,
    satisfies_ids: &[Id],
    actor_id: &Id,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    for target_id in satisfies_ids {
        if !reviewable_target_exists(&replay.case_space, target_id) {
            return Err(NativeCliError::invalid(format!(
                "unknown satisfies target {target_id}"
            )));
        }
    }

    let bytes = fs::read(input).map_err(|source| NativeCliError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    let cell = evidence_cell_from_bytes(&bytes)?;
    let sequence = replay.case_space.morphism_log.len() + 1;
    let relations = satisfies_ids
        .iter()
        .enumerate()
        .map(|(index, target_id)| {
            Ok(CaseRelation {
                id: Id::new(format!(
                    "relation:evidence:{}:{}",
                    path_segment(&cell.id),
                    index + 1
                ))?,
                relation_type: CaseRelationType::SatisfiesEvidenceRequirement,
                relation_strength: RelationStrength::Hard,
                from_id: cell.id.clone(),
                to_id: target_id.clone(),
                evidence_ids: vec![cell.id.clone()],
                source_ids: cell.source_ids.clone(),
                provenance: cell.provenance.clone(),
                metadata: Map::new(),
            })
        })
        .collect::<Result<Vec<_>, NativeCliError>>()?;
    let mut added_ids = vec![cell.id.clone()];
    added_ids.extend(relations.iter().map(|relation| relation.id.clone()));
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            added_cells: vec![cell.clone()],
            added_relations: relations,
            ..MorphismPayload::default()
        })?,
    );
    let morphism = CaseMorphism {
        morphism_id: generated_operation_id("morphism:evidence-attach", &cell.id, sequence)?,
        morphism_type: CaseMorphismType::EvidenceAttach,
        source_revision_id: Some(replay.current_revision_id.clone()),
        target_revision_id: generated_operation_id("revision:evidence-attach", &cell.id, sequence)?,
        added_ids,
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: satisfies_ids.to_vec(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: vec![cell.id.clone()],
        source_ids: cell.source_ids.clone(),
        metadata,
    };
    append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(actor_id.clone()),
        "casegraphen evidence attach",
    )
}

pub(in crate::native_cli) fn cell_transition(
    store: &Path,
    case_space_id: &Id,
    base_revision_id: &Id,
    cell_id: &Id,
    lifecycle: &str,
    actor_id: &Id,
    reason: Option<&str>,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(case_space_id)?;
    require_current_revision(&replay.current_revision_id, base_revision_id)?;
    let target_lifecycle = parse_lifecycle(lifecycle)?;
    let mut updated_cell = replay
        .case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == *cell_id)
        .cloned()
        .ok_or_else(|| NativeCliError::invalid(format!("unknown cell id {cell_id}")))?;
    let source_lifecycle = updated_cell.lifecycle;
    updated_cell.lifecycle = target_lifecycle;

    let sequence = replay.case_space.morphism_log.len() + 1;
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            updated_cells: vec![updated_cell.clone()],
            ..MorphismPayload::default()
        })?,
    );
    metadata.insert(
        "transition".to_owned(),
        json!({
            "from": source_lifecycle,
            "to": target_lifecycle,
            "reason": reason,
        }),
    );
    let morphism = CaseMorphism {
        morphism_id: generated_operation_id("morphism:cell-transition", cell_id, sequence)?,
        morphism_type: CaseMorphismType::Update,
        source_revision_id: Some(replay.current_revision_id.clone()),
        target_revision_id: generated_operation_id("revision:cell-transition", cell_id, sequence)?,
        added_ids: Vec::new(),
        updated_ids: vec![cell_id.clone()],
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: updated_cell.source_ids.clone(),
        metadata,
    };
    append_validated_morphism(
        &store_api,
        &replay.case_space,
        morphism,
        Some(actor_id.clone()),
        "casegraphen cell transition",
    )
}

fn evidence_cell_from_bytes(bytes: &[u8]) -> Result<CaseCell, NativeCliError> {
    let mut cell: CaseCell = serde_json::from_slice(bytes)?;
    if cell.cell_type != CaseCellType::Evidence {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} has cell_type {}; expected evidence",
            cell.id, cell.cell_type
        )));
    }
    if cell
        .metadata
        .get("evidence_boundary")
        .and_then(Value::as_str)
        == Some("accepted_evidence")
    {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} cannot claim evidence_boundary \"accepted_evidence\"; use review accept to promote evidence",
            cell.id
        )));
    }
    if cell.provenance.review_status == ReviewStatus::Accepted {
        return Err(NativeCliError::invalid(format!(
            "evidence attach input cell {} cannot claim accepted provenance; use review accept to promote evidence",
            cell.id
        )));
    }
    cell.metadata.insert(
        "content_hash".to_owned(),
        Value::String(crate::native_hash::sha256_hex(bytes)),
    );
    Ok(cell)
}

fn parse_lifecycle(value: &str) -> Result<CaseCellLifecycle, NativeCliError> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| NativeCliError::invalid(format!("invalid lifecycle {value:?}: {error}")))
}

fn review_target_kind(
    case_space: &CaseSpace,
    target_id: &Id,
) -> Result<NativeReviewTargetKind, NativeCliError> {
    if let Some(cell) = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == *target_id)
    {
        return Ok(match cell.cell_type {
            CaseCellType::Completion => NativeReviewTargetKind::Completion,
            CaseCellType::Evidence => NativeReviewTargetKind::Evidence,
            _ => NativeReviewTargetKind::Waiver,
        });
    }
    if case_space
        .case_relations
        .iter()
        .any(|relation| relation.id == *target_id)
    {
        return Ok(NativeReviewTargetKind::Waiver);
    }
    if case_space
        .morphism_log
        .iter()
        .any(|entry| entry.morphism_id == *target_id)
    {
        return Ok(NativeReviewTargetKind::Morphism);
    }
    Err(NativeCliError::invalid(format!(
        "unknown review target {target_id}"
    )))
}

fn reviewable_target_exists(case_space: &CaseSpace, target_id: &Id) -> bool {
    case_space
        .case_cells
        .iter()
        .any(|cell| cell.id == *target_id)
        || case_space
            .case_relations
            .iter()
            .any(|relation| relation.id == *target_id)
        || case_space
            .morphism_log
            .iter()
            .any(|entry| entry.morphism_id == *target_id)
}

fn generated_revision_id(
    case_space: &CaseSpace,
    operation: &str,
    subject_id: &Id,
) -> Result<Id, NativeCliError> {
    generated_operation_id(
        &format!("revision:{operation}"),
        subject_id,
        case_space.morphism_log.len() + 1,
    )
}

fn generated_operation_id(
    prefix: &str,
    subject_id: &Id,
    sequence: usize,
) -> Result<Id, NativeCliError> {
    Ok(Id::new(format!(
        "{prefix}:{}:{sequence}",
        path_segment(subject_id)
    ))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVIDENCE_CELL: &[u8] = br#"{
        "id": "evidence:unit-test",
        "cell_type": "evidence",
        "space_id": "space:unit-test",
        "title": "Unit test evidence",
        "lifecycle": "active",
        "source_ids": ["source:unit-test"],
        "structure_ids": [],
        "provenance": {
            "source": {"kind": "document", "title": "Unit test"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "metadata": {}
    }"#;

    #[test]
    fn evidence_cell_validation_adds_bare_sha256_content_hash() {
        let cell = evidence_cell_from_bytes(EVIDENCE_CELL).expect("valid evidence cell");
        let hash = cell.metadata["content_hash"]
            .as_str()
            .expect("content hash");

        assert_eq!(hash, crate::native_hash::sha256_hex(EVIDENCE_CELL));
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(cell.provenance.review_status, ReviewStatus::Unreviewed);
    }

    #[test]
    fn evidence_cell_validation_overwrites_caller_content_hash() {
        let bytes = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"metadata\": {}",
                "\"metadata\": {\"content_hash\":\"bogus\"}",
            );
        let cell = evidence_cell_from_bytes(bytes.as_bytes()).expect("valid evidence cell");

        assert_eq!(
            cell.metadata["content_hash"],
            json!(crate::native_hash::sha256_hex(bytes.as_bytes()))
        );
    }

    #[test]
    fn evidence_cell_validation_rejects_caller_claimed_acceptance() {
        let accepted_boundary = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"metadata\": {}",
                "\"metadata\": {\"evidence_boundary\":\"accepted_evidence\"}",
            );
        let boundary_error = evidence_cell_from_bytes(accepted_boundary.as_bytes())
            .expect_err("accepted boundary must require review");
        assert!(boundary_error.to_string().contains("review accept"));

        let accepted_review = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace(
                "\"review_status\": \"unreviewed\"",
                "\"review_status\": \"accepted\"",
            );
        let review_error = evidence_cell_from_bytes(accepted_review.as_bytes())
            .expect_err("accepted provenance must require review");
        assert!(review_error.to_string().contains("review accept"));
    }

    #[test]
    fn evidence_cell_validation_rejects_non_evidence_cells() {
        let bytes = String::from_utf8(EVIDENCE_CELL.to_vec())
            .expect("UTF-8 fixture")
            .replace("\"evidence\"", "\"work\"");
        let error = evidence_cell_from_bytes(bytes.as_bytes()).expect_err("reject work cell");

        assert!(error.to_string().contains("expected evidence"));
    }

    #[test]
    fn lifecycle_parser_uses_serde_names_and_lists_valid_values() {
        assert_eq!(
            parse_lifecycle("resolved").expect("resolved lifecycle"),
            CaseCellLifecycle::Resolved
        );
        let error = parse_lifecycle("Resolved").expect_err("serde names are lowercase");
        let message = error.to_string();
        assert!(message.contains("proposed"));
        assert!(message.contains("superseded"));
    }
}
