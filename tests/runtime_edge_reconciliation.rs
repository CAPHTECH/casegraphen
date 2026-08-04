#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{
        CompletenessPolicy, DeliveryMode, EdgeKind, ExecutionTopology, NodeInput, NodeOutput,
        Provenance, SideEffects, TopologyEdge, TopologyNode, EXECUTION_TOPOLOGY_SCHEMA,
    },
    runtime_integration::{GenericJsonlReconciler, IntegrationHalt},
    runtime_protocol::{
        derive_runtime_graph_expectation, observe_runtime_artifact, reconcile_runtime_reports,
        ReportedRuntimeIdentity, RuntimeArtifactObservation, RuntimeGraphExpectation,
        RuntimeNodeReport, RuntimeNodeStatus, RUNTIME_NODE_REPORT_SCHEMA,
        RUNTIME_REPORT_TRUST_BOUNDARY,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn topology() -> ExecutionTopology {
    let provenance = Provenance {
        source: "edge-test".into(),
        created_by: "actor:test".into(),
    };
    let node = |id: &str, input_names: &[&str], output_schema: &str, delivery| TopologyNode {
        node_id: id.into(),
        work_cell_id: format!("work:{id}"),
        purpose: format!("exercise {id}"),
        inputs: input_names
            .iter()
            .map(|name| NodeInput {
                name: (*name).into(),
                schema_id: "schema:item".into(),
                artifact_selector: format!("selector:{name}"),
            })
            .collect(),
        outputs: vec![NodeOutput {
            name: "result".into(),
            schema_id: output_schema.into(),
        }],
        side_effects: SideEffects::None,
        resource_claims: vec![],
        executor_class: "fixture".into(),
        verification_policy_id: None,
        budget_policy_id: None,
        idempotency_key: format!("key:{id}"),
        delivery,
        expansion_policy_id: None,
        estimated_duration_ms: Some(1),
        provenance: provenance.clone(),
    };
    let edge = |id: &str, from: &str, to: &str, input: &str| TopologyEdge {
        edge_id: id.into(),
        from: from.into(),
        to: to.into(),
        kind: EdgeKind::Data,
        output: Some("result".into()),
        input: Some(input.into()),
        schema_id: Some("schema:item".into()),
        blocking_predicate: "handoff absent".into(),
        dependency_witness: "target consumes source bytes".into(),
        removal_counterexample: "target could run without its declared input".into(),
        resource_scope: vec![],
        provenance: provenance.clone(),
    };
    ExecutionTopology {
        schema: EXECUTION_TOPOLOGY_SCHEMA.into(),
        schema_version: 0,
        topology_id: "topology:runtime-edge-test".into(),
        case_space_id: "case:runtime-edge-test".into(),
        nodes: vec![
            node("source-a", &[], "schema:item", DeliveryMode::Streaming),
            node("source-b", &[], "schema:item", DeliveryMode::Streaming),
            node(
                "reduce",
                &["from-a", "from-b"],
                "schema:item",
                DeliveryMode::Barrier,
            ),
            node(
                "fanout-target",
                &["from-a"],
                "schema:item",
                DeliveryMode::Barrier,
            ),
        ],
        edges: vec![
            edge("edge:a-reduce", "source-a", "reduce", "from-a"),
            edge("edge:b-reduce", "source-b", "reduce", "from-b"),
            edge("edge:a-fanout", "source-a", "fanout-target", "from-a"),
        ],
        verification_policy_ids: vec![],
        budget_policy_ids: vec![],
        expansion_policy_ids: vec![],
        completeness_policy: CompletenessPolicy::AllExpectedNodesReported,
        provenance,
    }
}

fn artifact(label: &str) -> (String, RuntimeArtifactObservation) {
    let bytes = label.as_bytes();
    let id = format!("artifact:sha256-{:x}", Sha256::digest(bytes));
    let observation = observe_runtime_artifact(id.clone(), bytes).unwrap();
    (id, observation)
}

fn report(
    expectation: &RuntimeGraphExpectation,
    node_id: &str,
    attempt_id: &str,
    output_id: String,
) -> RuntimeNodeReport {
    let expected = expectation
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .unwrap();
    RuntimeNodeReport {
        schema: RUNTIME_NODE_REPORT_SCHEMA.into(),
        schema_version: 0,
        report_id: format!("report:{attempt_id}"),
        runtime_graph_id: expectation.runtime_graph_id.clone(),
        runtime_graph_content_hash: expectation.runtime_graph_content_hash.clone(),
        node_id: node_id.into(),
        attempt_id: attempt_id.into(),
        retry_of_attempt_id: None,
        round_id: "round:1".into(),
        parent_node_ids: expected.expected_parent_node_ids.clone(),
        input_artifact_ids: vec![],
        output_artifact_ids: vec![output_id],
        expected_output_schema_id: expected.expected_output_schema_id.clone(),
        actual_output_schema_id: Some(expected.expected_output_schema_id.clone()),
        started_at: "2026-08-04T00:00:00Z".into(),
        finished_at: "2026-08-04T00:00:01Z".into(),
        status: RuntimeNodeStatus::Succeeded,
        failure_kind: None,
        runtime_identity: ReportedRuntimeIdentity {
            runtime_name: "fixture".into(),
            runtime_version: "1".into(),
            adapter_name: "fixture".into(),
            adapter_version: "1".into(),
        },
        reported_model: None,
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

fn valid_run() -> (
    RuntimeGraphExpectation,
    Vec<RuntimeNodeReport>,
    Vec<RuntimeArtifactObservation>,
) {
    let expectation = derive_runtime_graph_expectation(&topology()).unwrap();
    let (a, a_observation) = artifact("source-a-result");
    let (b, b_observation) = artifact("source-b-result");
    let (reduce, reduce_observation) = artifact("reduce-result");
    let (fanout, fanout_observation) = artifact("fanout-result");
    let source_a = report(&expectation, "source-a", "attempt:a", a.clone());
    let source_b = report(&expectation, "source-b", "attempt:b", b.clone());
    let mut reduce_report = report(&expectation, "reduce", "attempt:reduce", reduce);
    reduce_report.input_artifact_ids = vec![a.clone(), b];
    let mut fanout_report = report(&expectation, "fanout-target", "attempt:fanout", fanout);
    fanout_report.input_artifact_ids = vec![a];
    (
        expectation,
        vec![source_a, source_b, reduce_report, fanout_report],
        vec![
            a_observation,
            b_observation,
            reduce_observation,
            fanout_observation,
        ],
    )
}

#[test]
fn canonical_expectation_and_valid_fanout_reduce_prove_every_edge() {
    let (expectation, reports, observations) = valid_run();
    assert_eq!(expectation.edges.len(), 3);
    assert_eq!(
        expectation
            .edges
            .iter()
            .filter(|edge| edge.from_node_id == "source-a")
            .count(),
        2
    );
    let result = reconcile_runtime_reports(&expectation, &reports, &observations);
    assert!(result.node_complete);
    assert!(result.dataflow_complete);
    assert!(result.complete);
    assert_eq!(result.proven_edge_count, 3);
    assert_eq!(result.edge_proofs.len(), 3);
    assert!(result.edge_proofs.iter().all(|proof| {
        !proof.output_name.is_empty()
            && !proof.input_name.is_empty()
            && proof.schema_id == "schema:item"
            && proof.artifact_id.ends_with(&proof.artifact_content_sha256)
            && proof.artifact_byte_length > 0
    }));
}

#[test]
fn non_utf8_binary_bytes_are_content_addressed_and_prove_the_edge() {
    let topology = topology();
    let expectation = derive_runtime_graph_expectation(&topology).unwrap();
    let bytes = (0_u8..=255).cycle().take(65_536).collect::<Vec<_>>();
    assert!(std::str::from_utf8(&bytes).is_err());
    let artifact_id = format!("artifact:sha256-{:x}", Sha256::digest(&bytes));
    let observation = observe_runtime_artifact(artifact_id.clone(), &bytes).unwrap();

    let mut source_a = report(
        &expectation,
        "source-a",
        "attempt:binary-source",
        artifact_id.clone(),
    );
    let (source_b_id, source_b_observation) = artifact("source-b-result");
    let source_b = report(
        &expectation,
        "source-b",
        "attempt:binary-source-b",
        source_b_id.clone(),
    );
    let (reduce_id, reduce_observation) = artifact("binary-reduce-result");
    let mut reduce = report(&expectation, "reduce", "attempt:binary-reduce", reduce_id);
    reduce.input_artifact_ids = vec![artifact_id.clone(), source_b_id];
    let (fanout_id, fanout_observation) = artifact("binary-fanout-result");
    let mut fanout = report(
        &expectation,
        "fanout-target",
        "attempt:binary-fanout",
        fanout_id,
    );
    fanout.input_artifact_ids = vec![artifact_id.clone()];
    source_a.output_artifact_ids = vec![artifact_id.clone()];

    let result = reconcile_runtime_reports(
        &expectation,
        &[source_a, source_b, reduce, fanout],
        &[
            observation,
            source_b_observation,
            reduce_observation,
            fanout_observation,
        ],
    );
    assert!(result.complete);
    let binary_proofs = result
        .edge_proofs
        .iter()
        .filter(|proof| proof.artifact_id == artifact_id)
        .collect::<Vec<_>>();
    assert_eq!(binary_proofs.len(), 2);
    assert!(binary_proofs
        .iter()
        .all(|proof| proof.artifact_byte_length == 65_536));
}

#[test]
fn strict_expectation_example_round_trips_and_canonical_projection_owns_edges() {
    let example = include_str!("../schemas/experimental/runtime.graph_expectation.v0.example.json");
    let parsed: RuntimeGraphExpectation = serde_json::from_str(example).unwrap();
    assert_eq!(parsed.edges.len(), 3);
    let mut unknown: serde_json::Value = serde_json::from_str(example).unwrap();
    unknown["caller_invented_complete"] = serde_json::json!(true);
    assert!(serde_json::from_value::<RuntimeGraphExpectation>(unknown).is_err());

    let canonical = derive_runtime_graph_expectation(&topology()).unwrap();
    assert_eq!(
        canonical
            .edges
            .iter()
            .map(|edge| edge.edge_id.as_str())
            .collect::<Vec<_>>(),
        vec!["edge:a-fanout", "edge:a-reduce", "edge:b-reduce"]
    );
}

#[test]
fn missing_substituted_duplicated_and_uningested_handoffs_are_not_graph_complete() {
    let (expectation, reports, observations) = valid_run();
    let reduce_index = reports
        .iter()
        .position(|report| report.node_id == "reduce")
        .unwrap();

    let mut missing = reports.clone();
    missing[reduce_index].input_artifact_ids.remove(0);
    let result = reconcile_runtime_reports(&expectation, &missing, &observations);
    assert!(result.node_complete);
    assert!(!result.dataflow_complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "missing_edge_handoff_artifact"));

    let mut substituted = reports.clone();
    substituted[reduce_index].input_artifact_ids[0] = "artifact:sha256-deadbeef".into();
    let result = reconcile_runtime_reports(&expectation, &substituted, &observations);
    assert!(!result.complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "missing_edge_handoff_artifact"));

    let (extra, extra_observation) = artifact("extra-a-result");
    let mut duplicated = reports.clone();
    let source_a = duplicated
        .iter_mut()
        .find(|report| report.node_id == "source-a")
        .unwrap();
    source_a.output_artifact_ids.push(extra.clone());
    duplicated[reduce_index].input_artifact_ids.push(extra);
    let mut duplicated_observations = observations.clone();
    duplicated_observations.push(extra_observation);
    let result = reconcile_runtime_reports(&expectation, &duplicated, &duplicated_observations);
    assert!(!result.complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "duplicated_edge_handoff_artifact"));

    let result = reconcile_runtime_reports(&expectation, &reports, &observations[1..]);
    assert!(!result.complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "un_ingested_edge_handoff_artifact"));
}

