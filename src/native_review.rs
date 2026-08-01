use crate::{
    native_eval::{
        evaluate_native_case, latest_evidence_review_statuses, NativeCaseEvaluation,
        NativeCloseInvariantResult, NativeEvalError, NativeReviewGapType,
    },
    native_model::{
        CaseCellLifecycle, CaseCellType, CaseMorphism, CaseSpace, ProjectionAudience, ReviewAction,
    },
};
use higher_graphen_core::{Id, ReviewStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

mod support;
pub(crate) use support::canonical_review;
use support::*;

const REVIEW_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeReviewTargetKind {
    Completion,
    Evidence,
    Morphism,
    Plan,
    ResidualRisk,
    Waiver,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReviewRequest {
    pub target_kind: NativeReviewTargetKind,
    pub target_id: Id,
    pub action: ReviewAction,
    pub reviewer_id: Id,
    pub reviewed_at: String,
    pub reason: String,
    pub evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub target_revision_id: Id,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCloseCheckRequest {
    pub close_policy_id: Option<Id>,
    pub base_revision_id: Id,
    pub declared_projection_loss_ids: Vec<Id>,
    pub validation_evidence_ids: Vec<Id>,
    pub source_ids: Vec<Id>,
    pub operation_gate: Option<NativeOperationGate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeOperationGate {
    pub actor_id: Id,
    pub operation: String,
    pub operation_scope_id: Id,
    pub audience: ProjectionAudience,
    pub capability_ids: Vec<Id>,
    pub source_boundary_id: Id,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeOperationGateError {
    message: String,
    witness_ids: Vec<Id>,
}

impl std::fmt::Display for NativeOperationGateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for NativeOperationGateError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCloseCheck {
    pub check_id: Id,
    pub case_space_id: Id,
    pub revision_id: Id,
    pub close_policy_id: Option<Id>,
    pub closeable: bool,
    pub operation_gate: Option<NativeOperationGate>,
    pub invariant_results: Vec<NativeCloseInvariantResult>,
    pub blocker_ids: Vec<Id>,
}

pub type NativeReviewResult<T> = Result<T, NativeReviewError>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReviewError {
    pub message: String,
}

impl std::fmt::Display for NativeReviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for NativeReviewError {}

impl From<NativeEvalError> for NativeReviewError {
    fn from(error: NativeEvalError) -> Self {
        Self {
            message: format!("native case evaluation failed: {error:?}"),
        }
    }
}

pub fn accept_review_morphism(
    case_space: &CaseSpace,
    request: NativeReviewRequest,
) -> NativeReviewResult<CaseMorphism> {
    build_review_morphism(case_space, ReviewAction::Accept, request)
}

pub fn reject_review_morphism(
    case_space: &CaseSpace,
    request: NativeReviewRequest,
) -> NativeReviewResult<CaseMorphism> {
    build_review_morphism(case_space, ReviewAction::Reject, request)
}

pub fn reopen_review_morphism(
    case_space: &CaseSpace,
    request: NativeReviewRequest,
) -> NativeReviewResult<CaseMorphism> {
    build_review_morphism(case_space, ReviewAction::Reopen, request)
}

pub fn defer_review_morphism(
    case_space: &CaseSpace,
    request: NativeReviewRequest,
) -> NativeReviewResult<CaseMorphism> {
    build_review_morphism(case_space, ReviewAction::Defer, request)
}

pub fn build_review_morphism(
    case_space: &CaseSpace,
    action: ReviewAction,
    mut request: NativeReviewRequest,
) -> NativeReviewResult<CaseMorphism> {
    request.action = action;
    require_review_request(case_space, &request)?;
    let outcome_review_status = outcome_status(action);
    let morphism_type = morphism_type_for_review(request.target_kind, action);
    let mut source_ids = dedupe_ids(
        request
            .source_ids
            .iter()
            .chain(&request.evidence_ids)
            .cloned()
            .collect(),
    );
    if source_ids.is_empty() {
        source_ids = vec![request.target_id.clone()];
    }
    let morphism_id = generated_id(
        "morphism:review",
        &[
            target_kind_stem(request.target_kind),
            request.target_id.as_str(),
            action_stem(action),
            request.target_revision_id.as_str(),
        ],
    );
    Ok(CaseMorphism {
        morphism_id: morphism_id.clone(),
        morphism_type,
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: request.target_revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: if has_known_id(case_space, &request.target_id) {
            vec![request.target_id.clone()]
        } else {
            Vec::new()
        },
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: request.evidence_ids.clone(),
        source_ids: source_ids.clone(),
        metadata: review_metadata(&request, outcome_review_status, &morphism_id),
    })
}

pub fn check_native_close(
    case_space: &CaseSpace,
    request: NativeCloseCheckRequest,
) -> NativeReviewResult<NativeCloseCheck> {
    check_native_close_with_finding(case_space, request).map(|(check, _)| check)
}

pub(crate) fn check_native_close_with_finding(
    case_space: &CaseSpace,
    request: NativeCloseCheckRequest,
) -> NativeReviewResult<(NativeCloseCheck, bool)> {
    let evaluation = evaluate_native_case(case_space)?;
    let reviews = explicit_reviews(case_space);
    // Computed once and threaded down to `evidence_accepted_invariant`, the
    // same single source the evaluator's trust decision reads
    // (`NativeCaseIndex::latest_evidence_review_status`), so the close check
    // cannot answer "is this evidence accepted" differently than
    // `space reason` does for the same revision.
    let evidence_review_statuses = latest_evidence_review_statuses(case_space);
    let invariant_results = close_invariants(
        case_space,
        &request,
        &evaluation,
        &reviews,
        &evidence_review_statuses,
    );

    let blocker_ids = dedupe_ids(
        invariant_results
            .iter()
            .filter(|result| !result.passed)
            .flat_map(|result| result.witness_ids.iter().cloned())
            .collect(),
    );
    let check = NativeCloseCheck {
        check_id: generated_id(
            "close_check",
            &[
                case_space.case_space_id.as_str(),
                request.base_revision_id.as_str(),
                "native-review",
            ],
        ),
        case_space_id: case_space.case_space_id.clone(),
        revision_id: case_space.revision.revision_id.clone(),
        close_policy_id: request
            .close_policy_id
            .or_else(|| case_space.close_policy_id.clone()),
        closeable: invariant_results.iter().all(|result| result.passed),
        operation_gate: request.operation_gate,
        invariant_results,
        blocker_ids,
    };
    // A stale base revision is a close verdict, not a tool failure: close-check
    // deliberately reports "cannot close at this revision" in the payload rather than raising.
    let domain_finding = !check.closeable;
    Ok((check, domain_finding))
}

pub fn check_operation_gate(
    case_space: &CaseSpace,
    gate: &NativeOperationGate,
    expected_operation: &str,
) -> Result<(), NativeOperationGateError> {
    let failures = operation_gate_failures(case_space, gate, expected_operation);
    if failures.is_empty() {
        return Ok(());
    }
    let witness_ids = failures
        .iter()
        .flat_map(|failure| failure.witness_ids.iter().cloned())
        .collect();
    let labels = failures
        .iter()
        .map(|failure| failure.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    Err(NativeOperationGateError {
        message: format!("operation gate for {expected_operation:?} violates: {labels}"),
        witness_ids,
    })
}

struct OperationGateFailure {
    message: String,
    witness_ids: Vec<Id>,
}

fn operation_gate_failures(
    case_space: &CaseSpace,
    gate: &NativeOperationGate,
    expected_operation: &str,
) -> Vec<OperationGateFailure> {
    let mut failures = Vec::new();
    if gate.operation != expected_operation {
        failures.push(OperationGateFailure {
            message: "operation".to_owned(),
            witness_ids: vec![gate.actor_id.clone()],
        });
    }
    if gate.operation_scope_id != case_space.case_space_id {
        failures.push(OperationGateFailure {
            message: "operation_scope_id".to_owned(),
            witness_ids: vec![
                gate.operation_scope_id.clone(),
                case_space.case_space_id.clone(),
            ],
        });
    }
    if !matches!(
        gate.audience,
        ProjectionAudience::Audit | ProjectionAudience::System
    ) {
        failures.push(OperationGateFailure {
            message: "audience".to_owned(),
            witness_ids: vec![gate.actor_id.clone()],
        });
    }
    if gate.capability_ids.is_empty() {
        failures.push(OperationGateFailure {
            message: "capability_ids must not be empty".to_owned(),
            witness_ids: vec![gate.actor_id.clone()],
        });
    }
    for capability_id in &gate.capability_ids {
        let Some(capability) = case_space
            .case_cells
            .iter()
            .find(|cell| cell.id == *capability_id)
        else {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} does not resolve to an existing case cell"
                ),
                witness_ids: vec![capability_id.clone()],
            });
            continue;
        };
        if capability.cell_type != CaseCellType::Custom("capability".to_owned()) {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} must have cell_type custom:capability"
                ),
                witness_ids: vec![capability_id.clone()],
            });
        }
        if !matches!(
            capability.lifecycle,
            CaseCellLifecycle::Active | CaseCellLifecycle::Accepted
        ) {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} must have lifecycle active or accepted"
                ),
                witness_ids: vec![capability_id.clone()],
            });
        }
        if capability.provenance.review_status != ReviewStatus::Accepted {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} must have provenance.review_status accepted"
                ),
                witness_ids: vec![capability_id.clone()],
            });
        }
        let grants_actor = capability
            .metadata
            .get("actor_ids")
            .and_then(Value::as_array)
            .is_some_and(|actor_ids| {
                actor_ids
                    .iter()
                    .any(|actor_id| actor_id.as_str() == Some(gate.actor_id.as_str()))
            });
        if !grants_actor {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} does not grant acting actor {}; \
                     metadata.actor_ids must contain the gate actor id",
                    gate.actor_id
                ),
                witness_ids: vec![capability_id.clone(), gate.actor_id.clone()],
            });
        }
        // A capability names an authority, and an authority is over something.
        // Without this the four cells the shipped walkthrough splits by role
        // were four labels for one power: the dispatch-only runner could pass
        // `review accept` with its dispatch capability, which is the opposite of
        // what separating the roles was for (ADR 0007).
        //
        // `metadata.operations` is required rather than optional-and-permissive.
        // An absent list would have to mean something, and "every operation" is
        // the defect restated as a default. Capability cells enter only at
        // genesis and there is no amendment path, so a space written before this
        // is lifted again — which is how any capability change works here.
        let grants_operation = capability
            .metadata
            .get("operations")
            .and_then(Value::as_array)
            .is_some_and(|operations| {
                operations
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(expected_operation))
            });
        if !grants_operation {
            failures.push(OperationGateFailure {
                message: format!(
                    "capability {capability_id} does not authorize operation \
                     {expected_operation}; metadata.operations must list it"
                ),
                witness_ids: vec![capability_id.clone()],
            });
        }
    }
    if declared_source_boundary_id(case_space).as_ref() != Some(&gate.source_boundary_id) {
        failures.push(OperationGateFailure {
            message: "source_boundary_id".to_owned(),
            witness_ids: vec![gate.source_boundary_id.clone()],
        });
    }
    failures
}

