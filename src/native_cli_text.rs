use crate::exec::records::ExecutionTrace;
use crate::native_eval::{
    NativeAssurance, NativeCaseEvaluation, NativeCompletionCandidate,
    NativeEvidenceBoundaryViolation, NativeEvidenceFinding, NativeEvidenceFindingType,
    NativeEvidenceFindings, NativeObstruction, NativeProgress, NativeReviewGap,
    NativeReviewGapType,
};
use crate::native_halt::{Halt, HaltReport, NextOperation};
use crate::native_model::{CaseMorphismType, MorphismLogEntry};
use higher_graphen_core::Id;
use std::{collections::BTreeMap, fmt::Write, str::FromStr};

pub(super) fn render_native_case_evaluation(
    evaluation: &NativeCaseEvaluation,
    changes_since: Option<&[MorphismLogEntry]>,
) -> String {
    render_reason_sections(&ReasonSections {
        progress: evaluation.progress,
        assurance: evaluation.assurance,
        frontier_cell_ids: &evaluation.frontier_cell_ids,
        waiting_cell_ids: &evaluation.readiness.waiting_cell_ids,
        obstructions: &evaluation.obstructions,
        evidence_findings: &evaluation.evidence_findings,
        review_gaps: &evaluation.review_gaps,
        completion_candidates: &evaluation.completion_candidates,
        changes_since,
    })
}

/// Bundled rather than positional: this is a pure projection of
/// [`NativeCaseEvaluation`] (plus the optional log slice `--since-revision`
/// asks for), so every field here already exists in that struct or in the
/// morphism log — nothing is computed here that the evaluator did not
/// already decide.
struct ReasonSections<'a> {
    progress: NativeProgress,
    assurance: NativeAssurance,
    frontier_cell_ids: &'a [Id],
    waiting_cell_ids: &'a [Id],
    obstructions: &'a [NativeObstruction],
    evidence_findings: &'a NativeEvidenceFindings,
    review_gaps: &'a [NativeReviewGap],
    completion_candidates: &'a [NativeCompletionCandidate],
    changes_since: Option<&'a [MorphismLogEntry]>,
}

fn render_reason_sections(sections: &ReasonSections<'_>) -> String {
    let mut output = String::new();
    writeln!(output, "Progress: {}", progress_name(sections.progress))
        .expect("writing to String cannot fail");
    writeln!(output, "Assurance: {}", assurance_name(sections.assurance))
        .expect("writing to String cannot fail");

    push_id_section(&mut output, "Frontier", sections.frontier_cell_ids);
    push_id_section(&mut output, "Waiting", sections.waiting_cell_ids);
    push_obstructions(&mut output, sections.obstructions);
    push_evidence_findings(
        &mut output,
        sections.evidence_findings,
        sections.review_gaps,
    );
    push_review_gaps(&mut output, sections.review_gaps);
    push_completion_candidates(&mut output, sections.completion_candidates);
    if let Some(changes_since) = sections.changes_since {
        push_changes_since(&mut output, changes_since);
    }

    output.pop();
    output
}

fn push_id_section(output: &mut String, title: &str, ids: &[Id]) {
    writeln!(output, "\n{title}:").expect("writing to String cannot fail");
    if ids.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    let mut ids = ids.iter().collect::<Vec<_>>();
    ids.sort();
    for id in ids {
        writeln!(output, "  - {id}").expect("writing to String cannot fail");
    }
}

fn push_obstructions(output: &mut String, obstructions: &[NativeObstruction]) {
    writeln!(output, "\nObstructions:").expect("writing to String cannot fail");
    if obstructions.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    let mut obstructions = obstructions.iter().collect::<Vec<_>>();
    obstructions.sort_by(|left, right| left.id.cmp(&right.id));
    for obstruction in obstructions {
        writeln!(
            output,
            "  - [{}]: {}",
            obstruction.id, obstruction.explanation
        )
        .expect("writing to String cannot fail");
        push_ids(output, "witnesses", &obstruction.witness_ids);
    }
}

fn push_evidence_findings(
    output: &mut String,
    evidence: &NativeEvidenceFindings,
    review_gaps: &[NativeReviewGap],
) {
    writeln!(output, "\nUnaccepted evidence findings:").expect("writing to String cannot fail");
    let mut findings = evidence
        .findings
        .iter()
        .filter(|finding| !finding.review_status.is_accepted())
        .collect::<Vec<_>>();
    findings.sort_by(|left, right| left.id.cmp(&right.id));
    let mut violations = evidence.boundary_violations.iter().collect::<Vec<_>>();
    violations.sort_by(|left, right| left.id.cmp(&right.id));
    if findings.is_empty() && violations.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    for finding in findings {
        push_evidence_finding(output, finding, review_gaps);
    }
    for violation in violations {
        push_evidence_violation(output, violation);
    }
}

