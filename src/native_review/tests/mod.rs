use super::*;
use crate::{
    native_eval::{evaluate_native_case, NativeReviewGapType},
    native_model::{
        CaseCell, CaseCellLifecycle, CaseMorphismType, CaseRelation, CaseRelationType,
        MorphismLogEntry, Projection, ProjectionAudience, RelationStrength, Revision,
        NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION,
        NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
    },
};
use higher_graphen_core::{Confidence, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{json, Map};

mod close;

#[test]
fn builds_review_morphisms_for_all_explicit_outcomes() {
    let space = fixture_space_with_completion();

    let accepted = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Completion,
            "completion:source-backed-evidence",
            ReviewAction::Reject,
            "revision:accept",
        ),
    )
    .expect("accept completion");
    let rejected = reject_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Evidence,
            "evidence:source-backed",
            ReviewAction::Accept,
            "revision:reject",
        ),
    )
    .expect("reject evidence");
    let reopened = reopen_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Morphism,
            "morphism:generated",
            ReviewAction::Accept,
            "revision:reopen",
        ),
    )
    .expect("reopen morphism");
    let deferred = defer_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Waiver,
            "relation:needs-evidence",
            ReviewAction::Accept,
            "revision:defer",
        ),
    )
    .expect("defer waiver");

    assert_eq!(accepted.morphism_type, CaseMorphismType::Review);
    assert_eq!(accepted.metadata["action"], json!("accept"));
    assert_eq!(
        rejected.metadata["outcome_review_status"],
        json!("rejected")
    );
    assert_eq!(
        reopened.metadata["outcome_review_status"],
        json!("unreviewed")
    );
    assert_eq!(deferred.metadata["action"], json!("defer"));
    assert_eq!(accepted.added_ids, Vec::<Id>::new());
}

#[test]
fn invalid_review_target_is_rejected() {
    let space = fixture_space_with_completion();
    let err = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Evidence,
            "evidence:not-present",
            ReviewAction::Accept,
            "revision:bad",
        ),
    )
    .expect_err("invalid target");

    assert!(err.message.contains("unknown evidence target"));
}

#[test]
fn execution_topology_review_is_content_bound_and_generic_path_is_refused() {
    let mut space = fixture_space();
    let (topology_bytes, policy_manifest_bytes, target) = topology_review_target(&space);
    let artifact_id = target.artifact_id.to_string();
    let artifact_hash = artifact_id.trim_start_matches("artifact:sha256-");
    let mut claim = cell(
        "evidence:execution-topology",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Document,
        ReviewStatus::Unreviewed,
    );
    claim.metadata.extend([
        ("topology_id".to_owned(), json!("topology:review-fixture")),
        (
            "execution_topology_content_hash".to_owned(),
            json!(target.topology_content_hash),
        ),
        ("artifact_id".to_owned(), json!(artifact_id)),
        (
            "policy_manifest_content_hash".to_owned(),
            json!(target.policy_manifest_content_hash),
        ),
        (
            "case_space_id".to_owned(),
            json!(space.case_space_id.clone()),
        ),
    ]);
    let mut artifact = cell(
        &artifact_id,
        CaseCellType::Custom("artifact".to_owned()),
        CaseCellLifecycle::Resolved,
        SourceKind::Custom("tool_captured_artifact".to_owned()),
        ReviewStatus::Unreviewed,
    );
    artifact
        .metadata
        .insert("content_hash".to_owned(), json!(artifact_hash));
    space.case_cells.extend([claim, artifact]);
    space.case_relations.push(relation(
        "relation:execution-topology-artifact",
        CaseRelationType::DerivesFrom,
        "evidence:execution-topology",
        &artifact_id,
    ));

    let morphism = execution_topology_review_morphism(
        &space,
        ExecutionTopologyReviewRequest {
            target: target.clone(),
            action: ReviewAction::Accept,
            reviewer_id: id("reviewer:native"),
            reviewed_at: "2026-08-03T00:00:00Z".to_owned(),
            reason: "Exact topology bytes reviewed.".to_owned(),
            evidence_ids: Vec::new(),
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:execution-topology-reviewed"),
        },
        &topology_bytes,
        &policy_manifest_bytes,
    )
    .expect("dedicated topology review");
    let canonical = canonical_review(&morphism).expect("canonical topology review");
    assert_eq!(
        canonical.target_kind,
        NativeReviewTargetKind::ExecutionTopology
    );
    assert_eq!(canonical.execution_topology, Some(target));
    let advisories = morphism.metadata["execution_topology_review_advisories"]
        .as_array()
        .expect("review advisories are an array");
    assert!(!advisories.is_empty(), "fixture carries reviewer advice");
    assert!(advisories.iter().all(|finding| {
        finding.get("classification").and_then(Value::as_str) == Some("heuristic")
    }));

    let error = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::ExecutionTopology,
            "evidence:execution-topology",
            ReviewAction::Accept,
            "revision:generic-topology-review",
        ),
    )
    .expect_err("generic review cannot mint topology authority");
    assert!(error.message.contains("dedicated content-bound review API"));
}

