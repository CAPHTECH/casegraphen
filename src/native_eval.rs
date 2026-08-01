use crate::evidence_trust::evidence_is_acceptable;
use crate::native_model::{
    CaseCell, CaseCellLifecycle, CaseCellType, CaseRelation, CaseRelationType, CaseSpace,
    RelationStrength,
};
use higher_graphen_core::{Id, ReviewStatus, Severity};
use std::collections::{BTreeMap, BTreeSet};

mod graph;
mod sections;
#[cfg(test)]
mod tests;
mod types;
mod util;
mod validation;
pub use types::*;
pub use validation::validate_native_case_space;
// The close check's own crate-internal surface onto the same derivation the
// evaluator and `effective_evidence_review_status` read, so `native_review`
// can compute the map once and pass it down instead of re-deriving it.
pub(crate) use sections::latest_evidence_review_statuses;

use graph::NativeCaseIndex;
#[cfg(test)]
use sections::evidence_trust_input;
use sections::{
    close_check_skeleton, completion_candidates, correspondence_summaries, evidence_findings,
    evidence_trust_input_with_status, evolution_summary, projection_loss, review_gaps,
};
use util::*;

pub fn evaluate_native_case(case_space: &CaseSpace) -> NativeEvalResult<NativeCaseEvaluation> {
    validate_native_case_space(case_space)?;

    let context = NativeEvaluationContext::new(case_space);
    let cell_results = context.evaluate_cells();
    let readiness = readiness_result(case_space, &cell_results);
    let obstructions = merge_obstructions(&cell_results);
    let evidence_findings = evidence_findings(case_space, &obstructions);
    let completion_candidates = completion_candidates(case_space, &obstructions);
    let review_gaps = review_gaps(case_space, &evidence_findings, &completion_candidates);
    let projection_loss = projection_loss(case_space);
    let correspondence = correspondence_summaries(case_space);
    let evolution = evolution_summary(case_space);
    let close_check = close_check_skeleton(
        case_space,
        &obstructions,
        &completion_candidates,
        &review_gaps,
    );
    let frontier_cell_ids = frontier_cell_ids(&readiness, &context);
    let progress = progress_axis(case_space, &readiness, &obstructions);
    let assurance = assurance_axis(&obstructions, &evidence_findings, &review_gaps);

    Ok(NativeCaseEvaluation {
        progress,
        assurance,
        readiness,
        frontier_cell_ids,
        obstructions,
        completion_candidates,
        evidence_findings,
        review_gaps,
        projection_loss,
        correspondence,
        evolution,
        close_check,
    })
}

pub fn unsatisfied_evidence_requirement_ids(
    case_space: &CaseSpace,
    cell_id: &Id,
    requirement_ids: &[Id],
) -> NativeEvalResult<Vec<Id>> {
    validate_native_case_space(case_space)?;
    let context = NativeEvaluationContext::new(case_space);
    Ok(requirement_ids
        .iter()
        .filter(|requirement_id| !context.evidence_requirement_satisfied(cell_id, requirement_id))
        .cloned()
        .collect())
}

struct NativeEvaluationContext<'a> {
    case_space: &'a CaseSpace,
    index: NativeCaseIndex<'a>,
    cells: BTreeMap<&'a str, &'a CaseCell>,
    trusted_evidence_ids: BTreeSet<&'a str>,
    /// Targets that trusted evidence covers, taken from the morphisms that
    /// minted the coverage rather than from the graph. See
    /// `sections::canonical_evidence_coverage`.
    trusted_coverage_targets: BTreeSet<String>,
}

/// Targets that trusted evidence is *recorded* as covering.
///
/// One implementation, because "is this evidence requirement satisfied?" is one
/// question and it had three answers: the evaluator's, the close check's, and
/// `run --step`'s. The close check used to ask only whether the requirement was
/// itself an acceptable evidence cell, so a requirement satisfied through the
/// tool's own `evidence attach --satisfies` + `review accept` path passed
/// `close:native-no-hard-obstructions` and failed
/// `close:native-evidence-accepted-or-waived` on the same revision — the case
/// could never be closed on merit, only waived.
///
/// `is_trusted` is supplied by the caller because the two callers learn an
/// evidence cell's review status from different places: the evaluator from the
/// log-derived statuses it already indexed, the close check from the explicit
/// review records it is already holding.
pub(crate) fn trusted_coverage_targets(
    case_space: &CaseSpace,
    is_trusted: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    sections::canonical_evidence_coverage(case_space)
        .into_iter()
        .filter(|(evidence_id, _)| is_trusted(evidence_id))
        .map(|(_, target_id)| target_id)
        .collect()
}