fn push_evidence_finding(
    output: &mut String,
    finding: &NativeEvidenceFinding,
    review_gaps: &[NativeReviewGap],
) {
    // An unaccepted finding can still name evidence whose review gap the
    // evaluator has already marked `requirement_satisfied` (#20's
    // requirement-placeholder pattern): "this evidence is not accepted" and
    // "the hard requirement it would satisfy is already met another way" are
    // both true and not in tension, but only if the second fact is visible
    // too. There is no upstream finding↔gap association to read this off
    // of — `sections::review_gaps` never touches `evidence_findings.findings`
    // — so the join by `target_id`/`evidence_ids` below is this renderer's
    // own, not a reuse of one the evaluator already made.
    //
    // That join is sound only for finding types whose `evidence_ids` names
    // the same subject `requirement_satisfied` is about — the evidence cell
    // itself. `InferenceSeparated` and `PromotionRequired` are exactly
    // that (`sections.rs`'s `record_inference_evidence`/
    // `record_review_promotion` set `evidence_ids: vec![evidence.id]`).
    // `EvidenceMissing` is not: `sections.rs` sets its `evidence_ids` to
    // `obstruction.witness_ids`, the *(holder, requirement)* pair's
    // requirement half. Before issue #34, `requirement_satisfied`
    // (`compute_satisfied_requirement_ids`) was a union over every holder of
    // that requirement, so a finding about one still-blocked holder could
    // join onto a gap another, unrelated holder already satisfied — this
    // exclusion is what kept that join out. #34 scoped
    // `compute_satisfied_requirement_ids` to require every holder
    // (`docs/specs/requirement-satisfaction.fsl`'s `satisfied_for_all()`),
    // and `INV-EVID-001` there — proved by k-induction — makes a
    // `MissingEvidence` finding on a holder and `requirement_satisfied: true`
    // for its requirement mutually exclusive. So the contradiction this
    // exclusion was written to stop can no longer be produced, and the
    // exclusion is now redundant with respect to the evaluator's strictness.
    //
    // It stays, and the reason is not caution. The exclusion was never about
    // how strict the flag is: an `EvidenceMissing` finding's subject is a
    // *(holder, requirement)* pair while `requirement_satisfied` names the
    // requirement alone, and joining across a subject mismatch is wrong
    // however strict the flag happens to be. Strictness is a fact about
    // today's evaluator; the subject mismatch is a property of the two types.
    // A reader who observes that this branch can no longer fire must not
    // conclude it can be deleted — `native_halt.rs::is_clearable_by_review`
    // keeps its own constant comparison after its second producer was deleted,
    // for exactly this reason.
    //
    // This is a positive allowlist, not a filter on `EvidenceMissing`, so a
    // finding type added later defaults to unannotated rather than silently
    // joining on a key nobody checked for it.
    let requirement_satisfied = matches!(
        finding.finding_type,
        NativeEvidenceFindingType::InferenceSeparated
            | NativeEvidenceFindingType::PromotionRequired
    )
    .then(|| {
        review_gaps
            .iter()
            .find(|gap| finding.evidence_ids.contains(&gap.target_id))
            .map(|gap| gap.requirement_satisfied)
    })
    .flatten();
    match requirement_satisfied {
        Some(satisfied) => writeln!(
            output,
            "  - [{}]: {} [review_status={}] [requirement_satisfied={satisfied}]",
            finding.id, finding.summary, finding.review_status
        ),
        None => writeln!(
            output,
            "  - [{}]: {} [review_status={}]",
            finding.id, finding.summary, finding.review_status
        ),
    }
    .expect("writing to String cannot fail");
    push_ids(output, "evidence", &finding.evidence_ids);
}

fn push_evidence_violation(output: &mut String, violation: &NativeEvidenceBoundaryViolation) {
    writeln!(
        output,
        "  - [{}]: {} [evidence={}]",
        violation.id, violation.explanation, violation.evidence_id
    )
    .expect("writing to String cannot fail");
}

/// `UnreviewedInference` is the one gap type whose `requirement_satisfied` is
/// ever anything but a constant `false` — `sections::review_gaps`'s own doc
/// comment on the field says so, and its other three producers (one gap per
/// unreviewed completion candidate, log entry, and lossy projection) each
/// mint the identical explanation string per type, differing only by id. A
/// compact view printing one line per gap would reprint that same sentence
/// once per unreviewed log entry and grow without bound over a run's history
/// (#24 C3) — exactly the noise `NeedsReview` drowning out `NeedsEvidence`/
/// `NeedsExternal` in #23 was the same shape of mistake for a halt instead of
/// a render. So `UnreviewedInference` gaps — the ones carrying a per-target
/// fact worth reading individually — get their own line; every other type is
/// grouped under a count. This is a presentation choice, not a filter:
/// every id still appears, in the group's `targets` list, and the count is
/// exact.
fn push_review_gaps(output: &mut String, review_gaps: &[NativeReviewGap]) {
    writeln!(output, "\nReview gaps:").expect("writing to String cannot fail");
    if review_gaps.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    let mut inference_gaps = review_gaps
        .iter()
        .filter(|gap| gap.gap_type == NativeReviewGapType::UnreviewedInference)
        .collect::<Vec<_>>();
    inference_gaps.sort_by(|left, right| left.id.cmp(&right.id));
    for gap in inference_gaps {
        push_review_gap(output, gap);
    }
    let mut grouped = BTreeMap::<NativeReviewGapType, Vec<&NativeReviewGap>>::new();
    for gap in review_gaps
        .iter()
        .filter(|gap| gap.gap_type != NativeReviewGapType::UnreviewedInference)
    {
        grouped.entry(gap.gap_type).or_default().push(gap);
    }
    for (gap_type, gaps) in &grouped {
        // The representative explanation: every current producer of this
        // type mints the same sentence for every gap it produces, so the
        // first is as good as any — the count and the id list are what carry
        // the exactness, not this string.
        let explanation = gaps
            .first()
            .expect("grouped entry is never empty")
            .explanation
            .as_str();
        writeln!(
            output,
            "  - [{}]: {} gap(s) — {explanation}",
            review_gap_type_name(*gap_type),
            gaps.len(),
        )
        .expect("writing to String cannot fail");
        let target_ids = gaps
            .iter()
            .map(|gap| gap.target_id.clone())
            .collect::<Vec<_>>();
        push_ids(output, "targets", &target_ids);
    }
}

