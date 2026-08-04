//! Reproducible long-journal and crash-boundary pilot for the real allocator.

use casegraphen::{
    execution_topology::{
        execution_topology_content_hash, parse_execution_topology, ExecutionTopology,
    },
    resource_allocator::{ResourceAllocatorConfiguration, UnreviewedResourceJournal},
    resource_protocol::{
        declaration_grants, ReservationAssertionKind, ReservationDispositionAssertion,
        ResourceDeclaration, ResourceReservation, RESERVATION_ASSERTION_SCHEMA,
        RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
    },
};
use serde_json::json;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
    time::Instant,
};

const EVENT_TARGET: usize = 512;
const REPLAY_LIMIT_MS: u128 = 5_000;
const APPEND_LIMIT_MS: u128 = 60_000;

fn allocator(path: &Path) -> UnreviewedResourceJournal {
    UnreviewedResourceJournal::new(
        path,
        ResourceAllocatorConfiguration {
            schema: "casegraphen.experimental.resource.allocator_configuration.v0".into(),
            schema_version: 0,
            capacities: vec![],
        },
    )
    .expect("allocator configuration")
}

fn declaration(
    topology: &ExecutionTopology,
    node_index: usize,
    suffix: &str,
) -> ResourceDeclaration {
    let node = &topology.nodes[node_index];
    ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.into(),
        schema_version: 0,
        declaration_id: format!("declaration:{suffix}"),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    }
}

fn reservation(declaration: &ResourceDeclaration, suffix: &str) -> ResourceReservation {
    ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.into(),
        schema_version: 0,
        reservation_id: format!("reservation:{suffix}"),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: format!("attempt:{suffix}"),
        granted_at: "2026-08-04T00:00:00Z".into(),
        grants: declaration_grants(declaration),
    }
}

fn disposition(
    reservation: &ResourceReservation,
    suffix: &str,
    kind: ReservationAssertionKind,
    successor: Option<String>,
) -> ReservationDispositionAssertion {
    ReservationDispositionAssertion {
        schema: RESERVATION_ASSERTION_SCHEMA.into(),
        schema_version: 0,
        assertion_id: format!("assertion:{suffix}"),
        reservation_id: reservation.reservation_id.clone(),
        attempt_id: reservation.attempt_id.clone(),
        kind,
        asserted_by: "actor:durability-pilot".into(),
        reason: "bounded allocator durability pilot".into(),
        superseding_reservation_id: successor,
    }
}

