//! Reproducible long-journal and crash-boundary pilot for the real allocator.

use casegraphen::{
    execution_topology::{
        execution_topology_content_hash, parse_execution_topology, ExecutionTopology, ResourceMode,
    },
    resource_allocator::{
        ResourceAllocatorConfiguration, ResourceAllocatorRetentionPolicy,
        UnreviewedResourceJournal, RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA,
    },
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
    process::Command,
    sync::{Arc, Barrier},
    thread,
    time::Instant,
};

const DEFAULT_EVENT_TARGET: usize = 512;
const REPLAY_LIMIT_MS: u128 = 5_000;
const APPEND_LIMIT_MS: u128 = 60_000;
const APPEND_PAIR_P95_LIMIT_MS: u64 = 100;

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

fn latency_summary(mut samples: Vec<u64>) -> serde_json::Value {
    samples.sort_unstable();
    let percentile = |numerator: usize, denominator: usize| -> u64 {
        samples[((samples.len() - 1) * numerator) / denominator]
    };
    json!({
        "p50": percentile(50, 100),
        "p95": percentile(95, 100),
        "max": samples.last().copied().unwrap_or(0)
    })
}

fn active_and_mixed_workload(
    output: &Path,
    topology: &ExecutionTopology,
    event_target: usize,
) -> serde_json::Value {
    let active_target = match event_target {
        10_000.. => 10_000,
        _ => 128,
    };
    let root = output.with_extension("active-workload-journal");
    let _ = fs::remove_dir_all(&root);
    let mut shared_read_topology = topology.clone();
    shared_read_topology.nodes[0].resource_claims[0].mode = ResourceMode::Read;
    let journal = allocator(&root);
    let mut active = Vec::with_capacity(active_target);
    let mut active_append_ms = Vec::with_capacity(active_target);
    let active_started = Instant::now();
    for index in 0..active_target {
        let started = Instant::now();
        let declaration = declaration(&shared_read_topology, 0, &format!("active-{index}"));
        let reservation = reservation(&declaration, &format!("active-{index}"));
        journal
            .reserve_bounded(
                &shared_read_topology,
                "revision:active-workload",
                declaration,
                reservation.clone(),
                &format!("idem:active:{index}"),
            )
            .unwrap();
        active.push(reservation);
        active_append_ms.push(started.elapsed().as_millis() as u64);
    }
    let active_elapsed_ms = active_started.elapsed().as_millis();
    let active_snapshot_count = journal.snapshot().unwrap().active_reservations.len();

    let churn_count = match event_target {
        100_000.. => 4_096,
        10_000.. => 1_024,
        _ => active_target,
    };
    let mut churn_pair_ms = Vec::with_capacity(churn_count);
    let churn_started = Instant::now();
    for (index, prior) in active.iter().cloned().enumerate().take(churn_count) {
        let started = Instant::now();
        journal
            .disposition_bounded(
                "revision:mixed-workload",
                disposition(
                    &prior,
                    &format!("mixed-release-{index}"),
                    ReservationAssertionKind::Release,
                    None,
                ),
                &format!("idem:mixed:release:{index}"),
            )
            .unwrap();
        let declaration = declaration(&shared_read_topology, 0, &format!("mixed-{index}"));
        let replacement = reservation(&declaration, &format!("mixed-{index}"));
        journal
            .reserve_bounded(
                &shared_read_topology,
                "revision:mixed-workload",
                declaration,
                replacement,
                &format!("idem:mixed:reserve:{index}"),
            )
            .unwrap();
        churn_pair_ms.push(started.elapsed().as_millis() as u64);
    }
    let churn_elapsed_ms = churn_started.elapsed().as_millis();
    let final_active_count = journal.snapshot().unwrap().active_reservations.len();
    let report = json!({
        "all_active": {
            "reservation_count": active_target,
            "elapsed_ms": active_elapsed_ms,
            "append_latency_ms": latency_summary(active_append_ms),
            "observed_active_count": active_snapshot_count
        },
        "mixed_churn": {
            "release_reserve_pair_count": churn_count,
            "elapsed_ms": churn_elapsed_ms,
            "pair_latency_ms": latency_summary(churn_pair_ms),
            "observed_active_count": final_active_count
        },
        "bounded_operation_snapshot": true,
        "passed": active_snapshot_count == active_target && final_active_count == active_target
    });
    let _ = fs::remove_dir_all(root);
    report
}

