#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{
        execution_topology_content_hash, parse_execution_topology, EdgeKind, Provenance,
        ResourceClaim, ResourceMode, TopologyEdge, WorkspaceStrategy,
    },
    resource_protocol::{
        declaration_grants, ResourceDeclaration, ResourceReservation, RuntimeResourceAllocation,
        RESOURCE_ALLOCATION_SCHEMA, RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
        RUNTIME_ALLOCATION_TRUST_BOUNDARY,
    },
    runtime_integration::{
        GenericJsonlReconciler, RuntimeIntegrationReport, RuntimeResourceExpectation,
    },
    runtime_protocol::{
        derive_runtime_graph_expectation, observe_runtime_artifact, parse_runtime_node_report,
        RuntimeArtifactObservation, RuntimeGraphExpectation, RuntimeNodeReport,
    },
    streaming_reconciliation::{
        derive_streaming_acceptance, derive_streaming_resource_permits, reconcile_stream,
        RuntimeStreamEvent, StageReleaseSemantics, StreamEventPayload, StreamRunStatus,
        StreamingAcceptance, StreamingReconciliationInput, StreamingResourcePermits,
        STREAM_EVENT_SCHEMA,
    },
};
use sha2::{Digest, Sha256};

const REVISION: &str = "revision:native-contract-v1";

fn setup() -> (
    casegraphen::execution_topology::ExecutionTopology,
    RuntimeGraphExpectation,
) {
    let mut topology = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    topology.case_space_id = "case_space:native-case-management-contract".into();
    for node in &mut topology.nodes {
        node.resource_claims.clear();
    }
    topology
        .nodes
        .iter_mut()
        .find(|node| node.node_id == "node:reduce")
        .unwrap()
        .work_cell_id = "work:review-native-contract".into();
    topology
        .nodes
        .iter_mut()
        .find(|node| node.node_id == "node:reduce")
        .unwrap()
        .resource_claims = vec![ResourceClaim {
        resource: "workspace:reduce".into(),
        mode: ResourceMode::Exclusive,
        rate_limit_group: None,
        workspace_strategy: Some(WorkspaceStrategy::Ephemeral),
        network_scope: vec![],
        secret_scope: vec![],
    }];
    let expectation = derive_runtime_graph_expectation(&topology).unwrap();
    (topology, expectation)
}

#[test]
fn stream_event_example_round_trips_strictly() {
    let event: RuntimeStreamEvent = serde_json::from_str(include_str!(
        "../schemas/experimental/runtime.stream_event.example.json"
    ))
    .expect("strict stream event example");
    let canonical = serde_json::to_string(&event).unwrap();
    let reparsed: RuntimeStreamEvent = serde_json::from_str(&canonical).unwrap();
    assert_eq!(event, reparsed);
    let mut unknown: serde_json::Value = serde_json::from_str(&canonical).unwrap();
    unknown["runtime_authoritative"] = serde_json::json!(true);
    assert!(serde_json::from_value::<RuntimeStreamEvent>(unknown).is_err());
}

fn event(
    expectation: &RuntimeGraphExpectation,
    id: &str,
    sequence: u64,
    logical_order: u64,
) -> RuntimeStreamEvent {
    let content_hash = format!("{:x}", Sha256::digest(id.as_bytes()));
    RuntimeStreamEvent {
        schema: STREAM_EVENT_SCHEMA.into(),
        event_id: id.into(),
        runtime_graph_id: expectation.runtime_graph_id.clone(),
        runtime_graph_content_hash: expectation.runtime_graph_content_hash.clone(),
        node_id: "node:review-a".into(),
        attempt_id: "attempt:a".into(),
        sequence,
        logical_order,
        observed_at: "2026-08-03T00:00:00Z".into(),
        payload: StreamEventPayload::ArtifactChunk {
            edge_id: "edge:a-reduce".into(),
            artifact_id: format!("artifact:sha256-{content_hash}"),
            schema_id: "schema:findings".into(),
            chunk_index: sequence,
            chunk_sha256: content_hash,
            final_chunk: sequence == 1,
        },
    }
}

fn report(expectation: &RuntimeGraphExpectation, node_id: &str, schema: &str) -> RuntimeNodeReport {
    let mut report = parse_runtime_node_report(include_str!(
        "../schemas/experimental/runtime.node_report.example.json"
    ))
    .unwrap();
    report.runtime_graph_id = expectation.runtime_graph_id.clone();
    report.runtime_graph_content_hash = expectation.runtime_graph_content_hash.clone();
    report.node_id = node_id.into();
    report.attempt_id = format!("attempt:{node_id}");
    report.report_id = format!("report:{node_id}");
    report.expected_output_schema_id = schema.into();
    report.actual_output_schema_id = Some(schema.into());
    let bytes = node_id.as_bytes();
    report.output_artifact_ids = vec![format!("artifact:sha256-{:x}", Sha256::digest(bytes))];
    report.parent_node_ids = expectation
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .map(|node| node.expected_parent_node_ids.clone())
        .unwrap_or_default();
    report
}

