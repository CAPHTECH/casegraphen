#![allow(missing_docs)]

use casegraphen::{
    dynamic_expansion::{
        accepted_expansion_review_binding, apply_topology_patch, validate_expansion_policy,
        CandidateDisposition, CandidateDispositionRequest, ExpansionCandidate, ExpansionController,
        ExpansionHalt, ExpansionPolicy, TopologyPatch, TOPOLOGY_PATCH_SCHEMA,
    },
    execution_topology::parse_execution_topology,
    graph_compiler::CompilationMode,
};
use std::{collections::BTreeMap, fs, path::Path};

fn topology() -> casegraphen::execution_topology::ExecutionTopology {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/experimental/execution.topology.file-review.example.json");
    parse_execution_topology(&fs::read_to_string(path).unwrap()).unwrap()
}

fn policy() -> ExpansionPolicy {
    serde_json::from_str(include_str!(
        "../schemas/experimental/expansion.policy.example.json"
    ))
    .unwrap()
}

fn candidate(file: &str, disposition: CandidateDispositionRequest) -> ExpansionCandidate {
    let mut node = topology().nodes[0].clone();
    node.node_id = format!("node:{file}");
    node.work_cell_id = format!("work:{file}");
    node.idempotency_key = format!("expand:{file}");
    ExpansionCandidate {
        candidate_schema_id: "schema:bug-candidate".into(),
        dedupe_values: BTreeMap::from([
            ("file".into(), file.into()),
            ("symbol".into(), "handler".into()),
            ("failure_signature".into(), "panic-1".into()),
        ]),
        requested_disposition: disposition,
        topology_patch: TopologyPatch {
            schema: TOPOLOGY_PATCH_SCHEMA.into(),
            schema_version: 0,
            added_nodes: vec![node],
            removed_node_ids: vec![],
            updated_nodes: vec![],
            added_edges: vec![],
            removed_edge_ids: vec![],
        },
    }
}

#[test]
fn policy_requires_every_hard_limit_and_all_seen_keys() {
    assert!(validate_expansion_policy(&policy()).is_empty());
    let mut invalid = policy();
    invalid.dedupe_key.clear();
    invalid.dry_rounds_required = 0;
    invalid.max_iterations = 0;
    invalid.max_spawned_nodes = 0;
    invalid.max_cost = f64::INFINITY;
    invalid.max_latency_ms = 0;
    let codes = validate_expansion_policy(&invalid)
        .into_iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    for code in [
        "missing_dedupe_key",
        "missing_dry_round_limit",
        "missing_iteration_limit",
        "missing_node_limit",
        "missing_cost_limit",
        "missing_latency_limit",
    ] {
        assert!(
            codes.iter().any(|candidate| candidate == code),
            "missing {code}: {codes:?}"
        );
    }
}

#[test]
fn accounted_latency_is_a_typed_halt_and_defers_candidates() {
    let topology = topology();
    let mut bounded = policy();
    bounded.max_latency_ms = 10;
    let mut controller = ExpansionController::new(bounded, &topology).unwrap();
    controller
        .begin_attempt("attempt:latency", &topology)
        .unwrap();
    let result = controller
        .process_round(
            "attempt:latency",
            vec![candidate(
                "slow.rs",
                CandidateDispositionRequest::AcceptForProposal,
            )],
            0.0,
            10,
        )
        .unwrap();
    assert_eq!(result.halt, ExpansionHalt::MaxLatency);
    assert_eq!(result.total_latency_ms, 10);
    assert_eq!(
        result.decisions[0].disposition,
        CandidateDisposition::Deferred
    );
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "max_latency_reached"));
}

#[test]
fn all_seen_includes_rejected_and_deferred_and_two_dry_rounds_stop() {
    let topology = topology();
    let mut controller = ExpansionController::new(policy(), &topology).unwrap();
    controller.begin_attempt("attempt:1", &topology).unwrap();
    let first = controller
        .process_round(
            "attempt:1",
            vec![
                candidate("a.rs", CandidateDispositionRequest::Reject),
                candidate("b.rs", CandidateDispositionRequest::Defer),
            ],
            1.0,
            0,
        )
        .unwrap();
    assert_eq!(
        first.decisions[0].disposition,
        CandidateDisposition::Rejected
    );
    assert_eq!(
        first.decisions[1].disposition,
        CandidateDisposition::Deferred
    );
    let duplicate = controller
        .process_round(
            "attempt:1",
            vec![
                candidate("a.rs", CandidateDispositionRequest::AcceptForProposal),
                candidate("b.rs", CandidateDispositionRequest::AcceptForProposal),
            ],
            1.0,
            0,
        )
        .unwrap();
    assert!(duplicate
        .decisions
        .iter()
        .all(|decision| decision.disposition == CandidateDisposition::Duplicate));
    assert_eq!(duplicate.dry_rounds, 1);
    let dry = controller
        .process_round("attempt:1", vec![], 0.0, 0)
        .unwrap();
    assert_eq!(dry.halt, ExpansionHalt::Dry);
    assert_eq!(dry.dry_rounds, 2);
}

