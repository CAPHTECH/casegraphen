#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    graph_simulation::{
        compare_simulation_reports, simulate_execution_topology, GraphSimulationRequest,
        NodeCalibration, RetryPolicy, SimulationBudgets, U64Range, GRAPH_SIMULATION_REQUEST_SCHEMA,
    },
    topology_redesign::{
        diff_topology_versions, propose_redesign, ExpectedImpact, RedesignDisposition,
        RedesignDispositionLog, RedesignEvidenceRefs, RedesignProposalInput,
        ReviewerAuthorityRequirement, SimulationRefs,
    },
};
use std::collections::BTreeMap;

fn topologies() -> (
    casegraphen::execution_topology::ExecutionTopology,
    casegraphen::execution_topology::ExecutionTopology,
) {
    let old = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    let mut proposed = old.clone();
    proposed.nodes[0].purpose.push_str(" with focused routing");
    proposed.budget_policy_ids.push("budget:focused".into());
    (old, proposed)
}

fn artifact(seed: char) -> String {
    format!("artifact:sha256-{}", seed.to_string().repeat(64))
}

fn proposal_input() -> RedesignProposalInput {
    RedesignProposalInput {
        evidence: RedesignEvidenceRefs {
            audit_artifact_ids: vec![artifact('a')],
            integration_proposal_ids: vec![format!("proposal:sha256-{}", "2".repeat(64))],
            expansion_proposal_ids: vec![format!("proposal:sha256-{}", "3".repeat(64))],
        },
        expected_impact: vec![ExpectedImpact {
            metric: "latency_ms".into(),
            expected_direction: "decrease".into(),
            estimated_delta: Some(-90.0),
            rationale: "simulation comparison".into(),
        }],
        uncertainty: vec!["runtime latency calibration may drift".into()],
        information_loss: vec!["none observed; exact node diff retained".into()],
        reviewer_authority: ReviewerAuthorityRequirement {
            authority_policy_id: "authority:topology-review".into(),
            required_capability_ids: vec!["capability:review".into()],
        },
        simulation: SimulationRefs {
            input_artifact_id: artifact('b'),
            old_report_artifact_id: artifact('c'),
            proposed_report_artifact_id: artifact('d'),
        },
    }
}

#[test]
fn canonical_diff_is_order_independent_and_exact() {
    let (mut old, proposed) = topologies();
    let left = diff_topology_versions(&old, &proposed);
    old.nodes.reverse();
    old.edges.reverse();
    let right = diff_topology_versions(&old, &proposed);
    assert_eq!(left, right);
    assert_eq!(left.changed_nodes.len(), 1);
    assert_eq!(left.changed_nodes[0].id, "node:review-a");
    assert_eq!(left.policy_changes.budget.added, vec!["budget:focused"]);
    assert_ne!(
        left.old_topology_content_hash,
        left.proposed_topology_content_hash
    );
}

#[test]
fn semantic_set_reordering_does_not_create_false_entity_changes() {
    let (old, _) = topologies();
    let mut reordered = old.clone();
    reordered.nodes[0].inputs.reverse();
    reordered.nodes[0].outputs.reverse();
    reordered.nodes[0].resource_claims.reverse();
    reordered.edges[0].resource_scope.reverse();
    let diff = diff_topology_versions(&old, &reordered);
    assert_eq!(
        diff.old_topology_content_hash,
        diff.proposed_topology_content_hash
    );
    assert!(diff.changed_nodes.is_empty());
    assert!(diff.changed_edges.is_empty());
}

#[test]
fn proposal_requires_audit_and_simulation_artifacts_and_is_unreviewed() {
    let (old, proposed) = topologies();
    let proposal = propose_redesign(&old, &proposed, proposal_input()).unwrap();
    assert!(proposal.proposal_id.starts_with("redesign:sha256-"));
    assert_eq!(proposal.review_status, "unreviewed");
    assert_eq!(proposal.changes.changed_nodes.len(), 1);

    let mut missing = proposal_input();
    missing.evidence.audit_artifact_ids.clear();
    assert!(propose_redesign(&old, &proposed, missing)
        .unwrap_err()
        .iter()
        .any(|finding| finding.code == "missing_audit_artifact"));

    let mut reordered = proposal_input();
    reordered.evidence.audit_artifact_ids.push(artifact('9'));
    let first = propose_redesign(&old, &proposed, reordered).unwrap();
    let mut opposite = proposal_input();
    opposite.evidence.audit_artifact_ids = vec![artifact('9'), artifact('a')];
    let second = propose_redesign(&old, &proposed, opposite).unwrap();
    assert_eq!(first.proposal_id, second.proposal_id);
}

