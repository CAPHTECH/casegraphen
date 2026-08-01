use super::*;
use crate::native_model::{
    CaseMorphism, CaseMorphismType, MorphismLogEntry, Projection, ProjectionAudience, ReviewAction,
    Revision, NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION,
    NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
};
use higher_graphen_core::{Provenance, SourceKind, SourceRef};
use serde_json::{json, Map, Value};

const NATIVE_EXAMPLE: &str =
    include_str!("../../schemas/casegraphen/native.case.space.example.json");

#[test]
fn native_example_evaluates_with_domain_findings() {
    let space: CaseSpace = serde_json::from_str(NATIVE_EXAMPLE).expect("native example");
    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:review-native-contract")));
    assert!(evaluation
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:native-schema-json-valid")));
    assert_eq!(evaluation.projection_loss.len(), 1);
}

#[test]
fn hard_dependencies_control_ready_and_frontier() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:ready",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:blocked",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:dep",
        CaseCellType::Work,
        CaseCellLifecycle::Accepted,
    ));
    space.case_relations.push(relation(
        "relation:blocked-depends-on-ready-dep",
        CaseRelationType::DependsOn,
        "work:blocked",
        "work:dep",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:ready")));
    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:blocked")));
    assert!(evaluation.frontier_cell_ids.contains(&id("work:ready")));
    assert!(evaluation.frontier_cell_ids.contains(&id("work:blocked")));
}

#[test]
fn soft_dependencies_do_not_block_native_readiness() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:soft-dependent",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:soft-dependency",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut soft = relation(
        "relation:soft-dependent-depends-on-soft-dependency",
        CaseRelationType::DependsOn,
        "work:soft-dependent",
        "work:soft-dependency",
    );
    soft.relation_strength = RelationStrength::Soft;
    space.case_relations.push(soft);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:soft-dependent")));
    assert!(!evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:soft-dependent")));
}

#[test]
fn completed_or_superseded_targets_are_removed_from_frontier() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:completed-target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "event:completion",
        CaseCellType::Event,
        CaseCellLifecycle::Accepted,
    ));
    space.case_relations.push(relation(
        "relation:event-completes-work",
        CaseRelationType::Completes,
        "event:completion",
        "work:completed-target",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:completed-target")));
    assert!(!evaluation
        .frontier_cell_ids
        .contains(&id("work:completed-target")));
}

#[test]
fn missing_evidence_and_proof_block_readiness() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "proof:obligation",
        CaseCellType::Proof,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:needs-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs",
        "evidence:missing",
    ));
    space.case_relations.push(relation(
        "relation:needs-proof",
        CaseRelationType::RequiresProof,
        "work:needs",
        "proof:obligation",
    ));
    refresh_morphism(&mut space);

    let err = evaluate_native_case(&space).expect_err("dangling evidence is malformed");
    assert!(err
        .violations
        .iter()
        .any(|violation| violation.code == NativeEvalViolationCode::DanglingReference));

    let mut missing_evidence = cell(
        "evidence:missing",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    missing_evidence.source_ids.clear();
    space.case_cells.push(missing_evidence);
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    assert!(evaluation
        .obstructions
        .iter()
        .any(|obstruction| obstruction.obstruction_type == NativeObstructionType::MissingEvidence));
    assert!(evaluation
        .obstructions
        .iter()
        .any(|obstruction| obstruction.obstruction_type == NativeObstructionType::MissingProof));
}

#[test]
fn inferred_evidence_does_not_satisfy_requirement() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:ai-guess",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-ai-guess",
        CaseRelationType::RequiresEvidence,
        "work:needs-evidence",
        "evidence:ai-guess",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-evidence")));
    assert!(evaluation
        .evidence_findings
        .unreviewed_inference_ids
        .contains(&id("evidence:ai-guess")));
}