/// The coverage a review of `evidence_id` is about to make live, read from the
/// same derivation the decision reads. `review accept` shows a reviewer one
/// target id; the set of requirements their acceptance satisfies is wider than
/// that, and a record of a decision is misleading when the decision was wider
/// than what the command said it was.
pub(crate) fn recorded_coverage_targets(case_space: &CaseSpace, evidence_id: &str) -> Vec<String> {
    sections::canonical_evidence_coverage(case_space)
        .into_iter()
        .filter(|(covering_id, _)| covering_id == evidence_id)
        .map(|(_, target_id)| target_id)
        .collect()
}

/// This id's log-derived review status, if the log carries a canonical
/// review for it — the single source `evidence_findings`, the trust
/// decision, and the close check all consult
/// (`sections::latest_evidence_review_statuses`). `None` means the log has no
/// review for this id, full stop: no fallback to anything the cell itself
/// stores.
///
/// This is deliberately a *different question* from
/// [`effective_evidence_review_status`], and any caller that authorizes a
/// durable mutation from a review status — `packet resume` is the one that
/// exists today — must call this one, not that one. "No review in the log"
/// must never read as accepted on a path that authorizes a transition; it
/// legitimately does read as accepted for `evidence_findings`, which is
/// reporting on a cell's status, not deciding whether to mutate anything
/// because of it. Sharing one function across both questions is what let
/// `packet resume` read a genesis-authored evidence cell's stored
/// `provenance.review_status` as if a review had actually happened.
pub(crate) fn latest_evidence_review_status(
    case_space: &CaseSpace,
    evidence_id: &str,
) -> Option<ReviewStatus> {
    sections::latest_evidence_review_statuses(case_space)
        .get(evidence_id)
        .copied()
}

/// This cell's effective review status **for reporting**, folding the two
/// facts every caller otherwise had to fold themselves: whether it *is*
/// evidence, and if so, what its review status actually is. `None` for
/// anything that is not `cell_type: evidence` — nothing else has a review
/// status this rule speaks for. For an evidence cell: the log-derived status
/// when the log carries a canonical review for it, else the cell's own
/// stored `provenance.review_status`, because a review morphism never
/// rewrites the cell it reviews.
///
/// Do not call this to decide whether to authorize a mutation. The stored
/// fallback is correct for `evidence_findings` — a genesis-authored evidence
/// cell legitimately carries accepted provenance with no review morphism
/// anywhere in the log, and findings must say so — but on a path that
/// authorizes a transition, "no review in the log" must never read as
/// accepted. `packet resume` once called this function for exactly that
/// decision and let a packet whose `claim.id` named an unrelated
/// already-accepted cell resume with no evidence review anywhere in the log;
/// it must call [`latest_evidence_review_status`] instead, which has no
/// fallback.
pub(crate) fn effective_evidence_review_status(
    case_space: &CaseSpace,
    cell: &CaseCell,
) -> Option<ReviewStatus> {
    if cell.cell_type != CaseCellType::Evidence {
        return None;
    }
    Some(
        latest_evidence_review_status(case_space, cell.id.as_str())
            .unwrap_or(cell.provenance.review_status),
    )
}

struct CellEvaluation {
    cell_id: Id,
    lifecycle: CaseCellLifecycle,
    hard_dependency_ids: Vec<Id>,
    wait_ids: Vec<Id>,
    evidence_requirement_ids: Vec<Id>,
    proof_requirement_ids: Vec<Id>,
    obstructions: Vec<NativeObstruction>,
    rule_results: Vec<NativeReadinessRuleResult>,
}

