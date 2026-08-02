//! ADR 0016: the single halt vocabulary. "Why did the ledger stop" is
//! answered here and nowhere else — `run --step`, `run --frontier`,
//! `packet apply`, and `operate` all report a value this module derived,
//! never one they decided for themselves.
//!
//! [`Halt`] is a pure function of the evaluation and the log
//! (`docs/specs/operate-halt.fsl`'s `def halt()`): never stored, never
//! re-decided per command. `None` from [`derive_halt`] is the FSL model's
//! `Progress` — Rust's `Option` says "stopped iff `Some`" for free, which is
//! exactly `INV-OPERATE-003`/`INV-OPERATE-004`.

use crate::exec::records::{ExecutionDispatchState, ExecutionTrace};
use crate::exec::ExecutionPlan;
use crate::native_eval::{
    NativeCaseEvaluation, NativeObstruction, NativeObstructionType, NativeReviewGapType,
    REVIEW_ACCEPTED_CONSTRAINT_ID,
};
use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// ADR 0016 decision 1, plus `DispatchInProgress`: eight members now;
/// `Progress` is deliberately not one of them — see the module doc comment
/// for why `Option<Halt>` is the right Rust shape for the FSL model's
/// nine-value enum.
///
/// `DispatchInProgress` was carved out of what `NothingEligible` used to
/// absorb: another process holding a started dispatch (`select_steps`'s own
/// `dispatch_in_progress` reason) is a real, recorded reason the ledger
/// cannot advance, and reporting `NothingEligible` for it satisfied ADR
/// 0016's invariant only vacuously — that word asserts there is no reason.
/// Named in ADR 0016's own Context as one of the four scattered vocabularies
/// this ADR exists to consolidate, so its absence here was a gap, not a
/// scope decision. Cleared by `--supersede-trace` after externally
/// establishing the dispatch is dead (ADR 0014) — never a retry, and never
/// inferred by the loop.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Halt {
    RoundBudgetExhausted,
    NeedsReview,
    NeedsRetryDecision,
    NeedsPlanReview,
    NeedsEvidence,
    NeedsExternal,
    DispatchInProgress,
    NothingEligible,
}

/// ADR 0016 decision 2: a halt is a resumable object, and the operations
/// that would clear it are structured values, never assembled command
/// strings — the rule #19 landed for `packet apply`'s `next_operations`
/// after a packet-controlled id was found injectable into an assembled
/// command. This is that shape generalised: `packet apply`'s pause is one
/// producer of it now, not a second ad hoc shape beside it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NextOperation {
    pub command: String,
    pub arguments: BTreeMap<String, String>,
    pub note: Option<String>,
}

/// The resumable object a halt is. `completed_through` is the revision the
/// ledger got to before stopping; `target_ids` names what is blocked;
/// `next_operations` names what would clear it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HaltReport {
    pub halt: Halt,
    pub completed_through: Id,
    pub target_ids: Vec<Id>,
    pub next_operations: Vec<NextOperation>,
}

/// ADR 0016 decision 1's `def halt()`, transcribed from
/// `docs/specs/operate-halt.fsl`. `dispatchable` is the caller's own
/// selection result (`select_steps`, the same function `run --frontier`
/// dispatches from) — this function does not repeat that decision, only
/// interprets its outcome. `budget_exhausted` is meaningless outside
/// `operate`'s loop; `run --step`/`run --frontier` always pass `false`.
///
/// `derive_halt` is `derive_halts(..).first().copied()` — the single source
/// of the priority order the spec's `halt()` names is this function, and
/// `derive_halt` names its head, never a second, independent decision.
pub fn derive_halt(
    dispatchable: bool,
    budget_exhausted: bool,
    evaluation: &NativeCaseEvaluation,
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
    in_flight_step_ids: &BTreeSet<Id>,
) -> Option<Halt> {
    derive_halts(
        dispatchable,
        budget_exhausted,
        evaluation,
        plan,
        traces,
        solely_retry_blocked_step_ids,
        in_flight_step_ids,
    )
    .into_iter()
    .next()
}