fn main() {
    let output = PathBuf::from(env::args().nth(1).expect("output JSON path"));
    let root = output.with_extension("journal");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let topology = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.worktree.example.json"
    ))
    .unwrap();

    let total_started = Instant::now();
    for index in 0..(EVENT_TARGET / 2) {
        let declaration = declaration(&topology, 0, &format!("long-{index}"));
        let reservation = reservation(&declaration, &format!("long-{index}"));
        allocator(&root)
            .reserve(
                &topology,
                "revision:allocator-pilot",
                declaration,
                reservation.clone(),
                &format!("idem:reserve:{index}"),
            )
            .unwrap();
        allocator(&root)
            .disposition(
                "revision:allocator-pilot",
                disposition(
                    &reservation,
                    &format!("release-{index}"),
                    ReservationAssertionKind::Release,
                    None,
                ),
                &format!("idem:release:{index}"),
            )
            .unwrap();
    }
    let append_ms = total_started.elapsed().as_millis();
    let replay_started = Instant::now();
    let restarted_snapshot = allocator(&root).snapshot().unwrap();
    let replay_ms = replay_started.elapsed().as_millis();

    // A process crash before create-new publication leaves only a pending file.
    fs::write(root.join(".pending-crashed-writer.tmp"), b"partial").unwrap();
    let crash_before_publication_ignored = allocator(&root).snapshot().is_ok();

    // A crash/corruption after publication is a hard integrity refusal.
    let corrupt_path = root.join(format!("{:020}.json", EVENT_TARGET + 1));
    fs::write(&corrupt_path, b"partial").unwrap();
    let crash_after_publication_refused = allocator(&root).snapshot().is_err();
    fs::remove_file(corrupt_path).unwrap();

    // Real competing allocator instances race for the same exclusive resource.
    let race_root = output.with_extension("race-journal");
    let _ = fs::remove_dir_all(&race_root);
    let topology = Arc::new(topology);
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|index| {
            let topology = Arc::clone(&topology);
            let barrier = Arc::clone(&barrier);
            let race_root = race_root.clone();
            thread::spawn(move || {
                let declaration = declaration(&topology, index, &format!("race-{index}"));
                let reservation = reservation(&declaration, &format!("race-{index}"));
                barrier.wait();
                allocator(&race_root)
                    .reserve(
                        &topology,
                        "revision:race",
                        declaration,
                        reservation,
                        &format!("idem:race:{index}"),
                    )
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let concurrent_grants = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|granted| *granted)
        .count();

    // Supersede semantics are exercised on distinct resources in a separate journal.
    let supersede_root = output.with_extension("supersede-journal");
    let _ = fs::remove_dir_all(&supersede_root);
    let mut supersede_topology = (*topology).clone();
    supersede_topology.nodes[1].resource_claims[0].resource = "file:src/other.rs".into();
    let old_declaration = declaration(&supersede_topology, 0, "old");
    let old = reservation(&old_declaration, "old");
    allocator(&supersede_root)
        .reserve(
            &supersede_topology,
            "revision:1",
            old_declaration,
            old.clone(),
            "idem:old",
        )
        .unwrap();
    let new_declaration = declaration(&supersede_topology, 1, "new");
    let new = reservation(&new_declaration, "new");
    allocator(&supersede_root)
        .reserve(
            &supersede_topology,
            "revision:2",
            new_declaration,
            new.clone(),
            "idem:new",
        )
        .unwrap();
    allocator(&supersede_root)
        .disposition(
            "revision:3",
            disposition(
                &old,
                "supersede",
                ReservationAssertionKind::Supersede,
                Some(new.reservation_id.clone()),
            ),
            "idem:supersede",
        )
        .unwrap();
    let supersede_active_successor = allocator(&supersede_root)
        .snapshot()
        .unwrap()
        .active_reservations
        == vec![new];

    let passed = restarted_snapshot.generation as usize == EVENT_TARGET
        && restarted_snapshot.active_reservations.is_empty()
        && append_ms <= APPEND_LIMIT_MS
        && replay_ms <= REPLAY_LIMIT_MS
        && crash_before_publication_ignored
        && crash_after_publication_refused
        && concurrent_grants == 1
        && supersede_active_successor;
    let report = json!({
        "schema":"casegraphen.experimental.resource_allocator_durability_pilot.report.v0",
        "passed":passed,
        "accepted":false,
        "journal_event_count":restarted_snapshot.generation,
        "event_threshold":EVENT_TARGET,
        "append_elapsed_ms":append_ms,
        "append_threshold_ms":APPEND_LIMIT_MS,
        "restart_replay_ms":replay_ms,
        "restart_replay_threshold_ms":REPLAY_LIMIT_MS,
        "active_after_release_count":restarted_snapshot.active_reservations.len(),
        "concurrent_grant_count":concurrent_grants,
        "crash_before_publication_ignored":crash_before_publication_ignored,
        "crash_after_publication_refused":crash_after_publication_refused,
        "release_observed":true,
        "supersede_active_successor":supersede_active_successor,
        "restart_observed":true,
        "checkpoint_compaction":{
            "implemented":false,
            "finding":"full replay is O(event_count); checkpoint/compaction remains required before long-lived production allocation"
        },
        "authority_boundary":"unreviewed allocator mechanics only; no deployment authority or evidence acceptance",
        "halt":"operator_review_required"
    });
    fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(race_root);
    let _ = fs::remove_dir_all(supersede_root);
    if !passed {
        std::process::exit(1);
    }
}