impl<'a> NativeEvaluationContext<'a> {
    fn new(case_space: &'a CaseSpace) -> Self {
        let cells = case_space
            .case_cells
            .iter()
            .map(|cell| (cell.id.as_str(), cell))
            .collect();
        let index = NativeCaseIndex::from_case_space(case_space);
        let mut trusted_evidence_ids = BTreeSet::new();
        for cell in case_space
            .case_cells
            .iter()
            .filter(|cell| cell.cell_type == CaseCellType::Evidence)
        {
            let trust_input = evidence_trust_input_with_status(
                cell,
                index.latest_evidence_review_status(&cell.id),
            );
            if evidence_is_acceptable(trust_input) {
                trusted_evidence_ids.insert(cell.id.as_str());
            }
        }
        let trusted_coverage_targets = trusted_coverage_targets(case_space, |evidence_id| {
            trusted_evidence_ids.contains(evidence_id)
        });
        Self {
            case_space,
            index,
            cells,
            trusted_evidence_ids,
            trusted_coverage_targets,
        }
    }

    fn evaluate_cells(&self) -> Vec<CellEvaluation> {
        self.case_space
            .case_cells
            .iter()
            .filter(|cell| readiness_subject(cell))
            .map(|cell| self.evaluate_cell(cell))
            .collect()
    }

    fn evaluate_cell(&self, cell: &CaseCell) -> CellEvaluation {
        let hard_dependency_ids = self.requirement_ids(cell, CaseRelationType::DependsOn);
        let wait_ids = self.requirement_ids(cell, CaseRelationType::WaitsFor);
        let evidence_requirement_ids =
            self.requirement_ids(cell, CaseRelationType::RequiresEvidence);
        let proof_requirement_ids = self.requirement_ids(cell, CaseRelationType::RequiresProof);
        let mut by_check = BTreeMap::<ReadinessCheck, Vec<NativeObstruction>>::new();

        if let Some(obstruction) = self.lifecycle_obstruction(cell) {
            by_check
                .entry(ReadinessCheck::Lifecycle)
                .or_default()
                .push(obstruction);
        }
        self.add_dependency_obstructions(cell, &hard_dependency_ids, &mut by_check);
        self.add_wait_obstructions(cell, &wait_ids, &mut by_check);
        self.add_evidence_obstructions(cell, &evidence_requirement_ids, &mut by_check);
        self.add_proof_obstructions(cell, &proof_requirement_ids, &mut by_check);
        self.add_contradiction_obstructions(cell, &mut by_check);
        self.add_review_obstructions(cell, &mut by_check);

        let obstructions = sorted_obstructions(&by_check);
        let rule_results = self.rule_results_for(cell, &by_check);
        CellEvaluation {
            cell_id: cell.id.clone(),
            lifecycle: cell.lifecycle,
            hard_dependency_ids,
            wait_ids,
            evidence_requirement_ids,
            proof_requirement_ids,
            obstructions,
            rule_results,
        }
    }

    fn add_dependency_obstructions(
        &self,
        cell: &CaseCell,
        dependency_ids: &[Id],
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for dependency_id in dependency_ids {
            if self.complete_cell(dependency_id) {
                continue;
            }
            push_obstruction(
                by_check,
                ReadinessCheck::Dependencies,
                obstruction(
                    NativeObstructionType::UnresolvedDependency,
                    &cell.id,
                    dependency_id,
                    "constraint:native-dependency-closure",
                    format!(
                        "{cell_id} depends on unresolved cell {dependency_id}.",
                        cell_id = cell.id
                    ),
                    Severity::High,
                    "Resolve, accept, or retire the hard dependency before treating this cell as ready.",
                ),
            );
        }
    }

    fn add_wait_obstructions(
        &self,
        cell: &CaseCell,
        wait_ids: &[Id],
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for wait_id in wait_ids {
            if self.wait_satisfied(wait_id) {
                continue;
            }
            push_obstruction(
                by_check,
                ReadinessCheck::Waits,
                obstruction(
                    NativeObstructionType::ExternalWait,
                    &cell.id,
                    wait_id,
                    "constraint:native-wait-resolution",
                    format!("{} waits for unresolved cell {}.", cell.id, wait_id),
                    Severity::Medium,
                    "Record the waited-for event/review/evidence or explicitly waive the wait by accepted review.",
                ),
            );
        }
    }