/// Every halt reason independently true right now, in the spec's `def
/// halt()` priority order: `RoundBudgetExhausted` (only when dispatchable
/// work remains) > `Progress` (the empty list) > `NeedsReview` >
/// `NeedsRetryDecision` > `NeedsPlanReview` > `NeedsEvidence` >
/// `NeedsExternal` > `DispatchInProgress` > `NothingEligible` (only when
/// nothing else is true).
///
/// A single case space can have more than one independently-true reason —
/// one step needing a retry decision while another needs plan review, say —
/// and `derive_halt`'s single answer names only the highest-priority one.
/// Reporting only that one made every other true reason *absent* from the
/// report, not merely deprioritised: a reader had no way to learn a second
/// obstruction existed without re-deriving it, which is the halt vocabulary
/// being re-implemented outside this module. This is the full, ranked list;
/// `derive_halt`'s single answer remains its head.
pub fn derive_halts(
    dispatchable: bool,
    budget_exhausted: bool,
    evaluation: &NativeCaseEvaluation,
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
    in_flight_step_ids: &BTreeSet<Id>,
) -> Vec<Halt> {
    if budget_exhausted && dispatchable {
        return vec![Halt::RoundBudgetExhausted];
    }
    if dispatchable {
        return Vec::new();
    }
    let mut halts = Vec::new();
    if needs_review(evaluation, plan) {
        halts.push(Halt::NeedsReview);
    }
    let (needs_retry, needs_plan_review) =
        retry_and_plan_review(plan, traces, solely_retry_blocked_step_ids);
    if needs_retry {
        halts.push(Halt::NeedsRetryDecision);
    }
    if needs_plan_review {
        halts.push(Halt::NeedsPlanReview);
    }
    if needs_evidence(evaluation, plan) {
        halts.push(Halt::NeedsEvidence);
    }
    if needs_external(evaluation, plan) {
        halts.push(Halt::NeedsExternal);
    }
    if !in_flight_step_ids.is_empty() {
        halts.push(Halt::DispatchInProgress);
    }
    if halts.is_empty() {
        halts.push(Halt::NothingEligible);
    }
    halts
}

