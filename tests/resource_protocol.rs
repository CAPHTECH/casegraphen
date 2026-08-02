#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{
        execution_topology_content_hash, parse_execution_topology, ResourceClaim, ResourceMode,
        WorkspaceStrategy,
    },
    resource_protocol::{
        declaration_grants, grant_reservation, grant_topology_reservation,
        reconcile_resource_allocations, reservation_is_active, validate_worktree_record,
        GitWorktreeRecord, RateLimitCapacity, ReservationAssertionKind,
        ReservationDispositionAssertion, ResourceDeclaration, ResourceReconciliation,
        ResourceReservation, RuntimeResourceAllocation, WorktreeState, RATE_LIMIT_CAPACITY_SCHEMA,
        RESERVATION_ASSERTION_SCHEMA, RESOURCE_ALLOCATION_SCHEMA, RESOURCE_DECLARATION_SCHEMA,
        RESOURCE_RESERVATION_SCHEMA, RUNTIME_ALLOCATION_TRUST_BOUNDARY,
    },
};
use serde::Deserialize;
use std::{fs, path::Path, process::Command};

fn declaration(id: &str, resource: &str, mode: ResourceMode) -> ResourceDeclaration {
    ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: id.to_owned(),
        runtime_graph_id: "runtime_graph:test".to_owned(),
        runtime_graph_content_hash: "a".repeat(64),
        node_id: format!("node:{id}"),
        claims: vec![ResourceClaim {
            resource: resource.to_owned(),
            mode,
            rate_limit_group: None,
            workspace_strategy: Some(WorkspaceStrategy::Shared),
            network_scope: vec![],
            secret_scope: vec![],
        }],
    }
}

fn reservation(id: &str, attempt: &str, declaration: &ResourceDeclaration) -> ResourceReservation {
    ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: id.to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: attempt.to_owned(),
        granted_at: "2020-01-01T00:00:00Z".to_owned(),
        grants: declaration_grants(declaration),
    }
}

fn allocation(reservation: &ResourceReservation, index: usize) -> RuntimeResourceAllocation {
    let grant = &reservation.grants[index];
    RuntimeResourceAllocation {
        schema: RESOURCE_ALLOCATION_SCHEMA.to_owned(),
        schema_version: 0,
        allocation_id: format!("allocation:{index}"),
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
    }
}

#[test]
fn compatible_readers_receive_overlapping_reservations() {
    let left_declaration = declaration("declaration:left", "file:src/lib.rs", ResourceMode::Read);
    let right_declaration = declaration("declaration:right", "file:src/lib.rs", ResourceMode::Read);
    let left = reservation("reservation:left", "attempt:left", &left_declaration);
    let right = reservation("reservation:right", "attempt:right", &right_declaration);
    assert!(grant_reservation(&left_declaration, &left, &[], &[], &[]).is_ok());
    assert!(grant_reservation(&right_declaration, &right, &[left], &[], &[]).is_ok());
}

#[test]
fn topology_aware_grant_refuses_stale_hash_node_or_claim_substitution() {
    let topology = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .expect("topology example");
    let node = &topology.nodes[0];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:topology-review-a".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    };
    let topology_reservation = reservation(
        "reservation:topology-review-a",
        "attempt:topology-review-a",
        &declaration,
    );
    assert!(grant_topology_reservation(
        &topology,
        &declaration,
        &topology_reservation,
        &[],
        &[],
        &[]
    )
    .is_ok());

    for (changed, expected_code) in [
        (
            {
                let mut value = declaration.clone();
                value.runtime_graph_content_hash = "0".repeat(64);
                value
            },
            "declaration_graph_join_mismatch",
        ),
        (
            {
                let mut value = declaration.clone();
                value.node_id = "node:other".to_owned();
                value
            },
            "unknown_declaration_node",
        ),
        (
            {
                let mut value = declaration.clone();
                value.claims[0].resource = "file:src/other.rs".to_owned();
                value
            },
            "declaration_topology_claim_mismatch",
        ),
    ] {
        let changed_reservation = reservation("reservation:changed", "attempt:changed", &changed);
        let findings =
            grant_topology_reservation(&topology, &changed, &changed_reservation, &[], &[], &[])
                .expect_err("substituted declaration cannot be granted");
        assert!(
            findings.iter().any(|finding| finding.code == expected_code),
            "missing {expected_code}: {findings:?}"
        );
    }
}