fn push_review_gap(output: &mut String, gap: &NativeReviewGap) {
    writeln!(
        output,
        "  - [{}]: {} [target={}] [gap_type={}] [requirement_satisfied={}]",
        gap.id,
        gap.explanation,
        gap.target_id,
        review_gap_type_name(gap.gap_type),
        gap.requirement_satisfied
    )
    .expect("writing to String cannot fail");
}

fn push_completion_candidates(output: &mut String, candidates: &[NativeCompletionCandidate]) {
    writeln!(output, "\nCompletion candidates:").expect("writing to String cannot fail");
    if candidates.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    let mut candidates = candidates.iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.id.cmp(&right.id));
    for candidate in candidates {
        writeln!(output, "  - [{}]: {}", candidate.id, candidate.rationale)
            .expect("writing to String cannot fail");
        push_ids(output, "targets", &candidate.target_ids);
    }
}

/// `--since-revision` names a revision already in this case space's history
/// (`ops::case_reason_text` refuses otherwise, the same assertion discipline
/// as `--base-revision-id`/`--completed-through`); `entries` is exactly the
/// log slice recorded after it. This prints the same fields `space history`
/// prints for those entries — no field here is computed, only sliced.
fn push_changes_since(output: &mut String, entries: &[MorphismLogEntry]) {
    writeln!(output, "\nChanged since:").expect("writing to String cannot fail");
    if entries.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    for entry in entries {
        writeln!(
            output,
            "  - {}: {} [actor={}] [recorded_at={}]",
            entry.target_revision_id,
            entry.morphism.morphism_type,
            entry.actor_id,
            entry.recorded_at
        )
        .expect("writing to String cannot fail");
        push_ids(output, "added", &entry.morphism.added_ids);
        push_ids(output, "updated", &entry.morphism.updated_ids);
        push_ids(output, "retired", &entry.morphism.retired_ids);
    }
}

fn push_ids(output: &mut String, label: &str, ids: &[Id]) {
    let mut ids = ids.iter().collect::<Vec<_>>();
    ids.sort();
    let joined = ids
        .into_iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(output, "    {label}: {joined}").expect("writing to String cannot fail");
}

fn progress_name(progress: NativeProgress) -> &'static str {
    match progress {
        NativeProgress::Active => "active",
        NativeProgress::Blocked => "blocked",
        NativeProgress::Complete => "complete",
    }
}

fn assurance_name(assurance: NativeAssurance) -> &'static str {
    match assurance {
        NativeAssurance::Unreviewed => "unreviewed",
        NativeAssurance::ReviewRequired => "review_required",
        NativeAssurance::Accepted => "accepted",
        NativeAssurance::Rejected => "rejected",
    }
}

fn review_gap_type_name(gap_type: NativeReviewGapType) -> &'static str {
    match gap_type {
        NativeReviewGapType::UnreviewedCompletion => "unreviewed_completion",
        NativeReviewGapType::UnreviewedInference => "unreviewed_inference",
        NativeReviewGapType::UnreviewedMorphism => "unreviewed_morphism",
        NativeReviewGapType::UnreviewedProjectionLoss => "unreviewed_projection_loss",
    }
}

/// Whether `space history --format text` was able to read the execution
/// traces its fold depends on. Kept as a caller-supplied fact rather than a
/// `Result` the renderer computes: reading `runs/` can fail for reasons (a
/// missing or malformed trace file elsewhere in the store) that have nothing
/// to do with the log this function projects, and the renderer must not turn
/// that read failure into a guess about what the log says.
pub(super) enum TraceAvailability<'a> {
    Available(&'a [ExecutionTrace]),
    Unreadable(String),
}