fn main() {
    let output = PathBuf::from(env::args().nth(1).expect("output JSON path"));
    let event_target = env::var("CASEGRAPHEN_ALLOCATOR_EVENT_TARGET")
        .ok()
        .map(|value| value.parse::<usize>().expect("positive even event target"))
        .unwrap_or(DEFAULT_EVENT_TARGET);
    assert!(event_target > 0 && event_target % 2 == 0);
    let root = output.with_extension("journal");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let topology = parse_execution_topology(include_str!(
        "../schemas/experimental/execution.topology.worktree.example.json"
    ))
    .unwrap();
    let active_and_mixed = active_and_mixed_workload(&output, &topology, event_target);

    let total_started = Instant::now();
    let mut append_pair_ms = Vec::with_capacity(event_target / 2);
    let mut peak_rss_bytes = observed_rss_bytes();
    // This is the same long-lived allocator shape used by the operational
    // host. Restart replay is measured separately below with a fresh value.
    let long_lived_allocator = allocator(&root);
    for index in 0..(event_target / 2) {
        let pair_started = Instant::now();
        let declaration = declaration(&topology, 0, &format!("long-{index}"));
        let reservation = reservation(&declaration, &format!("long-{index}"));
        long_lived_allocator
            .reserve_bounded(
                &topology,
                "revision:allocator-pilot",
                declaration,
                reservation.clone(),
                &format!("idem:reserve:{index}"),
            )
            .unwrap();
        long_lived_allocator
            .disposition_bounded(
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
        append_pair_ms.push(pair_started.elapsed().as_millis() as u64);
        if index % 128 == 0 {
            peak_rss_bytes = peak_rss_bytes.max(observed_rss_bytes());
        }
    }
    append_pair_ms.sort_unstable();
    let percentile = |numerator: usize, denominator: usize| -> u64 {
        append_pair_ms[((append_pair_ms.len() - 1) * numerator) / denominator]
    };
    let append_pair_p50_ms = percentile(50, 100);
    let append_pair_p95_ms = percentile(95, 100);
    let append_ms = total_started.elapsed().as_millis();
    let replay_started = Instant::now();
    let restarted_snapshot = allocator(&root).snapshot().unwrap();
    let replay_ms = replay_started.elapsed().as_millis();
    let checkpoint_started = Instant::now();
    let checkpoint = allocator(&root).create_checkpoint().unwrap();
    let checkpoint_create_ms = checkpoint_started.elapsed().as_millis();
    let checkpoint_size_bytes = fs::metadata(
        fs::read_dir(root.join("checkpoints"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path(),
    )
    .unwrap()
    .len();
    let verify_started = Instant::now();
    let proof = allocator(&root).verify_checkpoint().unwrap();
    let checkpoint_verify_ms = verify_started.elapsed().as_millis();
    let compact_started = Instant::now();
    let compaction = allocator(&root)
        .compact(
            &ResourceAllocatorRetentionPolicy {
                schema: RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA.into(),
                schema_version: 0,
                retain_active_event_count: 32,
            },
            &proof,
        )
        .unwrap();
    let compaction_ms = compact_started.elapsed().as_millis();
    let suffix_replay_started = Instant::now();
    let suffix_snapshot = allocator(&root).snapshot().unwrap();
    let suffix_replay_ms = suffix_replay_started.elapsed().as_millis();
    let full_after_compaction = allocator(&root).full_replay_snapshot().unwrap();
    let checkpoint_full_replay_equivalent =
        suffix_snapshot == full_after_compaction && suffix_snapshot == restarted_snapshot;

    // A process crash before create-new publication leaves only a pending file.
    fs::write(root.join(".pending-crashed-writer.tmp"), b"partial").unwrap();
    let crash_before_publication_ignored = allocator(&root).snapshot().is_ok();

    // A crash/corruption after publication is a hard integrity refusal.
    let corrupt_path = root.join(format!("{:020}.json", event_target + 1));
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

    let append_limit_ms = match event_target {
        100_000.. => 2_400_000,
        10_000.. => 300_000,
        _ => APPEND_LIMIT_MS,
    };
    let replay_limit_ms = match event_target {
        100_000.. => 120_000,
        10_000.. => 30_000,
        _ => REPLAY_LIMIT_MS,
    };
    let checkpoint_operation_limit_ms = if event_target >= 100_000 {
        600_000
    } else {
        120_000
    };
    let checkpoint_size_limit_bytes = (event_target as u64)
        .saturating_mul(2_048)
        .saturating_add(1_048_576);
    let passed = restarted_snapshot.generation as usize == event_target
        && restarted_snapshot.active_reservations.is_empty()
        && append_ms <= append_limit_ms
        && append_pair_p95_ms <= APPEND_PAIR_P95_LIMIT_MS
        && replay_ms <= replay_limit_ms
        && checkpoint_create_ms <= checkpoint_operation_limit_ms
        && checkpoint_verify_ms <= checkpoint_operation_limit_ms
        && compaction_ms <= checkpoint_operation_limit_ms
        && checkpoint_size_bytes <= checkpoint_size_limit_bytes
        && crash_before_publication_ignored
        && crash_after_publication_refused
        && concurrent_grants == 1
        && supersede_active_successor;
    let passed = passed
        && checkpoint_full_replay_equivalent
        && active_and_mixed["passed"].as_bool() == Some(true);
    let report = json!({
        "schema":"casegraphen.experimental.resource_allocator_durability_pilot.report.v0",
        "passed":passed,
        "accepted":false,
        "journal_event_count":restarted_snapshot.generation,
        "event_threshold":event_target,
        "append_elapsed_ms":append_ms,
        "append_pair_latency_ms":{"p50":append_pair_p50_ms,"p95":append_pair_p95_ms,"max":append_pair_ms.last().copied().unwrap_or(0)},
        "append_pair_p95_threshold_ms":APPEND_PAIR_P95_LIMIT_MS,
        "append_threshold_ms":append_limit_ms,
        "restart_replay_ms":replay_ms,
        "restart_replay_threshold_ms":replay_limit_ms,
        "active_after_release_count":restarted_snapshot.active_reservations.len(),
        "concurrent_grant_count":concurrent_grants,
        "crash_before_publication_ignored":crash_before_publication_ignored,
        "crash_after_publication_refused":crash_after_publication_refused,
        "release_observed":true,
        "supersede_active_successor":supersede_active_successor,
        "restart_observed":true,
        "checkpoint_compaction":{
            "implemented":true,
            "checkpoint_sequence":checkpoint.last_event_sequence,
            "checkpoint_content_hash":checkpoint.checkpoint_content_hash,
            "checkpoint_size_bytes":checkpoint_size_bytes,
            "checkpoint_size_threshold_bytes":checkpoint_size_limit_bytes,
            "checkpoint_create_ms":checkpoint_create_ms,
            "checkpoint_independent_verify_ms":checkpoint_verify_ms,
            "checkpoint_operation_threshold_ms":checkpoint_operation_limit_ms,
            "compaction_ms":compaction_ms,
            "archived_event_count":compaction.archived_event_count,
            "active_event_count":compaction.active_event_count,
            "suffix_replay_ms":suffix_replay_ms,
            "full_replay_equivalent":checkpoint_full_replay_equivalent
        },
        "workloads": active_and_mixed,
        "observed_peak_rss_bytes":peak_rss_bytes,
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

fn observed_rss_bytes() -> u64 {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1024)
}
