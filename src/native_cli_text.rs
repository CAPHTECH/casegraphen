use crate::native_eval::{
    NativeCaseEvaluation, NativeCompletionCandidate, NativeEvidenceBoundaryViolation,
    NativeEvidenceFinding, NativeEvidenceFindings, NativeObstruction, NativeReasoningStatus,
};
use higher_graphen_core::Id;
use std::fmt::Write;

pub(super) fn render_native_case_evaluation(evaluation: &NativeCaseEvaluation) -> String {
    render_reason_sections(
        evaluation.status,
        &evaluation.frontier_cell_ids,
        &evaluation.obstructions,
        &evaluation.evidence_findings,
        &evaluation.completion_candidates,
    )
}

fn render_reason_sections(
    status: NativeReasoningStatus,
    frontier_cell_ids: &[Id],
    obstructions: &[NativeObstruction],
    evidence_findings: &NativeEvidenceFindings,
    completion_candidates: &[NativeCompletionCandidate],
) -> String {
    let mut output = String::new();
    writeln!(output, "Status: {}", reasoning_status(status))
        .expect("writing to String cannot fail");

    push_id_section(&mut output, "Frontier", frontier_cell_ids);
    push_obstructions(&mut output, obstructions);
    push_evidence_findings(&mut output, evidence_findings);
    push_completion_candidates(&mut output, completion_candidates);

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
            "  - {}: {}",
            obstruction.id, obstruction.explanation
        )
        .expect("writing to String cannot fail");
        push_ids(output, "witnesses", &obstruction.witness_ids);
    }
}

fn push_evidence_findings(output: &mut String, evidence: &NativeEvidenceFindings) {
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
        push_evidence_finding(output, finding);
    }
    for violation in violations {
        push_evidence_violation(output, violation);
    }
}

fn push_evidence_finding(output: &mut String, finding: &NativeEvidenceFinding) {
    writeln!(
        output,
        "  - {}: {} [review_status={}]",
        finding.id, finding.summary, finding.review_status
    )
    .expect("writing to String cannot fail");
    push_ids(output, "evidence", &finding.evidence_ids);
}

fn push_evidence_violation(output: &mut String, violation: &NativeEvidenceBoundaryViolation) {
    writeln!(
        output,
        "  - {}: {} [evidence={}]",
        violation.id, violation.explanation, violation.evidence_id
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
        writeln!(output, "  - {}: {}", candidate.id, candidate.rationale)
            .expect("writing to String cannot fail");
        push_ids(output, "targets", &candidate.target_ids);
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

fn reasoning_status(status: NativeReasoningStatus) -> &'static str {
    match status {
        NativeReasoningStatus::Ready => "ready",
        NativeReasoningStatus::Blocked => "blocked",
        NativeReasoningStatus::Incomplete => "incomplete",
        NativeReasoningStatus::ReviewRequired => "review_required",
    }
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

                let rendered = render_reason_sections(
                    NativeReasoningStatus::Blocked,
                    &[],
                    &obstructions,
                    &evidence,
                    &[],
                );

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
}
