use super::*;

#[test]
fn generalized_operation_gate_accepts_a_matching_plan_review_gate() {
    let space = fixture_space();
    let gate = NativeOperationGate {
        actor_id: id("actor:plan-review"),
        operation: "plan-review".to_owned(),
        operation_scope_id: space.case_space_id.clone(),
        audience: ProjectionAudience::System,
        capability_ids: vec![id("capability:plan-review")],
        source_boundary_id: id("source_boundary:review-fixture"),
    };

    check_operation_gate(&space, &gate, "plan-review").expect("matching operation gate");
}

#[test]
fn generalized_operation_gate_rejects_each_rule_violation() {
    let space = fixture_space();
    let valid = NativeOperationGate {
        actor_id: id("actor:plan-review"),
        operation: "plan-review".to_owned(),
        operation_scope_id: space.case_space_id.clone(),
        audience: ProjectionAudience::Audit,
        capability_ids: vec![id("capability:plan-review")],
        source_boundary_id: id("source_boundary:review-fixture"),
    };
    let cases = [
        (
            "operation",
            NativeOperationGate {
                operation: "close-check".to_owned(),
                ..valid.clone()
            },
        ),
        (
            "operation_scope_id",
            NativeOperationGate {
                operation_scope_id: id("case_space:other"),
                ..valid.clone()
            },
        ),
        (
            "audience",
            NativeOperationGate {
                audience: ProjectionAudience::AiAgent,
                ..valid.clone()
            },
        ),
        (
            "capability_ids",
            NativeOperationGate {
                capability_ids: Vec::new(),
                ..valid.clone()
            },
        ),
        (
            "source_boundary_id",
            NativeOperationGate {
                source_boundary_id: id("source_boundary:other"),
                ..valid
            },
        ),
    ];

    for (expected_violation, gate) in cases {
        let error =
            check_operation_gate(&space, &gate, "plan-review").expect_err(expected_violation);
        assert!(
            error.to_string().contains(expected_violation),
            "{expected_violation}: {error}"
        );
    }
}

#[test]
fn generalized_operation_gate_rejects_fabricated_capability_id() {
    let space = fixture_space();
    let gate = NativeOperationGate {
        actor_id: id("actor:plan-review"),
        operation: "plan-review".to_owned(),
        operation_scope_id: space.case_space_id.clone(),
        audience: ProjectionAudience::Audit,
        capability_ids: vec![id("capability:fabricated")],
        source_boundary_id: id("source_boundary:review-fixture"),
    };

    let error =
        check_operation_gate(&space, &gate, "plan-review").expect_err("fabricated capability");

    assert!(error.to_string().contains("existing case cell id"));
}

#[test]
fn close_requires_validation_evidence_to_name_existing_evidence() {
    let space = fixture_space();
    let close = check_native_close(
        &space,
        NativeCloseCheckRequest {
            validation_evidence_ids: vec![id("evidence:missing")],
            ..close_request()
        },
    )
    .expect("close check");

    assert!(!close.closeable);
    assert!(close.blocker_ids.contains(&id("evidence:missing")));
}

#[test]
fn close_reports_source_boundary_and_policy_capability_gate() {
    let space = fixture_space();
    let close = check_native_close(&space, close_request()).expect("close check");

    assert!(close
        .invariant_results
        .iter()
        .any(
            |result| result.invariant_id == id("close:native-source-boundary-declared")
                && result.passed
        ));

    let mut no_policy_space = space.clone();
    no_policy_space.close_policy_id = None;
    let blocked = check_native_close(
        &no_policy_space,
        NativeCloseCheckRequest {
            close_policy_id: None,
            source_ids: Vec::new(),
            ..close_request_for(&no_policy_space)
        },
    )
    .expect("close check");

    let gate = blocked
        .invariant_results
        .iter()
        .find(|result| result.invariant_id == id("close:native-policy-capability-gate"))
        .expect("policy/capability gate invariant");
    assert!(!gate.passed);
    assert!(gate.witness_ids.contains(&id("case_space:review-fixture")));
    assert!(gate.witness_ids.contains(&id("revision:fixture")));
}

#[test]
fn close_operation_gate_must_match_scope_audience_and_source_boundary() {
    let space = fixture_space();
    let mut request = close_request();
    request.operation_gate = Some(NativeOperationGate {
        actor_id: id("actor:native-review-test"),
        operation: "close".to_owned(),
        operation_scope_id: id("case_space:other"),
        audience: ProjectionAudience::AiAgent,
        capability_ids: Vec::new(),
        source_boundary_id: id("source_boundary:other"),
    });

    let blocked = check_native_close(&space, request).expect("close check");

    let gate = blocked
        .invariant_results
        .iter()
        .find(|result| result.invariant_id == id("close:native-policy-capability-gate"))
        .expect("policy/capability gate invariant");
    assert!(!gate.passed);
    assert!(gate.witness_ids.contains(&id("case_space:other")));
    assert!(gate.witness_ids.contains(&id("source_boundary:other")));
}

#[test]
fn close_evidence_requirement_requires_evidence_cell() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-non-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    space.case_cells.push(cell(
        "case:not-evidence",
        CaseCellType::Case,
        CaseCellLifecycle::Accepted,
        SourceKind::Document,
        ReviewStatus::Accepted,
    ));
    space.case_cells.push(cell(
        "evidence:validation",
        CaseCellType::Evidence,
        CaseCellLifecycle::Accepted,
        SourceKind::Document,
        ReviewStatus::Accepted,
    ));
    space.case_relations.push(relation(
        "relation:needs-non-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs-non-evidence",
        "case:not-evidence",
    ));
    refresh_added_ids(&mut space);

    let close = check_native_close(
        &space,
        NativeCloseCheckRequest {
            validation_evidence_ids: vec![id("evidence:validation")],
            ..close_request()
        },
    )
    .expect("close check");

    assert!(!close.closeable);
    assert!(close.blocker_ids.contains(&id("case:not-evidence")));
}

#[test]
fn close_blocks_before_review_and_closes_after_reviews_and_declarations() {
    let mut space = fixture_space_with_completion();

    let blocked = check_native_close(&space, close_request()).expect("close check");
    assert!(!blocked.closeable);
    assert!(blocked
        .blocker_ids
        .contains(&id("completion:source-backed-evidence")));
    assert!(blocked.blocker_ids.contains(&id("morphism:generated")));
    assert!(blocked.blocker_ids.contains(&id("projection:lossy")));

    let completion_review = defer_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Completion,
            "completion:source-backed-evidence",
            ReviewAction::Defer,
            "revision:completion-deferred",
        ),
    )
    .expect("defer completion");
    append_review_for_test(&mut space, completion_review, "entry:completion-deferred");
    let morphism_review = accept_review_morphism(
        &space,
        request_for_space(
            &space,
            NativeReviewTargetKind::Morphism,
            "morphism:generated",
            ReviewAction::Accept,
            "revision:morphism-reviewed",
        ),
    )
    .expect("accept generated morphism");
    append_review_for_test(&mut space, morphism_review, "entry:morphism-reviewed");

    let close = check_native_close(
        &space,
        NativeCloseCheckRequest {
            declared_projection_loss_ids: vec![id("projection:lossy")],
            ..close_request_for(&space)
        },
    )
    .expect("close check");

    assert!(close.closeable);
}