fn observed(artifact_id: &str, bytes: &[u8]) -> RuntimeArtifactObservation {
    observe_runtime_artifact(artifact_id.to_owned(), bytes).unwrap()
}

fn resource_integration(
    topology: &casegraphen::execution_topology::ExecutionTopology,
    expectation: &RuntimeGraphExpectation,
    node_id: &str,
) -> (Vec<RuntimeResourceExpectation>, RuntimeIntegrationReport) {
    let node = topology
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .unwrap();
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: format!("declaration:{node_id}"),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: expectation.runtime_graph_content_hash.clone(),
        node_id: node_id.to_owned(),
        claims: node.resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: format!("reservation:{node_id}"),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: format!("attempt:{node_id}"),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let grant = &reservation.grants[0];
    let allocation = RuntimeResourceAllocation {
        schema: RESOURCE_ALLOCATION_SCHEMA.to_owned(),
        schema_version: 0,
        allocation_id: format!("allocation:{node_id}"),
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
    let expectations = vec![RuntimeResourceExpectation {
        declaration,
        reservation,
    }];
    let node_report = report(
        expectation,
        node_id,
        expectation
            .nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .unwrap()
            .expected_output_schema_id
            .as_str(),
    );
    let input = [
        serde_json::json!({"kind":"node_report","report":node_report}).to_string(),
        serde_json::json!({"kind":"resource_allocation","allocation":allocation}).to_string(),
    ]
    .join("\n");
    let mut reconciler = GenericJsonlReconciler::new();
    assert!(reconciler.ingest_jsonl(&input).is_empty());
    let integration =
        reconciler.reconcile_with_resources(topology, "revision:stream", &expectations);
    assert!(integration.resource_reconciliations[0].complete);
    (expectations, integration)
}

fn resources(
    topology: &casegraphen::execution_topology::ExecutionTopology,
    expectation: &RuntimeGraphExpectation,
    node_id: &str,
) -> StreamingResourcePermits {
    let (resource_expectations, integration) = resource_integration(topology, expectation, node_id);
    derive_streaming_resource_permits(
        topology,
        &resource_expectations,
        &integration,
        &acceptance(topology),
    )
    .expect("topology-bound resource permits")
}

fn acceptance(
    topology: &casegraphen::execution_topology::ExecutionTopology,
) -> StreamingAcceptance {
    let case_space: casegraphen::native_model::CaseSpace = serde_json::from_str(include_str!(
        "../schemas/casegraphen/native.case.space.example.json"
    ))
    .unwrap();
    derive_streaming_acceptance(&case_space, topology).expect("canonical streaming acceptance")
}

#[test]
fn duplicate_delayed_and_out_of_order_delivery_is_deterministic() {
    let (topology, expectation) = setup();
    let first = event(&expectation, "event:0", 0, 10);
    let second = event(&expectation, "event:1", 1, 11);
    let resources = resources(&topology, &expectation, "node:reduce");
    let reconcile = |events: &[RuntimeStreamEvent]| {
        reconcile_stream(StreamingReconciliationInput {
            topology: &topology,
            expectation: &expectation,
            events,
            terminal_reports: &[],
            observed_artifacts: &[],
            expected_case_revision_id: REVISION,
            resource_permits: Some(&resources),
            acceptance: None,
            run_closed: false,
        })
    };
    let left = reconcile(&[second.clone(), first.clone(), first.clone()]);
    let right = reconcile(&[first, second]);
    assert_eq!(left.logical_events, right.logical_events);
    assert_eq!(left.stage_release_proposals, right.stage_release_proposals);
    assert_eq!(left.duplicate_event_count, 1);
    assert!(left
        .stage_release_proposals
        .iter()
        .all(|proposal| !proposal.accepted));
}

#[test]
fn a_slow_sibling_allows_safe_progress_without_hiding_incompleteness() {
    let (topology, expectation) = setup();
    let resources = resources(&topology, &expectation, "node:reduce");
    let mut event = event(&expectation, "event:chunk", 0, 0);
    let artifact_id = match &mut event.payload {
        StreamEventPayload::ArtifactChunk {
            artifact_id,
            final_chunk,
            ..
        } => {
            *final_chunk = true;
            artifact_id.clone()
        }
        _ => unreachable!(),
    };
    let mut terminal = report(&expectation, "node:review-a", "schema:findings");
    terminal.attempt_id = "attempt:a".into();
    terminal.report_id = "report:attempt:a".into();
    terminal.output_artifact_ids = vec![artifact_id.clone()];
    let artifact = observed(&artifact_id, b"event:chunk");
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[event],
        terminal_reports: &[terminal],
        observed_artifacts: &[artifact],
        expected_case_revision_id: REVISION,
        resource_permits: Some(&resources),
        acceptance: None,
        run_closed: false,
    });
    assert_eq!(result.status, StreamRunStatus::PartiallyProgressing);
    assert_eq!(
        result.release_semantics,
        StageReleaseSemantics::TerminalArtifactStagePipeliningV0
    );
    assert_eq!(result.stage_release_proposals.len(), 1);
    assert_eq!(
        result.stage_release_proposals[0].release_semantics,
        StageReleaseSemantics::TerminalArtifactStagePipeliningV0
    );
    assert_eq!(
        result.stage_release_proposals[0].target_attempt_id,
        "attempt:node:reduce"
    );
    let proposal = &result.stage_release_proposals[0];
    assert_eq!(
        proposal.topology_content_hash,
        expectation.runtime_graph_content_hash
    );
    assert_eq!(proposal.case_revision_id, REVISION);
    assert_eq!(proposal.to_node_id, "node:reduce");
    assert_eq!(proposal.resource_reconciliation_hash.len(), 64);
    assert!(result
        .unfinished_node_ids
        .contains(&"node:review-b".to_owned()));
    assert!(!result.final_completeness.complete);
}

