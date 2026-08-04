#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{
        execution_topology_content_hash, CompletenessPolicy, DeliveryMode, EdgeKind,
        ExecutionTopology, NodeInput, NodeOutput, Provenance, ResourceClaim, ResourceMode,
        SideEffects, TopologyEdge, TopologyNode, WorkspaceStrategy, EXECUTION_TOPOLOGY_SCHEMA,
    },
    resource_protocol::{
        declaration_grants, ResourceDeclaration, ResourceReservation, RuntimeResourceAllocation,
        RESOURCE_ALLOCATION_SCHEMA, RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
        RUNTIME_ALLOCATION_TRUST_BOUNDARY,
    },
    runtime_integration::{
        GenericJsonlReconciler, IntegrationHalt, ProposalReviewStatus, RuntimeResourceExpectation,
    },
    runtime_protocol::{
        ReportedRuntimeIdentity, RuntimeFailureKind, RuntimeNodeReport, RuntimeNodeStatus,
        RUNTIME_NODE_REPORT_SCHEMA, RUNTIME_REPORT_TRUST_BOUNDARY,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn topology() -> ExecutionTopology {
    let provenance = Provenance {
        source: "test".into(),
        created_by: "actor:test".into(),
    };
    let node = |id: &str, schema: &str| TopologyNode {
        node_id: id.into(),
        work_cell_id: format!("work:{id}"),
        purpose: id.into(),
        inputs: if id == "reduce" {
            vec![NodeInput {
                name: "items".into(),
                schema_id: "schema:item".into(),
                artifact_selector: "fan-*".into(),
            }]
        } else {
            vec![]
        },
        outputs: vec![NodeOutput {
            name: "output".into(),
            schema_id: schema.into(),
        }],
        side_effects: SideEffects::None,
        resource_claims: vec![],
        executor_class: "external".into(),
        verification_policy_id: None,
        budget_policy_id: None,
        idempotency_key: format!("{id}:key"),
        delivery: if id == "reduce" {
            DeliveryMode::Barrier
        } else {
            DeliveryMode::Streaming
        },
        expansion_policy_id: None,
        estimated_duration_ms: Some(1),
        provenance: provenance.clone(),
    };
    let edge = |id: &str, from: &str| TopologyEdge {
        edge_id: id.into(),
        from: from.into(),
        to: "reduce".into(),
        kind: EdgeKind::Control,
        output: None,
        input: None,
        schema_id: None,
        blocking_predicate: "source missing".into(),
        dependency_witness: "reduce requires every fanout result".into(),
        removal_counterexample: "removal permits omission".into(),
        resource_scope: vec![],
        provenance: provenance.clone(),
    };
    ExecutionTopology {
        schema: EXECUTION_TOPOLOGY_SCHEMA.into(),
        schema_version: 0,
        topology_id: "runtime_graph:fanout-reduce".into(),
        case_space_id: "case:test".into(),
        nodes: vec![
            node("fan-a", "schema:item"),
            node("fan-b", "schema:item"),
            node("reduce", "schema:summary"),
        ],
        edges: vec![edge("edge:a", "fan-a"), edge("edge:b", "fan-b")],
        verification_policy_ids: vec![],
        budget_policy_ids: vec![],
        expansion_policy_ids: vec![],
        completeness_policy: CompletenessPolicy::AllExpectedNodesReported,
        provenance,
    }
}

fn artifact(content: &str) -> (String, String) {
    let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
    let id = format!("artifact:sha256-{hash}");
    (id.clone(), json!({"kind":"artifact","artifact_id":id,"media_type":"application/json","content":content}).to_string())
}

fn report(
    topology: &ExecutionTopology,
    node: &str,
    schema: &str,
    artifact: &str,
    attempt: &str,
) -> RuntimeNodeReport {
    RuntimeNodeReport {
        schema: RUNTIME_NODE_REPORT_SCHEMA.into(),
        schema_version: 0,
        report_id: format!("report:{attempt}"),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(topology).unwrap(),
        node_id: node.into(),
        attempt_id: attempt.into(),
        retry_of_attempt_id: None,
        round_id: "round:1".into(),
        parent_node_ids: topology
            .edges
            .iter()
            .filter(|edge| edge.to == node)
            .map(|edge| edge.from.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        input_artifact_ids: vec![],
        output_artifact_ids: vec![artifact.into()],
        expected_output_schema_id: schema.into(),
        actual_output_schema_id: Some(schema.into()),
        started_at: "2026-08-03T00:00:00Z".into(),
        finished_at: "2026-08-03T00:00:01Z".into(),
        status: RuntimeNodeStatus::Succeeded,
        failure_kind: None,
        runtime_identity: ReportedRuntimeIdentity {
            runtime_name: "fixture".into(),
            runtime_version: "1".into(),
            adapter_name: "generic-jsonl".into(),
            adapter_version: "0".into(),
        },
        reported_model: Some("untrusted-model-claim".into()),
        reported_context_id: None,
        token_usage: None,
        cost: None,
        resource_allocations: vec![],
        worktree_id: None,
        commit_sha: None,
        verifier_report_ids: vec![],
        trust_boundary: RUNTIME_REPORT_TRUST_BOUNDARY.into(),
    }
}

fn report_line(report: &RuntimeNodeReport) -> String {
    json!({"kind":"node_report","report":report}).to_string()
}

fn allocation_line(allocation: &RuntimeResourceAllocation) -> String {
    json!({"kind":"resource_allocation","allocation":allocation}).to_string()
}

#[test]
fn omit_one_then_append_reconciles_to_unreviewed_proposals_and_review_halt() {
    let topology = topology();
    let (a_id, a) = artifact("a");
    let (b_id, b) = artifact("b");
    let (summary_id, summary) = artifact("summary");
    let a_report = report(&topology, "fan-a", "schema:item", &a_id, "attempt:a");
    let b_report = report(&topology, "fan-b", "schema:item", &b_id, "attempt:b");
    let reduce_report = report(
        &topology,
        "reduce",
        "schema:summary",
        &summary_id,
        "attempt:reduce",
    );
    let first = [
        a,
        b,
        summary,
        report_line(&a_report),
        report_line(&b_report),
    ]
    .join("\n");
    let mut reconciler = GenericJsonlReconciler::new();
    assert!(reconciler.ingest_jsonl(&first).is_empty());
    let incomplete = reconciler.reconcile(&topology, "revision:caller-observed");
    assert!(!incomplete.accepted);
    assert_eq!(incomplete.halt, IntegrationHalt::IncompleteRuntimeReports);
    assert_eq!(incomplete.completeness.missing_report_count, 1);
    assert!(incomplete.proposals.is_empty());

    assert!(reconciler
        .ingest_jsonl(&report_line(&reduce_report))
        .is_empty());
    // Exact replay is idempotent, including across an ingest call boundary.
    assert!(reconciler
        .ingest_jsonl(&report_line(&reduce_report))
        .is_empty());
    let complete = reconciler.reconcile(&topology, "revision:caller-observed");
    assert!(complete.completeness.complete);
    assert!(!complete.accepted);
    assert_eq!(complete.base_revision_id, "revision:caller-observed");
    assert_eq!(complete.halt, IntegrationHalt::NeedsReview);
    assert_eq!(complete.proposals.len(), 4);
    assert!(complete
        .proposals
        .iter()
        .all(|proposal| proposal.review_status == ProposalReviewStatus::Unreviewed));
    assert!(complete
        .proposals
        .iter()
        .all(|proposal| proposal.payload["runtime_claim_accepted"] == false));
}

#[test]
fn retry_lineage_is_delegated_to_runtime_protocol_and_hashes_fail_closed() {
    let topology = topology();
    let (a_id, a) = artifact("a");
    let mut failed = report(&topology, "fan-a", "schema:item", &a_id, "attempt:failed");
    failed.status = RuntimeNodeStatus::Failed;
    failed.failure_kind = Some(RuntimeFailureKind::ExecutionError);
    let mut retry = report(&topology, "fan-a", "schema:item", &a_id, "attempt:retry");
    retry.retry_of_attempt_id = Some(failed.attempt_id.clone());
    let mut reconciler = GenericJsonlReconciler::new();
    assert!(reconciler
        .ingest_jsonl(&[a, report_line(&failed), report_line(&retry)].join("\n"))
        .is_empty());
    let result = reconciler.reconcile(&topology, "revision:1");
    assert!(!result
        .completeness
        .findings
        .iter()
        .any(|finding| finding.code == "invalid_retry_lineage"));

    let bad = json!({"kind":"artifact","artifact_id":"artifact:sha256-deadbeef","media_type":"text/plain","content":"changed"}).to_string();
    assert_eq!(
        reconciler.ingest_jsonl(&bad)[0].code,
        "artifact_hash_mismatch"
    );
}

#[test]
fn a_report_cannot_complete_integration_without_its_declared_output_bytes() {
    let topology = topology();
    let (missing_id, _) = artifact("not-ingested");
    let reports = [
        report(&topology, "fan-a", "schema:item", &missing_id, "attempt:a"),
        report(&topology, "fan-b", "schema:item", &missing_id, "attempt:b"),
        report(
            &topology,
            "reduce",
            "schema:summary",
            &missing_id,
            "attempt:r",
        ),
    ];
    let mut reconciler = GenericJsonlReconciler::new();
    assert!(reconciler
        .ingest_jsonl(
            &reports
                .iter()
                .map(report_line)
                .collect::<Vec<_>>()
                .join("\n")
        )
        .is_empty());
    let result = reconciler.reconcile(&topology, "revision:1");
    // Canonical protocol completeness concerns report joins. The integration
    // boundary additionally refuses to propose evidence without observed bytes.
    assert!(result.completeness.complete);
    assert_eq!(result.halt, IntegrationHalt::IncompleteRuntimeReports);
    assert!(result.proposals.is_empty());
    assert!(result
        .ingest_findings
        .iter()
        .any(|finding| finding.code == "missing_declared_artifact"));
}

#[test]
fn typed_resource_allocations_are_reconciled_and_mismatch_blocks_review_proposals() {
    let mut topology = topology();
    topology.nodes[0].resource_claims = vec![ResourceClaim {
        resource: "file:src/fan-a.rs".to_owned(),
        mode: ResourceMode::Read,
        rate_limit_group: None,
        workspace_strategy: Some(WorkspaceStrategy::Shared),
        network_scope: vec![],
        secret_scope: vec![],
    }];
    let (a_id, a) = artifact("a-resource");
    let (b_id, b) = artifact("b-resource");
    let (summary_id, summary) = artifact("summary-resource");
    let reports = [
        report(&topology, "fan-a", "schema:item", &a_id, "attempt:a"),
        report(&topology, "fan-b", "schema:item", &b_id, "attempt:b"),
        report(
            &topology,
            "reduce",
            "schema:summary",
            &summary_id,
            "attempt:reduce",
        ),
    ];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:fan-a".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        node_id: "fan-a".to_owned(),
        claims: topology.nodes[0].resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: "reservation:fan-a".to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: "attempt:a".to_owned(),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let grant = &reservation.grants[0];
    let allocation = RuntimeResourceAllocation {
        schema: RESOURCE_ALLOCATION_SCHEMA.to_owned(),
        schema_version: 0,
        allocation_id: "allocation:fan-a".to_owned(),
        reservation_id: reservation.reservation_id.clone(),
        attempt_id: reservation.attempt_id.clone(),
        resource_id: grant.resource_id.clone(),
        mode: grant.mode,
        rate_limit_group: grant.rate_limit_group.clone(),
        rate_limit_units: grant.rate_limit_units,
        workspace_strategy: grant.workspace_strategy,
        network_scope: grant.network_scope.clone(),
        secret_scope: grant.secret_scope.clone(),
        worktree_id: None,
        trust_boundary: RUNTIME_ALLOCATION_TRUST_BOUNDARY.to_owned(),
    };
    let expectation = RuntimeResourceExpectation {
        declaration,
        reservation,
    };
    let base_lines = [a, b, summary]
        .into_iter()
        .chain(reports.iter().map(report_line))
        .collect::<Vec<_>>();

    let mut valid = GenericJsonlReconciler::new();
    assert!(valid
        .ingest_jsonl(
            &base_lines
                .iter()
                .cloned()
                .chain(std::iter::once(allocation_line(&allocation)))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .is_empty());
    let valid_result = valid.reconcile_with_resources(
        &topology,
        "revision:resource",
        std::slice::from_ref(&expectation),
    );
    assert_eq!(valid_result.halt, IntegrationHalt::NeedsReview);
    assert!(valid_result.reconciliation_complete);
    assert!(valid_result.resource_reconciliations[0].complete);

    let mut mismatched = allocation;
    mismatched.resource_id = "file:src/substituted.rs".to_owned();
    let mut invalid = GenericJsonlReconciler::new();
    assert!(invalid
        .ingest_jsonl(
            &base_lines
                .iter()
                .cloned()
                .chain(std::iter::once(allocation_line(&mismatched)))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .is_empty());
    let invalid_result = invalid.reconcile_with_resources(
        &topology,
        "revision:resource",
        std::slice::from_ref(&expectation),
    );
    assert_eq!(
        invalid_result.halt,
        IntegrationHalt::ResourceReconciliationIncomplete
    );
    assert!(!invalid_result.reconciliation_complete);
    assert!(invalid_result.proposals.is_empty());
    assert!(!invalid_result.resource_reconciliations[0].complete);
    assert!(invalid_result.ingest_findings.iter().any(|finding| {
        finding.code == "resource_reconciliation_mismatch"
            && finding.detail.contains("unexpected_allocation")
    }));
}