#[test]
fn execution_topology_review_refuses_stale_revision_and_changed_binding() {
    let mut space = fixture_space();
    let (topology_bytes, policy_manifest_bytes, mut target) = topology_review_target(&space);
    let artifact_id = target.artifact_id.to_string();
    let artifact_hash = artifact_id.trim_start_matches("artifact:sha256-");
    let mut claim = cell(
        "evidence:execution-topology",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Document,
        ReviewStatus::Unreviewed,
    );
    claim.metadata.extend([
        ("topology_id".to_owned(), json!("topology:review-fixture")),
        (
            "execution_topology_content_hash".to_owned(),
            json!(target.topology_content_hash),
        ),
        ("artifact_id".to_owned(), json!(artifact_id)),
        (
            "policy_manifest_content_hash".to_owned(),
            json!(target.policy_manifest_content_hash),
        ),
        (
            "case_space_id".to_owned(),
            json!(space.case_space_id.clone()),
        ),
    ]);
    let mut artifact = cell(
        &artifact_id,
        CaseCellType::Custom("artifact".to_owned()),
        CaseCellLifecycle::Resolved,
        SourceKind::Custom("tool_captured_artifact".to_owned()),
        ReviewStatus::Unreviewed,
    );
    artifact
        .metadata
        .insert("content_hash".to_owned(), json!(artifact_hash));
    space.case_cells.extend([claim, artifact]);
    space.case_relations.push(relation(
        "relation:execution-topology-artifact",
        CaseRelationType::DerivesFrom,
        "evidence:execution-topology",
        &artifact_id,
    ));
    let request = ExecutionTopologyReviewRequest {
        target: {
            target.observed_base_revision_id = id("revision:stale");
            target
        },
        action: ReviewAction::Accept,
        reviewer_id: id("reviewer:native"),
        reviewed_at: "2026-08-03T00:00:00Z".to_owned(),
        reason: "Changed topology must refuse.".to_owned(),
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        target_revision_id: id("revision:execution-topology-reviewed"),
    };
    let error = execution_topology_review_morphism(
        &space,
        request,
        &topology_bytes,
        &policy_manifest_bytes,
    )
    .expect_err("stale review target refuses");
    assert!(error.message.contains("observed revision is stale"));
}

#[test]
fn execution_topology_review_refuses_policy_manifest_substitution() {
    let mut space = fixture_space();
    let (topology_bytes, policy_manifest_bytes, target) = topology_review_target(&space);
    install_topology_review_target(&mut space, &target);
    let mut substituted: Value = serde_json::from_slice(&policy_manifest_bytes).unwrap();
    substituted["budget_policies"][0]["content_hash"] = json!("f".repeat(64));
    let substituted = serde_json::to_vec(&substituted).unwrap();
    let error = execution_topology_review_morphism(
        &space,
        ExecutionTopologyReviewRequest {
            target,
            action: ReviewAction::Accept,
            reviewer_id: id("reviewer:native"),
            reviewed_at: "2026-08-03T00:00:00Z".to_owned(),
            reason: "Substitution must refuse.".to_owned(),
            evidence_ids: Vec::new(),
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:manifest-substitution"),
        },
        &topology_bytes,
        &substituted,
    )
    .expect_err("changed policy manifest bytes are not reviewed authority");
    assert!(error
        .message
        .contains("policy manifest bytes do not match the review target"));
}