/// Builds the resumable object for an already-derived halt. Kept separate
/// from `derive_halt` itself: deciding *which* reason applies and describing
/// *how to clear it* are different questions, and every caller needs the
/// first but not always the second (a property test over `derive_halt` alone
/// should not need a store path or a case-space id to construct one).
#[allow(clippy::too_many_arguments)]
pub fn build_halt_report(
    halt: Halt,
    store: &Path,
    case_space_id: &Id,
    plan: &ExecutionPlan,
    completed_through: &Id,
    evaluation: &NativeCaseEvaluation,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
    in_flight_step_ids: &BTreeSet<Id>,
) -> HaltReport {
    let store_display = store.display().to_string();
    let mut arguments = BTreeMap::new();
    arguments.insert("store".to_owned(), store_display.clone());
    arguments.insert("case_space_id".to_owned(), case_space_id.to_string());
    arguments.insert("base_revision_id".to_owned(), completed_through.to_string());

    let (target_ids, next_operations) = match halt {
        Halt::RoundBudgetExhausted => {
            let mut operate_arguments = arguments.clone();
            operate_arguments.insert("plan_id".to_owned(), plan.plan_id.to_string());
            (
                Vec::new(),
                vec![NextOperation {
                    command: "operate".to_owned(),
                    arguments: operate_arguments,
                    note: Some(
                        "the round budget was spent while dispatchable work remained; run \
                         another operate invocation"
                            .to_owned(),
                    ),
                }],
            )
        }
        Halt::NeedsReview => {
            // The spec's split (`docs/specs/operate-halt.fsl`'s
            // `PendingClaimReview`/`PendingGateReview`): these are two
            // producers of the *same* halt with two genuinely different
            // clearing acts, and naming the wrong one is exactly the
            // "deadlock wearing a vocabulary word" ADR 0016 forbids. A claim
            // review gap is cleared by `review accept` (an evidence cell's
            // review status is log-derived); a gate obstruction
            // (`is_clearable_by_review`) is not — `review accept` appends a
            // morphism that changes no cell state, so only the witness
            // review cell's own lifecycle transition to `accepted` clears it.
            let claim_target_ids = claim_review_target_ids(evaluation);
            let gate_target_ids = gate_review_target_ids(evaluation, plan);
            let mut next_operations = Vec::new();
            for target_id in &claim_target_ids {
                let mut review_arguments = arguments.clone();
                review_arguments.insert("target_id".to_owned(), target_id.to_string());
                next_operations.push(NextOperation {
                    command: "review accept".to_owned(),
                    arguments: review_arguments,
                    note: Some(
                        "must run under a different actor's gate holding the review \
                         operation, not the actor that dispatched the work being reviewed"
                            .to_owned(),
                    ),
                });
            }
            for target_id in &gate_target_ids {
                let mut transition_arguments = arguments.clone();
                transition_arguments.insert("cell_id".to_owned(), target_id.to_string());
                transition_arguments.insert("to".to_owned(), "accepted".to_owned());
                next_operations.push(NextOperation {
                    command: "cell transition".to_owned(),
                    arguments: transition_arguments,
                    note: Some(
                        "a review morphism records a decision but changes no cell state, so \
                         `review accept` on this id cannot clear it; the witness review \
                         cell's own lifecycle must reach `accepted` directly. Its current \
                         lifecycle may require intermediate transitions first — a transition \
                         that is not legal from the cell's current lifecycle is refused, not \
                         silently skipped"
                            .to_owned(),
                    ),
                });
            }
            let target_ids = dedupe(
                claim_target_ids
                    .into_iter()
                    .chain(gate_target_ids)
                    .collect(),
            );
            (target_ids, next_operations)
        }
        Halt::NeedsRetryDecision | Halt::NeedsPlanReview => {
            let target_ids = failed_step_ids(plan, traces, halt, solely_retry_blocked_step_ids);
            let next_operations = if halt == Halt::NeedsRetryDecision {
                if retryable_target_has_binding_integrity_failure(plan, traces, &target_ids) {
                    let mut binding_arguments = arguments.clone();
                    binding_arguments.remove("base_revision_id");
                    vec![NextOperation {
                        command: "binding register".to_owned(),
                        arguments: binding_arguments,
                        note: Some(
                            "the worker binding for the named step id(s) no longer matches what \
                             the accepted plan authorized (identity or content hash mismatch); \
                             retrying without re-registering the binding will fail identically. \
                             Re-register the binding, then retry"
                                .to_owned(),
                        ),
                    }]
                } else {
                    let mut run_arguments = arguments.clone();
                    run_arguments.insert("plan_id".to_owned(), plan.plan_id.to_string());
                    run_arguments.insert(
                        "retry_step_ids".to_owned(),
                        target_ids
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    vec![NextOperation {
                        command: "run --frontier".to_owned(),
                        arguments: run_arguments,
                        note: Some(
                            "retry is an explicit operator decision, never something the loop \
                             decides on its own; pass --retry-step for each named step id"
                                .to_owned(),
                        ),
                    }]
                }
            } else {
                let mut plan_arguments = arguments.clone();
                plan_arguments.remove("base_revision_id");
                plan_arguments.insert("superseded_plan_id".to_owned(), plan.plan_id.to_string());
                vec![NextOperation {
                    command: "plan propose".to_owned(),
                    arguments: plan_arguments,
                    note: Some(
                        "the accepted plan does not authorize the transition produced for the \
                         named step id(s); propose and accept a superseding plan"
                            .to_owned(),
                    ),
                }]
            };
            (target_ids, next_operations)
        }
        Halt::NeedsEvidence => {
            let target_ids = blocked_target_ids(evaluation, plan, &is_missing_evidence_or_proof);
            (
                target_ids,
                vec![NextOperation {
                    command: "evidence attach".to_owned(),
                    arguments,
                    note: Some(
                        "attach source-backed evidence satisfying the named requirement id(s), \
                         or promote inferred evidence through accepted review"
                            .to_owned(),
                    ),
                }],
            )
        }
        Halt::NeedsExternal => {
            let target_ids = waiting_target_ids(evaluation, plan);
            let next_operations = target_ids
                .iter()
                .map(|target_id| {
                    let mut review_arguments = arguments.clone();
                    review_arguments.insert("target_id".to_owned(), target_id.to_string());
                    NextOperation {
                        command: "review accept".to_owned(),
                        arguments: review_arguments,
                        note: Some(
                            "record the waited-for event/review/evidence, or explicitly waive \
                             the wait by accepted review"
                                .to_owned(),
                        ),
                    }
                })
                .collect();
            (target_ids, next_operations)
        }
        Halt::DispatchInProgress => {
            let target_ids = started_trace_ids(plan, traces, in_flight_step_ids);
            let next_operations = target_ids
                .iter()
                .map(|trace_id| {
                    let mut supersede_arguments = arguments.clone();
                    supersede_arguments.insert("plan_id".to_owned(), plan.plan_id.to_string());
                    supersede_arguments.insert("supersede_trace".to_owned(), trace_id.to_string());
                    NextOperation {
                        command: "run --frontier".to_owned(),
                        arguments: supersede_arguments,
                        note: Some(
                            "another process holds a started dispatch for this trace; this is \
                             not a retry decision. Only assert --supersede-trace after \
                             externally establishing that the dispatch is dead (ADR 0014) — \
                             the loop never infers this on its own"
                                .to_owned(),
                        ),
                    }
                })
                .collect();
            (target_ids, next_operations)
        }
        Halt::NothingEligible => (Vec::new(), Vec::new()),
    };

    HaltReport {
        halt,
        completed_through: completed_through.clone(),
        target_ids,
        next_operations,
    }
}

/// [`build_halt_report`] applied to every entry [`derive_halts`] names, in
/// its priority order. `derive_halts(..).first()` and `build_halt_reports`
/// with the same inputs are the same list a caller would get from calling
/// [`build_halt_report`] once per [`derive_halt`]-style answer — this exists
/// so no caller re-derives the list by calling `derive_halt` repeatedly
/// under different assumptions.
#[allow(clippy::too_many_arguments)]
pub fn build_halt_reports(
    halts: &[Halt],
    store: &Path,
    case_space_id: &Id,
    plan: &ExecutionPlan,
    completed_through: &Id,
    evaluation: &NativeCaseEvaluation,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
    in_flight_step_ids: &BTreeSet<Id>,
) -> Vec<HaltReport> {
    halts
        .iter()
        .map(|&halt| {
            build_halt_report(
                halt,
                store,
                case_space_id,
                plan,
                completed_through,
                evaluation,
                traces,
                solely_retry_blocked_step_ids,
                in_flight_step_ids,
            )
        })
        .collect()
}

fn plan_work_cell_ids(plan: &ExecutionPlan) -> BTreeSet<&Id> {
    plan.steps.iter().map(|step| &step.work_cell_id).collect()
}

/// Every blocking obstruction that stands between the plan and its work,
/// read from `evaluation.readiness.not_ready_cells` and
/// `evaluation.obstructions` and never recomputed — starting at the plan's
/// own work cells and following each `UnresolvedDependency` and
/// `Contradiction` to the cell it names, transitively.
///
/// The walk exists because those two obstruction types are *pointers*, not
/// causes. `native_eval` attaches an `UnresolvedDependency` to the dependent
/// cell, naming the dependency in `witness_ids`; what would actually clear it
/// is whatever is blocking that dependency — and the dependency's own
/// obstruction is the one that names an operation someone can run. Stopping
/// at the plan's own cells reported the pointer as if it were the cause,
/// which is how `needs_review` came to be emitted for a dependency chain
/// whose real, clearable blocker was a missing evidence requirement further
/// up (reproduced end to end; see `is_clearable_by_review`).
///
/// A `Contradiction`'s witness is a relation id rather than a cell id, so the
/// walk simply finds nothing to expand there and the contradiction stays in
/// the returned set on its own. `visited` makes a dependency cycle terminate
/// rather than recurse.
fn blocked_obstructions_for_plan<'a>(
    evaluation: &'a NativeCaseEvaluation,
    plan: &ExecutionPlan,
) -> Vec<&'a NativeObstruction> {
    let obstruction_by_id: BTreeMap<&Id, &NativeObstruction> = evaluation
        .obstructions
        .iter()
        .map(|obstruction| (&obstruction.id, obstruction))
        .collect();
    let not_ready_by_cell: BTreeMap<&Id, &Vec<Id>> = evaluation
        .readiness
        .not_ready_cells
        .iter()
        .map(|cell| (&cell.cell_id, &cell.obstruction_ids))
        .collect();

    let mut pending = plan_work_cell_ids(plan).into_iter().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut collected = Vec::new();
    while let Some(cell_id) = pending.pop() {
        if !visited.insert(cell_id) {
            continue;
        }
        let Some(obstruction_ids) = not_ready_by_cell.get(cell_id) else {
            continue;
        };
        for obstruction in obstruction_ids
            .iter()
            .filter_map(|id| obstruction_by_id.get(id).copied())
            .filter(|obstruction| obstruction.blocking)
        {
            collected.push(obstruction);
            if matches!(
                obstruction.obstruction_type,
                NativeObstructionType::UnresolvedDependency | NativeObstructionType::Contradiction
            ) {
                pending.extend(obstruction.witness_ids.iter());
            }
        }
    }
    collected
}