/// Projects the morphism log into text, annotating an `execution_trace_anchor`
/// entry with the dispatches it superseded (ADR 0014). There is exactly one
/// log entry to consider per line, never several to collapse: a superseded
/// trace is by construction one `run --frontier`/`operate` killed before it
/// finished, so it never reached `write_and_anchor_trace` and never got a log
/// entry of its own (`decide_superseded_traces` in `ops/run.rs` refuses to
/// let `--supersede-trace` name a trace that ever did). What this annotates
/// is the *surviving* entry's own trace file naming the ids it superseded in
/// `metadata.superseded_trace_ids` — folding here means adding that fact to
/// the one line that exists, not merging lines that don't. Three rules keep
/// that a projection rather than a second opinion about what happened, each
/// with a test named for it in this module's test list:
///
/// 1. **Collapse only what is actually named.** The annotation lists exactly
///    the trace ids this entry's own trace names in `superseded_trace_ids` —
///    never a trace merely sharing its step id or sitting next to it in the
///    log. Adjacency is not supersession, and treating it as one would have
///    this projection invent a relationship the log does not record.
/// 2. **An unreadable trace renders unfolded.** If `traces` is
///    [`TraceAvailability::Unreadable`], every entry renders with no
///    annotation. A fold that silently swallowed an entry it could not
///    verify would read as covered when it was not — exactly what the log
///    exists to prevent.
/// 3. **The count must be the real count.** The "N attempts" an annotated
///    line reports is exactly `superseded_trace_ids.len() + 1` (the entry's
///    own trace). If any id in that list fails to parse, the whole
///    annotation is dropped for that entry rather than reporting a count
///    that might be wrong.
pub(super) fn render_case_history(
    entries: &[MorphismLogEntry],
    traces: TraceAvailability<'_>,
) -> String {
    let mut output = String::new();
    if let TraceAvailability::Unreadable(reason) = &traces {
        writeln!(
            output,
            "Execution traces unreadable, rendering entries unfolded: {reason}\n"
        )
        .expect("writing to String cannot fail");
    }
    writeln!(output, "Entries:").expect("writing to String cannot fail");
    if entries.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        output.pop();
        return output;
    }
    let traces_by_id = match &traces {
        TraceAvailability::Available(traces) => traces
            .iter()
            .map(|trace| (trace.trace_id.clone(), trace))
            .collect::<BTreeMap<_, _>>(),
        TraceAvailability::Unreadable(_) => BTreeMap::new(),
    };
    for entry in entries {
        let trace_id = anchor_trace_id(entry);
        let attempts = trace_id.as_ref().and_then(|trace_id| {
            let trace = traces_by_id.get(trace_id)?;
            let mut attempts = superseded_trace_ids(trace)?;
            attempts.push(trace_id.clone());
            Some(attempts)
        });
        push_history_entry(&mut output, entry, trace_id.as_ref(), attempts.as_deref());
    }
    output.pop();
    output
}

/// `run --step`/`run --frontier`/`operate`/`packet apply`/`packet resume`
/// `--format text` (issue #35): a pure projection of the `HaltReport`
/// value(s) those commands already computed and put in the JSON report
/// (`result.halt`/`result.halts`) — nothing here decides anything about
/// *why* the ledger stopped, only how to print what `derive_halts`
/// (`native_halt.rs`, the single implementation of that decision) already
/// answered. `halt` is the head `derive_halt` reports and `halts` is the
/// full ranked list (ADR 0016 decision 2); both are rendered in full so a
/// reader never has to re-derive one from the other.
pub(super) fn render_halt_section(halt: Option<&HaltReport>, halts: &[HaltReport]) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "Halt: {}",
        halt.map_or("(none)", |report| halt_name(report.halt))
    )
    .expect("writing to String cannot fail");
    if let Some(report) = halt {
        writeln!(output, "Completed through: {}", report.completed_through)
            .expect("writing to String cannot fail");
    }
    push_halt_list(&mut output, halts);
    output.pop();
    output
}

fn halt_name(halt: Halt) -> &'static str {
    match halt {
        Halt::RoundBudgetExhausted => "round_budget_exhausted",
        Halt::NeedsReview => "needs_review",
        Halt::NeedsRetryDecision => "needs_retry_decision",
        Halt::NeedsPlanReview => "needs_plan_review",
        Halt::NeedsEvidence => "needs_evidence",
        Halt::NeedsExternal => "needs_external",
        Halt::DispatchInProgress => "dispatch_in_progress",
        Halt::NothingEligible => "nothing_eligible",
    }
}

fn push_halt_list(output: &mut String, halts: &[HaltReport]) {
    writeln!(output, "\nAll halts:").expect("writing to String cannot fail");
    if halts.is_empty() {
        writeln!(output, "  (none)").expect("writing to String cannot fail");
        return;
    }
    for report in halts {
        writeln!(output, "  - {}", halt_name(report.halt)).expect("writing to String cannot fail");
        writeln!(
            output,
            "    Completed through: {}",
            report.completed_through
        )
        .expect("writing to String cannot fail");
        let mut target_ids = report.target_ids.iter().collect::<Vec<_>>();
        target_ids.sort();
        if target_ids.is_empty() {
            writeln!(output, "    Targets: (none)").expect("writing to String cannot fail");
        } else {
            writeln!(output, "    Targets:").expect("writing to String cannot fail");
            for id in target_ids {
                writeln!(output, "      - {id}").expect("writing to String cannot fail");
            }
        }
        push_next_operations(output, &report.next_operations);
    }
}