#[test]
fn retry_replacement_uses_only_the_canonical_terminal_attempt() {
    let (expectation, mut reports, mut observations) = valid_run();
    let source_index = reports
        .iter()
        .position(|report| report.node_id == "source-a")
        .unwrap();
    let mut failed = reports[source_index].clone();
    failed.attempt_id = "attempt:a:failed".into();
    failed.report_id = "report:attempt:a:failed".into();
    failed.status = RuntimeNodeStatus::Failed;
    failed.failure_kind = Some(casegraphen::runtime_protocol::RuntimeFailureKind::ExecutionError);
    let (replacement, replacement_observation) = artifact("source-a-retry-result");
    reports[source_index].retry_of_attempt_id = Some(failed.attempt_id.clone());
    reports[source_index].output_artifact_ids = vec![replacement.clone()];
    for report in reports
        .iter_mut()
        .filter(|report| report.node_id == "reduce" || report.node_id == "fanout-target")
    {
        report.input_artifact_ids[0] = replacement.clone();
    }
    reports.push(failed);
    observations.push(replacement_observation);
    let result = reconcile_runtime_reports(&expectation, &reports, &observations);
    assert!(result.complete, "{:?}", result.findings);
    assert_eq!(
        result
            .terminal_attempt_ids
            .get("source-a")
            .map(String::as_str),
        Some("attempt:a")
    );
    assert!(result
        .edge_proofs
        .iter()
        .filter(|proof| proof.from_node_id == "source-a")
        .all(|proof| proof.artifact_id == replacement));
}