#[test]
fn canonical_review_accept_flips_inferred_evidence_findings_and_assurance() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:log-accepted-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let before = evaluate_native_case(&space).expect("evaluation before review");
    assert!(before
        .evidence_findings
        .unreviewed_inference_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(!before
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(before.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedInference
            && gap.target_id == id("evidence:log-accepted-inference")
    }));
    assert_ne!(before.assurance, NativeAssurance::Accepted);

    // The stored `provenance.review_status` never moves — review morphisms
    // have empty `updated_ids` by design — so only the log carries the
    // acceptance. This mirrors how `native_review::accept_review_morphism`
    // builds the canonical review the CLI's `review accept` appends.
    let review = crate::native_review::accept_review_morphism(
        &space,
        crate::native_review::NativeReviewRequest {
            target_kind: crate::native_review::NativeReviewTargetKind::Evidence,
            target_id: id("evidence:log-accepted-inference"),
            action: ReviewAction::Accept,
            reviewer_id: id("reviewer:native-eval-test"),
            reviewed_at: "2026-04-26T00:30:00Z".to_owned(),
            reason: "Reviewed during the evidence-findings divergence test.".to_owned(),
            evidence_ids: vec![id("evidence:log-accepted-inference")],
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:log-accepted-review"),
        },
    )
    .expect("canonical accept");
    append_review_entry(&mut space, review, "entry:log-accepted-review");
    assert!(
        space
            .case_cells
            .iter()
            .any(|cell| cell.id == id("evidence:log-accepted-inference")
                && cell.provenance.review_status == ReviewStatus::Unreviewed),
        "the stored provenance must stay unreviewed; only the log carries the acceptance"
    );

    let after = evaluate_native_case(&space).expect("evaluation after review");
    assert!(
        !after
            .evidence_findings
            .unreviewed_inference_ids
            .contains(&id("evidence:log-accepted-inference")),
        "an accepted-by-review inferred claim must stop reading as unreviewed"
    );
    assert!(after
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(!after.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedInference
            && gap.target_id == id("evidence:log-accepted-inference")
    }));
    assert_eq!(after.assurance, NativeAssurance::Accepted);
}

#[test]
fn canonical_review_reject_produces_rejected_evidence_boundary_violation() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:log-rejected-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let review = crate::native_review::reject_review_morphism(
        &space,
        crate::native_review::NativeReviewRequest {
            target_kind: crate::native_review::NativeReviewTargetKind::Evidence,
            target_id: id("evidence:log-rejected-inference"),
            action: ReviewAction::Reject,
            reviewer_id: id("reviewer:native-eval-test"),
            reviewed_at: "2026-04-26T00:30:00Z".to_owned(),
            reason: "Reviewed and rejected during the evidence-findings divergence test."
                .to_owned(),
            evidence_ids: vec![id("evidence:log-rejected-inference")],
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:log-rejected-review"),
        },
    )
    .expect("canonical reject");
    append_review_entry(&mut space, review, "entry:log-rejected-review");

    let after = evaluate_native_case(&space).expect("evaluation after rejection");
    assert!(after
        .evidence_findings
        .boundary_violations
        .iter()
        .any(|violation| violation.violation_type
            == NativeEvidenceBoundaryViolationType::RejectedEvidenceUsed
            && violation.evidence_id == id("evidence:log-rejected-inference")));
    assert_eq!(after.assurance, NativeAssurance::Rejected);
}

#[test]
fn evidence_without_explicit_boundary_does_not_satisfy_hard_requirement() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-explicit-boundary",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:document-without-boundary",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Document, ReviewStatus::Accepted);
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-explicit-boundary",
        CaseRelationType::RequiresEvidence,
        "work:needs-explicit-boundary",
        "evidence:document-without-boundary",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-explicit-boundary")));
    assert!(evaluation
        .evidence_findings
        .inference_record_ids
        .contains(&id("evidence:document-without-boundary")));
}

#[test]
fn review_promoted_evidence_requires_accepted_review() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-promoted-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:pending-promotion",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Human, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("review_promoted".to_owned()),
    );
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-promoted-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs-promoted-evidence",
        "evidence:pending-promotion",
    ));
    refresh_morphism(&mut space);

    let pending = evaluate_native_case(&space).expect("pending evaluation");
    assert!(pending
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-promoted-evidence")));

    let promoted = space
        .case_cells
        .iter_mut()
        .find(|cell| cell.id == id("evidence:pending-promotion"))
        .expect("promoted evidence");
    promoted.provenance = provenance(SourceKind::Human, ReviewStatus::Accepted);
    let accepted = evaluate_native_case(&space).expect("accepted evaluation");
    assert!(accepted
        .readiness
        .ready_cell_ids
        .contains(&id("work:needs-promoted-evidence")));
}

