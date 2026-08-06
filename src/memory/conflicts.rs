use super::AuthorityLevel;
use crate::native_model::{
    CaseCellLifecycle, CaseCellType, CaseRelation, CaseRelationType, CaseSpace, RelationStrength,
};
use higher_graphen_core::{Id, ReviewStatus};
use std::collections::BTreeSet;

pub(crate) fn accepted_relation_ids(case_space: &CaseSpace) -> BTreeSet<Id> {
    let mut accepted = case_space
        .case_relations
        .iter()
        .filter(|relation| relation.provenance.review_status == ReviewStatus::Accepted)
        .map(|relation| relation.id.clone())
        .collect::<BTreeSet<_>>();
    accepted.extend(
        case_space
            .morphism_log
            .iter()
            .filter(|entry| entry.morphism.review_status == ReviewStatus::Accepted)
            .flat_map(|entry| entry.morphism.added_ids.iter().cloned()),
    );
    accepted
}

fn accepted_relation(accepted_relation_ids: &BTreeSet<Id>, relation: &CaseRelation) -> bool {
    accepted_relation_ids.contains(&relation.id)
}

pub(crate) fn relation_name(relation: &CaseRelation) -> String {
    relation.relation_type.serialized_value()
}

pub(crate) fn superseding_claims<'a>(
    case_space: &'a CaseSpace,
    accepted_relation_ids: &'a BTreeSet<Id>,
    target_id: &'a str,
) -> impl Iterator<Item = &'a str> {
    case_space
        .case_relations
        .iter()
        .filter_map(move |relation| {
            (relation.to_id.as_str() == target_id
                && relation.relation_type == CaseRelationType::Supersedes
                && accepted_relation(accepted_relation_ids, relation))
            .then_some(relation.from_id.as_str())
        })
}

pub(crate) fn retracting_claims<'a>(
    case_space: &'a CaseSpace,
    accepted_relation_ids: &'a BTreeSet<Id>,
    target_id: &'a str,
) -> impl Iterator<Item = &'a str> {
    case_space
        .case_relations
        .iter()
        .filter_map(move |relation| {
            (relation.to_id.as_str() == target_id
                && is_retraction(&relation.relation_type)
                && accepted_relation(accepted_relation_ids, relation))
            .then_some(relation.from_id.as_str())
        })
}

fn is_retraction(relation_type: &CaseRelationType) -> bool {
    relation_type == &CaseRelationType::Invalidates
        || matches!(relation_type, CaseRelationType::Custom(value) if value == "retracts")
}

pub(crate) fn contradictions<'a>(
    case_space: &'a CaseSpace,
    accepted_relation_ids: &'a BTreeSet<Id>,
    claim_id: &'a str,
    hard_relation_names: &'a [String],
) -> impl Iterator<Item = (&'a str, bool)> + 'a {
    case_space
        .case_relations
        .iter()
        .filter_map(move |relation| {
            if !accepted_relation(accepted_relation_ids, relation)
                || relation.relation_type != CaseRelationType::Contradicts
            {
                return None;
            }
            let other = if relation.from_id.as_str() == claim_id {
                relation.to_id.as_str()
            } else if relation.to_id.as_str() == claim_id {
                relation.from_id.as_str()
            } else {
                return None;
            };
            let hard = relation.relation_strength == RelationStrength::Hard
                || hard_relation_names.contains(&relation_name(relation));
            Some((other, hard))
        })
}

pub(crate) fn has_source_relation(
    case_space: &CaseSpace,
    accepted_relation_ids: &BTreeSet<Id>,
    claim_id: &str,
    source_id: &str,
) -> bool {
    case_space.case_relations.iter().any(|relation| {
        relation.from_id.as_str() == claim_id
            && relation.to_id.as_str() == source_id
            && relation.relation_type == CaseRelationType::DerivesFrom
            && accepted_relation(accepted_relation_ids, relation)
    })
}

pub(crate) fn has_authority_binding(
    case_space: &CaseSpace,
    accepted_relation_ids: &BTreeSet<Id>,
    claim_id: &str,
    required_authority: AuthorityLevel,
) -> bool {
    case_space.case_relations.iter().any(|relation| {
        relation.from_id.as_str() == claim_id
            && matches!(
                &relation.relation_type,
                CaseRelationType::Custom(value) if value == "authorized_by"
            )
            && relation.relation_strength == RelationStrength::Hard
            && accepted_relation(accepted_relation_ids, relation)
            && case_space.case_cells.iter().any(|cell| {
                cell.id == relation.to_id
                    && cell.cell_type == CaseCellType::Custom("capability".to_owned())
                    && matches!(
                        cell.lifecycle,
                        CaseCellLifecycle::Active | CaseCellLifecycle::Accepted
                    )
                    && cell.provenance.review_status == ReviewStatus::Accepted
                    && cell
                        .metadata
                        .get("memory_authority_level")
                        .cloned()
                        .and_then(|value| serde_json::from_value::<AuthorityLevel>(value).ok())
                        .is_some_and(|authority| authority >= required_authority)
            })
    })
}