#[test]
fn wrong_parent_or_schema_lineage_is_a_stable_deterministic_failure() {
    let (expectation, mut reports, observations) = valid_run();
    let reduce = reports
        .iter_mut()
        .find(|report| report.node_id == "reduce")
        .unwrap();
    reduce.parent_node_ids = vec!["source-a".into(), "intruder".into()];
    let result = reconcile_runtime_reports(&expectation, &reports, &observations);
    assert!(!result.node_complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "parent_node_lineage_mismatch"));

    let (expectation, mut reports, observations) = valid_run();
    reports
        .iter_mut()
        .find(|report| report.node_id == "source-b")
        .unwrap()
        .actual_output_schema_id = Some("schema:substituted".into());
    let result = reconcile_runtime_reports(&expectation, &reports, &observations);
    assert!(!result.complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "output_schema_mismatch"));
}

#[test]
fn generic_jsonl_only_reaches_review_after_the_designed_graph_flowed() {
    let (_, reports, _) = valid_run();
    let labels = [
        ("source-a", "source-a-result"),
        ("source-b", "source-b-result"),
        ("reduce", "reduce-result"),
        ("fanout-target", "fanout-result"),
    ];
    let artifact_lines = labels.iter().map(|(node_id, content)| {
        let report = reports
            .iter()
            .find(|report| report.node_id == *node_id)
            .unwrap();
        json!({
            "kind": "artifact",
            "artifact_id": report.output_artifact_ids[0],
            "media_type": "application/json",
            "content": content
        })
        .to_string()
    });
    let report_lines = reports
        .iter()
        .map(|report| json!({"kind":"node_report", "report":report}).to_string());
    let mut reconciler = GenericJsonlReconciler::new();
    assert!(reconciler
        .ingest_jsonl(
            &artifact_lines
                .chain(report_lines)
                .collect::<Vec<_>>()
                .join("\n")
        )
        .is_empty());
    let result = reconciler.reconcile(&topology(), "revision:edge-integration");
    assert!(result.completeness.node_complete);
    assert!(result.completeness.dataflow_complete);
    assert!(result.completeness.complete);
    assert!(result.reconciliation_complete);
    assert_eq!(result.halt, IntegrationHalt::NeedsReview);
    assert!(!result.accepted);
    assert!(result.proposals.iter().all(|proposal| {
        proposal.review_status == casegraphen::runtime_integration::ProposalReviewStatus::Unreviewed
    }));
}
