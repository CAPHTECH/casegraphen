//! Canonical binary and scale reconciliation pilot for issue #85.

use casegraphen::{
    execution_topology::{
        canonical_execution_topology, execution_topology_content_hash, CompletenessPolicy,
        DeliveryMode, EdgeKind, ExecutionTopology, NodeInput, NodeOutput, Provenance, SideEffects,
        TopologyEdge, TopologyNode, EXECUTION_TOPOLOGY_SCHEMA,
    },
    runtime_protocol::{
        derive_runtime_graph_expectation, observe_runtime_artifact, reconcile_runtime_reports,
        ReportedRuntimeIdentity, RuntimeFailureKind, RuntimeNodeReport, RuntimeNodeStatus,
        RUNTIME_NODE_REPORT_SCHEMA, RUNTIME_REPORT_TRUST_BOUNDARY,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, time::Instant};

const NODE_COUNT: usize = 512;
const RETRY_COUNT: usize = 128;
const LATENCY_LIMIT_MS: u128 = 5_000;

fn topology() -> ExecutionTopology {
    let provenance = Provenance {
        source: "issue-85-canonical-scale-pilot".into(),
        created_by: "actor:durability-pilot".into(),
    };
    let nodes = (0..NODE_COUNT)
        .map(|index| TopologyNode {
            node_id: format!("node:{index:04}"),
            work_cell_id: format!("work:{index:04}"),
            purpose: "canonical scale and binary handoff pilot".into(),
            inputs: (index > 0)
                .then(|| NodeInput {
                    name: "input".into(),
                    schema_id: "schema:opaque-binary".into(),
                    artifact_selector: format!("edge:{:04}", index - 1),
                })
                .into_iter()
                .collect(),
            outputs: vec![NodeOutput {
                name: "result".into(),
                schema_id: "schema:opaque-binary".into(),
            }],
            side_effects: SideEffects::None,
            resource_claims: vec![],
            executor_class: "durability-fixture".into(),
            verification_policy_id: None,
            budget_policy_id: None,
            idempotency_key: format!("durability:{index:04}"),
            delivery: DeliveryMode::Streaming,
            expansion_policy_id: None,
            estimated_duration_ms: Some(1),
            provenance: provenance.clone(),
        })
        .collect::<Vec<_>>();
    let edges = (0..NODE_COUNT - 1)
        .map(|index| TopologyEdge {
            edge_id: format!("edge:{index:04}"),
            from: format!("node:{index:04}"),
            to: format!("node:{:04}", index + 1),
            kind: EdgeKind::Data,
            output: Some("result".into()),
            input: Some("input".into()),
            schema_id: Some("schema:opaque-binary".into()),
            blocking_predicate: "terminal source artifact absent".into(),
            dependency_witness: "target input consumes exact source bytes".into(),
            removal_counterexample: "target would lose its declared input".into(),
            resource_scope: vec![],
            provenance: provenance.clone(),
        })
        .collect();
    ExecutionTopology {
        schema: EXECUTION_TOPOLOGY_SCHEMA.into(),
        schema_version: 0,
        topology_id: "topology:issue-85-scale-binary".into(),
        case_space_id: "case-space:issue-85-pilot".into(),
        nodes,
        edges,
        verification_policy_ids: vec![],
        budget_policy_ids: vec![],
        expansion_policy_ids: vec![],
        completeness_policy: CompletenessPolicy::AllExpectedNodesReported,
        provenance,
    }
}

fn report(
    expectation: &casegraphen::runtime_protocol::RuntimeGraphExpectation,
    index: usize,
    attempt: &str,
    retry_of: Option<String>,
    status: RuntimeNodeStatus,
    input_artifact_ids: Vec<String>,
    output_artifact_ids: Vec<String>,
) -> RuntimeNodeReport {
    let node_id = format!("node:{index:04}");
    let expected = expectation
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .expect("node expectation");
    RuntimeNodeReport {
        schema: RUNTIME_NODE_REPORT_SCHEMA.into(),
        schema_version: 0,
        report_id: format!("report:{attempt}"),
        runtime_graph_id: expectation.runtime_graph_id.clone(),
        runtime_graph_content_hash: expectation.runtime_graph_content_hash.clone(),
        node_id,
        attempt_id: attempt.into(),
        retry_of_attempt_id: retry_of,
        round_id: "round:scale".into(),
        parent_node_ids: expected.expected_parent_node_ids.clone(),
        input_artifact_ids,
        output_artifact_ids,
        expected_output_schema_id: expected.expected_output_schema_id.clone(),
        actual_output_schema_id: (status == RuntimeNodeStatus::Succeeded)
            .then(|| expected.expected_output_schema_id.clone()),
        started_at: "2026-08-04T00:00:00Z".into(),
        finished_at: "2026-08-04T00:00:01Z".into(),
        status,
        failure_kind: (status == RuntimeNodeStatus::Failed)
            .then_some(RuntimeFailureKind::ExecutionError),
        runtime_identity: ReportedRuntimeIdentity {
            runtime_name: "canonical-durability-pilot".into(),
            runtime_version: env!("CARGO_PKG_VERSION").into(),
            adapter_name: "rust-direct".into(),
            adapter_version: "0".into(),
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

fn main() {
    let output = PathBuf::from(env::args().nth(1).expect("output directory"));
    let reviewed_deployment_hash = env::args()
        .nth(2)
        .expect("reviewed deployment manifest hash");
    fs::create_dir_all(&output).expect("create output directory");
    let topology = topology();
    let topology_hash = execution_topology_content_hash(&topology).unwrap();
    let expectation = derive_runtime_graph_expectation(&topology).unwrap();
    let binary = (0_u8..=255).cycle().take(65_536).collect::<Vec<_>>();
    let mut artifact_bytes = Vec::with_capacity(NODE_COUNT);
    artifact_bytes.push(binary);
    artifact_bytes.extend((1..NODE_COUNT).map(|index| format!("artifact-{index:04}").into_bytes()));
    let artifact_ids = artifact_bytes
        .iter()
        .map(|bytes| format!("artifact:sha256-{:x}", Sha256::digest(bytes)))
        .collect::<Vec<_>>();
    let observations = artifact_ids
        .iter()
        .zip(&artifact_bytes)
        .map(|(id, bytes)| observe_runtime_artifact(id.clone(), bytes).unwrap())
        .collect::<Vec<_>>();
    let mut reports = Vec::with_capacity(NODE_COUNT + RETRY_COUNT);
    for index in 0..NODE_COUNT {
        let input = (index > 0)
            .then(|| artifact_ids[index - 1].clone())
            .into_iter()
            .collect::<Vec<_>>();
        if index < RETRY_COUNT {
            reports.push(report(
                &expectation,
                index,
                &format!("attempt:{index:04}:failed"),
                None,
                RuntimeNodeStatus::Failed,
                input.clone(),
                vec![],
            ));
        }
        reports.push(report(
            &expectation,
            index,
            &format!("attempt:{index:04}:terminal"),
            (index < RETRY_COUNT).then(|| format!("attempt:{index:04}:failed")),
            RuntimeNodeStatus::Succeeded,
            input,
            vec![artifact_ids[index].clone()],
        ));
    }
    let started = Instant::now();
    let completeness = reconcile_runtime_reports(&expectation, &reports, &observations);
    let elapsed_ms = started.elapsed().as_millis();
    let passed = completeness.complete
        && completeness.proven_edge_count == (NODE_COUNT - 1) as u64
        && completeness.actual_report_count == (NODE_COUNT + RETRY_COUNT) as u64
        && elapsed_ms <= LATENCY_LIMIT_MS;
    fs::write(
        output.join("execution.topology.json"),
        canonical_execution_topology(&topology).unwrap(),
    )
    .unwrap();
    fs::write(
        output.join("runtime.expectation.json"),
        serde_json::to_vec_pretty(&expectation).unwrap(),
    )
    .unwrap();
    fs::write(
        output.join("runtime.reports.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    fs::write(output.join("binary-artifact.bin"), &artifact_bytes[0]).unwrap();
    fs::write(
        output.join("runtime.completeness.json"),
        serde_json::to_vec_pretty(&completeness).unwrap(),
    )
    .unwrap();
    let summary = json!({
        "schema":"casegraphen.experimental.runtime_reconciliation_durability_pilot.report.v0",
        "passed":passed,
        "accepted":false,
        "halt":"operator_review_required",
        "topology_content_hash":topology_hash,
        "reviewed_deployment_hash":reviewed_deployment_hash,
        "node_count":NODE_COUNT,
        "edge_count":NODE_COUNT - 1,
        "retry_count":RETRY_COUNT,
        "report_count":reports.len(),
        "binary_artifact_id":artifact_ids[0],
        "binary_byte_length":artifact_bytes[0].len(),
        "binary_media_type":"application/octet-stream",
        "non_utf8_observed":std::str::from_utf8(&artifact_bytes[0]).is_err(),
        "node_complete":completeness.node_complete,
        "dataflow_complete":completeness.dataflow_complete,
        "complete":completeness.complete,
        "proven_edge_count":completeness.proven_edge_count,
        "edge_proof_set_hash":format!("{:x}", Sha256::digest(serde_json::to_vec(&completeness.edge_proofs).unwrap())),
        "reconciliation_ms":elapsed_ms,
        "reconciliation_threshold_ms":LATENCY_LIMIT_MS,
        "runtime_version":env!("CARGO_PKG_VERSION"),
        "failure_disposition":"audit_or_redesign_proposal_only"
    });
    fs::write(
        output.join("canonical-runtime-report.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    if !passed {
        std::process::exit(1);
    }
}