#[test]
fn disposition_log_preserves_proposed_and_normal_review_binding_without_mutation() {
    let (old, proposed) = topologies();
    let proposal = propose_redesign(&old, &proposed, proposal_input()).unwrap();
    let mut log = RedesignDispositionLog::new(&proposal);
    let accepted = log
        .append(
            &proposal,
            RedesignDisposition::AcceptedBinding {
                review_id: "review:normal-ledger-review".into(),
                revision_id: "revision:42".into(),
                reviewer_authority_id: "authority:topology-review".into(),
            },
        )
        .unwrap();
    assert_eq!(accepted.sequence, 1);
    assert!(accepted.previous_entry_hash.is_some());
    assert_eq!(log.entries.len(), 2);
    assert!(log
        .append(
            &proposal,
            RedesignDisposition::Rejected {
                review_id: "review:later".into(),
                revision_id: "revision:43".into(),
                reason: "cannot rewrite history".into(),
            }
        )
        .is_err());
    // The API returns only a binding record; the caller's topology is untouched.
    assert_ne!(
        execution_topology_content_hash(&old).unwrap(),
        proposal.proposed_topology_content_hash
    );
}

#[test]
fn accepted_binding_must_name_the_proposals_authority_policy() {
    let (old, proposed) = topologies();
    let proposal = propose_redesign(&old, &proposed, proposal_input()).unwrap();
    let mut log = RedesignDispositionLog::new(&proposal);
    let error = log
        .append(
            &proposal,
            RedesignDisposition::AcceptedBinding {
                review_id: "review:1".into(),
                revision_id: "revision:1".into(),
                reviewer_authority_id: "authority:unrelated".into(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "reviewer_authority_mismatch");
}

fn simulation_request(
    topology: &casegraphen::execution_topology::ExecutionTopology,
    focused_latency: u64,
) -> GraphSimulationRequest {
    GraphSimulationRequest {
        schema: GRAPH_SIMULATION_REQUEST_SCHEMA.into(),
        schema_version: 0,
        topology_content_hash: execution_topology_content_hash(topology).unwrap(),
        seed: 7,
        iterations: 10,
        max_parallelism: 4,
        resource_capacities: BTreeMap::new(),
        fan_in_penalty_ms_per_input: 0,
        release_semantics: casegraphen::streaming_reconciliation::StageReleaseSemantics::TerminalArtifactStagePipeliningV0,
        retry_policy: RetryPolicy {
            maximum_attempts: 1,
        },
        expansion_bounds: BTreeMap::new(),
        budgets: SimulationBudgets {
            maximum_latency_ms: None,
            maximum_cost_microunits: None,
            maximum_total_tokens: None,
        },
        calibrations: topology
            .nodes
            .iter()
            .map(|node| NodeCalibration {
                node_id: Some(node.node_id.clone()),
                executor_class: None,
                latency_ms: Some(U64Range {
                    minimum: if node.node_id == "node:review-a" {
                        focused_latency
                    } else {
                        100
                    },
                    maximum: if node.node_id == "node:review-a" {
                        focused_latency
                    } else {
                        100
                    },
                }),
                cost_microunits: Some(U64Range {
                    minimum: 1,
                    maximum: 1,
                }),
                failure_basis_points: Some(0),
                input_tokens: Some(U64Range {
                    minimum: 1,
                    maximum: 1,
                }),
                output_tokens: Some(U64Range {
                    minimum: 1,
                    maximum: 1,
                }),
            })
            .collect(),
        routing_candidates: vec![],
    }
}

#[test]
fn old_and_proposed_topologies_are_compared_by_two_real_simulations() {
    let (old, proposed) = topologies();
    let old_report = simulate_execution_topology(&old, &simulation_request(&old, 200)).unwrap();
    let proposed_report =
        simulate_execution_topology(&proposed, &simulation_request(&proposed, 10)).unwrap();
    let comparison = compare_simulation_reports(&old_report, &proposed_report);
    assert_eq!(comparison.review_status, "unreviewed");
    assert!(comparison
        .latency_p50_delta_ms
        .is_some_and(|delta| delta < 0));
    assert!(
        proposed_report.latency_ms.unwrap().p50 < old_report.latency_ms.unwrap().p50,
        "redesign expected impact must be grounded in distinct topology-bound reports"
    );
}