/// Structured, never assembled into a runnable command string — the same
/// discipline `NextOperation`'s own doc comment requires of every consumer
/// (issue #19's rule, generalised by ADR 0016 decision 2). Each argument is
/// printed as its own `key: value` line so a reader can see every field a
/// command string would have otherwise hidden inside concatenation.
fn push_next_operations(output: &mut String, next_operations: &[NextOperation]) {
    if next_operations.is_empty() {
        writeln!(output, "    Next operations: (none)").expect("writing to String cannot fail");
        return;
    }
    writeln!(output, "    Next operations:").expect("writing to String cannot fail");
    for operation in next_operations {
        writeln!(output, "      - {}", operation.command).expect("writing to String cannot fail");
        for (key, value) in &operation.arguments {
            writeln!(output, "          {key}: {value}").expect("writing to String cannot fail");
        }
        if let Some(note) = &operation.note {
            writeln!(output, "        note: {note}").expect("writing to String cannot fail");
        }
    }
}

/// The trace ids this trace's own file names as superseded, plus itself
/// appended last — `None` when there is nothing to report (no field, an
/// empty list) or the field could not be trusted (any id failed to parse:
/// rule 3, never report a count that might be wrong).
fn superseded_trace_ids(trace: &ExecutionTrace) -> Option<Vec<Id>> {
    let raw = trace
        .metadata
        .get("superseded_trace_ids")
        .and_then(|value| value.as_array())?;
    if raw.is_empty() {
        return None;
    }
    raw.iter()
        .map(|value| value.as_str().and_then(|raw| Id::from_str(raw).ok()))
        .collect()
}

fn push_history_entry(
    output: &mut String,
    entry: &MorphismLogEntry,
    trace_id: Option<&Id>,
    attempts: Option<&[Id]>,
) {
    match (trace_id, attempts) {
        // The attempts list already ends with this entry's own trace id, so
        // a separate `[trace_id=...]` tag would only repeat it.
        (_, Some(attempts)) => {
            let listed = attempts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                output,
                "  - [{}]: {} ({} attempts: {listed}) [actor={}] [recorded_at={}]",
                entry.target_revision_id,
                entry.morphism.morphism_type,
                attempts.len(),
                entry.actor_id,
                entry.recorded_at,
            )
        }
        (Some(trace_id), None) => writeln!(
            output,
            "  - [{}]: {} [trace_id={trace_id}] [actor={}] [recorded_at={}]",
            entry.target_revision_id,
            entry.morphism.morphism_type,
            entry.actor_id,
            entry.recorded_at
        ),
        (None, None) => writeln!(
            output,
            "  - [{}]: {} [actor={}] [recorded_at={}]",
            entry.target_revision_id,
            entry.morphism.morphism_type,
            entry.actor_id,
            entry.recorded_at
        ),
    }
    .expect("writing to String cannot fail");
}