fn topology_review_target(space: &CaseSpace) -> (Vec<u8>, Vec<u8>, ExecutionTopologyReviewTarget) {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .expect("topology fixture");
    value["topology_id"] = json!("topology:review-fixture");
    value["case_space_id"] = json!(space.case_space_id.clone());
    topology_review_target_from_value(space, value)
}

fn topology_review_target_from_value(
    space: &CaseSpace,
    value: Value,
) -> (Vec<u8>, Vec<u8>, ExecutionTopologyReviewTarget) {
    let bytes = serde_json::to_vec(&value).expect("topology fixture bytes");
    let topology: crate::execution_topology::ExecutionTopology =
        serde_json::from_slice(&bytes).expect("typed topology fixture");
    let topology_content_hash =
        crate::execution_topology::execution_topology_content_hash(&topology)
            .expect("topology fixture hash");
    let artifact_hash = crate::native_hash::sha256_hex(&bytes);
    let verification_policies = topology
        .verification_policy_ids
        .iter()
        .map(|id| (id.clone(), json!({"verification_policy_id": id})))
        .collect();
    let budget_policies = topology
        .budget_policy_ids
        .iter()
        .map(|id| (id.clone(), json!({"policy_id": id})))
        .collect();
    let expansion_policies = BTreeMap::new();
    let policy_manifest = crate::deployment_policy::deployment_policy_manifest(
        &topology,
        &topology_content_hash,
        &verification_policies,
        &budget_policies,
        &expansion_policies,
    );
    let policy_manifest_bytes = serde_json::to_vec(&policy_manifest).unwrap();
    let policy_manifest_content_hash =
        crate::deployment_policy::deployment_policy_manifest_content_hash(&policy_manifest)
            .unwrap();
    (
        bytes,
        policy_manifest_bytes,
        ExecutionTopologyReviewTarget {
            topology_id: id("topology:review-fixture"),
            topology_content_hash,
            case_space_id: space.case_space_id.clone(),
            observed_base_revision_id: space.revision.revision_id.clone(),
            claim_cell_id: id("evidence:execution-topology"),
            artifact_id: id(&format!("artifact:sha256-{artifact_hash}")),
            policy_manifest_content_hash,
            expansion_proposal_id: None,
        },
    )
}

fn install_topology_review_target(space: &mut CaseSpace, target: &ExecutionTopologyReviewTarget) {
    let artifact_id = target.artifact_id.to_string();
    let artifact_hash = artifact_id.trim_start_matches("artifact:sha256-");
    let mut claim = cell(
        target.claim_cell_id.as_str(),
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Document,
        ReviewStatus::Unreviewed,
    );
    claim.metadata.extend([
        ("topology_id".to_owned(), json!(target.topology_id)),
        (
            "execution_topology_content_hash".to_owned(),
            json!(target.topology_content_hash),
        ),
        ("artifact_id".to_owned(), json!(artifact_id)),
        ("case_space_id".to_owned(), json!(target.case_space_id)),
        (
            "policy_manifest_content_hash".to_owned(),
            json!(target.policy_manifest_content_hash),
        ),
    ]);
    let mut artifact = cell(
        &artifact_id,
        CaseCellType::Custom("artifact".to_owned()),
        CaseCellLifecycle::Resolved,
        SourceKind::Custom("tool_captured_artifact".to_owned()),
        ReviewStatus::Unreviewed,
    );
    artifact
        .metadata
        .insert("content_hash".to_owned(), json!(artifact_hash));
    space.case_cells.extend([claim, artifact]);
    space.case_relations.push(relation(
        "relation:execution-topology-artifact",
        CaseRelationType::DerivesFrom,
        target.claim_cell_id.as_str(),
        &artifact_id,
    ));
}

