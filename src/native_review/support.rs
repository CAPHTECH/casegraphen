use super::{NativeReviewError, NativeReviewTargetKind, REVIEW_SCHEMA_VERSION};
use crate::{
    native_eval::{NativeCloseInvariantResult, NativeCompletionCandidate, NativeObstruction},
    native_model::{
        CaseCell, CaseCellType, CaseMorphism, CaseMorphismType, CaseRelationType, CaseSpace,
        EvidenceBoundary, ReviewAction,
    },
};
use higher_graphen_core::{Id, ReviewStatus, Severity};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn review_metadata(
    request: &super::NativeReviewRequest,
    outcome_review_status: ReviewStatus,
    morphism_id: &Id,
) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert(
        "native_review_schema_version".to_owned(),
        json!(REVIEW_SCHEMA_VERSION),
    );
    metadata.insert(
        "review_id".to_owned(),
        json!(generated_id("review", &[morphism_id.as_str()])),
    );
    metadata.insert("target_kind".to_owned(), json!(request.target_kind));
    metadata.insert("target_id".to_owned(), json!(request.target_id));
    metadata.insert("action".to_owned(), json!(request.action));
    metadata.insert(
        "outcome_review_status".to_owned(),
        json!(outcome_review_status),
    );
    metadata.insert("reviewer_id".to_owned(), json!(request.reviewer_id));
    metadata.insert("reviewed_at".to_owned(), json!(request.reviewed_at));
    metadata.insert("reason".to_owned(), json!(request.reason.trim()));
    metadata
}

pub(super) fn explicit_reviews(case_space: &CaseSpace) -> BTreeMap<Id, Vec<ExplicitReview>> {
    let mut reviews = BTreeMap::<Id, Vec<ExplicitReview>>::new();
    for morphism in case_space.morphism_log.iter().map(|entry| &entry.morphism) {
        let Some(review) = canonical_review(morphism) else {
            continue;
        };
        reviews
            .entry(review.target_id.clone())
            .or_default()
            .push(ExplicitReview {
                target_id: review.target_id,
                action: review.action,
                outcome: review.outcome,
            });
    }
    reviews
}

#[derive(Clone, Debug)]
pub(crate) struct CanonicalReview {
    pub(crate) target_kind: NativeReviewTargetKind,
    pub(crate) target_id: Id,
    pub(crate) action: ReviewAction,
    pub(crate) outcome: ReviewStatus,
}

pub(crate) fn canonical_review(morphism: &CaseMorphism) -> Option<CanonicalReview> {
    if morphism.morphism_type != CaseMorphismType::Review
        || morphism.review_status != ReviewStatus::Accepted
        || morphism
            .metadata
            .get("native_review_schema_version")
            .and_then(Value::as_u64)
            != Some(u64::from(REVIEW_SCHEMA_VERSION))
    {
        return None;
    }
    for field in ["review_id", "target_id", "reviewer_id"] {
        let value = morphism.metadata.get(field)?.as_str()?;
        if !Id::is_valid_value(value) {
            return None;
        }
    }
    for field in ["reviewed_at", "reason"] {
        if !morphism
            .metadata
            .get(field)?
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return None;
        }
    }
    let target_kind: NativeReviewTargetKind =
        serde_json::from_value(morphism.metadata.get("target_kind")?.clone()).ok()?;
    let target_id = Id::new(morphism.metadata.get("target_id")?.as_str()?).ok()?;
    let action: ReviewAction =
        serde_json::from_value(morphism.metadata.get("action")?.clone()).ok()?;
    let outcome: ReviewStatus =
        serde_json::from_value(morphism.metadata.get("outcome_review_status")?.clone()).ok()?;
    if outcome != outcome_status(action) {
        return None;
    }
    Some(CanonicalReview {
        target_kind,
        target_id,
        action,
        outcome,
    })
}

#[derive(Clone, Debug)]
pub(super) struct ExplicitReview {
    target_id: Id,
    action: ReviewAction,
    outcome: ReviewStatus,
}