/// ADR 0016 decision 1's `needs_review`.
///
/// Only `UnreviewedInference` review gaps are consulted, and only through
/// `requirement_satisfied` — never recomputed. Every other `review_gaps`
/// entry (`UnreviewedCompletion`, `UnreviewedMorphism`,
/// `UnreviewedProjectionLoss`) hard-codes `requirement_satisfied: false`
/// unconditionally, including `UnreviewedCompletion`, which
/// `sections::completion_candidates` mints for essentially every blocking
/// obstruction type (`candidate_shape` covers `MissingEvidence`,
/// `MissingProof`, `ReviewRequired`, `ExternalWait`, `UnresolvedDependency`,
/// `Contradiction`). Treating "any unresolved review gap" as `needs_review`
/// was tried and measured: it made `needs_review` fire for nearly every
/// blocked case space, regardless of which obstruction actually blocks the
/// plan, drowning out `NeedsEvidence`/`NeedsExternal`/etc. entirely.
/// `UnreviewedInference` is the one gap type `requirement_satisfied` was
/// introduced for (commit 881b01e) and the only type where "already
/// satisfied" is a real, checkable fact rather than a permanent `false`.
///
/// `UnresolvedDependency` and `Contradiction` are deliberately *not* here.
/// See [`is_clearable_by_review`]: `review accept` provably does not clear
/// either, so reporting them as `NeedsReview` produced a halt naming an
/// operation that cannot discharge it. `blocked_obstructions_for_plan`
/// follows them to the obstruction that can instead.
fn needs_review(evaluation: &NativeCaseEvaluation, plan: &ExecutionPlan) -> bool {
    if has_open_inference_gap(evaluation) {
        return true;
    }
    blocked_obstructions_for_plan(evaluation, plan)
        .iter()
        .any(|obstruction| is_clearable_by_review(obstruction))
}