#[test]
fn reservation_and_attempt_identities_cannot_be_reused_while_active() {
    let declaration = declaration("declaration:reader", "file:README.md", ResourceMode::Read);
    let active = reservation("reservation:reader", "attempt:reader", &declaration);
    let duplicate = reservation("reservation:reader", "attempt:reader", &declaration);
    let findings = grant_reservation(&declaration, &duplicate, &[active], &[], &[])
        .expect_err("active identities cannot be granted twice even for compatible reads");
    assert!(findings
        .iter()
        .any(|finding| finding.code == "duplicate_reservation_id"));
    assert!(findings
        .iter()
        .any(|finding| finding.code == "attempt_already_reserved"));
}

#[test]
fn an_active_exclusive_writer_blocks_a_second_writer() {
    let left_declaration = declaration(
        "declaration:left",
        "git-branch:main",
        ResourceMode::Exclusive,
    );
    let right_declaration =
        declaration("declaration:right", "git-branch:main", ResourceMode::Write);
    let left = reservation("reservation:left", "attempt:left", &left_declaration);
    let right = reservation("reservation:right", "attempt:right", &right_declaration);
    let findings = grant_reservation(&right_declaration, &right, &[left], &[], &[])
        .expect_err("exclusive main branch must block a writer");
    assert!(findings
        .iter()
        .any(|finding| finding.code == "resource_conflict"));
}

#[test]
fn rate_limit_capacity_is_enforced_deterministically() {
    let mut left_declaration = declaration("declaration:left", "api:github", ResourceMode::Read);
    left_declaration.claims[0].rate_limit_group = Some("rate-limit:github".to_owned());
    let mut right_declaration = declaration("declaration:right", "api:github", ResourceMode::Read);
    right_declaration.claims[0].rate_limit_group = Some("rate-limit:github".to_owned());
    let left = reservation("reservation:left", "attempt:left", &left_declaration);
    let right = reservation("reservation:right", "attempt:right", &right_declaration);
    let capacity = [RateLimitCapacity {
        schema: RATE_LIMIT_CAPACITY_SCHEMA.to_owned(),
        schema_version: 0,
        group_id: "rate-limit:github".to_owned(),
        capacity: 1,
    }];
    let findings = grant_reservation(&right_declaration, &right, &[left], &[], &capacity)
        .expect_err("capacity one cannot grant a second unit");
    assert!(findings
        .iter()
        .any(|finding| finding.code == "rate_limit_capacity_exceeded"));
}

#[test]
fn allocation_mismatch_is_visible_and_incomplete() {
    let declaration = declaration("declaration:writer", "file:src/lib.rs", ResourceMode::Write);
    let reservation = reservation("reservation:writer", "attempt:writer", &declaration);
    let mut actual = allocation(&reservation, 0);
    actual.resource_id = "file:src/other.rs".to_owned();
    let result = reconcile_resource_allocations(&declaration, &reservation, &[actual]);
    assert!(!result.complete);
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "unexpected_allocation"));
    assert!(result
        .findings
        .iter()
        .any(|finding| finding.code == "missing_allocation"));
}

#[test]
fn elapsed_time_and_a_mismatched_assertion_never_release_a_reservation() {
    let declaration = declaration(
        "declaration:old",
        "git-branch:main",
        ResourceMode::Exclusive,
    );
    let old = reservation("reservation:old", "attempt:old", &declaration);
    assert_eq!(old.granted_at, "2020-01-01T00:00:00Z");
    assert!(reservation_is_active(&old, &[]));
    let wrong_attempt = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:wrong".to_owned(),
        reservation_id: old.reservation_id.clone(),
        attempt_id: "attempt:different".to_owned(),
        kind: ReservationAssertionKind::Release,
        asserted_by: "operator:test".to_owned(),
        reason: "external liveness check".to_owned(),
        superseding_reservation_id: None,
    };
    assert!(reservation_is_active(&old, &[wrong_attempt]));
    let release = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:release".to_owned(),
        reservation_id: old.reservation_id.clone(),
        attempt_id: old.attempt_id.clone(),
        kind: ReservationAssertionKind::Release,
        asserted_by: "operator:test".to_owned(),
        reason: "holder termination independently established".to_owned(),
        superseding_reservation_id: None,
    };
    assert!(!reservation_is_active(&old, &[release]));
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorktreeFixture {
    description: String,
    records: Vec<GitWorktreeRecord>,
}