    fn add_evidence_obstructions(
        &self,
        cell: &CaseCell,
        requirement_ids: &[Id],
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for requirement_id in requirement_ids {
            if self.evidence_requirement_satisfied(&cell.id, requirement_id) {
                continue;
            }
            push_obstruction(
                by_check,
                ReadinessCheck::Evidence,
                obstruction(
                    NativeObstructionType::MissingEvidence,
                    &cell.id,
                    requirement_id,
                    "constraint:native-evidence-availability",
                    format!("{} requires source-backed or accepted evidence {}, but none is available.", cell.id, requirement_id),
                    Severity::Medium,
                    "Attach source-backed evidence or promote inferred evidence through accepted review.",
                ),
            );
        }
    }

    fn add_proof_obstructions(
        &self,
        cell: &CaseCell,
        proof_ids: &[Id],
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for proof_id in proof_ids {
            if self.proof_requirement_satisfied(&cell.id, proof_id) {
                continue;
            }
            push_obstruction(
                by_check,
                ReadinessCheck::Proof,
                obstruction(
                    NativeObstructionType::MissingProof,
                    &cell.id,
                    proof_id,
                    "constraint:native-proof-availability",
                    format!(
                        "{} requires accepted proof {}, but no accepted proof is available.",
                        cell.id, proof_id
                    ),
                    Severity::Medium,
                    "Complete or accept the proof cell, or attach accepted proof evidence.",
                ),
            );
        }
    }

    fn add_contradiction_obstructions(
        &self,
        cell: &CaseCell,
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for relation in self.contradiction_relations(&cell.id) {
            push_obstruction(
                by_check,
                ReadinessCheck::Contradictions,
                obstruction(
                    NativeObstructionType::Contradiction,
                    &cell.id,
                    &relation.id,
                    "constraint:native-no-hard-contradiction",
                    format!(
                        "{} participates in hard contradictory relation {}.",
                        cell.id, relation.id
                    ),
                    Severity::High,
                    "Resolve or review the contradiction before treating this cell as ready.",
                ),
            );
        }
    }

    fn add_review_obstructions(
        &self,
        cell: &CaseCell,
        by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) {
        for relation in self.required_review_relations(&cell.id) {
            if self.review_satisfied(&relation.to_id) {
                continue;
            }
            push_obstruction(
                by_check,
                ReadinessCheck::Reviews,
                obstruction(
                    NativeObstructionType::ReviewRequired,
                    &cell.id,
                    &relation.to_id,
                    "constraint:native-review-accepted",
                    format!(
                        "{} requires accepted review {}, but it is not accepted.",
                        cell.id, relation.to_id
                    ),
                    Severity::Medium,
                    "Record an accepted review before treating this cell as ready.",
                ),
            );
        }
    }

    fn requirement_ids(&self, cell: &CaseCell, relation_type: CaseRelationType) -> Vec<Id> {
        self.index.direct_targets(&cell.id, relation_type)
    }

    fn lifecycle_obstruction(&self, cell: &CaseCell) -> Option<NativeObstruction> {
        let (severity, explanation, resolution) = match cell.lifecycle {
            CaseCellLifecycle::Rejected => (
                Severity::High,
                format!("{} is rejected and cannot be ready.", cell.id),
                "Create or accept a replacement cell.",
            ),
            CaseCellLifecycle::Retired | CaseCellLifecycle::Superseded => (
                Severity::Medium,
                format!(
                    "{} is retired or superseded and cannot be frontier work.",
                    cell.id
                ),
                "Use the active replacement cell if one exists.",
            ),
            _ => return None,
        };
        Some(obstruction(
            NativeObstructionType::ReviewRequired,
            &cell.id,
            &cell.id,
            "constraint:native-cell-lifecycle",
            explanation,
            severity,
            resolution,
        ))
    }

    fn complete_cell(&self, cell_id: &Id) -> bool {
        self.cells.get(cell_id.as_str()).is_some_and(|cell| {
            matches!(
                cell.lifecycle,
                CaseCellLifecycle::Resolved
                    | CaseCellLifecycle::Accepted
                    | CaseCellLifecycle::Retired
                    | CaseCellLifecycle::Superseded
            ) || cell.provenance.review_status == ReviewStatus::Accepted
        })
    }