/// `NativeObstructionType::ReviewRequired` has two producers in
/// `native_eval.rs` that share the one type tag: `add_review_obstructions`,
/// for an unaccepted `Accepts`/`Rejects` relation (clearable — an
/// independent actor can `review accept` the target), and
/// `lifecycle_obstruction`, for a `Rejected`/`Retired`/`Superseded` cell,
/// whose own `required_resolution` text says "create or accept a
/// replacement cell" — no review of that cell clears it. Matching the type
/// tag alone would let the second producer emit a `needs_review` halt that
/// nothing can clear, which is exactly the deadlock
/// `docs/specs/operate-halt.fsl`'s `REQ-OPERATE-009` (`NeedsReview` always
/// has an exit) says must not happen.
///
/// `lifecycle_obstruction` cannot fire today: `evaluate_cell` only runs on
/// `readiness_subject` cells (`native_eval.rs::evaluate_cells`), and
/// `readiness_subject` excludes every lifecycle `lifecycle_obstruction`
/// would fire for — so this ambiguity currently has no reachable witness.
/// Matching on the type tag alone would therefore *happen* to be correct
/// today, but only because dead code keeps the two meanings apart; that
/// would make a proved invariant hold by accident of a branch nobody calls,
/// not by construction. Matching on `source_constraint_id` instead — reading
/// `native_eval::REVIEW_ACCEPTED_CONSTRAINT_ID`, the exact value the
/// producer stamps, rather than a second literal copied here — holds it by
/// construction, so it stays correct even if `lifecycle_obstruction` ever
/// becomes reachable, and a future rename of the constant cannot silently
/// desync the two sides. Do not simplify this back to a type-tag match.
///
/// `UnresolvedDependency` and `Contradiction` are excluded, and that
/// exclusion was measured rather than reasoned. `review accept` on a plain
/// cell appends a canonical review morphism whose `added_ids`, `updated_ids`,
/// and `retired_ids` are all empty (`native_review::build_review_morphism`) —
/// it records a decision, it never rewrites the cell it reviews. But
/// `complete_cell` (`native_eval.rs`), which is what an `UnresolvedDependency`
/// actually tests, requires the dependency's *own* lifecycle to be
/// `Resolved|Accepted|Retired|Superseded` or its own
/// `provenance.review_status` to be `Accepted`. Running the suggested
/// `review accept` against a dependency end to end therefore appends a real,
/// gated `waiver` morphism, advances the revision, and leaves the obstruction
/// exactly where it was — a halt that is a deadlock wearing a vocabulary
/// word, which `docs/specs/operate-halt.fsl`'s `REQ-OPERATE-009` and ADR
/// 0016's decision 2 both forbid. `Contradiction` is the same shape: it is
/// cleared by an `Unblocks` relation from an already-accepted review *cell*,
/// which no `review accept` invocation creates.
fn is_clearable_by_review(obstruction: &NativeObstruction) -> bool {
    match obstruction.obstruction_type {
        NativeObstructionType::ReviewRequired => {
            obstruction.source_constraint_id.as_str() == REVIEW_ACCEPTED_CONSTRAINT_ID
        }
        _ => false,
    }
}