#[test]
fn caller_declared_native_review_promotion_is_not_acceptable_without_review() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:caller-promoted",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Human, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("review_promoted".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);
    let evidence = space
        .case_cells
        .iter()
        .find(|cell| cell.id == id("evidence:caller-promoted"))
        .expect("caller-promoted evidence");

    assert!(!crate::evidence_trust::evidence_is_acceptable(
        evidence_trust_input(&space, evidence)
    ));
}

#[test]
fn projection_loss_and_evolution_summaries_are_reported() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:kept",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.projections.push(Projection {
        projection_id: id("projection:lossy"),
        audience: ProjectionAudience::AiAgent,
        revision_id: space.revision.revision_id.clone(),
        represented_cell_ids: Vec::new(),
        represented_relation_ids: Vec::new(),
        omitted_cell_ids: vec![id("work:kept")],
        omitted_relation_ids: Vec::new(),
        information_loss: vec![crate::native_model::ProjectionLoss {
            description: "AI projection hides work cell.".to_owned(),
            represented_ids: Vec::new(),
            omitted_ids: vec![id("work:kept")],
        }],
        allowed_operations: Vec::new(),
        source_ids: Vec::new(),
        warnings: vec![crate::native_model::ProjectionWarning::InformationLoss],
        metadata: Map::new(),
    });
    refresh_morphism(&mut space);
    space.morphism_log[0].morphism.violated_invariant_ids = vec![id("invariant:loss-disclosed")];

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.projection_loss.len(), 1);
    assert_eq!(evaluation.evolution.invariant_breaks.len(), 1);
}

#[test]
fn projection_revision_and_loss_references_are_validated() {
    let mut space = fixture_space();
    space.projections.push(Projection {
        projection_id: id("projection:stale"),
        audience: ProjectionAudience::AiAgent,
        revision_id: id("revision:missing"),
        represented_cell_ids: Vec::new(),
        represented_relation_ids: Vec::new(),
        omitted_cell_ids: Vec::new(),
        omitted_relation_ids: Vec::new(),
        information_loss: vec![crate::native_model::ProjectionLoss {
            description: "Stale projection references missing loss ids.".to_owned(),
            represented_ids: Vec::new(),
            omitted_ids: vec![id("work:missing")],
        }],
        allowed_operations: Vec::new(),
        source_ids: Vec::new(),
        warnings: Vec::new(),
        metadata: Map::new(),
    });
    refresh_morphism(&mut space);

    let err = evaluate_native_case(&space).expect_err("invalid projection references");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::DanglingReference
            && violation.field == "projection.revision_id"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::DanglingReference
            && violation.field == "projection.information_loss.ids"
    }));
}

#[test]
fn close_check_is_blocked_by_obstructions_and_review_gaps() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut placeholder = cell(
        "evidence:placeholder",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    placeholder.source_ids.clear();
    space.case_cells.push(placeholder);
    space.case_relations.push(relation(
        "relation:needs-placeholder",
        CaseRelationType::RequiresEvidence,
        "work:needs-evidence",
        "evidence:placeholder",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(!evaluation.close_check.closable);
    assert!(evaluation
        .close_check
        .invariant_results
        .iter()
        .any(|result| !result.passed));
}

#[test]
fn invalid_morphism_is_structured_error() {
    let mut space = fixture_space();
    space.morphism_log[0].morphism_id = id("morphism:outer");
    space.morphism_log[0].morphism.morphism_id = id("morphism:inner");

    let err = evaluate_native_case(&space).expect_err("invalid morphism");

    assert!(err
        .violations
        .iter()
        .any(|violation| violation.code == NativeEvalViolationCode::InvalidMorphism));
}

#[test]
fn empty_morphism_log_is_structured_error() {
    let mut space = fixture_space();
    space.morphism_log.clear();

    let err = evaluate_native_case(&space).expect_err("empty morphism log");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "morphism_log"
    }));
}