#[test]
fn proposals_are_unreviewed_content_addressed_and_review_does_not_mutate() {
    let topology = topology();
    let mut controller = ExpansionController::new(policy(), &topology).unwrap();
    controller.begin_attempt("attempt:1", &topology).unwrap();
    let result = controller
        .process_round(
            "attempt:1",
            vec![candidate(
                "new.rs",
                CandidateDispositionRequest::AcceptForProposal,
            )],
            1.0,
            0,
        )
        .unwrap();
    assert_eq!(result.halt, ExpansionHalt::NeedsReview);
    let proposal = &result.proposals[0];
    assert!(proposal.proposal_id.starts_with("proposal:sha256-"));
    assert_eq!(proposal.review_status, "unreviewed");
    assert_eq!(proposal.morphism_proposal["accepted_graph_mutated"], false);
    let mut reviewed = topology.clone();
    reviewed.nodes[0].purpose.push_str(" reviewed expansion");
    let refusal = accepted_expansion_review_binding(
        &CompilationMode::Proposal,
        &proposal.proposal_id,
        &reviewed,
    )
    .unwrap_err();
    assert_eq!(refusal.code, "accepted_review_required");
}

#[test]
fn topology_hash_and_attempt_cannot_switch_mid_attempt() {
    let topology = topology();
    let mut controller = ExpansionController::new(policy(), &topology).unwrap();
    controller.begin_attempt("attempt:1", &topology).unwrap();
    let mut changed = topology.clone();
    changed.nodes[0].purpose.push_str(" changed");
    assert_eq!(
        controller
            .begin_attempt("attempt:1", &changed)
            .unwrap_err()
            .code,
        "topology_hash_switch"
    );
    assert_eq!(
        controller
            .begin_attempt("attempt:2", &topology)
            .unwrap_err()
            .code,
        "attempt_in_progress"
    );
    controller.finish_attempt("attempt:1").unwrap();
}

#[test]
fn accounted_round_cost_counts_duplicate_discovery_and_limits_emit_findings() {
    let topology = topology();
    let mut bounded = policy();
    bounded.max_cost = 2.0;
    let mut controller = ExpansionController::new(bounded, &topology).unwrap();
    controller.begin_attempt("attempt:cost", &topology).unwrap();
    controller
        .process_round(
            "attempt:cost",
            vec![candidate("same.rs", CandidateDispositionRequest::Reject)],
            1.0,
            0,
        )
        .unwrap();
    let limited = controller
        .process_round(
            "attempt:cost",
            vec![candidate(
                "same.rs",
                CandidateDispositionRequest::AcceptForProposal,
            )],
            1.0,
            0,
        )
        .unwrap();
    assert_eq!(limited.total_cost, 2.0);
    assert_eq!(limited.halt, ExpansionHalt::MaxCost);
    assert!(limited
        .findings
        .iter()
        .any(|finding| finding.code == "max_cost_reached"));
    assert_eq!(
        limited.decisions[0].disposition,
        CandidateDisposition::Duplicate
    );
}

#[test]
fn caller_cannot_claim_a_transition_without_a_canonical_review_binding() {
    let topology = topology();
    let mut controller = ExpansionController::new(policy(), &topology).unwrap();
    controller.begin_attempt("attempt:1", &topology).unwrap();
    let proposal = controller
        .process_round(
            "attempt:1",
            vec![candidate(
                "new.rs",
                CandidateDispositionRequest::AcceptForProposal,
            )],
            1.0,
            0,
        )
        .unwrap()
        .proposals
        .remove(0);
    assert_eq!(
        accepted_expansion_review_binding(
            &CompilationMode::Proposal,
            &proposal.proposal_id,
            &topology
        )
        .unwrap_err()
        .code,
        "accepted_review_required"
    );
}

#[test]
fn one_hundred_node_patch_is_deferred_atomically_under_smaller_budget() {
    let topology = topology();
    let mut bounded = policy();
    bounded.max_spawned_nodes = 20;
    let mut candidate = candidate("bulk-0.rs", CandidateDispositionRequest::AcceptForProposal);
    candidate.topology_patch.added_nodes = (0..100)
        .map(|index| {
            let mut node = topology.nodes[0].clone();
            node.node_id = format!("node:bulk-{index}");
            node.work_cell_id = format!("work:bulk-{index}");
            node.idempotency_key = format!("expand:bulk-{index}");
            node
        })
        .collect();
    let mut controller = ExpansionController::new(bounded, &topology).unwrap();
    controller.begin_attempt("attempt:bulk", &topology).unwrap();
    let result = controller
        .process_round("attempt:bulk", vec![candidate], 0.0, 0)
        .unwrap();
    assert_eq!(result.spawned_nodes, 0);
    assert!(result.proposals.is_empty());
    assert_eq!(
        result.decisions[0].disposition,
        CandidateDisposition::Deferred
    );
    assert_eq!(
        result.decisions[0].finding.as_ref().unwrap().code,
        "max_spawned_nodes_reached"
    );
}