fn has_open_inference_gap(evaluation: &NativeCaseEvaluation) -> bool {
    evaluation.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedInference && !gap.requirement_satisfied
    })
}

/// `docs/specs/operate-halt.fsl`'s `PendingClaimReview`: an unreviewed
/// inference gap, cleared by `review accept` on the evidence cell itself.
fn claim_review_target_ids(evaluation: &NativeCaseEvaluation) -> Vec<Id> {
    evaluation
        .review_gaps
        .iter()
        .filter(|gap| {
            gap.gap_type == NativeReviewGapType::UnreviewedInference && !gap.requirement_satisfied
        })
        .map(|gap| gap.target_id.clone())
        .collect()
}

/// `docs/specs/operate-halt.fsl`'s `PendingGateReview`: a hard `accepts`
/// obstruction naming an unaccepted review cell in its `witness_ids`,
/// cleared by transitioning that cell's own lifecycle — never by
/// `review accept`. See [`is_clearable_by_review`].
fn gate_review_target_ids(evaluation: &NativeCaseEvaluation, plan: &ExecutionPlan) -> Vec<Id> {
    blocked_target_ids(evaluation, plan, &is_clearable_by_review)
}

fn needs_evidence(evaluation: &NativeCaseEvaluation, plan: &ExecutionPlan) -> bool {
    !blocked_target_ids(evaluation, plan, &is_missing_evidence_or_proof).is_empty()
}

fn needs_external(evaluation: &NativeCaseEvaluation, plan: &ExecutionPlan) -> bool {
    !waiting_target_ids(evaluation, plan).is_empty()
}

/// ADR 0016 decision 5: `needs_external` reads `readiness.waiting_cell_ids`
/// directly rather than re-deriving "is this cell waiting" from obstruction
/// types — that list already *is* the waiting state.
fn waiting_target_ids(evaluation: &NativeCaseEvaluation, plan: &ExecutionPlan) -> Vec<Id> {
    let work_cell_ids = plan_work_cell_ids(plan);
    evaluation
        .readiness
        .waiting_cell_ids
        .iter()
        .filter(|id| work_cell_ids.contains(id))
        .cloned()
        .collect()
}