pub(super) fn unresolved_hard_obstruction(
    obstruction: &NativeObstruction,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> bool {
    if !obstruction.blocking || !matches!(obstruction.severity, Severity::High | Severity::Critical)
    {
        return false;
    }
    !target_has_action(reviews, &obstruction.id, ReviewAction::Waive)
        && !target_has_action(reviews, &obstruction.id, ReviewAction::Defer)
}

pub(super) fn completion_reviewed_or_deferred(
    candidate: &NativeCompletionCandidate,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> bool {
    candidate.review_status.has_review_action()
        || target_has_terminal_review(reviews, &candidate.id)
}

pub(super) fn evidence_requirement_blockers(
    case_space: &CaseSpace,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> Vec<Id> {
    let cells = case_space
        .case_cells
        .iter()
        .map(|cell| (cell.id.clone(), cell))
        .collect::<BTreeMap<_, _>>();
    let mut blockers = Vec::new();
    for relation in case_space
        .case_relations
        .iter()
        .filter(|relation| relation.relation_type == CaseRelationType::RequiresEvidence)
    {
        if target_has_action(reviews, &relation.id, ReviewAction::Waive)
            || target_has_action(reviews, &relation.to_id, ReviewAction::Waive)
        {
            continue;
        }
        let acceptable = cells.get(&relation.to_id).is_some_and(|cell| {
            cell.cell_type == CaseCellType::Evidence && evidence_acceptable_for_close(cell, reviews)
        });
        if !acceptable {
            blockers.push(relation.to_id.clone());
        }
    }
    dedupe_ids(blockers)
}

fn evidence_acceptable_for_close(
    cell: &CaseCell,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> bool {
    if cell.provenance.review_status == ReviewStatus::Rejected
        || target_has_action(reviews, &cell.id, ReviewAction::Reject)
    {
        return false;
    }
    let boundary = cell
        .metadata
        .get("evidence_boundary")
        .and_then(Value::as_str)
        .map(evidence_boundary_value)
        .unwrap_or(EvidenceBoundary::Inferred);
    let review_promoted = target_has_action(reviews, &cell.id, ReviewAction::Accept);
    let has_source = !cell.source_ids.is_empty();
    let accepted = cell.provenance.review_status == ReviewStatus::Accepted;
    match boundary {
        EvidenceBoundary::SourceBacked => has_source,
        EvidenceBoundary::ReviewPromoted => has_source && (accepted || review_promoted),
        EvidenceBoundary::Inferred | EvidenceBoundary::WorkerOutput => {
            has_source && review_promoted
        }
        EvidenceBoundary::Rejected | EvidenceBoundary::Contradicting => false,
    }
}

fn evidence_boundary_value(value: &str) -> EvidenceBoundary {
    match value {
        "source_backed" | "source_backed_evidence" => EvidenceBoundary::SourceBacked,
        "worker_output" => EvidenceBoundary::WorkerOutput,
        "review_promoted" | "review_promotion" => EvidenceBoundary::ReviewPromoted,
        "rejected" => EvidenceBoundary::Rejected,
        "contradicting" => EvidenceBoundary::Contradicting,
        _ => EvidenceBoundary::Inferred,
    }
}

pub(super) fn target_has_terminal_review(
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
    target_id: &Id,
) -> bool {
    latest_review_for(reviews, target_id).is_some_and(|review| {
        matches!(
            review.action,
            ReviewAction::Accept | ReviewAction::Reject | ReviewAction::Defer
        ) && review.outcome.has_review_action()
    })
}

pub(super) fn target_has_action(
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
    target_id: &Id,
    action: ReviewAction,
) -> bool {
    latest_review_for(reviews, target_id)
        .is_some_and(|review| review.action == action && review.outcome.has_review_action())
}

fn latest_review_for<'a>(
    reviews: &'a BTreeMap<Id, Vec<ExplicitReview>>,
    target_id: &Id,
) -> Option<&'a ExplicitReview> {
    reviews
        .get(target_id)?
        .iter()
        .rev()
        .find(|review| review.target_id == *target_id)
}

pub(super) fn close_invariant(
    invariant_id: &str,
    witness_ids: Vec<Id>,
    message: &str,
) -> NativeCloseInvariantResult {
    NativeCloseInvariantResult {
        invariant_id: id(invariant_id),
        passed: witness_ids.is_empty(),
        severity: Severity::High,
        witness_ids,
        message: Some(message.to_owned()),
    }
}

pub(super) fn outcome_status(action: ReviewAction) -> ReviewStatus {
    match action {
        ReviewAction::Accept | ReviewAction::Waive => ReviewStatus::Accepted,
        ReviewAction::Reject => ReviewStatus::Rejected,
        ReviewAction::Reopen => ReviewStatus::Unreviewed,
        ReviewAction::Defer | ReviewAction::Supersede => ReviewStatus::Reviewed,
    }
}

pub(super) fn has_known_id(case_space: &CaseSpace, target_id: &Id) -> bool {
    case_space
        .case_cells
        .iter()
        .any(|cell| cell.id == *target_id)
        || case_space
            .case_relations
            .iter()
            .any(|relation| relation.id == *target_id)
        || case_space
            .projections
            .iter()
            .any(|projection| projection.projection_id == *target_id)
        || case_space
            .morphism_log
            .iter()
            .any(|entry| entry.entry_id == *target_id || entry.morphism_id == *target_id)
        || case_space.revision.revision_id == *target_id
}

pub(super) fn target_kind_stem(target_kind: NativeReviewTargetKind) -> &'static str {
    match target_kind {
        NativeReviewTargetKind::Completion => "completion",
        NativeReviewTargetKind::Evidence => "evidence",
        NativeReviewTargetKind::Morphism => "morphism",
        NativeReviewTargetKind::Plan => "plan",
        NativeReviewTargetKind::ResidualRisk => "residual-risk",
        NativeReviewTargetKind::Waiver => "waiver",
    }
}

pub(super) fn action_stem(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::Accept => "accept",
        ReviewAction::Reject => "reject",
        ReviewAction::Reopen => "reopen",
        ReviewAction::Waive => "waive",
        ReviewAction::Defer => "defer",
        ReviewAction::Supersede => "supersede",
    }
}

pub(super) fn generated_id(prefix: &str, parts: &[&str]) -> Id {
    let suffix = parts
        .iter()
        .map(|part| sanitize(part))
        .collect::<Vec<_>>()
        .join(":");
    id(&format!("{prefix}:{suffix}"))
}

pub(super) fn dedupe_ids(ids: Vec<Id>) -> Vec<Id> {
    ids.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn id(value: &str) -> Id {
    Id::new(value).expect("static or generated id")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' => character,
            _ => '-',
        })
        .collect()
}

pub(super) fn error(message: impl Into<String>) -> NativeReviewError {
    NativeReviewError {
        message: message.into(),
    }
}

pub(super) fn morphism_type_for_review(
    _target_kind: NativeReviewTargetKind,
    _action: ReviewAction,
) -> CaseMorphismType {
    CaseMorphismType::Review
}