fn anchor_trace_id(entry: &MorphismLogEntry) -> Option<Id> {
    if entry.morphism.morphism_type != CaseMorphismType::Custom("execution_trace_anchor".to_owned())
    {
        return None;
    }
    entry
        .morphism
        .metadata
        .get("trace_id")
        .and_then(|value| value.as_str())
        .and_then(|raw| Id::from_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_evaluator_obstruction_appears_in_text_output() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let selectors: Vec<u8> = u.arbitrary()?;
                let obstructions = selectors
                    .into_iter()
                    .take(32)
                    .enumerate()
                    .map(|(index, selector)| {
                        serde_json::from_value::<NativeObstruction>(json!({
                            "id": format!("obstruction:property-{index}-{selector}"),
                            "obstruction_type": "missing_evidence",
                            "affected_ids": [],
                            "source_constraint_id": format!("constraint:property-{index}"),
                            "witness_ids": [format!("witness:property-{index}")],
                            "explanation": format!("property explanation {index} {selector}"),
                            "severity": "high",
                            "required_resolution": "supply evaluator-owned evidence",
                            "blocking": true,
                            "provenance": {
                                "source": {"kind": "document"},
                                "confidence": 1.0,
                                "review_status": "unreviewed"
                            }
                        }))
                        .expect("property obstruction")
                    })
                    .collect::<Vec<_>>();
                let evidence = NativeEvidenceFindings {
                    accepted_evidence_ids: Vec::new(),
                    source_backed_evidence_ids: Vec::new(),
                    inference_record_ids: Vec::new(),
                    unreviewed_inference_ids: Vec::new(),
                    promoted_evidence_ids: Vec::new(),
                    boundary_violations: Vec::new(),
                    findings: Vec::new(),
                };

                let rendered = render_reason_sections(&ReasonSections {
                    progress: NativeProgress::Blocked,
                    assurance: NativeAssurance::ReviewRequired,
                    frontier_cell_ids: &[],
                    waiting_cell_ids: &[],
                    obstructions: &obstructions,
                    evidence_findings: &evidence,
                    review_gaps: &[],
                    completion_candidates: &[],
                    changes_since: None,
                });

                for obstruction in &obstructions {
                    assert!(rendered.contains(obstruction.id.as_str()));
                    assert!(rendered.contains(&obstruction.explanation));
                    for witness_id in &obstruction.witness_ids {
                        assert!(rendered.contains(witness_id.as_str()));
                    }
                }
                Ok(())
            },
        );
    }

    #[test]
    fn every_review_gap_appears_in_text_output_at_its_own_or_its_groups_cardinality() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let selectors: Vec<u8> = u.arbitrary()?;
                // Every real producer of a grouped type mints the identical
                // explanation for every gap of that type (`sections::
                // review_gaps`); the constant-per-type strings here match
                // that, so the grouping property (C3) is exercised the way
                // production data actually looks, not defeated by giving
                // every generated gap a unique sentence.
                let grouped_gap_types = [
                    (
                        "unreviewed_completion",
                        "constant explanation for unreviewed_completion",
                    ),
                    (
                        "unreviewed_morphism",
                        "constant explanation for unreviewed_morphism",
                    ),
                    (
                        "unreviewed_projection_loss",
                        "constant explanation for unreviewed_projection_loss",
                    ),
                ];
                let review_gaps = selectors
                    .into_iter()
                    .take(16)
                    .enumerate()
                    .map(|(index, selector)| {
                        let is_inference = selector % 2 == 0;
                        let (gap_type, explanation) = if is_inference {
                            (
                                "unreviewed_inference",
                                format!("property inference explanation {index}"),
                            )
                        } else {
                            let (gap_type, explanation) =
                                grouped_gap_types[index % grouped_gap_types.len()];
                            (gap_type, explanation.to_owned())
                        };
                        serde_json::from_value::<NativeReviewGap>(json!({
                            "id": format!("review_gap:property-{index}-{selector}"),
                            "target_id": format!("target:property-{index}"),
                            "gap_type": gap_type,
                            "explanation": explanation,
                            "requirement_satisfied": is_inference && selector % 4 == 0,
                        }))
                        .expect("property review gap")
                    })
                    .collect::<Vec<_>>();
                let evidence = NativeEvidenceFindings {
                    accepted_evidence_ids: Vec::new(),
                    source_backed_evidence_ids: Vec::new(),
                    inference_record_ids: Vec::new(),
                    unreviewed_inference_ids: Vec::new(),
                    promoted_evidence_ids: Vec::new(),
                    boundary_violations: Vec::new(),
                    findings: Vec::new(),
                };

                let rendered = render_reason_sections(&ReasonSections {
                    progress: NativeProgress::Blocked,
                    assurance: NativeAssurance::ReviewRequired,
                    frontier_cell_ids: &[],
                    waiting_cell_ids: &[],
                    obstructions: &[],
                    evidence_findings: &evidence,
                    review_gaps: &review_gaps,
                    completion_candidates: &[],
                    changes_since: None,
                });

                // `UnreviewedInference` is the one type carrying a
                // per-target fact: every one of its gaps renders its own
                // line, in full.
                for gap in review_gaps
                    .iter()
                    .filter(|gap| gap.gap_type == NativeReviewGapType::UnreviewedInference)
                {
                    assert!(rendered.contains(gap.id.as_str()));
                    assert!(rendered.contains(gap.target_id.as_str()));
                    assert!(rendered.contains(&gap.explanation));
                    assert!(rendered.contains(&format!(
                        "[requirement_satisfied={}]",
                        gap.requirement_satisfied
                    )));
                }

                // Every other type is grouped: the target id still appears
                // (nothing is hidden), and the count next to that type's
                // name is exactly the number of gaps of that type (nothing
                // is misrepresented).
                for (gap_type_name, _) in grouped_gap_types {
                    let count = review_gaps
                        .iter()
                        .filter(|gap| {
                            serde_json::to_value(gap.gap_type).expect("gap type serializes")
                                == json!(gap_type_name)
                        })
                        .count();
                    if count == 0 {
                        continue;
                    }
                    for gap in review_gaps.iter().filter(|gap| {
                        serde_json::to_value(gap.gap_type).expect("gap type serializes")
                            == json!(gap_type_name)
                    }) {
                        assert!(rendered.contains(gap.target_id.as_str()));
                    }
                    assert!(
                        rendered.contains(&format!("[{gap_type_name}]: {count} gap(s)")),
                        "expected an exact count for {gap_type_name}: {rendered}"
                    );
                }
                Ok(())
            },
        );
    }

    /// The defect this fix closes (#24): an unaccepted evidence finding and
    /// a satisfied review gap for the same evidence id must render together,
    /// not as two facts that read as contradictory.
    #[test]
    fn an_unaccepted_finding_with_a_satisfied_review_gap_renders_both_facts() {
        let evidence_id = "evidence:required";
        let evidence = NativeEvidenceFindings {
            accepted_evidence_ids: Vec::new(),
            source_backed_evidence_ids: Vec::new(),
            inference_record_ids: Vec::new(),
            unreviewed_inference_ids: Vec::new(),
            promoted_evidence_ids: Vec::new(),
            boundary_violations: Vec::new(),
            findings: vec![serde_json::from_value(json!({
                "id": "finding:inference-separated",
                "finding_type": "inference_separated",
                "evidence_ids": [evidence_id],
                "summary": format!("{evidence_id} is inference and is not accepted evidence."),
                "review_status": "unreviewed"
            }))
            .expect("finding")],
        };
        let review_gaps = vec![serde_json::from_value(json!({
            "id": "review_gap:evidence-required-inference",
            "target_id": evidence_id,
            "gap_type": "unreviewed_inference",
            "explanation": "AI inference is separated from accepted evidence until review promotion.",
            "requirement_satisfied": true
        }))
        .expect("review gap")];

        let rendered = render_reason_sections(&ReasonSections {
            progress: NativeProgress::Active,
            assurance: NativeAssurance::Accepted,
            frontier_cell_ids: &[],
            waiting_cell_ids: &[],
            obstructions: &[],
            evidence_findings: &evidence,
            review_gaps: &review_gaps,
            completion_candidates: &[],
            changes_since: None,
        });

        assert!(rendered.contains("evidence:required is inference and is not accepted evidence."));
        assert!(rendered.contains("[review_status=unreviewed]"));
        assert!(rendered.contains("[requirement_satisfied=true]"));
    }

    /// C4 (#24 review): an `EvidenceMissing` finding's `evidence_ids` names
    /// the *requirement* half of a (holder, requirement) pair
    /// (`sections.rs` sets it to `obstruction.witness_ids`), not the
    /// evidence the finding is about. Before issue #34, `requirement_satisfied`
    /// was a union over every holder of that requirement, so a finding about
    /// one still-blocked holder could share a `target_id` with a gap another,
    /// unrelated holder already satisfied — annotating it anyway would have
    /// asserted "none is available" and "requirement_satisfied=true" in the
    /// same line, exactly the contradiction #24 exists to stop. #34 scoped
    /// `compute_satisfied_requirement_ids` to require every holder, which
    /// makes the shared-`target_id`, satisfied-via-an-unrelated-holder gap
    /// this test constructs by hand no longer reachable from a real
    /// evaluation — the fixture below stays a hand-built worst case, kept as
    /// a belt-and-suspenders test of the allowlist itself rather than a
    /// reproduction of a still-live defect. The allowlist in
    /// `push_evidence_finding` must never annotate this type, even when a
    /// same-`target_id` gap marked `true` exists.
    #[test]
    fn an_evidence_missing_finding_is_never_annotated_even_when_a_same_target_gap_is_satisfied() {
        let requirement_id = "evidence:shared-requirement";
        let evidence = NativeEvidenceFindings {
            accepted_evidence_ids: Vec::new(),
            source_backed_evidence_ids: Vec::new(),
            inference_record_ids: Vec::new(),
            unreviewed_inference_ids: Vec::new(),
            promoted_evidence_ids: Vec::new(),
            boundary_violations: Vec::new(),
            findings: vec![serde_json::from_value(json!({
                "id": "finding:work-w2-evidence-missing",
                "finding_type": "evidence_missing",
                "evidence_ids": [requirement_id],
                "summary": format!(
                    "work:w2 requires source-backed or accepted evidence {requirement_id}, but none is available."
                ),
                "review_status": "unreviewed"
            }))
            .expect("finding")],
        };
        // A gap sharing the same `target_id`, satisfied via an unrelated
        // holder — exactly the coarser-key reproduction from the review.
        let review_gaps = vec![serde_json::from_value(json!({
            "id": "review_gap:shared-requirement-inference",
            "target_id": requirement_id,
            "gap_type": "unreviewed_inference",
            "explanation": "AI inference is separated from accepted evidence until review promotion.",
            "requirement_satisfied": true
        }))
        .expect("review gap")];

        let rendered = render_reason_sections(&ReasonSections {
            progress: NativeProgress::Blocked,
            assurance: NativeAssurance::ReviewRequired,
            frontier_cell_ids: &[],
            waiting_cell_ids: &[],
            obstructions: &[],
            evidence_findings: &evidence,
            review_gaps: &review_gaps,
            completion_candidates: &[],
            changes_since: None,
        });

        // The finding's own line, verbatim: no `[requirement_satisfied=...]`
        // tag at all, even though a same-`target_id` gap exists and is
        // satisfied. (The "Review gaps" section below still shows the gap's
        // own `requirement_satisfied` — that is a different line about a
        // different subject, so this checks the finding's line specifically
        // rather than the whole rendered text.)
        assert!(rendered.contains(&format!(
            "  - [finding:work-w2-evidence-missing]: work:w2 requires source-backed or accepted \
             evidence {requirement_id}, but none is available. [review_status=unreviewed]\n"
        )));
    }

    /// `halt_name`'s hand-rolled match must never drift from `Halt`'s own
    /// `#[serde(rename_all = "snake_case")]` encoding — the same encoding
    /// `native-cli.report.schema.json`'s `halt_report.halt` enum lists and
    /// every JSON report actually emits. This pins the two together so a new
    /// `Halt` variant that only updates one of them fails here rather than
    /// silently rendering the wrong word.
    #[test]
    fn halt_names_match_their_serde_encoding() {
        for halt in ALL_HALTS {
            let expected = serde_json::to_value(halt)
                .expect("Halt serializes")
                .as_str()
                .expect("Halt serializes to a string")
                .to_owned();
            assert_eq!(halt_name(halt), expected);
        }
    }

    #[test]
    fn render_halt_section_shows_none_when_there_is_no_halt() {
        let rendered = render_halt_section(None, &[]);
        assert!(rendered.contains("Halt: (none)"));
        assert!(rendered.contains("All halts:\n  (none)"));
        assert!(!rendered.contains("Completed through"));
    }

    #[test]
    fn render_halt_section_shows_the_primary_halt_and_its_next_operations() {
        let report: HaltReport = serde_json::from_value(json!({
            "halt": "needs_evidence",
            "completed_through": "revision:one",
            "target_ids": ["evidence:required"],
            "next_operations": [{
                "command": "evidence attach",
                "arguments": {
                    "store": "/tmp/store",
                    "case_space_id": "case_space:x"
                },
                "note": "attach source-backed evidence"
            }]
        }))
        .expect("halt report");
        let halts = vec![report.clone()];

        let rendered = render_halt_section(Some(&report), &halts);

        assert!(rendered.contains("Halt: needs_evidence"));
        assert!(rendered.contains("Completed through: revision:one"));
        assert!(rendered.contains("evidence:required"));
        assert!(rendered.contains("evidence attach"));
        assert!(rendered.contains("store: /tmp/store"));
        assert!(rendered.contains("case_space_id: case_space:x"));
        assert!(rendered.contains("attach source-backed evidence"));
    }

    #[test]
    fn render_halt_section_shows_no_next_operations_as_none() {
        let report: HaltReport = serde_json::from_value(json!({
            "halt": "nothing_eligible",
            "completed_through": "revision:one",
            "target_ids": [],
            "next_operations": []
        }))
        .expect("halt report");
        let halts = vec![report.clone()];

        let rendered = render_halt_section(Some(&report), &halts);

        assert!(rendered.contains("Targets: (none)"));
        assert!(rendered.contains("Next operations: (none)"));
    }

    const ALL_HALTS: [Halt; 8] = [
        Halt::RoundBudgetExhausted,
        Halt::NeedsReview,
        Halt::NeedsRetryDecision,
        Halt::NeedsPlanReview,
        Halt::NeedsEvidence,
        Halt::NeedsExternal,
        Halt::DispatchInProgress,
        Halt::NothingEligible,
    ];

    /// The render-only property issue #35 asks for: every halt member and
    /// every `next_operation` present in the report appears in the text
    /// rendering, and the text rendering names no halt member the report did
    /// not have. This is what would fail if `render_halt_section` decided
    /// something (summarised, filtered, or renamed a halt) instead of
    /// projecting exactly what `derive_halts` already computed.
    #[test]
    fn text_rendering_shows_exactly_the_halts_and_next_operations_the_report_has() {
        arbtest::arbtest(
            |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
                let halt_count = u.int_in_range(0_usize..=3)?;
                let mut halts = Vec::new();
                for index in 0..halt_count {
                    let halt = *u.choose(&ALL_HALTS)?;
                    let target_count = u.int_in_range(0_usize..=2)?;
                    let target_ids = (0..target_count)
                        .map(|target_index| format!("target:property-{index}-{target_index}"))
                        .collect::<Vec<_>>();
                    let operation_count = u.int_in_range(0_usize..=2)?;
                    let mut next_operations = Vec::new();
                    for operation_index in 0..operation_count {
                        let has_note: bool = u.arbitrary()?;
                        next_operations.push(json!({
                            "command": format!("command-property-{index}-{operation_index}"),
                            "arguments": {
                                format!("arg-{index}-{operation_index}"):
                                    format!("value-{index}-{operation_index}")
                            },
                            "note": if has_note {
                                json!(format!("note-property-{index}-{operation_index}"))
                            } else {
                                json!(null)
                            }
                        }));
                    }
                    let report: HaltReport = serde_json::from_value(json!({
                        "halt": serde_json::to_value(halt).expect("halt serializes"),
                        "completed_through": format!("revision:property-{index}"),
                        "target_ids": target_ids,
                        "next_operations": next_operations,
                    }))
                    .expect("property halt report");
                    halts.push(report);
                }
                let halt_head = halts.first().cloned();

                let rendered = render_halt_section(halt_head.as_ref(), &halts);

                for report in &halts {
                    let name = serde_json::to_value(report.halt)
                        .expect("halt serializes")
                        .as_str()
                        .expect("halt serializes to a string")
                        .to_owned();
                    assert!(
                        rendered.contains(&name),
                        "missing halt member {name}: {rendered}"
                    );
                    assert!(rendered.contains(report.completed_through.as_str()));
                    for target_id in &report.target_ids {
                        assert!(rendered.contains(target_id.as_str()));
                    }
                    for operation in &report.next_operations {
                        assert!(rendered.contains(&operation.command));
                        for (key, value) in &operation.arguments {
                            assert!(rendered.contains(key.as_str()));
                            assert!(rendered.contains(value.as_str()));
                        }
                        if let Some(note) = &operation.note {
                            assert!(rendered.contains(note.as_str()));
                        }
                    }
                }

                // The other direction: no halt member appears in the
                // rendering that this report list does not actually have.
                for candidate in ALL_HALTS {
                    let name = serde_json::to_value(candidate)
                        .expect("halt serializes")
                        .as_str()
                        .expect("halt serializes to a string")
                        .to_owned();
                    let present_in_data = halts.iter().any(|report| report.halt == candidate);
                    let present_in_text = rendered.contains(&name);
                    assert_eq!(
                        present_in_text, present_in_data,
                        "halt member {name} presence mismatch: {rendered}"
                    );
                }

                Ok(())
            },
        );
    }
}