/// The ids a halt should name, for the obstructions `selects` matches.
///
/// `witness_ids`, not the affected cell. Every obstruction `native_eval.rs`
/// builds puts the thing an operator must *act on* in `witness_ids` and the
/// thing that is merely blocked in `affected_ids`: a `MissingEvidence`
/// witnesses the unsatisfied requirement id that `evidence attach --satisfies`
/// takes, and a `ReviewRequired` witnesses the review that `review accept`
/// takes. Naming the affected cell instead produced a `next_operations` entry
/// whose `target_id` was the one id the suggested command must not be given.
fn blocked_target_ids(
    evaluation: &NativeCaseEvaluation,
    plan: &ExecutionPlan,
    selects: &dyn Fn(&NativeObstruction) -> bool,
) -> Vec<Id> {
    dedupe(
        blocked_obstructions_for_plan(evaluation, plan)
            .into_iter()
            .filter(|obstruction| selects(obstruction))
            .flat_map(|obstruction| obstruction.witness_ids.iter().cloned())
            .collect(),
    )
}

fn is_missing_evidence_or_proof(obstruction: &NativeObstruction) -> bool {
    matches!(
        obstruction.obstruction_type,
        NativeObstructionType::MissingEvidence | NativeObstructionType::MissingProof
    )
}

fn dedupe(ids: Vec<Id>) -> Vec<Id> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// `finished_at`/`started_at` are always `timestamp()`'s `"unix:<epoch
/// seconds>"` (`src/native_cli/ops/io.rs::timestamp`) — comparing the parsed
/// number, not the string. A lexicographic comparison of the raw string
/// happens to agree with numeric order only while every representable
/// timestamp has equal digit width; that coincidence is not a fact this
/// module should depend on. `unwrap_or(0)` never fires against a
/// tool-written trace, and there is nothing safer to do with a malformed one
/// than sort it oldest.
fn finished_at_seconds(finished_at: &str) -> u64 {
    finished_at
        .strip_prefix("unix:")
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

/// The most recently finished trace for `step_id` under `plan`, ignoring any
/// trace still `Started` (an in-flight dispatch has nothing finished to
/// report). Shared by every place that classifies a step's most recent
/// outcome (`retry_or_plan_review`, `failed_step_ids`,
/// `retryable_target_has_binding_integrity_failure`) so "which trace is
/// authoritative for this step" is decided once, not re-filtered at each
/// call site with its own copy of the same predicate.
fn latest_finished_trace<'a>(
    plan: &ExecutionPlan,
    traces: &'a [ExecutionTrace],
    step_id: &Id,
) -> Option<&'a ExecutionTrace> {
    traces
        .iter()
        .filter(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == *step_id
                && trace.dispatch_state != ExecutionDispatchState::Started
        })
        .max_by_key(|trace| finished_at_seconds(&trace.finished_at))
}

/// The still-`Started` trace ids for the named steps — what `--supersede-trace`
/// (ADR 0014) takes, not the step id: `DispatchInProgress`'s target names the
/// specific started dispatch an operator must externally establish is dead,
/// the same distinction `blocked_target_ids` draws between a witness and the
/// cell it merely blocks.
fn started_trace_ids(
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    step_ids: &BTreeSet<Id>,
) -> Vec<Id> {
    dedupe(
        traces
            .iter()
            .filter(|trace| {
                trace.plan_id == plan.plan_id
                    && step_ids.contains(&trace.step_id)
                    && trace.dispatch_state == ExecutionDispatchState::Started
            })
            .map(|trace| trace.trace_id.clone())
            .collect(),
    )
}