#[test]
fn reference_worktree_fixture_isolates_two_attempts_and_records_commits() {
    let fixture: WorktreeFixture = serde_json::from_str(include_str!(
        "fixtures/resources/two-isolated-worktrees.json"
    ))
    .expect("parse worktree fixture");
    assert!(fixture.description.contains("reference records"));
    assert_eq!(fixture.records.len(), 2);
    let paths = fixture
        .records
        .iter()
        .map(|record| record.path_identity.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let branches = fixture
        .records
        .iter()
        .map(|record| record.branch.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let commits = fixture
        .records
        .iter()
        .map(|record| record.resulting_commit_sha.as_deref().expect("commit"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(paths.len(), 2);
    assert_eq!(branches.len(), 2);
    assert_eq!(commits.len(), 2);
    for record in &fixture.records {
        assert_eq!(record.state, WorktreeState::Committed);
        let mut declaration = declaration(
            &record.reservation_id.replace("reservation", "declaration"),
            &format!("git-worktree:{}", record.worktree_id),
            ResourceMode::Exclusive,
        );
        declaration.claims[0].workspace_strategy = Some(WorkspaceStrategy::IsolatedWorktree);
        let reservation = reservation(&record.reservation_id, &record.attempt_id, &declaration);
        assert!(validate_worktree_record(record, &reservation).is_empty());
    }
}

#[test]
fn uncommitted_and_unexpected_worktree_writes_are_reported() {
    let mut declaration = declaration(
        "declaration:worktree",
        "git-worktree:review",
        ResourceMode::Exclusive,
    );
    declaration.claims[0].workspace_strategy = Some(WorkspaceStrategy::IsolatedWorktree);
    let reservation = reservation("reservation:worktree", "attempt:worktree", &declaration);
    let mut record: GitWorktreeRecord = serde_json::from_str(include_str!(
        "../schemas/experimental/git.worktree_record.v0.example.json"
    ))
    .expect("worktree example");
    record.reservation_id = reservation.reservation_id.clone();
    record.attempt_id = reservation.attempt_id.clone();
    record.resulting_commit_sha = None;
    record.working_tree_clean = false;
    record.unexpected_write_paths = vec!["path:outside-declared-scope".to_owned()];
    record.state = WorktreeState::Active;
    let findings = validate_worktree_record(&record, &reservation);
    assert!(findings
        .iter()
        .any(|finding| finding.code == "worktree_uncommitted"));
    assert!(findings
        .iter()
        .any(|finding| finding.code == "unexpected_worktree_writes"));
}

#[test]
fn secret_values_are_rejected_where_only_named_scopes_are_allowed() {
    let mut declaration = declaration(
        "declaration:secret",
        "api:deployment",
        ResourceMode::Exclusive,
    );
    declaration.claims[0].secret_scope = vec!["secret:token=actual-value".to_owned()];
    let reservation = reservation("reservation:secret", "attempt:secret", &declaration);
    let findings = grant_reservation(&declaration, &reservation, &[], &[], &[])
        .expect_err("secret values must not be grantable as named scopes");
    assert!(findings
        .iter()
        .any(|finding| finding.code == "invalid_named_scope"));
}

#[test]
fn every_protocol_example_matches_its_schema_and_serde_record() {
    let pairs = [
        ("resource.declaration.v0", "declaration"),
        ("resource.reservation.v0", "reservation"),
        ("resource.reservation_disposition.v0", "assertion"),
        ("resource.rate_limit_capacity.v0", "capacity"),
        ("runtime.resource_allocation.v0", "allocation"),
        ("resource.reconciliation.v0", "reconciliation"),
        ("git.worktree_record.v0", "worktree"),
    ];
    for (stem, kind) in pairs {
        let schema = root().join(format!("schemas/experimental/{stem}.schema.json"));
        let example = root().join(format!("schemas/experimental/{stem}.example.json"));
        let output = Command::new("python3")
            .args(["-m", "jsonschema", "-i"])
            .arg(&example)
            .arg(&schema)
            .output()
            .expect("run jsonschema");
        assert!(
            output.status.success(),
            "{stem}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json = fs::read_to_string(&example).expect("read protocol example");
        match kind {
            "declaration" => drop(serde_json::from_str::<ResourceDeclaration>(&json).unwrap()),
            "reservation" => drop(serde_json::from_str::<ResourceReservation>(&json).unwrap()),
            "assertion" => {
                drop(serde_json::from_str::<ReservationDispositionAssertion>(&json).unwrap())
            }
            "capacity" => drop(serde_json::from_str::<RateLimitCapacity>(&json).unwrap()),
            "allocation" => drop(serde_json::from_str::<RuntimeResourceAllocation>(&json).unwrap()),
            "reconciliation" => {
                drop(serde_json::from_str::<ResourceReconciliation>(&json).unwrap())
            }
            "worktree" => drop(serde_json::from_str::<GitWorktreeRecord>(&json).unwrap()),
            _ => unreachable!(),
        }
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
