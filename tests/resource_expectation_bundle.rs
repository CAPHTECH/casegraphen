#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    resource_protocol::{
        declaration_grants, ResourceDeclaration, ResourceReservation, RuntimeResourceAllocation,
        RESOURCE_ALLOCATION_SCHEMA, RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
        RUNTIME_ALLOCATION_TRUST_BOUNDARY,
    },
    runtime_integration::{
        ResourceExpectationBundle, ResourceExpectationBundleEntry,
        RESOURCE_EXPECTATION_BUNDLE_SCHEMA,
    },
};

fn fixture() -> (
    casegraphen::execution_topology::ExecutionTopology,
    ResourceExpectationBundle,
) {
    let topology = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    let node = &topology.nodes[0];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:bundle-review-a".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: "reservation:bundle-review-a".to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: "attempt:bundle-review-a:1".to_owned(),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let grant = &reservation.grants[0];
    let allocation = RuntimeResourceAllocation {
        schema: RESOURCE_ALLOCATION_SCHEMA.to_owned(),
        schema_version: 0,
        allocation_id: "allocation:bundle-review-a".to_owned(),
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
    let bundle = ResourceExpectationBundle {
        schema: RESOURCE_EXPECTATION_BUNDLE_SCHEMA.to_owned(),
        schema_version: 0,
        topology_content_hash: execution_topology_content_hash(&topology).unwrap(),
        case_revision_id: "revision:bundle".to_owned(),
        expectations: vec![ResourceExpectationBundleEntry {
            node_id: node.node_id.clone(),
            attempt_id: reservation.attempt_id.clone(),
            declaration,
            reservation,
            allocations: vec![allocation],
            disposition_evidence: vec![],
        }],
    };
    (topology, bundle)
}

#[test]
fn valid_bundle_derives_canonical_expectations_and_allocation_jsonl() {
    let (topology, bundle) = fixture();
    let expectations = bundle.validate(&topology, "revision:bundle").unwrap();
    assert_eq!(expectations.len(), 1);
    let jsonl = bundle.allocation_jsonl();
    assert!(jsonl.contains("resource_allocation"));
    assert!(jsonl.contains("allocation:bundle-review-a"));
}

#[test]
fn stale_substituted_and_duplicated_bundle_records_fail_closed() {
    let (topology, bundle) = fixture();
    let mut stale = bundle.clone();
    stale.case_revision_id = "revision:stale".to_owned();
    assert!(stale
        .validate(&topology, "revision:bundle")
        .unwrap_err()
        .iter()
        .any(|finding| finding.code == "resource_bundle_revision_mismatch"));

    let mut substituted = bundle.clone();
    substituted.expectations[0].allocations[0].attempt_id = "attempt:other".to_owned();
    assert!(substituted
        .validate(&topology, "revision:bundle")
        .unwrap_err()
        .iter()
        .any(|finding| finding.code == "resource_bundle_allocation_join_mismatch"));

    let mut duplicated = bundle;
    duplicated
        .expectations
        .push(duplicated.expectations[0].clone());
    let findings = duplicated
        .validate(&topology, "revision:bundle")
        .unwrap_err();
    assert!(findings
        .iter()
        .any(|finding| finding.code == "duplicate_resource_bundle_node"));
    assert!(findings
        .iter()
        .any(|finding| finding.code == "duplicate_resource_bundle_allocation"));
}
