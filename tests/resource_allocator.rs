#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    resource_allocator::{
        AtomicResourceAllocator, ResourceAllocatorConfiguration, ResourceAllocatorRetentionPolicy,
        UnreviewedResourceJournal, RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA,
    },
    resource_protocol::{
        declaration_grants, RateLimitCapacity, ReservationAssertionKind,
        ReservationDispositionAssertion, ResourceDeclaration, ResourceReservation,
        RATE_LIMIT_CAPACITY_SCHEMA, RESERVATION_ASSERTION_SCHEMA, RESOURCE_DECLARATION_SCHEMA,
        RESOURCE_RESERVATION_SCHEMA,
    },
};
use fs2::FileExt;
use std::{
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
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

fn append_release_pair(path: &PathBuf, suffix: &str) {
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let mut reservation = reservation(&declaration, suffix);
    reservation.reservation_id = format!("reservation:{suffix}");
    reservation.attempt_id = format!("attempt:{suffix}");
    allocator(path)
        .reserve(
            &topology,
            &format!("revision:{suffix}:reserve"),
            declaration,
            reservation.clone(),
            &format!("idem:{suffix}:reserve"),
        )
        .unwrap();
    allocator(path)
        .disposition(
            &format!("revision:{suffix}:release"),
            ReservationDispositionAssertion {
                schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
                schema_version: 0,
                assertion_id: format!("assertion:{suffix}"),
                reservation_id: reservation.reservation_id,
                attempt_id: reservation.attempt_id,
                kind: ReservationAssertionKind::Release,
                asserted_by: "actor:test".to_owned(),
                reason: "checkpoint test release".to_owned(),
                superseding_reservation_id: None,
            },
            &format!("idem:{suffix}:release"),
        )
        .unwrap();
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn subprocess_allocator_helper() {
    let Ok(mode) = std::env::var("CASEGRAPHEN_ALLOCATOR_HELPER_MODE") else {
        return;
    };
    let path = PathBuf::from(std::env::var("CASEGRAPHEN_ALLOCATOR_HELPER_PATH").unwrap());
    fs::create_dir_all(&path).unwrap();
    let suffix = std::env::var("CASEGRAPHEN_ALLOCATOR_HELPER_SUFFIX")
        .unwrap_or_else(|_| "helper".to_owned());
    let ready = path.join(format!("helper-{suffix}.ready"));
    match mode.as_str() {
        "hold" | "crash" => {
            let lock = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(path.join(".allocator-writer-lock"))
                .unwrap();
            FileExt::lock_exclusive(&lock).unwrap();
            fs::write(&ready, b"ready").unwrap();
            if mode == "crash" {
                std::process::abort();
            }
            thread::sleep(Duration::from_secs(5));
        }
        "reserve" => {
            fs::write(&ready, b"ready").unwrap();
            let start = path.join("helpers.start");
            wait_for_file(&start);
            let topology = topology();
            let index = std::env::var("CASEGRAPHEN_ALLOCATOR_HELPER_INDEX")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let declaration = declaration(&topology, index);
            let result = allocator(&path).reserve(
                &topology,
                "revision:subprocess-race",
                declaration.clone(),
                reservation(&declaration, &suffix),
                &format!("idem:{suffix}"),
            );
            fs::write(
                path.join(format!("helper-{suffix}.result")),
                if result.is_ok() {
                    b"granted"
                } else {
                    b"refused"
                },
            )
            .unwrap();
        }
        other => panic!("unknown helper mode {other}"),
    }
}

fn spawn_helper(
    path: &PathBuf,
    mode: &str,
    suffix: &str,
    index: Option<usize>,
) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg("subprocess_allocator_helper")
        .arg("--nocapture")
        .env("CASEGRAPHEN_ALLOCATOR_HELPER_MODE", mode)
        .env("CASEGRAPHEN_ALLOCATOR_HELPER_PATH", path)
        .env("CASEGRAPHEN_ALLOCATOR_HELPER_SUFFIX", suffix)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(index) = index {
        command.env("CASEGRAPHEN_ALLOCATOR_HELPER_INDEX", index.to_string());
    }
    command.spawn().unwrap()
}

#[test]
fn writer_lock_times_out_and_process_crash_releases_it() {
    let path = temp("subprocess-writer-lock");
    let mut holder = spawn_helper(&path, "hold", "hold", None);
    wait_for_file(&path.join("helper-hold.ready"));
    let error = allocator(&path).snapshot().unwrap_err();
    assert!(matches!(
        error,
        casegraphen::resource_allocator::ResourceAllocatorError::WriterBusy { .. }
    ));
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 0);

    let mut crashing = spawn_helper(&path, "crash", "crash", None);
    wait_for_file(&path.join("helper-crash.ready"));
    let _ = crashing.wait();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 0);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn competing_processes_cannot_both_commit_exclusive_reservations() {
    let path = temp("subprocess-contention");
    let mut left = spawn_helper(&path, "reserve", "left", Some(0));
    let mut right = spawn_helper(&path, "reserve", "right", Some(1));
    wait_for_file(&path.join("helper-left.ready"));
    wait_for_file(&path.join("helper-right.ready"));
    fs::write(path.join("helpers.start"), b"start").unwrap();
    assert!(left.wait().unwrap().success());
    assert!(right.wait().unwrap().success());
    let results = ["left", "right"]
        .map(|suffix| fs::read_to_string(path.join(format!("helper-{suffix}.result"))).unwrap());
    assert_eq!(
        results.iter().filter(|result| *result == "granted").count(),
        1
    );
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

fn retention(retain_active_event_count: u64) -> ResourceAllocatorRetentionPolicy {
    ResourceAllocatorRetentionPolicy {
        schema: RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA.to_owned(),
        schema_version: 0,
        retain_active_event_count,
    }
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
fn long_lived_cache_observes_other_instances_and_refuses_stale_head_shortcuts() {
    let path = temp("long-lived-cache-invalidation");
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let reservation = reservation(&declaration, "cached");
    let first = allocator(&path);
    let second = allocator(&path);

    first
        .reserve(
            &topology,
            "revision:1",
            declaration,
            reservation.clone(),
            "idem:cached:reserve",
        )
        .unwrap();
    assert_eq!(second.snapshot().unwrap().generation, 1);
    let generation_one_hint = fs::read(path.join(".allocator-head-hint")).unwrap();

    first
        .disposition(
            "revision:2",
            ReservationDispositionAssertion {
                schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
                schema_version: 0,
                assertion_id: "assertion:cached".to_owned(),
                reservation_id: reservation.reservation_id,
                attempt_id: reservation.attempt_id,
                kind: ReservationAssertionKind::Release,
                asserted_by: "actor:test".to_owned(),
                reason: "cache invalidation test".to_owned(),
                superseding_reservation_id: None,
            },
            "idem:cached:release",
        )
        .unwrap();

    // A rolled-back hint cannot hide the next authoritative event from a
    // process whose in-memory cache still represents generation one.
    fs::write(path.join(".allocator-head-hint"), generation_one_hint).unwrap();
    let refreshed = second.snapshot().unwrap();
    assert_eq!(refreshed.generation, 2);
    assert!(refreshed.active_reservations.is_empty());

    // The hint is never authority: malformed or absent bytes force full
    // canonical replay and are repaired from that replay.
    fs::write(path.join(".allocator-head-hint"), b"{").unwrap();
    assert_eq!(second.snapshot().unwrap(), refreshed);
    fs::remove_file(path.join(".allocator-head-hint")).unwrap();
    assert_eq!(second.snapshot().unwrap(), refreshed);
    assert!(serde_json::from_slice::<serde_json::Value>(
        &fs::read(path.join(".allocator-head-hint")).unwrap()
    )
    .is_ok());

    fs::remove_dir_all(path).unwrap();
}

#[test]
fn durable_event_survives_head_hint_publication_failure() {
    let path = temp("head-hint-failure");
    fs::create_dir_all(path.join(".allocator-head-hint")).unwrap();
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let outcome = allocator(&path)
        .reserve_bounded(
            &topology,
            "revision:hint-failure",
            declaration.clone(),
            reservation(&declaration, "hint-failure"),
            "idem:hint-failure",
        )
        .unwrap();
    assert!(!outcome.snapshot.head_hint_healthy);
    assert!(path.join("00000000000000000001.json").is_file());

    fs::remove_dir(path.join(".allocator-head-hint")).unwrap();
    let restarted = allocator(&path).snapshot().unwrap();
    assert_eq!(restarted.generation, 1);
    assert_eq!(restarted.active_reservations.len(), 1);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn legacy_projection_cannot_fail_after_durable_commit_on_hint_loss() {
    let path = temp("legacy-head-hint-failure");
    fs::create_dir_all(path.join(".allocator-head-hint")).unwrap();
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let outcome = allocator(&path)
        .reserve(
            &topology,
            "revision:legacy-hint-failure",
            declaration.clone(),
            reservation(&declaration, "legacy-hint-failure"),
            "idem:legacy-hint-failure",
        )
        .unwrap();
    assert_eq!(outcome.snapshot.generation, 1);
    assert_eq!(outcome.snapshot.active_reservations.len(), 1);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn hot_cache_preserves_decision_safety_but_restart_audit_refuses_middle_event_tampering() {
    let path = temp("hot-cache-tamper-boundary");
    let journal = allocator(&path);
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let reservation = reservation(&declaration, "tamper");
    journal
        .reserve(
            &topology,
            "revision:tamper",
            declaration.clone(),
            reservation.clone(),
            "idem:tamper",
        )
        .unwrap();
    journal
        .disposition(
            "revision:tamper-release",
            ReservationDispositionAssertion {
                schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
                schema_version: 0,
                assertion_id: "assertion:tamper".to_owned(),
                reservation_id: reservation.reservation_id,
                attempt_id: reservation.attempt_id,
                kind: ReservationAssertionKind::Release,
                asserted_by: "actor:test".to_owned(),
                reason: "establish a cached terminal state".to_owned(),
                superseding_reservation_id: None,
            },
            "idem:tamper-release",
        )
        .unwrap();
    assert_eq!(journal.snapshot().unwrap().generation, 2);

    let first_event = path.join("00000000000000000001.json");
    let mut bytes = fs::read(&first_event).unwrap();
    let position = bytes.iter().position(|byte| *byte == b'r').unwrap();
    bytes[position] = b'R';
    fs::write(&first_event, bytes).unwrap();

    // The hot cache is ephemeral process authority: it does not reinterpret
    // out-of-band journal mutations, so its already-derived safe state holds.
    assert_eq!(journal.snapshot().unwrap().generation, 2);
    // Restart/full audit re-reads every authoritative byte and refuses.
    assert!(allocator(&path).full_replay_snapshot().is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn released_reservation_identity_cannot_be_reused() {
    let path = temp("historical-reservation-identity");
    let topology = topology();
    let declaration = declaration(&topology, 0);
    let original = reservation(&declaration, "identity");
    let journal = allocator(&path);
    journal
        .reserve(
            &topology,
            "revision:1",
            declaration.clone(),
            original.clone(),
            "idem:identity:first",
        )
        .unwrap();
    journal
        .disposition(
            "revision:2",
            ReservationDispositionAssertion {
                schema: RESERVATION_ASSERTION_SCHEMA.to_owned(),
                schema_version: 0,
                assertion_id: "assertion:identity".to_owned(),
                reservation_id: original.reservation_id.clone(),
                attempt_id: original.attempt_id.clone(),
                kind: ReservationAssertionKind::Release,
                asserted_by: "actor:test".to_owned(),
                reason: "release before replay attack".to_owned(),
                superseding_reservation_id: None,
            },
            "idem:identity:release",
        )
        .unwrap();
    assert!(journal
        .reserve(
            &topology,
            "revision:3",
            declaration,
            original,
            "idem:identity:reuse",
        )
        .is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn legacy_full_outcome_and_bounded_outcome_are_explicit_compatible_surfaces() {
    let legacy_path = temp("legacy-outcome");
    let bounded_path = temp("bounded-outcome");
    let topology = topology();
    let legacy_declaration = declaration(&topology, 0);
    let legacy_reservation = reservation(&legacy_declaration, "surface");
    let legacy = allocator(&legacy_path)
        .reserve(
            &topology,
            "revision:surface",
            legacy_declaration,
            legacy_reservation,
            "idem:surface",
        )
        .unwrap();
    assert_eq!(legacy.snapshot.active_reservations.len(), 1);

    let bounded_declaration = declaration(&topology, 0);
    let bounded_reservation = reservation(&bounded_declaration, "surface");
    let bounded = allocator(&bounded_path)
        .reserve_bounded(
            &topology,
            "revision:surface",
            bounded_declaration,
            bounded_reservation,
            "idem:surface",
        )
        .unwrap();
    assert_eq!(bounded.snapshot.active_reservation_count, 1);
    assert_eq!(legacy.snapshot.generation, bounded.snapshot.generation);
    fs::remove_dir_all(legacy_path).unwrap();
    fs::remove_dir_all(bounded_path).unwrap();
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

#[test]
fn checkpoint_suffix_replay_and_full_replay_are_equivalent() {
    let path = temp("checkpoint-equivalence");
    append_release_pair(&path, "one");
    append_release_pair(&path, "two");
    let journal = allocator(&path);
    let checkpoint = journal.create_checkpoint().unwrap();
    assert_eq!(checkpoint.last_event_sequence, 4);
    append_release_pair(&path, "three");
    assert_eq!(
        allocator(&path).snapshot().unwrap(),
        allocator(&path).full_replay_snapshot().unwrap()
    );
    allocator(&path).verify_checkpoint().unwrap();
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn checkpoint_substitution_truncation_cross_configuration_and_cross_journal_fail_closed() {
    let path = temp("checkpoint-tamper");
    append_release_pair(&path, "one");
    append_release_pair(&path, "two");
    allocator(&path).create_checkpoint().unwrap();

    let checkpoint_path = fs::read_dir(path.join("checkpoints"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = fs::read(&checkpoint_path).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["terminal_event_hash"] = serde_json::json!("0".repeat(64));
    fs::write(&checkpoint_path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::write(&checkpoint_path, original).unwrap();

    let first_path = path.join("00000000000000000001.json");
    let second_path = path.join("00000000000000000002.json");
    let first = fs::read(&first_path).unwrap();
    let second = fs::read(&second_path).unwrap();
    fs::write(&first_path, &second).unwrap();
    fs::write(&second_path, &first).unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::write(&first_path, first).unwrap();
    fs::write(&second_path, second).unwrap();

    fs::remove_file(first_path).unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::remove_dir_all(&path).unwrap();

    let source = temp("checkpoint-source");
    append_release_pair(&source, "source");
    allocator(&source).create_checkpoint().unwrap();
    let destination = temp("checkpoint-destination");
    copy_tree(&source, &destination);
    assert!(allocator(&destination).snapshot().is_err());

    let capacity = RateLimitCapacity {
        schema: RATE_LIMIT_CAPACITY_SCHEMA.to_owned(),
        schema_version: 0,
        group_id: "rate_limit_group:different".to_owned(),
        capacity: 1,
    };
    assert!(
        UnreviewedResourceJournal::new(&source, config(vec![capacity]))
            .unwrap()
            .snapshot()
            .is_err()
    );
    fs::remove_dir_all(source).unwrap();
    fs::remove_dir_all(destination).unwrap();
}

fn copy_tree(source: &PathBuf, destination: &PathBuf) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn verified_compaction_preserves_full_replay_and_refuses_archive_tampering() {
    let path = temp("checkpoint-compaction");
    for suffix in ["one", "two", "three"] {
        append_release_pair(&path, suffix);
    }
    let journal = allocator(&path);
    journal.create_checkpoint().unwrap();
    let proof = journal.verify_checkpoint().unwrap();
    let before = journal.full_replay_snapshot().unwrap();
    let outcome = journal.compact(&retention(2), &proof).unwrap();
    assert_eq!(outcome.archived_event_count, 4);
    assert_eq!(outcome.active_event_count, 2);
    assert_eq!(allocator(&path).snapshot().unwrap(), before);
    assert_eq!(allocator(&path).full_replay_snapshot().unwrap(), before);

    fs::write(path.join("archive/00000000000000000001.json"), b"{}").unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn checkpoint_and_compaction_crash_boundaries_fail_safe() {
    let path = temp("checkpoint-crash");
    append_release_pair(&path, "one");
    let journal = allocator(&path);
    journal.create_checkpoint().unwrap();
    fs::write(path.join("checkpoints/.pending-crash.tmp"), b"partial").unwrap();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 2);

    // Archive publication before the compaction record leaves duplicate
    // authoritative bytes. Replay accepts only byte-identical duplicates.
    fs::create_dir_all(path.join("archive")).unwrap();
    fs::hard_link(
        path.join("00000000000000000001.json"),
        path.join("archive/00000000000000000001.json"),
    )
    .unwrap();
    assert_eq!(
        allocator(&path).full_replay_snapshot().unwrap().generation,
        2
    );

    let proof = allocator(&path).verify_checkpoint().unwrap();
    allocator(&path).compact(&retention(0), &proof).unwrap();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 2);

    // Crash after the compaction record but during active deletion leaves
    // some byte-identical active/archive duplicates.
    fs::hard_link(
        path.join("archive/00000000000000000001.json"),
        path.join("00000000000000000001.json"),
    )
    .unwrap();
    assert_eq!(allocator(&path).snapshot().unwrap().generation, 2);

    fs::write(path.join("compactions/00000000000000000003.json"), b"{").unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn published_partial_checkpoint_refuses_instead_of_falling_back() {
    let path = temp("checkpoint-published-partial");
    append_release_pair(&path, "one");
    allocator(&path).create_checkpoint().unwrap();
    fs::write(
        path.join("checkpoints/00000000000000000003-partial.json"),
        b"{",
    )
    .unwrap();
    assert!(allocator(&path).snapshot().is_err());
    fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "release-scale lane; run with --release --ignored"]
fn checkpoint_compaction_preserves_semantics_at_ten_thousand_events() {
    let path = temp("checkpoint-scale-10000");
    for index in 0..5_000 {
        append_release_pair(&path, &format!("scale-{index}"));
    }
    let journal = allocator(&path);
    let before = journal.full_replay_snapshot().unwrap();
    assert_eq!(before.generation, 10_000);
    journal.create_checkpoint().unwrap();
    let proof = journal.verify_checkpoint().unwrap();
    journal.compact(&retention(512), &proof).unwrap();
    assert_eq!(journal.snapshot().unwrap(), before);
    assert_eq!(journal.full_replay_snapshot().unwrap(), before);
    fs::remove_dir_all(path).unwrap();
}