#[test]
fn patch_rejects_duplicate_ids_and_invalid_removals() {
    let topology = topology();
    let mut duplicate = candidate(
        "duplicate.rs",
        CandidateDispositionRequest::AcceptForProposal,
    )
    .topology_patch;
    duplicate.added_nodes.push(duplicate.added_nodes[0].clone());
    let duplicate_codes = apply_topology_patch(&topology, &duplicate)
        .unwrap_err()
        .into_iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert!(duplicate_codes
        .iter()
        .any(|code| code == "duplicate_patch_id"));

    let mut invalid_removal = candidate(
        "invalid-removal.rs",
        CandidateDispositionRequest::AcceptForProposal,
    )
    .topology_patch;
    invalid_removal.removed_node_ids.push("node:missing".into());
    let removal_codes = apply_topology_patch(&topology, &invalid_removal)
        .unwrap_err()
        .into_iter()
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert!(removal_codes
        .iter()
        .any(|code| code == "patch_target_missing"));
}

#[test]
fn topology_patch_wire_type_is_strict_and_versioned() {
    let topology = topology();
    let patch =
        candidate("strict.rs", CandidateDispositionRequest::AcceptForProposal).topology_patch;
    let mut unknown = serde_json::to_value(&patch).unwrap();
    unknown["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TopologyPatch>(unknown).is_err());

    let mut unsupported = patch;
    unsupported.schema_version = 1;
    let findings = apply_topology_patch(&topology, &unsupported).unwrap_err();
    assert!(findings
        .iter()
        .any(|finding| finding.code == "unsupported_topology_patch_schema"));
}

#[test]
fn canonical_patch_identity_ignores_input_order_and_binds_base_hash() {
    let topology = topology();
    let mut first = candidate("order-a.rs", CandidateDispositionRequest::AcceptForProposal);
    let second_node = candidate("order-b.rs", CandidateDispositionRequest::AcceptForProposal)
        .topology_patch
        .added_nodes
        .remove(0);
    first.topology_patch.added_nodes.push(second_node);
    let mut reversed = first.clone();
    reversed
        .dedupe_values
        .insert("file".into(), "order-reversed.rs".into());
    reversed.topology_patch.added_nodes.reverse();

    let mut controller = ExpansionController::new(policy(), &topology).unwrap();
    controller
        .begin_attempt("attempt:canonical", &topology)
        .unwrap();
    let first_result = controller
        .process_round("attempt:canonical", vec![first], 0.0, 0)
        .unwrap();
    let first_id = first_result.proposals[0].proposal_id.clone();
    let duplicate = controller
        .process_round("attempt:canonical", vec![reversed], 0.0, 0)
        .unwrap();
    assert_eq!(
        duplicate.decisions[0].disposition,
        CandidateDisposition::Duplicate
    );
    assert_eq!(
        duplicate.decisions[0].finding.as_ref().unwrap().code,
        "duplicate_proposal"
    );
    assert!(first_id.starts_with("proposal:sha256-"));
}

#[test]
fn cumulative_budget_counts_actual_additions_but_not_updates() {
    let topology = topology();
    let mut bounded = policy();
    bounded.max_spawned_nodes = 3;
    let mut controller = ExpansionController::new(bounded, &topology).unwrap();
    controller
        .begin_attempt("attempt:cumulative", &topology)
        .unwrap();

    let first = controller
        .process_round(
            "attempt:cumulative",
            vec![
                candidate("one.rs", CandidateDispositionRequest::AcceptForProposal),
                candidate("two.rs", CandidateDispositionRequest::AcceptForProposal),
            ],
            0.0,
            0,
        )
        .unwrap();
    assert_eq!(first.spawned_nodes, 2);

    let mut update = candidate("update.rs", CandidateDispositionRequest::AcceptForProposal);
    update.topology_patch.added_nodes.clear();
    let mut updated_node = topology.nodes[0].clone();
    updated_node.purpose.push_str(" updated");
    update.topology_patch.updated_nodes.push(updated_node);
    let second = controller
        .process_round(
            "attempt:cumulative",
            vec![
                update,
                candidate("three.rs", CandidateDispositionRequest::AcceptForProposal),
            ],
            0.0,
            0,
        )
        .unwrap();
    assert_eq!(second.spawned_nodes, 3);
    assert_eq!(second.proposals.len(), 2);
    assert_eq!(second.proposals[0].topology_diff.added_node_ids.len(), 0);
    assert_eq!(second.halt, ExpansionHalt::MaxSpawnedNodes);
}