/// `(needs_retry_decision, needs_plan_review)`, independently — both can be
/// `true` at once when different steps of the same plan need each, and
/// `derive_halts` reports both rather than only the higher-priority one. A
/// failed trace's own `obstructions` decide which bucket a step's failure
/// falls into, split from the single, coarser `dispatch_state: Failed` the
/// trace already carries for both outcomes. Only the most recently
/// *finished* trace per step is consulted — an already-superseded earlier
/// failure that a later success or a later, differently-classified failure
/// has moved past must not keep naming a halt nothing can still clear that
/// way.
///
/// A failed step counts toward `NeedsRetryDecision` only when
/// `solely_retry_blocked_step_ids` — `select_steps`'s own eligibility
/// verdict, not a second one computed here — says the failed trace is the
/// step's *only* blocking reason. A step whose work cell has since left the
/// frontier (resolved by another step, or by hand) has a permanent
/// ineligibility no `--retry-step` can waive; `select_steps` already knows
/// this, and re-dispatching the trace-classification logic without
/// consulting it produced a `needs_retry_decision` naming a step no retry
/// could ever make dispatchable again.
/// The one classification `retry_and_plan_review` (does either halt apply at
/// all) and `failed_step_ids` (which steps does a given halt name) both
/// need, so neither re-walks the plan with its own copy of the same rule.
/// Every step whose latest finished trace failed is classified as exactly
/// one of `NeedsRetryDecision` or `NeedsPlanReview` — never both — and a
/// step failed but neither plan-review-shaped nor solely blocked by that
/// failure (per `select_steps`) is simply absent: it has failed, but nothing
/// here can yet act on it.
fn classify_failed_steps<'a>(
    plan: &'a ExecutionPlan,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
) -> Vec<(&'a Id, Halt)> {
    plan.steps
        .iter()
        .filter_map(|step| {
            let latest = latest_finished_trace(plan, traces, &step.step_id)?;
            if latest.dispatch_state != ExecutionDispatchState::Failed {
                return None;
            }
            let is_plan_review = latest
                .obstructions
                .iter()
                .any(|obstruction| is_plan_review_obstruction(&obstruction.obstruction_type));
            if is_plan_review {
                Some((&step.step_id, Halt::NeedsPlanReview))
            } else if solely_retry_blocked_step_ids.contains(&step.step_id) {
                Some((&step.step_id, Halt::NeedsRetryDecision))
            } else {
                None
            }
        })
        .collect()
}

fn retry_and_plan_review(
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
) -> (bool, bool) {
    let classified = classify_failed_steps(plan, traces, solely_retry_blocked_step_ids);
    (
        classified
            .iter()
            .any(|(_, halt)| *halt == Halt::NeedsRetryDecision),
        classified
            .iter()
            .any(|(_, halt)| *halt == Halt::NeedsPlanReview),
    )
}

fn is_plan_review_obstruction(obstruction_type: &str) -> bool {
    matches!(
        obstruction_type,
        "transition_not_authorized" | "success_conditions_unsatisfied" | "invariant_regression"
    )
}

/// A tamper signal, not an ordinary worker failure: the binding the plan
/// accepted no longer matches what is registered (`binding_hash_mismatch`)
/// or what actually resolves (`binding_identity_mismatch`) —
/// `src/native_cli/ops/run.rs::inspect_worker_binding`'s two rejection
/// reasons for exactly that. Retrying against a still-mismatched binding
/// cannot succeed, so this stays under `needs_retry_decision` (it is still
/// an explicit operator decision, not something the plan's authorized
/// transition classes have any opinion about — `needs_plan_review` would be
/// the wrong instruction here, not merely an imprecise one) but changes what
/// `build_halt_report` tells the operator to do about it: re-register the
/// binding, not merely retry it.
fn is_binding_integrity_obstruction(obstruction_type: &str) -> bool {
    matches!(
        obstruction_type,
        "binding_identity_mismatch" | "binding_hash_mismatch"
    )
}

fn failed_step_ids(
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    halt: Halt,
    solely_retry_blocked_step_ids: &BTreeSet<Id>,
) -> Vec<Id> {
    classify_failed_steps(plan, traces, solely_retry_blocked_step_ids)
        .into_iter()
        .filter(|(_, classified_halt)| *classified_halt == halt)
        .map(|(step_id, _)| step_id.clone())
        .collect()
}

/// Whether any of `target_ids`' latest finished trace failed on a binding
/// integrity obstruction — read by `build_halt_report` to choose the
/// `needs_retry_decision` guidance, never by `derive_halt`/`failed_step_ids`,
/// which must not grow a second notion of what counts as retryable.
fn retryable_target_has_binding_integrity_failure(
    plan: &ExecutionPlan,
    traces: &[ExecutionTrace],
    target_ids: &[Id],
) -> bool {
    target_ids.iter().any(|step_id| {
        latest_finished_trace(plan, traces, step_id).is_some_and(|trace| {
            trace
                .obstructions
                .iter()
                .any(|obstruction| is_binding_integrity_obstruction(&obstruction.obstruction_type))
        })
    })
}

#[cfg(test)]
mod tests;
