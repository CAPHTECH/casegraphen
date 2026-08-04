#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    resource_allocator::{
        AtomicResourceAllocator, ResourceAllocatorConfiguration, UnreviewedResourceJournal,
    },
    resource_protocol::{
        declaration_grants, RateLimitCapacity, ReservationAssertionKind,
        ReservationDispositionAssertion, ResourceDeclaration, ResourceReservation,
        RATE_LIMIT_CAPACITY_SCHEMA, RESERVATION_ASSERTION_SCHEMA, RESOURCE_DECLARATION_SCHEMA,
        RESOURCE_RESERVATION_SCHEMA,
    },
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

fn topology() -> casegraphen::execution_topology::ExecutionTopology {
    parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.worktree.example.json"
    ))
    .unwrap()
}

fn declaration(
    topology: &casegraphen::execution_topology::ExecutionTopology,
    index: usize,
) -> ResourceDeclaration {
    let node = &topology.nodes[index];
    ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: format!("declaration:{}", node.node_id),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    }
}

fn reservation(declaration: &ResourceDeclaration, suffix: &str) -> ResourceReservation {
    ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: format!("reservation:{suffix}"),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: format!("attempt:{suffix}"),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(declaration),
    }
}

fn config(capacities: Vec<RateLimitCapacity>) -> ResourceAllocatorConfiguration {
    ResourceAllocatorConfiguration {
        schema: "casegraphen.experimental.resource.allocator_configuration.v0".to_owned(),
        schema_version: 0,
        capacities,
    }
}

fn allocator(path: &PathBuf) -> UnreviewedResourceJournal {
    UnreviewedResourceJournal::new(path, config(vec![])).unwrap()
}

fn temp(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "casegraphen-allocator-{label}-{}",
        std::process::id()
    ));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    path
}