    fn wait_satisfied(&self, wait_id: &Id) -> bool {
        self.complete_cell(wait_id) || self.trusted_evidence_exists(wait_id, wait_id)
    }

    fn evidence_requirement_satisfied(&self, cell_id: &Id, requirement_id: &Id) -> bool {
        self.trusted_evidence_exists(requirement_id, cell_id)
    }

    fn proof_requirement_satisfied(&self, cell_id: &Id, proof_id: &Id) -> bool {
        self.cells.get(proof_id.as_str()).is_some_and(|cell| {
            cell.cell_type == CaseCellType::Proof && self.complete_cell(proof_id)
        }) || self.trusted_evidence_exists(proof_id, cell_id)
    }

    /// Is the requirement satisfied by evidence that is both trusted and
    /// *recorded as covering it*?
    ///
    /// This used to be six disjuncts, five of which read coverage out of the
    /// current graph — an evidence cell's `structure_ids`, a
    /// `satisfies`/`verifies`/`accepts` edge into the requirement or the work
    /// cell, and a relation's `evidence_ids`. Every one of those is writable by
    /// any actor the gate admits, so an already-promoted piece of evidence could
    /// be re-pointed at a requirement nobody reviewed it for, and the hard
    /// obstruction disappeared with no review anywhere in the log.
    ///
    /// Both halves are now log-derived: `trusted_evidence_ids` from the review
    /// morphisms, `trusted_coverage_targets` from the morphisms that minted the
    /// coverage. The graph still carries the edges — they are what a reader
    /// sees — but the decision no longer asks it.
    fn trusted_evidence_exists(&self, requirement_id: &Id, cell_id: &Id) -> bool {
        self.trusted_evidence_ids.contains(requirement_id.as_str())
            || self
                .trusted_coverage_targets
                .contains(requirement_id.as_str())
            || self.trusted_coverage_targets.contains(cell_id.as_str())
    }

    fn contradiction_relations(&self, cell_id: &Id) -> Vec<&'a CaseRelation> {
        self.index
            .relations_from(cell_id)
            .iter()
            .copied()
            .chain(self.index.relations_to(cell_id).iter().copied())
            .filter(|relation| {
                relation.relation_strength == RelationStrength::Hard
                    && matches!(
                        relation.relation_type,
                        CaseRelationType::Contradicts
                            | CaseRelationType::Invalidates
                            | CaseRelationType::Blocks
                    )
                    && !self.unblocked_by_review(relation)
            })
            .map(|relation| (relation.id.as_str(), relation))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect()
    }

    fn unblocked_by_review(&self, blocked_relation: &CaseRelation) -> bool {
        self.index
            .relations_to(&blocked_relation.id)
            .iter()
            .any(|relation| {
                relation.relation_strength == RelationStrength::Hard
                    && relation.relation_type == CaseRelationType::Unblocks
                    && relation.to_id == blocked_relation.id
                    && self.review_satisfied(&relation.from_id)
            })
    }

    fn required_review_relations(&self, cell_id: &Id) -> Vec<&'a CaseRelation> {
        self.index
            .relations_from(cell_id)
            .iter()
            .copied()
            .filter(|relation| {
                relation.relation_strength == RelationStrength::Hard
                    && matches!(
                        relation.relation_type,
                        CaseRelationType::Accepts | CaseRelationType::Rejects
                    )
            })
            .collect()
    }

    fn review_satisfied(&self, review_id: &Id) -> bool {
        self.cells.get(review_id.as_str()).is_some_and(|cell| {
            cell.cell_type == CaseCellType::Review
                && (cell.lifecycle == CaseCellLifecycle::Accepted
                    || cell.provenance.review_status == ReviewStatus::Accepted)
        })
    }

    fn rule_results_for(
        &self,
        cell: &CaseCell,
        by_check: &BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    ) -> Vec<NativeReadinessRuleResult> {
        let mut checks = by_check.keys().copied().collect::<BTreeSet<_>>();
        if checks.is_empty() {
            checks.insert(ReadinessCheck::Lifecycle);
        }
        checks
            .into_iter()
            .map(|check| {
                let rule_id = default_rule_id(check);
                let obstruction_ids: Vec<Id> = by_check
                    .get(&check)
                    .map(|records| records.iter().map(|record| record.id.clone()).collect())
                    .unwrap_or_default();
                NativeReadinessRuleResult {
                    id: generated_id("readiness_result", &[cell.id.as_str(), rule_id.as_str()]),
                    rule_id,
                    target_cell_id: cell.id.clone(),
                    ready: obstruction_ids.is_empty(),
                    obstruction_ids,
                }
            })
            .collect()
    }
}