#[test]
fn execution_topology_accept_uses_canonical_semantic_validation_but_reject_remains_auditable() {
    let base: Value = serde_json::from_str(include_str!(
        "../../../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .expect("topology fixture");
    let mut cases = Vec::new();

    let mut unknown_node = base.clone();
    unknown_node["edges"][0]["to"] = json!("node:missing");
    cases.push((unknown_node, "unknown_edge_target", "$.edges[0].to"));

    let mut invalid_binding = base.clone();
    invalid_binding["edges"][0]["output"] = json!("missing_output");
    cases.push((
        invalid_binding,
        "unknown_output_binding",
        "$.edges[0].output",
    ));

    let mut self_edge = base.clone();
    let self_source = self_edge["edges"][0]["from"].clone();
    self_edge["edges"][0]["to"] = self_source;
    cases.push((self_edge, "self_edge", "$.edges[0]"));

    let mut resource_mismatch = base.clone();
    resource_mismatch["edges"][3]["kind"] = json!("resource_exclusion");
    resource_mismatch["edges"][3]["resource_scope"] = json!(["file:not-shared"]);
    cases.push((
        resource_mismatch,
        "unknown_resource_scope",
        "$.edges[3].resource_scope",
    ));

    let mut policy_mismatch = base;
    policy_mismatch["nodes"][0]["verification_policy_id"] = json!("verification:missing");
    cases.push((
        policy_mismatch,
        "unknown_policy_reference",
        "$.nodes[0].verification_policy_id",
    ));

    for (mut value, code, location) in cases {
        let mut space = fixture_space();
        value["topology_id"] = json!("topology:review-fixture");
        value["case_space_id"] = json!(space.case_space_id.clone());
        let (topology_bytes, manifest_bytes, target) =
            topology_review_target_from_value(&space, value);
        install_topology_review_target(&mut space, &target);
        let request = |action| ExecutionTopologyReviewRequest {
            target: target.clone(),
            action,
            reviewer_id: id("reviewer:native"),
            reviewed_at: "2026-08-03T00:00:00Z".to_owned(),
            reason: "Record semantic disposition.".to_owned(),
            evidence_ids: Vec::new(),
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:semantic-disposition"),
        };
        let error = execution_topology_review_morphism(
            &space,
            request(ReviewAction::Accept),
            &topology_bytes,
            &manifest_bytes,
        )
        .expect_err("semantic invalidity prevents acceptance");
        assert!(error.message.contains(code), "{}", error.message);
        assert!(error.message.contains(location), "{}", error.message);

        execution_topology_review_morphism(
            &space,
            request(ReviewAction::Reject),
            &topology_bytes,
            &manifest_bytes,
        )
        .expect("invalid proposal can still be rejected audibly");
    }
}

#[test]
fn generated_completion_review_does_not_preserve_virtual_target_id() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-generated-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    space.case_cells.push(cell(
        "evidence:generated-placeholder",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    space
        .case_cells
        .last_mut()
        .expect("placeholder evidence")
        .source_ids
        .clear();
    space.case_relations.push(relation(
        "relation:needs-generated-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs-generated-evidence",
        "evidence:generated-placeholder",
    ));
    refresh_added_ids(&mut space);
    let candidate_id = evaluate_native_case(&space)
        .expect("evaluation")
        .completion_candidates
        .into_iter()
        .find(|candidate| {
            candidate
                .target_ids
                .contains(&id("work:needs-generated-evidence"))
        })
        .expect("generated completion")
        .id;

    let review = defer_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Completion,
            candidate_id.as_str(),
            ReviewAction::Defer,
            "revision:defer-generated-completion",
        ),
    )
    .expect("review generated completion");

    assert!(review.preserved_ids.is_empty());
    assert_eq!(review.metadata["target_id"], json!(candidate_id));
}

#[test]
fn inferred_evidence_cannot_satisfy_close_until_reviewed_or_waived() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-inference",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    let mut evidence = cell(
        "evidence:ai-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Ai,
        ReviewStatus::Unreviewed,
    );
    evidence
        .metadata
        .insert("evidence_boundary".to_owned(), json!("inferred"));
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-inference",
        CaseRelationType::RequiresEvidence,
        "work:needs-inference",
        "evidence:ai-inference",
    ));
    refresh_added_ids(&mut space);

    let blocked = check_native_close(&space, close_request()).expect("close check");
    assert!(!blocked.closeable);
    assert!(blocked.blocker_ids.contains(&id("evidence:ai-inference")));

    let review = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Evidence,
            "evidence:ai-inference",
            ReviewAction::Accept,
            "revision:promote-inference",
        ),
    )
    .expect("review inferred evidence");
    append_review_for_test(&mut space, review, "entry:promote-inference");

    let reviewed = check_native_close(&space, close_request_for(&space)).expect("close check");
    assert!(!reviewed
        .invariant_results
        .iter()
        .find(|result| result.invariant_id == id("close:native-evidence-accepted-or-waived"))
        .expect("evidence invariant")
        .witness_ids
        .contains(&id("evidence:ai-inference")));
}