#[test]
fn invalid_log_continuity_and_entry_version_are_structured_errors() {
    let mut space = fixture_space();
    space.morphism_log[0].schema_version = 2;
    space.morphism_log[0].source_revision_id = Some(id("revision:unexpected-parent"));
    space.morphism_log[0].morphism.source_revision_id = Some(id("revision:unexpected-parent"));

    let err = evaluate_native_case(&space).expect_err("invalid log contract");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::UnsupportedSchemaVersion
            && violation.field == "schema_version"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "source_revision_id"
    }));
}

#[test]
fn materialized_revision_must_match_latest_log_checksum_and_case_space() {
    let mut space = fixture_space();
    space.revision.case_space_id = id("case_space:other");
    space.revision.checksum = "sha256:stale".to_owned();

    let err = evaluate_native_case(&space).expect_err("invalid revision materialization");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "revision.case_space_id"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "revision.checksum"
    }));
}

#[test]
fn case_space_source_boundary_is_required() {
    let mut space = fixture_space();
    space.metadata.remove("source_boundary");

    let err = evaluate_native_case(&space).expect_err("missing source boundary");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "metadata.source_boundary"
    }));
}

#[test]
fn genesis_morphism_must_preserve_lift_boundary() {
    let mut space = fixture_space();
    space.morphism_log[0].morphism.metadata.clear();

    let err = evaluate_native_case(&space).expect_err("missing lift boundary");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "morphism.metadata.lift_semantics"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "morphism.metadata.source_boundary"
    }));
}

#[test]
fn blocked_work_and_review_gap_are_both_reported() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:blocked",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:dependency",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:blocked-depends-on-dependency",
        CaseRelationType::DependsOn,
        "work:blocked",
        "work:dependency",
    ));
    let mut inference = cell(
        "evidence:unreviewed-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    inference.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    inference.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(inference);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Blocked);
    assert_eq!(evaluation.assurance, NativeAssurance::ReviewRequired);
}

#[test]
fn resolved_space_reports_complete_progress_and_pending_assurance() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:done",
        CaseCellType::Work,
        CaseCellLifecycle::Resolved,
    ));
    let mut inference = cell(
        "evidence:unreviewed-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    inference.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    inference.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(inference);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Complete);
    assert_eq!(evaluation.assurance, NativeAssurance::ReviewRequired);
}

#[test]
fn rejected_evidence_drives_assurance_to_rejected() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:open",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut rejected = cell(
        "evidence:rejected",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    rejected.provenance = provenance(SourceKind::Human, ReviewStatus::Rejected);
    rejected.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("rejected".to_owned()),
    );
    space.case_cells.push(rejected);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.assurance, NativeAssurance::Rejected);
}

#[test]
fn clean_accepted_evidence_reports_accepted_assurance() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:open",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:accepted",
        CaseCellType::Evidence,
        CaseCellLifecycle::Accepted,
    );
    evidence.provenance = provenance(SourceKind::Document, ReviewStatus::Accepted);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("source_backed".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Active);
    assert_eq!(evaluation.assurance, NativeAssurance::Accepted);
}

#[test]
fn empty_space_is_active_and_unreviewed() {
    let space = fixture_space();

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Active);
    assert_eq!(evaluation.assurance, NativeAssurance::Unreviewed);
}