#[test]
fn resource_permits_refuse_cross_graph_and_cross_node_substitution() {
    let (topology, expectation) = setup();
    let (resource_expectations, integration) =
        resource_integration(&topology, &expectation, "node:reduce");

    let mut other_graph = topology.clone();
    other_graph.nodes[0].purpose.push_str(" other graph");
    let graph_findings = derive_streaming_resource_permits(
        &other_graph,
        &resource_expectations,
        &integration,
        &acceptance(&topology),
    )
    .expect_err("a reconciliation from another graph cannot grant a permit");
    assert!(graph_findings
        .iter()
        .any(|finding| finding.code == "resource_permit_graph_mismatch"));

    let mut substituted = resource_expectations.clone();
    substituted[0].declaration.node_id = "node:verify".to_owned();
    let node_findings = derive_streaming_resource_permits(
        &topology,
        &substituted,
        &integration,
        &acceptance(&topology),
    )
    .expect_err("a reconciliation cannot be associated with another node");
    assert!(node_findings
        .iter()
        .any(|finding| finding.code == "resource_permit_declaration_mismatch"));

    let mut substituted_attempt = resource_expectations.clone();
    substituted_attempt[0].reservation.attempt_id = "attempt:substituted".to_owned();
    let attempt_findings = derive_streaming_resource_permits(
        &topology,
        &substituted_attempt,
        &integration,
        &acceptance(&topology),
    )
    .expect_err("a reconciliation cannot be associated with another attempt");
    assert!(attempt_findings
        .iter()
        .any(|finding| finding.code == "resource_permit_reconciliation_join_mismatch"));

    let mut substituted_result = integration.clone();
    substituted_result.resource_reconciliations[0].actual_allocation_count += 1;
    let result_findings = derive_streaming_resource_permits(
        &topology,
        &resource_expectations,
        &substituted_result,
        &acceptance(&topology),
    )
    .expect_err("modified reconciliation bytes cannot retain canonical provenance");
    assert!(result_findings
        .iter()
        .any(|finding| finding.code == "resource_permit_missing_integration_provenance"));
}

#[test]
fn acceptance_and_resource_permits_cannot_be_replayed_at_a_new_revision() {
    let (topology, expectation) = setup();
    let acceptance_at_a = acceptance(&topology);
    let (resource_expectations, integration) =
        resource_integration(&topology, &expectation, "node:reduce");
    let permits_at_a = derive_streaming_resource_permits(
        &topology,
        &resource_expectations,
        &integration,
        &acceptance_at_a,
    )
    .unwrap();
    let event = event(&expectation, "event:stale-revision", 0, 0);
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[event],
        terminal_reports: &[],
        observed_artifacts: &[],
        expected_case_revision_id: "revision:native-contract-v2",
        resource_permits: Some(&permits_at_a),
        acceptance: Some(&acceptance_at_a),
        run_closed: false,
    });
    assert!(result.stage_release_proposals.is_empty());
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "stale_streaming_acceptance"));
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "stage_release_blocked"));
}

#[test]
fn an_empty_expected_revision_fails_closed() {
    let (topology, expectation) = setup();
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[],
        terminal_reports: &[],
        observed_artifacts: &[],
        expected_case_revision_id: "",
        resource_permits: None,
        acceptance: None,
        run_closed: false,
    });
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "empty_expected_case_revision"));
}