fn close_invariants(
    case_space: &CaseSpace,
    request: &NativeCloseCheckRequest,
    evaluation: &NativeCaseEvaluation,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
    evidence_review_statuses: &BTreeMap<&str, ReviewStatus>,
) -> Vec<NativeCloseInvariantResult> {
    vec![
        base_revision_invariant(case_space, request),
        source_boundary_declared_invariant(case_space),
        hard_obstructions_invariant(evaluation, reviews),
        completions_reviewed_invariant(evaluation, reviews),
        morphisms_reviewed_invariant(evaluation, reviews),
        evidence_accepted_invariant(case_space, reviews, evidence_review_statuses),
        projection_loss_declared_invariant(request, evaluation, reviews),
        policy_capability_gate_invariant(case_space, request),
        validation_evidence_invariant(case_space, request),
    ]
}

fn base_revision_invariant(
    case_space: &CaseSpace,
    request: &NativeCloseCheckRequest,
) -> NativeCloseInvariantResult {
    let witness_ids = if request.base_revision_id == case_space.revision.revision_id {
        Vec::new()
    } else {
        vec![
            request.base_revision_id.clone(),
            case_space.revision.revision_id.clone(),
        ]
    };
    close_invariant(
        "close:native-base-revision-matches",
        witness_ids,
        "The close-check base revision must match the materialized case-space revision.",
    )
}