/// The Progress axis of the reasoning status. A projection of the evaluation
/// already computed — it reads obstruction `blocking` flags and the readiness
/// subject set and decides nothing itself. Its pair is [`assurance_axis`];
/// both are registered as one rule in the invariant-duplication-auditor table.
///
/// `complete` means the space has cells and no readiness subject remains:
/// every goal/work-like cell reached a complete lifecycle. An empty case
/// space is `active` — open, with nothing achieved — not complete.
fn progress_axis(
    case_space: &CaseSpace,
    readiness: &NativeReadiness,
    obstructions: &[NativeObstruction],
) -> NativeProgress {
    if obstructions.iter().any(|obstruction| obstruction.blocking) {
        NativeProgress::Blocked
    } else if !case_space.case_cells.is_empty() && readiness.evaluated_cell_ids.is_empty() {
        NativeProgress::Complete
    } else {
        NativeProgress::Active
    }
}

/// The Assurance axis of the reasoning status: a worst-wins fold over the
/// review-relevant facts the evaluation already carries. Rejected evidence in
/// use is worst; any open review gap, review-required obstruction, or evidence
/// boundary violation is pending review; accepted evidence with a clean review
/// story is accepted; a space where nothing was ever reviewed is unreviewed.
fn assurance_axis(
    obstructions: &[NativeObstruction],
    evidence_findings: &NativeEvidenceFindings,
    review_gaps: &[NativeReviewGap],
) -> NativeAssurance {
    let rejected_in_use = evidence_findings
        .boundary_violations
        .iter()
        .any(|violation| {
            violation.violation_type == NativeEvidenceBoundaryViolationType::RejectedEvidenceUsed
        });
    if rejected_in_use {
        NativeAssurance::Rejected
    } else if !review_gaps.is_empty()
        || !evidence_findings.boundary_violations.is_empty()
        || obstructions.iter().any(|obstruction| {
            obstruction.obstruction_type == NativeObstructionType::ReviewRequired
        })
    {
        NativeAssurance::ReviewRequired
    } else if !evidence_findings.accepted_evidence_ids.is_empty() {
        NativeAssurance::Accepted
    } else {
        NativeAssurance::Unreviewed
    }
}