#[test]
fn acceptance_gate_or_missing_resource_blocks_early_release() {
    let (mut topology, expectation) = setup();
    topology.edges.push(TopologyEdge {
        edge_id: "edge:authority-reduce".into(),
        from: "node:review-b".into(),
        to: "node:reduce".into(),
        kind: EdgeKind::ReviewOrAuthority,
        output: None,
        input: None,
        schema_id: None,
        blocking_predicate: "review missing".into(),
        dependency_witness: "external accepted review".into(),
        removal_counterexample: "unreviewed data could flow".into(),
        resource_scope: vec![],
        provenance: Provenance {
            source: "test".into(),
            created_by: "actor:test".into(),
        },
    });
    let expectation = RuntimeGraphExpectation {
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        ..expectation
    };
    let event = event(&expectation, "event:gated", 0, 0);
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[event],
        terminal_reports: &[],
        observed_artifacts: &[],
        expected_case_revision_id: REVISION,
        resource_permits: Some(&resources(&topology, &expectation, "node:reduce")),
        acceptance: None,
        run_closed: false,
    });
    assert!(result.stage_release_proposals.is_empty());
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "stage_release_blocked"));
}

#[test]
fn terminal_completeness_is_owned_by_runtime_protocol() {
    let (topology, expectation) = setup();
    let mut reports = expectation
        .nodes
        .iter()
        .map(|node| report(&expectation, &node.node_id, &node.expected_output_schema_id))
        .collect::<Vec<_>>();
    for edge in &expectation.edges {
        let artifact_id = reports
            .iter()
            .find(|report| report.node_id == edge.from_node_id)
            .unwrap()
            .output_artifact_ids[0]
            .clone();
        reports
            .iter_mut()
            .find(|report| report.node_id == edge.to_node_id)
            .unwrap()
            .input_artifact_ids
            .push(artifact_id);
    }
    let artifacts = reports
        .iter()
        .map(|report| observed(&report.output_artifact_ids[0], report.node_id.as_bytes()))
        .collect::<Vec<_>>();
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[],
        terminal_reports: &reports,
        observed_artifacts: &artifacts,
        expected_case_revision_id: REVISION,
        resource_permits: None,
        acceptance: None,
        run_closed: true,
    });
    assert_eq!(result.status, StreamRunStatus::Complete);
    assert!(result.final_completeness.complete);
}

#[test]
fn topology_expectation_mismatch_blocks_release_even_when_event_joins_expectation() {
    let (mut topology, expectation) = setup();
    let resources = resources(&topology, &expectation, "node:reduce");
    topology.nodes[0]
        .purpose
        .push_str(" changed after deployment");
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[event(&expectation, "event:stale-topology", 0, 0)],
        terminal_reports: &[],
        observed_artifacts: &[],
        expected_case_revision_id: REVISION,
        resource_permits: Some(&resources),
        acceptance: None,
        run_closed: false,
    });
    assert!(result.stage_release_proposals.is_empty());
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "topology_expectation_mismatch"));
}

#[test]
fn caller_cannot_omit_canonical_edges_to_claim_stream_completion() {
    let (topology, canonical) = setup();
    let mut declared = canonical.clone();
    declared.edges.clear();
    let reports = declared
        .nodes
        .iter()
        .map(|node| report(&declared, &node.node_id, &node.expected_output_schema_id))
        .collect::<Vec<_>>();
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &declared,
        events: &[],
        terminal_reports: &reports,
        observed_artifacts: &[],
        expected_case_revision_id: REVISION,
        resource_permits: None,
        acceptance: None,
        run_closed: true,
    });
    assert_ne!(result.status, StreamRunStatus::Complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "topology_expectation_mismatch"));
}

#[test]
fn sequence_and_chunk_identity_collisions_are_not_hidden_by_deduplication() {
    let (topology, expectation) = setup();
    let first = event(&expectation, "event:first", 0, 0);
    let first_artifact_id = match &first.payload {
        StreamEventPayload::ArtifactChunk { artifact_id, .. } => artifact_id.clone(),
        _ => unreachable!(),
    };
    let mut collision = event(&expectation, "event:collision", 0, 1);
    if let StreamEventPayload::ArtifactChunk {
        artifact_id,
        final_chunk,
        ..
    } = &mut collision.payload
    {
        *artifact_id = first_artifact_id;
        *final_chunk = true;
    }
    let result = reconcile_stream(StreamingReconciliationInput {
        topology: &topology,
        expectation: &expectation,
        events: &[first, collision],
        terminal_reports: &[],
        observed_artifacts: &[],
        expected_case_revision_id: REVISION,
        resource_permits: Some(&resources(&topology, &expectation, "node:reduce")),
        acceptance: None,
        run_closed: false,
    });
    for code in ["attempt_sequence_collision", "chunk_index_collision"] {
        assert!(
            result.findings.iter().any(|finding| finding.code == code),
            "missing {code}: {:?}",
            result.findings
        );
    }
    assert!(result.stage_release_proposals.is_empty());
}