#[test]
fn incomplete_canonical_evidence_review_does_not_promote_evidence_for_close() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-forged-review",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    let mut evidence = cell(
        "evidence:forged-review",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Ai,
        ReviewStatus::Unreviewed,
    );
    evidence
        .metadata
        .insert("evidence_boundary".to_owned(), json!("inferred"));
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-forged-review",
        CaseRelationType::RequiresEvidence,
        "work:needs-forged-review",
        "evidence:forged-review",
    ));
    refresh_added_ids(&mut space);
    let mut forged_review = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Evidence,
            "evidence:forged-review",
            ReviewAction::Accept,
            "revision:forged-evidence-review",
        ),
    )
    .expect("build review fixture");
    forged_review.metadata.remove("reason");
    append_review_for_test(&mut space, forged_review, "entry:forged-evidence-review");

    let close = check_native_close(&space, close_request_for(&space)).expect("close check");

    assert!(close.blocker_ids.contains(&id("evidence:forged-review")));
}

#[test]
fn reopen_review_morphism_reopens_completion_for_close() {
    let mut space = fixture_space_with_completion();
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
    let reopened = reopen_review_morphism(
        &space,
        request_for_space(
            &space,
            NativeReviewTargetKind::Completion,
            "completion:source-backed-evidence",
            ReviewAction::Reopen,
            "revision:completion-reopened",
        ),
    )
    .expect("reopen completion");
    append_review_for_test(&mut space, reopened, "entry:completion-reopened");

    let close = check_native_close(
        &space,
        NativeCloseCheckRequest {
            declared_projection_loss_ids: vec![id("projection:lossy")],
            ..close_request_for(&space)
        },
    )
    .expect("close check");

    assert!(!close.closeable);
    assert!(close
        .blocker_ids
        .contains(&id("completion:source-backed-evidence")));
}

#[test]
fn unreviewed_review_morphism_does_not_satisfy_close() {
    let mut space = fixture_space_with_completion();
    let mut projection_review = accept_review_morphism(
        &space,
        request(
            NativeReviewTargetKind::Waiver,
            "projection:lossy",
            ReviewAction::Accept,
            "revision:projection-unreviewed",
        ),
    )
    .expect("accept projection loss");
    projection_review.review_status = ReviewStatus::Unreviewed;
    append_review_for_test(&mut space, projection_review, "entry:projection-unreviewed");

    let close = check_native_close(&space, close_request_for(&space)).expect("close check");

    assert!(!close.closeable);
    assert!(close.blocker_ids.contains(&id("projection:lossy")));
}

#[test]
fn generated_morphism_remains_reviewable_until_explicit_review() {
    let space = fixture_space_with_completion();
    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedMorphism
            && gap.target_id == id("morphism:generated")
    }));
    assert!(
        !check_native_close(&space, close_request())
            .expect("close check")
            .closeable
    );
}

fn fixture_space_with_completion() -> CaseSpace {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
        SourceKind::Human,
        ReviewStatus::Reviewed,
    ));
    let mut source_backed_evidence = cell(
        "evidence:source-backed",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
        SourceKind::Document,
        ReviewStatus::Reviewed,
    );
    source_backed_evidence
        .metadata
        .insert("evidence_boundary".to_owned(), json!("source_backed"));
    space.case_cells.push(source_backed_evidence);
    space.case_cells.push(cell(
        "completion:source-backed-evidence",
        CaseCellType::Completion,
        CaseCellLifecycle::Proposed,
        SourceKind::Ai,
        ReviewStatus::Unreviewed,
    ));
    space.case_relations.push(relation(
        "relation:needs-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs-evidence",
        "evidence:source-backed",
    ));
    space.projections.push(Projection {
        projection_id: id("projection:lossy"),
        audience: ProjectionAudience::AiAgent,
        revision_id: space.revision.revision_id.clone(),
        represented_cell_ids: vec![id("work:needs-evidence")],
        represented_relation_ids: Vec::new(),
        omitted_cell_ids: vec![id("completion:source-backed-evidence")],
        omitted_relation_ids: Vec::new(),
        information_loss: vec![crate::native_model::ProjectionLoss {
            description: "Completion candidate omitted from AI projection.".to_owned(),
            represented_ids: vec![id("work:needs-evidence")],
            omitted_ids: vec![id("completion:source-backed-evidence")],
        }],
        allowed_operations: Vec::new(),
        source_ids: vec![id("source:test")],
        warnings: vec![crate::native_model::ProjectionWarning::InformationLoss],
        metadata: Map::new(),
    });
    refresh_added_ids(&mut space);
    space.morphism_log.push(generated_morphism(&space, 2));
    space.revision.revision_id = id("revision:generated");
    space.revision.parent_revision_id = Some(id("revision:fixture"));
    space.revision.applied_morphism_ids = vec![id("morphism:generated")];
    space.revision.checksum = "fixture-generated".to_owned();
    space.morphism_log[1].target_revision_id = space.revision.revision_id.clone();
    space.morphism_log[1].morphism.target_revision_id = space.revision.revision_id.clone();
    space.morphism_log[1].source_revision_id = Some(id("revision:fixture"));
    space.morphism_log[1].morphism.source_revision_id = Some(id("revision:fixture"));
    for projection in &mut space.projections {
        projection.revision_id = space.revision.revision_id.clone();
    }
    space
}