fn hard_obstructions_invariant(
    evaluation: &NativeCaseEvaluation,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> NativeCloseInvariantResult {
    close_invariant(
        "close:native-no-hard-obstructions",
        evaluation
            .obstructions
            .iter()
            .filter(|obstruction| unresolved_hard_obstruction(obstruction, reviews))
            .map(|obstruction| obstruction.id.clone())
            .collect(),
        "No unresolved high or critical hard obstruction may remain.",
    )
}

fn source_boundary_declared_invariant(case_space: &CaseSpace) -> NativeCloseInvariantResult {
    let witness_ids = if has_source_boundary(&case_space.metadata) {
        Vec::new()
    } else {
        vec![case_space.case_space_id.clone()]
    };
    close_invariant(
        "close:native-source-boundary-declared",
        witness_ids,
        "Close checks require a declared source boundary for the lifted case space.",
    )
}

fn completions_reviewed_invariant(
    evaluation: &NativeCaseEvaluation,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> NativeCloseInvariantResult {
    close_invariant(
        "close:native-completions-reviewed",
        evaluation
            .completion_candidates
            .iter()
            .filter(|candidate| !completion_reviewed_or_deferred(candidate, reviews))
            .map(|candidate| candidate.id.clone())
            .collect(),
        "Completion candidates must be accepted, rejected, or explicitly deferred.",
    )
}

fn morphisms_reviewed_invariant(
    evaluation: &NativeCaseEvaluation,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> NativeCloseInvariantResult {
    close_invariant(
        "close:native-morphisms-reviewed",
        evaluation
            .review_gaps
            .iter()
            .filter(|gap| gap.gap_type == NativeReviewGapType::UnreviewedMorphism)
            .filter(|gap| !target_has_terminal_review(reviews, &gap.target_id))
            .map(|gap| gap.target_id.clone())
            .collect(),
        "Generated morphisms must remain reviewable until accepted, rejected, or deferred.",
    )
}

fn evidence_accepted_invariant(
    case_space: &CaseSpace,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
    evidence_review_statuses: &BTreeMap<&str, ReviewStatus>,
) -> NativeCloseInvariantResult {
    close_invariant(
        "close:native-evidence-accepted-or-waived",
        evidence_requirement_blockers(case_space, reviews, evidence_review_statuses),
        "Required evidence must be source-backed, review-promoted, accepted, or explicitly waived.",
    )
}

fn projection_loss_declared_invariant(
    request: &NativeCloseCheckRequest,
    evaluation: &NativeCaseEvaluation,
    reviews: &BTreeMap<Id, Vec<ExplicitReview>>,
) -> NativeCloseInvariantResult {
    close_invariant(
        "close:native-projection-loss-declared",
        evaluation
            .projection_loss
            .iter()
            .filter(|loss| {
                !request
                    .declared_projection_loss_ids
                    .contains(&loss.projection_id)
            })
            .filter(|loss| !target_has_action(reviews, &loss.projection_id, ReviewAction::Accept))
            .map(|loss| loss.projection_id.clone())
            .collect(),
        "Projection loss must be declared by the close-check caller or accepted by review.",
    )
}

fn validation_evidence_invariant(
    case_space: &CaseSpace,
    request: &NativeCloseCheckRequest,
) -> NativeCloseInvariantResult {
    let evidence_ids = case_space
        .case_cells
        .iter()
        .filter(|cell| cell.cell_type == CaseCellType::Evidence)
        .map(|cell| cell.id.clone())
        .collect::<BTreeSet<_>>();
    let witness_ids = if request.validation_evidence_ids.is_empty() {
        vec![case_space.revision.revision_id.clone()]
    } else {
        request
            .validation_evidence_ids
            .iter()
            .filter(|id| !evidence_ids.contains(*id))
            .cloned()
            .collect()
    };
    close_invariant(
        "close:native-validation-evidence-named",
        witness_ids,
        "Close checks must name validation evidence for the exact revision.",
    )
}

fn policy_capability_gate_invariant(
    case_space: &CaseSpace,
    request: &NativeCloseCheckRequest,
) -> NativeCloseInvariantResult {
    let has_close_policy =
        request.close_policy_id.is_some() || case_space.close_policy_id.is_some();
    let has_operation_source = !request.source_ids.is_empty();
    let mut witness_ids = Vec::new();
    if !has_close_policy {
        witness_ids.push(case_space.case_space_id.clone());
    }
    if !has_operation_source {
        witness_ids.push(case_space.revision.revision_id.clone());
    }
    let Some(gate) = &request.operation_gate else {
        witness_ids.push(case_space.case_space_id.clone());
        return close_invariant(
            "close:native-policy-capability-gate",
            dedupe_ids(witness_ids),
            "Close checks must include an operation gate with actor, capability, scope, audience, and source boundary.",
        );
    };
    if let Err(error) = check_operation_gate(case_space, gate, "close-check") {
        witness_ids.extend(error.witness_ids);
    }
    close_invariant(
        "close:native-policy-capability-gate",
        dedupe_ids(witness_ids),
        "Close checks must name a close policy, source evidence, and a matching operation gate for actor, capability, scope, audience, and source boundary.",
    )
}

pub(crate) fn declared_source_boundary_id(case_space: &CaseSpace) -> Option<Id> {
    source_boundary_id_from_value(case_space.metadata.get("source_boundary")).or_else(|| {
        case_space
            .morphism_log
            .first()
            .and_then(|entry| entry.morphism.metadata.get("source_boundary_id"))
            .and_then(Value::as_str)
            .and_then(|value| Id::new(value.to_owned()).ok())
    })
}

fn source_boundary_id_from_value(value: Option<&Value>) -> Option<Id> {
    value
        .and_then(Value::as_object)
        .and_then(|boundary| boundary.get("id"))
        .and_then(Value::as_str)
        .and_then(|value| Id::new(value.to_owned()).ok())
}

fn has_source_boundary(metadata: &serde_json::Map<String, Value>) -> bool {
    metadata
        .get("source_boundary")
        .and_then(Value::as_object)
        .is_some_and(|boundary| {
            boundary
                .get("included_sources")
                .and_then(Value::as_array)
                .is_some_and(|values| !values.is_empty())
                && boundary
                    .get("adapters")
                    .and_then(Value::as_array)
                    .is_some_and(|values| !values.is_empty())
                && boundary
                    .get("accepted_fact_policy")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                && boundary
                    .get("inference_policy")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                && boundary
                    .get("information_loss")
                    .and_then(Value::as_array)
                    .is_some()
        })
}

fn require_review_request(
    case_space: &CaseSpace,
    request: &NativeReviewRequest,
) -> NativeReviewResult<()> {
    if request.reason.trim().is_empty() {
        return Err(error("review reason must not be empty"));
    }
    if request.target_revision_id == case_space.revision.revision_id {
        return Err(error("review target_revision_id must advance the revision"));
    }
    for evidence_id in &request.evidence_ids {
        if !has_known_id(case_space, evidence_id) {
            return Err(error(format!("unknown review evidence id {evidence_id}")));
        }
    }
    match request.target_kind {
        NativeReviewTargetKind::Completion => {
            require_completion_target(case_space, &request.target_id)
        }
        NativeReviewTargetKind::Evidence => require_cell_target(
            case_space,
            &request.target_id,
            CaseCellType::Evidence,
            "evidence",
        ),
        NativeReviewTargetKind::Morphism => {
            if case_space
                .morphism_log
                .iter()
                .any(|entry| entry.morphism_id == request.target_id)
            {
                Ok(())
            } else {
                Err(error(format!(
                    "unknown morphism target {}",
                    request.target_id
                )))
            }
        }
        NativeReviewTargetKind::Plan => Ok(()),
        NativeReviewTargetKind::ResidualRisk => {
            require_obstruction_target(case_space, &request.target_id)
        }
        NativeReviewTargetKind::Waiver => {
            if has_known_id(case_space, &request.target_id)
                || evaluate_native_case(case_space)?
                    .obstructions
                    .iter()
                    .any(|obstruction| obstruction.id == request.target_id)
            {
                Ok(())
            } else {
                Err(error(format!(
                    "unknown waiver target {}",
                    request.target_id
                )))
            }
        }
    }
}

fn require_completion_target(case_space: &CaseSpace, target_id: &Id) -> NativeReviewResult<()> {
    if case_space
        .case_cells
        .iter()
        .any(|cell| cell.id == *target_id && cell.cell_type == CaseCellType::Completion)
        || evaluate_native_case(case_space)?
            .completion_candidates
            .iter()
            .any(|candidate| candidate.id == *target_id)
    {
        Ok(())
    } else {
        Err(error(format!("unknown completion target {target_id}")))
    }
}

fn require_obstruction_target(case_space: &CaseSpace, target_id: &Id) -> NativeReviewResult<()> {
    if evaluate_native_case(case_space)?
        .obstructions
        .iter()
        .any(|obstruction| obstruction.id == *target_id)
    {
        Ok(())
    } else {
        Err(error(format!("unknown residual-risk target {target_id}")))
    }
}

fn require_cell_target(
    case_space: &CaseSpace,
    target_id: &Id,
    cell_type: CaseCellType,
    label: &str,
) -> NativeReviewResult<()> {
    if case_space
        .case_cells
        .iter()
        .any(|cell| cell.id == *target_id && cell.cell_type == cell_type)
    {
        Ok(())
    } else {
        Err(error(format!("unknown {label} target {target_id}")))
    }
}

#[cfg(test)]
mod tests;
