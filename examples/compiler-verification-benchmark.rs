//! Bounded semantic deployment-bundle verification benchmark driver.

use casegraphen::{
    exec::AllowedTransitionClass,
    execution_topology::{parse_execution_topology, EdgeKind, ExecutionTopology, TopologyEdge},
    graph_compiler::{
        compile_execution_topology, verify_deployment_bundle_with_metrics, CompilationMode,
        CompilationTarget, CompilerRequest, NodePlanMapping,
    },
    native_model::{CaseCellLifecycle, CaseCellType, CaseMorphismType},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, env, process};

fn scaled_topology(node_count: usize) -> ExecutionTopology {
    let base = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .expect("repository topology fixture");
    let template = base.nodes[0].clone();
    let mut topology = base;
    topology.topology_id = format!("topology:compiler-benchmark:{node_count}");
    topology.case_space_id = format!("case_space:compiler-benchmark:{node_count}");
    topology.nodes.clear();
    topology.edges.clear();
    topology.verification_policy_ids.clear();
    topology.budget_policy_ids.clear();
    topology.expansion_policy_ids.clear();
    for index in 0..node_count {
        let node_id = format!("node:{index:06}");
        let mut node = template.clone();
        node.node_id = node_id.clone();
        node.work_cell_id = format!("work:{index:06}");
        node.purpose = format!("Compiler verification benchmark node {index}");
        node.inputs[0].name = "payload".to_owned();
        node.inputs[0].schema_id = "schema:compiler-benchmark-payload".to_owned();
        node.inputs[0].artifact_selector = if index % 2 == 1 {
            format!("node:{:06}#payload", index - 1)
        } else {
            format!("fixture:seed:{index:06}")
        };
        node.outputs[0].name = "payload".to_owned();
        node.outputs[0].schema_id = "schema:compiler-benchmark-payload".to_owned();
        node.resource_claims.clear();
        node.verification_policy_id = Some(format!("verification:{index:06}"));
        node.budget_policy_id = Some(format!("budget:{index:06}"));
        node.expansion_policy_id = Some(format!("expansion:{index:06}"));
        node.idempotency_key = format!("compiler-benchmark:{index:06}");
        topology
            .verification_policy_ids
            .push(format!("verification:{index:06}"));
        topology
            .budget_policy_ids
            .push(format!("budget:{index:06}"));
        topology
            .expansion_policy_ids
            .push(format!("expansion:{index:06}"));
        topology.nodes.push(node);
        if index % 2 == 1 {
            topology.edges.push(TopologyEdge {
                edge_id: format!("edge:{:06}-{index:06}", index - 1),
                from: format!("node:{:06}", index - 1),
                to: node_id,
                kind: EdgeKind::Data,
                output: Some("payload".to_owned()),
                input: Some("payload".to_owned()),
                schema_id: Some("schema:compiler-benchmark-payload".to_owned()),
                blocking_predicate: "producer payload is absent".to_owned(),
                dependency_witness: "target consumes the paired producer payload".to_owned(),
                removal_counterexample: "without the edge the paired handoff is unproved"
                    .to_owned(),
                resource_scope: Vec::new(),
                provenance: topology.provenance.clone(),
            });
        }
    }
    topology
}

fn request(topology: &ExecutionTopology) -> CompilerRequest {
    let transition = AllowedTransitionClass {
        morphism_type: CaseMorphismType::Update,
        target_cell_types: vec![CaseCellType::Work],
        to_lifecycles: vec![CaseCellLifecycle::Resolved],
    };
    CompilerRequest {
        mode: CompilationMode::Proposal,
        target: CompilationTarget::GenericJsonlV0,
        case_space_id: topology.case_space_id.clone(),
        base_revision_id: "revision:compiler-benchmark".to_owned(),
        plan_id: format!("plan:{}", topology.topology_id),
        node_plan_mappings: topology
            .nodes
            .iter()
            .map(|node| NodePlanMapping {
                node_id: node.node_id.clone(),
                worker_binding_id: format!("binding:{}", node.node_id),
                success_evidence_requirement_ids: vec![format!("requirement:{}", node.node_id)],
                allowed_transition_classes: vec![transition.clone()],
            })
            .collect(),
        verification_policies: topology
            .verification_policy_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    json!({
                        "schema": "casegraphen.experimental.verification_policy.v0",
                        "verification_policy_id": id,
                        "producer_constraints": {"capability_ids": ["capability:producer"]},
                        "verifier_constraints": {"capability_ids": ["capability:verifier"]},
                        "actor_must_differ": true,
                        "lenses": ["correctness"],
                        "quorum": {"minimum_accepts": 1, "total_verifiers": 1},
                        "required_anchors": ["anchor:artifact"],
                        "allowed_runtime_attestations": ["separate_session"],
                        "provenance": {"source": "benchmark:compiler-verification", "created_by": "actor:benchmark"}
                    }),
                )
            })
            .collect(),
        budget_policies: topology
            .budget_policy_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    json!({"policy_id": id, "max_cost": 10, "max_latency_ms": 1000}),
                )
            })
            .collect::<BTreeMap<String, Value>>(),
        expansion_policies: topology
            .expansion_policy_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    json!({
                        "schema": "casegraphen.experimental.expansion.policy.v0",
                        "schema_version": 0,
                        "expansion_policy_id": id,
                        "candidate_schema_id": "schema:compiler-benchmark-candidate",
                        "dedupe_key": ["candidate_id"],
                        "dedupe_scope": "all_seen",
                        "dry_rounds_required": 2,
                        "max_iterations": 4,
                        "max_spawned_nodes": 8,
                        "max_cost": 10.0,
                        "cost_currency": "USD",
                        "max_latency_ms": 1000,
                        "candidate_disposition": "unreviewed_morphism_proposal",
                        "provenance": {"source": "benchmark:compiler-verification", "created_by": "actor:benchmark"}
                    }),
                )
            })
            .collect(),
    }
}

fn main() {
    let node_count = env::args()
        .nth(1)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            eprintln!("usage: compiler-verification-benchmark <positive-node-count>");
            process::exit(2);
        });
    let topology = scaled_topology(node_count);
    let request = request(&topology);
    let bundle = compile_execution_topology(&topology, &request).unwrap_or_else(|report| {
        eprintln!("{}", serde_json::to_string(&report).unwrap());
        process::exit(1);
    });
    let manifest_hash = bundle.manifest_content_hash.clone();
    let (_, metrics) = verify_deployment_bundle_with_metrics(bundle, &manifest_hash)
        .unwrap_or_else(|finding| {
            eprintln!("{}", serde_json::to_string(&finding).unwrap());
            process::exit(1);
        });
    println!("{}", serde_json::to_string(&metrics).unwrap());
}