fn fixture_space() -> CaseSpace {
    let source_boundary = source_boundary_metadata();
    let revision = Revision {
        revision_id: id("revision:fixture"),
        case_space_id: id("case_space:review-fixture"),
        applied_entry_ids: vec![id("entry:genesis")],
        applied_morphism_ids: vec![id("morphism:genesis")],
        checksum: "fixture".to_owned(),
        parent_revision_id: None,
        created_at: "2026-04-26T00:00:00Z".to_owned(),
        source_ids: vec![id("source:test")],
        metadata: Map::new(),
    };
    let mut morphism_metadata = Map::new();
    morphism_metadata.insert("lift_semantics".to_owned(), json!("fixture_to_case_space"));
    morphism_metadata.insert(
        "source_boundary_id".to_owned(),
        json!("source_boundary:review-fixture"),
    );
    morphism_metadata.insert("source_boundary".to_owned(), source_boundary.clone());
    let morphism = CaseMorphism {
        morphism_id: id("morphism:genesis"),
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
    let mut space = CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: id("case_space:review-fixture"),
        space_id: id("space:review-fixture"),
        case_cells: vec![
            cell(
                "capability:plan-review",
                CaseCellType::Custom("capability".to_owned()),
                CaseCellLifecycle::Accepted,
                SourceKind::Human,
                ReviewStatus::Accepted,
            ),
            cell(
                "capability:native-review-test:close-check",
                CaseCellType::Custom("capability".to_owned()),
                CaseCellLifecycle::Accepted,
                SourceKind::Human,
                ReviewStatus::Accepted,
            ),
        ],
        case_relations: Vec::new(),
        morphism_log: vec![MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: 1,
            case_space_id: id("case_space:review-fixture"),
            sequence: 1,
            entry_id: id("entry:genesis"),
            morphism_id: id("morphism:genesis"),
            source_revision_id: None,
            target_revision_id: revision.revision_id.clone(),
            morphism,
            actor_id: id("actor:test"),
            recorded_at: "2026-04-26T00:00:00Z".to_owned(),
            provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
            source_ids: vec![id("source:test")],
            previous_entry_hash: None,
            replay_checksum: "fixture".to_owned(),
        }],
        projections: Vec::new(),
        revision,
        close_policy_id: Some(id("close_policy:native-default")),
        metadata,
    };
    space.case_cells[0]
        .metadata
        .insert("actor_ids".to_owned(), json!(["actor:plan-review"]));
    space.case_cells[0]
        .metadata
        .insert("operations".to_owned(), json!(["plan-review"]));
    space.case_cells[1]
        .metadata
        .insert("actor_ids".to_owned(), json!(["actor:native-review-test"]));
    space.case_cells[1]
        .metadata
        .insert("operations".to_owned(), json!(["review", "close-check"]));
    refresh_added_ids(&mut space);
    space
}

