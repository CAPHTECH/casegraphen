use crate::native_model::{CaseRelation, CaseRelationType, CaseSpace, RelationStrength};
use higher_graphen_core::{Id, ReviewStatus};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct NativeCaseIndex<'a> {
    relations_by_from: BTreeMap<&'a str, Vec<&'a CaseRelation>>,
    relations_by_to: BTreeMap<&'a str, Vec<&'a CaseRelation>>,
    completed_targets: BTreeSet<Id>,
    latest_evidence_review_statuses: BTreeMap<&'a str, ReviewStatus>,
}

impl<'a> NativeCaseIndex<'a> {
    pub(super) fn from_case_space(case_space: &'a CaseSpace) -> Self {
        let mut relations_by_from = BTreeMap::<&str, Vec<&CaseRelation>>::new();
        let mut relations_by_to = BTreeMap::<&str, Vec<&CaseRelation>>::new();
        let mut completed_targets = BTreeSet::new();

        for relation in &case_space.case_relations {
            relations_by_from
                .entry(relation.from_id.as_str())
                .or_default()
                .push(relation);
            relations_by_to
                .entry(relation.to_id.as_str())
                .or_default()
                .push(relation);
            if relation.relation_strength == RelationStrength::Hard
                && matches!(
                    relation.relation_type,
                    CaseRelationType::Completes | CaseRelationType::Supersedes
                )
            {
                completed_targets.insert(relation.to_id.clone());
            }
        }

        Self {
            relations_by_from,
            relations_by_to,
            completed_targets,
            latest_evidence_review_statuses: super::sections::latest_evidence_review_statuses(
                case_space,
            ),
        }
    }

    pub(super) fn direct_targets(&self, cell_id: &Id, relation_type: CaseRelationType) -> Vec<Id> {
        self.relations_from(cell_id)
            .iter()
            .filter(|relation| {
                relation.relation_strength == RelationStrength::Hard
                    && relation.relation_type == relation_type
            })
            .map(|relation| relation.to_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(super) fn relations_from(&self, id: &Id) -> &[&'a CaseRelation] {
        self.relations_by_from
            .get(id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn relations_to(&self, id: &Id) -> &[&'a CaseRelation] {
        self.relations_by_to
            .get(id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn completed_targets(&self) -> &BTreeSet<Id> {
        &self.completed_targets
    }

    pub(super) fn latest_evidence_review_status(&self, evidence_id: &Id) -> Option<ReviewStatus> {
        self.latest_evidence_review_statuses
            .get(evidence_id.as_str())
            .copied()
    }
}