#[test]
fn concurrent_exclusive_requests_cannot_both_commit() {
    let path = temp("race");
    let topology = Arc::new(topology());
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|index| {
            let path = path.clone();
            let topology = Arc::clone(&topology);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let declaration = declaration(&topology, index);
                let reservation = reservation(&declaration, &format!("race-{index}"));
                barrier.wait();
                allocator(&path)
                    .reserve(
                        &topology,
                        "revision:race",
                        declaration,
                        reservation,
                        &format!("idem:race-{index}"),
                    )
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let grants = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|granted| *granted)
        .count();
    assert_eq!(grants, 1);
    assert_eq!(
        allocator(&path)
            .snapshot()
            .unwrap()
            .active_reservations
            .len(),
        1
    );
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn restart_release_expiry_and_idempotent_replay_are_durable() {
    let path = temp("lifecycle");
    let topology = topology();
    let first_declaration = declaration(&topology, 0);
    let first_reservation = reservation(&first_declaration, "first");
    let first = allocator(&path)
        .reserve(
            &topology,
            "revision:1",
            first_declaration.clone(),
            first_reservation.clone(),
            "idem:first",
        )
        .unwrap();
    assert!(!first.replayed);
    let replay = allocator(&path)
        .reserve(
            &topology,
            "revision:1",
            first_declaration,
            first_reservation.clone(),
            "idem:first",
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.snapshot.generation, 1);

    let release = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:release-first".to_owned(),
        reservation_id: first_reservation.reservation_id.clone(),
        attempt_id: first_reservation.attempt_id.clone(),
        kind: ReservationAssertionKind::Release,
        asserted_by: "actor:operator".to_owned(),
        reason: "explicit release".to_owned(),
        superseding_reservation_id: None,
    };
    allocator(&path)
        .disposition("revision:2", release.clone(), "idem:release")
        .unwrap();
    assert!(
        allocator(&path)
            .disposition("revision:2", release, "idem:release")
            .unwrap()
            .replayed
    );
    let second_declaration = declaration(&topology, 1);
    let second_reservation = reservation(&second_declaration, "second");
    allocator(&path)
        .reserve(
            &topology,
            "revision:3",
            second_declaration.clone(),
            second_reservation.clone(),
            "idem:second",
        )
        .unwrap();
    let expiry = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:expire-second".to_owned(),
        reservation_id: second_reservation.reservation_id.clone(),
        attempt_id: second_reservation.attempt_id.clone(),
        kind: ReservationAssertionKind::Expire,
        asserted_by: "actor:lease-controller".to_owned(),
        reason: "externally observed lease expiry".to_owned(),
        superseding_reservation_id: None,
    };
    allocator(&path)
        .disposition("revision:4", expiry.clone(), "idem:expire")
        .unwrap();
    assert!(
        allocator(&path)
            .disposition("revision:4", expiry, "idem:expire")
            .unwrap()
            .replayed
    );
    assert!(allocator(&path)
        .snapshot()
        .unwrap()
        .active_reservations
        .is_empty());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn supersession_requires_a_canonical_active_successor() {
    let path = temp("supersede");
    let mut topology = topology();
    topology.nodes[1].resource_claims[0].resource = "file:src/other.rs".to_owned();
    let old_declaration = declaration(&topology, 0);
    let old_reservation = reservation(&old_declaration, "old");
    let new_declaration = declaration(&topology, 1);
    let new_reservation = reservation(&new_declaration, "new");
    allocator(&path)
        .reserve(
            &topology,
            "revision:1",
            old_declaration,
            old_reservation.clone(),
            "idem:old",
        )
        .unwrap();
    allocator(&path)
        .reserve(
            &topology,
            "revision:2",
            new_declaration,
            new_reservation.clone(),
            "idem:new",
        )
        .unwrap();
    let assertion = ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
        schema_version: 0,
        assertion_id: "assertion:supersede".to_owned(),
        reservation_id: old_reservation.reservation_id.clone(),
        attempt_id: old_reservation.attempt_id.clone(),
        kind: ReservationAssertionKind::Supersede,
        asserted_by: "actor:operator".to_owned(),
        reason: "replacement is active".to_owned(),
        superseding_reservation_id: Some(new_reservation.reservation_id.clone()),
    };
    allocator(&path)
        .disposition("revision:3", assertion.clone(), "idem:supersede")
        .unwrap();
    assert!(
        allocator(&path)
            .disposition("revision:3", assertion, "idem:supersede")
            .unwrap()
            .replayed
    );
    let active = allocator(&path).snapshot().unwrap().active_reservations;
    assert_eq!(active, vec![new_reservation]);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn canonical_capacity_and_corrupt_or_partial_journal_fail_closed() {
    let path = temp("capacity");
    let mut topology = topology();
    topology.nodes[0].resource_claims[0].rate_limit_group = Some("rate_limit_group:api".to_owned());
    topology.nodes[1].resource_claims[0].resource = "file:src/other.rs".to_owned();
    topology.nodes[1].resource_claims[0].rate_limit_group = Some("rate_limit_group:api".to_owned());
    let capacity = RateLimitCapacity {
        schema: RATE_LIMIT_CAPACITY_SCHEMA.to_owned(),
        schema_version: 0,
        group_id: "rate_limit_group:api".to_owned(),
        capacity: 1,
    };
    let allocator = UnreviewedResourceJournal::new(&path, config(vec![capacity.clone()])).unwrap();
    let left = declaration(&topology, 0);
    allocator
        .reserve(
            &topology,
            "revision:1",
            left.clone(),
            reservation(&left, "left"),
            "idem:left",
        )
        .unwrap();
    let right = declaration(&topology, 1);
    assert!(allocator
        .reserve(
            &topology,
            "revision:2",
            right.clone(),
            reservation(&right, "right"),
            "idem:right"
        )
        .is_err());
    fs::write(path.join("00000000000000000002.json"), b"{").unwrap();
    assert!(AtomicResourceAllocator::new(&path, config(vec![capacity]))
        .unwrap()
        .snapshot()
        .is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn pre_publish_temporary_event_is_ignored_but_published_corruption_refuses() {
    let path = temp("crash-boundary");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join(".pending-crashed-writer.tmp"), b"partial json").unwrap();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 0);

    fs::write(path.join("00000000000000000001.json"), b"partial json").unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::remove_dir_all(path).unwrap();
}