fn readiness_result(case_space: &CaseSpace, results: &[CellEvaluation]) -> NativeReadiness {
    let evaluated_cell_ids = case_space
        .case_cells
        .iter()
        .filter(|cell| readiness_subject(cell))
        .map(|cell| cell.id.clone())
        .collect();
    let ready_cell_ids = results
        .iter()
        .filter(|result| {
            result
                .obstructions
                .iter()
                .all(|obstruction| !obstruction.blocking)
        })
        .map(|result| result.cell_id.clone())
        .collect();
    let not_ready_cells = results
        .iter()
        .filter(|result| {
            result
                .obstructions
                .iter()
                .any(|obstruction| obstruction.blocking)
        })
        .map(|result| NativeNotReadyCell {
            cell_id: result.cell_id.clone(),
            lifecycle: result.lifecycle,
            hard_dependency_ids: result.hard_dependency_ids.clone(),
            wait_ids: result.wait_ids.clone(),
            evidence_requirement_ids: result.evidence_requirement_ids.clone(),
            proof_requirement_ids: result.proof_requirement_ids.clone(),
            obstruction_ids: result
                .obstructions
                .iter()
                .filter(|obstruction| obstruction.blocking)
                .map(|obstruction| obstruction.id.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let blocked_cell_ids = not_ready_cells
        .iter()
        .map(|cell| cell.cell_id.clone())
        .collect::<Vec<_>>();
    let waiting_cell_ids = not_ready_cells
        .iter()
        .filter(|cell| {
            !cell.wait_ids.is_empty()
                && cell
                    .obstruction_ids
                    .iter()
                    .all(|id| id.as_str().contains("external-wait"))
        })
        .map(|cell| cell.cell_id.clone())
        .collect();
    let rule_results = results
        .iter()
        .flat_map(|result| result.rule_results.iter().cloned())
        .collect();

    NativeReadiness {
        evaluated_cell_ids,
        ready_cell_ids,
        not_ready_cells,
        waiting_cell_ids,
        blocked_cell_ids,
        rule_results,
    }
}

fn frontier_cell_ids(
    readiness: &NativeReadiness,
    context: &NativeEvaluationContext<'_>,
) -> Vec<Id> {
    readiness
        .ready_cell_ids
        .iter()
        .filter(|id| !context.index.completed_targets().contains(*id))
        .filter(|id| {
            context.cells.get(id.as_str()).is_some_and(|cell| {
                cell.id == **id
                    && !matches!(
                        cell.lifecycle,
                        CaseCellLifecycle::Resolved
                            | CaseCellLifecycle::Accepted
                            | CaseCellLifecycle::Retired
                            | CaseCellLifecycle::Rejected
                            | CaseCellLifecycle::Superseded
                    )
            })
        })
        .cloned()
        .collect()
}

fn merge_obstructions(results: &[CellEvaluation]) -> Vec<NativeObstruction> {
    let mut by_id = BTreeMap::new();
    for obstruction in results
        .iter()
        .flat_map(|result| result.obstructions.iter().cloned())
    {
        by_id.entry(obstruction.id.clone()).or_insert(obstruction);
    }
    by_id.into_values().collect()
}

fn readiness_subject(cell: &CaseCell) -> bool {
    !matches!(
        cell.cell_type,
        CaseCellType::Evidence
            | CaseCellType::Review
            | CaseCellType::Projection
            | CaseCellType::Revision
            | CaseCellType::Morphism
            | CaseCellType::ExternalRef
    ) && !matches!(
        cell.lifecycle,
        CaseCellLifecycle::Resolved
            | CaseCellLifecycle::Accepted
            | CaseCellLifecycle::Retired
            | CaseCellLifecycle::Rejected
            | CaseCellLifecycle::Superseded
    )
}

fn obstruction(
    obstruction_type: NativeObstructionType,
    cell_id: &Id,
    witness_id: &Id,
    constraint_id: &str,
    explanation: String,
    severity: Severity,
    required_resolution: &str,
) -> NativeObstruction {
    NativeObstruction {
        id: generated_id(
            "obstruction",
            &[
                obstruction_type_stem(obstruction_type),
                cell_id.as_str(),
                witness_id.as_str(),
            ],
        ),
        obstruction_type,
        affected_ids: vec![cell_id.clone()],
        source_constraint_id: id(constraint_id),
        witness_ids: vec![witness_id.clone()],
        explanation,
        severity,
        required_resolution: required_resolution.to_owned(),
        blocking: true,
        provenance: generated_provenance("Native readiness obstruction", 0.84),
    }
}

fn push_obstruction(
    by_check: &mut BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
    check: ReadinessCheck,
    obstruction: NativeObstruction,
) {
    by_check.entry(check).or_default().push(obstruction);
}

fn sorted_obstructions(
    by_check: &BTreeMap<ReadinessCheck, Vec<NativeObstruction>>,
) -> Vec<NativeObstruction> {
    let mut obstructions = by_check
        .values()
        .flat_map(|records| records.iter().cloned())
        .collect::<Vec<_>>();
    obstructions.sort_by(|left, right| left.id.cmp(&right.id));
    obstructions
}

fn default_rule_id(check: ReadinessCheck) -> Id {
    id(match check {
        ReadinessCheck::Lifecycle => "readiness:native-lifecycle-allows-work",
        ReadinessCheck::Dependencies => "readiness:native-dependencies-resolved",
        ReadinessCheck::Waits => "readiness:native-waits-satisfied",
        ReadinessCheck::Evidence => "readiness:native-evidence-available",
        ReadinessCheck::Proof => "readiness:native-proof-available",
        ReadinessCheck::Contradictions => "readiness:native-no-contradictions",
        ReadinessCheck::Reviews => "readiness:native-reviews-accepted",
    })
}