fn fixture_space() -> CaseSpace {
    let source_boundary = source_boundary_metadata();
    let revision = Revision {
        revision_id: id("revision:native-fixture-v1"),
        case_space_id: id("case_space:native-fixture"),
        applied_entry_ids: vec![id("morphism_log_entry:genesis")],
        applied_morphism_ids: vec![id("morphism:create-fixture")],
        checksum: "sha256:fixture".to_owned(),
        parent_revision_id: None,
        created_at: "2026-04-26T00:00:00Z".to_owned(),
        source_ids: vec![id("source:test")],
        metadata: Map::new(),
    };
    let mut morphism_metadata = Map::new();
    morphism_metadata.insert("lift_semantics".to_owned(), json!("fixture_to_case_space"));
    morphism_metadata.insert(
        "source_boundary_id".to_owned(),
        json!("source_boundary:native-fixture"),
    );
    morphism_metadata.insert("source_boundary".to_owned(), source_boundary.clone());
    let morphism = CaseMorphism {
        morphism_id: id("morphism:create-fixture"),
        morphism_type: CaseMorphismType::Create,
        source_revision_id: None,
        target_revision_id: revision.revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        metadata: morphism_metadata,
    };
    let mut metadata = Map::new();
    metadata.insert("source_boundary".to_owned(), source_boundary);
    CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: id("case_space:native-fixture"),
        space_id: id("space:native-fixture"),
        case_cells: Vec::new(),
        case_relations: Vec::new(),
        morphism_log: vec![MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: 1,
            case_space_id: id("case_space:native-fixture"),
            sequence: 1,
            entry_id: id("morphism_log_entry:genesis"),
            morphism_id: id("morphism:create-fixture"),
            source_revision_id: None,
            target_revision_id: revision.revision_id.clone(),
            morphism,
            actor_id: id("actor:test"),
            recorded_at: "2026-04-26T00:00:00Z".to_owned(),
            provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
            source_ids: vec![id("source:test")],
            previous_entry_hash: None,
            replay_checksum: "sha256:fixture".to_owned(),
        }],
        projections: Vec::new(),
        revision,
        close_policy_id: Some(id("close_policy:native-default")),
        metadata,
    }
}

fn source_boundary_metadata() -> Value {
    json!({
        "id": "source_boundary:native-fixture",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["native.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture makes no inferred claims",
        "information_loss": []
    })
}

fn refresh_morphism(space: &mut CaseSpace) {
    space.morphism_log[0].morphism.added_ids = space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .collect();
}

/// Appends a canonical review morphism the way an appended log entry actually
/// looks, so evaluation sees the same structural log
/// `evidence_findings`/`latest_evidence_review_statuses` must read, not a
/// synthetic shortcut.
fn append_review_entry(space: &mut CaseSpace, morphism: CaseMorphism, entry_id: &str) {
    let previous_revision_id = space.revision.revision_id.clone();
    let target_revision_id = morphism.target_revision_id.clone();
    let previous_entry_hash = space
        .morphism_log
        .last()
        .map(crate::native_hash::morphism_log_entry_hash)
        .transpose()
        .expect("previous entry hash");
    space.morphism_log.push(MorphismLogEntry {
        schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
        schema_version: 1,
        case_space_id: space.case_space_id.clone(),
        sequence: space.morphism_log.len() as u64 + 1,
        entry_id: id(entry_id),
        morphism_id: morphism.morphism_id.clone(),
        source_revision_id: Some(previous_revision_id.clone()),
        target_revision_id: target_revision_id.clone(),
        morphism,
        actor_id: id("actor:reviewer"),
        recorded_at: "2026-04-26T00:30:00Z".to_owned(),
        provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
        source_ids: vec![id("source:test")],
        previous_entry_hash,
        replay_checksum: "fixture-review".to_owned(),
    });
    space.revision.revision_id = target_revision_id;
    space.revision.parent_revision_id = Some(previous_revision_id);
    space.revision.checksum = "fixture-review".to_owned();
    for projection in &mut space.projections {
        projection.revision_id = space.revision.revision_id.clone();
    }
}

fn cell(id_value: &str, cell_type: CaseCellType, lifecycle: CaseCellLifecycle) -> CaseCell {
    CaseCell {
        id: id(id_value),
        cell_type,
        space_id: id("space:native-fixture"),
        title: id_value.to_owned(),
        summary: None,
        lifecycle,
        source_ids: vec![id("source:test")],
        structure_ids: Vec::new(),
        provenance: provenance(SourceKind::Human, ReviewStatus::Reviewed),
        metadata: Map::new(),
    }
}

fn relation(
    id_value: &str,
    relation_type: CaseRelationType,
    from_id: &str,
    to_id: &str,
) -> CaseRelation {
    CaseRelation {
        id: id(id_value),
        relation_type,
        relation_strength: RelationStrength::Hard,
        from_id: id(from_id),
        to_id: id(to_id),
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        provenance: provenance(SourceKind::Human, ReviewStatus::Reviewed),
        metadata: Map::new(),
    }
}

fn provenance(kind: SourceKind, review_status: ReviewStatus) -> Provenance {
    Provenance::new(SourceRef::new(kind), confidence(1.0)).with_review_status(review_status)
}