fn source_boundary_metadata() -> serde_json::Value {
    json!({
        "id": "source_boundary:review-fixture",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["native.review.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture makes no inferred claims",
        "information_loss": []
    })
}

fn generated_morphism(space: &CaseSpace, sequence: u64) -> MorphismLogEntry {
    let morphism = CaseMorphism {
        morphism_id: id("morphism:generated"),
        morphism_type: CaseMorphismType::Review,
        source_revision_id: Some(space.revision.revision_id.clone()),
        target_revision_id: id("revision:generated"),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: vec![id("work:needs-evidence")],
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Unreviewed,
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        metadata: Map::new(),
    };
    MorphismLogEntry {
        schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
        schema_version: 1,
        case_space_id: space.case_space_id.clone(),
        sequence,
        entry_id: id("entry:generated"),
        morphism_id: morphism.morphism_id.clone(),
        source_revision_id: morphism.source_revision_id.clone(),
        target_revision_id: morphism.target_revision_id.clone(),
        morphism,
        actor_id: id("actor:ai"),
        recorded_at: "2026-04-26T00:10:00Z".to_owned(),
        provenance: provenance(SourceKind::Ai, ReviewStatus::Unreviewed),
        source_ids: vec![id("source:test")],
        previous_entry_hash: space
            .morphism_log
            .last()
            .map(crate::native_hash::morphism_log_entry_hash)
            .transpose()
            .expect("previous entry hash"),
        replay_checksum: "fixture-generated".to_owned(),
    }
}

fn append_review_for_test(space: &mut CaseSpace, morphism: CaseMorphism, entry_id: &str) {
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
        recorded_at: "2026-04-26T00:20:00Z".to_owned(),
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

fn refresh_added_ids(space: &mut CaseSpace) {
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

fn cell(
    id_value: &str,
    cell_type: CaseCellType,
    lifecycle: CaseCellLifecycle,
    source_kind: SourceKind,
    review_status: ReviewStatus,
) -> CaseCell {
    CaseCell {
        id: id(id_value),
        cell_type,
        space_id: id("space:review-fixture"),
        title: id_value.to_owned(),
        summary: None,
        lifecycle,
        source_ids: vec![id("source:test")],
        structure_ids: Vec::new(),
        provenance: provenance(source_kind, review_status),
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

fn request(
    target_kind: NativeReviewTargetKind,
    target_id: &str,
    action: ReviewAction,
    target_revision_id: &str,
) -> NativeReviewRequest {
    NativeReviewRequest {
        target_kind,
        target_id: id(target_id),
        action,
        reviewer_id: id("reviewer:native"),
        reviewed_at: "2026-04-26T00:30:00Z".to_owned(),
        reason: "Reviewed during native review API test.".to_owned(),
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        target_revision_id: id(target_revision_id),
    }
}

fn request_for_space(
    space: &CaseSpace,
    target_kind: NativeReviewTargetKind,
    target_id: &str,
    action: ReviewAction,
    target_revision_id: &str,
) -> NativeReviewRequest {
    let request = request(target_kind, target_id, action, target_revision_id);
    assert_ne!(request.target_revision_id, space.revision.revision_id);
    request
}

fn close_request() -> NativeCloseCheckRequest {
    NativeCloseCheckRequest {
        close_policy_id: Some(id("close_policy:native-default")),
        base_revision_id: id("revision:fixture"),
        declared_projection_loss_ids: Vec::new(),
        validation_evidence_ids: vec![id("source:test")],
        source_ids: vec![id("source:test")],
        operation_gate: Some(NativeOperationGate {
            actor_id: id("actor:native-review-test"),
            operation: "close-check".to_owned(),
            operation_scope_id: id("case_space:review-fixture"),
            audience: ProjectionAudience::Audit,
            capability_ids: vec![id("capability:native-review-test:close-check")],
            source_boundary_id: id("source_boundary:review-fixture"),
        }),
    }
}

fn close_request_for(space: &CaseSpace) -> NativeCloseCheckRequest {
    let validation_evidence_ids = space
        .case_cells
        .iter()
        .find(|cell| cell.cell_type == CaseCellType::Evidence)
        .map(|cell| vec![cell.id.clone()])
        .unwrap_or_else(|| close_request().validation_evidence_ids);
    NativeCloseCheckRequest {
        base_revision_id: space.revision.revision_id.clone(),
        validation_evidence_ids,
        ..close_request()
    }
}

fn provenance(kind: SourceKind, status: ReviewStatus) -> Provenance {
    Provenance::new(
        SourceRef::new(kind),
        Confidence::new(1.0).expect("confidence"),
    )
    .with_review_status(status)
}
