use super::*;
use crate::native_model::{CaseMorphismType, CaseSpace, ProjectionAudience};
use higher_graphen_core::{Id, ReviewStatus};
use std::{
    fs::{self, FileTimes, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const NATIVE_EXAMPLE: &str =
    include_str!("../../schemas/casegraphen/native.case.space.example.json");

#[test]
fn import_list_inspect_history_and_replay_case_space() {
    let root = temp_root("round-trip");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();

    let imported = store
        .import_case_space(&case_space)
        .expect("import native case space");
    let listed = store.list_case_spaces().expect("list case spaces");
    let inspected = store
        .inspect_case_space(&case_space.case_space_id)
        .expect("inspect case space");
    let history = store
        .history_entries(&case_space.case_space_id)
        .expect("history entries");
    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay case space");

    assert_eq!(
        imported.current_revision_id,
        case_space.revision.revision_id
    );
    assert_eq!(listed, vec![inspected.clone()]);
    assert_eq!(inspected.history_entry_count, 1);
    assert_eq!(history, case_space.morphism_log);
    assert_eq!(replay.case_space, case_space);
    assert!(
        store
            .validate_case_space(&replay.case_space_id)
            .expect("validate case space")
            .valid
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn public_store_append_cannot_mint_an_execution_trace_anchor() {
    let root = temp_root("reserved-trace-anchor");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let mut entry = metadata_entry(&case_space);
    entry.morphism.morphism_type = CaseMorphismType::Custom("execution_trace_anchor".to_owned());

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("public store API must not mint a tool-observed trace anchor");

    assert!(
        error
            .to_string()
            .contains("reserved for the canonical run path"),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history remains readable")
            .len(),
        1,
        "refusal must not append the forged anchor"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn genesis_payload_round_trips_after_its_snapshot_is_rebuilt() {
    let root = temp_root("genesis-rebuild");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import reconstructable genesis");
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("genesis snapshot path");
    fs::remove_file(&snapshot_path).expect("delete genesis snapshot");

    let rebuild = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect("rebuild genesis from log");
    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay rebuilt genesis");

    assert_eq!(rebuild.revisions.len(), 1);
    assert_eq!(
        rebuild.revisions[0].snapshot_status,
        NativeSnapshotStatus::Rebuilt
    );
    assert_eq!(replay.case_space, case_space);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn replay_and_validation_fold_from_empty_when_all_snapshots_are_deleted() {
    let root = temp_root("no-snapshot-replay");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second revision");
    let expected = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay before deleting all snapshots")
        .case_space;
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("genesis snapshot path");
    fs::remove_file(&snapshot_path).expect("delete only snapshot");

    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay without any snapshot");
    let validation = store
        .validate_case_space(&case_space.case_space_id)
        .expect("validate fold without any snapshot");

    assert_eq!(replay.case_space, expected);
    assert!(validation.valid);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rebuild_recovers_only_a_deleted_periodic_snapshot() {
    let root = temp_root("periodic-rebuild");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let expected = append_through_sequence(&store, &case_space.case_space_id, SNAPSHOT_INTERVAL);
    let snapshot_path = store
        .resolve_snapshot_path(
            &store
                .relative_snapshot_path(&case_space.case_space_id, &expected.revision.revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("periodic snapshot path");
    fs::remove_file(&snapshot_path).expect("delete periodic snapshot");

    let rebuild = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect("recover periodic snapshot");
    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay recovered periodic snapshot");

    assert_eq!(rebuild.revisions.len(), SNAPSHOT_INTERVAL as usize);
    assert_eq!(
        rebuild.revisions[0].snapshot_status,
        NativeSnapshotStatus::Agrees
    );
    assert!(rebuild.revisions[1..SNAPSHOT_INTERVAL as usize - 1]
        .iter()
        .all(|revision| revision.snapshot_status == NativeSnapshotStatus::NotScheduled));
    assert_eq!(
        rebuild.revisions[SNAPSHOT_INTERVAL as usize - 1].snapshot_status,
        NativeSnapshotStatus::Rebuilt
    );
    assert!(snapshot_path.exists());
    assert_eq!(replay.case_space, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn snapshot_disagreement_with_fold_fails_rebuild_and_validation() {
    let root = temp_root("fold-disagreement");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("snapshot path");
    let mut snapshot: CaseSpace =
        serde_json::from_value(read_json(&snapshot_path)).expect("typed snapshot");
    snapshot.morphism_log[0].morphism.metadata["payload"]["added_cells"][0]["title"] =
        serde_json::json!("Title supplied only by the folded log");
    snapshot.revision.checksum.clear();
    snapshot.morphism_log[0].replay_checksum.clear();
    let checksum = case_space_checksum(&snapshot).expect("tampered snapshot checksum");
    snapshot.revision.checksum = checksum.clone();
    snapshot.morphism_log[0].replay_checksum = checksum;
    write_json_value(
        &snapshot_path,
        &serde_json::to_value(&snapshot).expect("snapshot value"),
    );
    rewrite_history(&store, &case_space.case_space_id, &snapshot.morphism_log);

    let rebuild_error = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect_err("disagreeing snapshot must not be overwritten");
    let validation_error = store
        .validate_case_space(&case_space.case_space_id)
        .expect_err("validation must compare snapshot with folded log");

    for error in [rebuild_error, validation_error] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error
            .to_string()
            .contains("disagrees with folded morphism log"));
        assert!(error
            .to_string()
            .contains(case_space.revision.revision_id.as_str()));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_refuses_a_morphism_without_an_operation_gate() {
    let root = temp_root("missing-operation-gate");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let mut entry = metadata_entry(&case_space);
    entry.morphism.metadata.remove("operation_gate");

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("ungated append must fail at the store boundary");

    assert!(matches!(error, NativeStoreError::InvalidMorphism { .. }));
    assert!(error
        .to_string()
        .contains("missing required metadata.operation_gate"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn genesis_import_is_the_ungated_append_exemption() {
    let root = temp_root("ungated-genesis");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    assert!(!case_space.morphism_log[0]
        .morphism
        .metadata
        .contains_key("operation_gate"));

    let record = store
        .import_case_space(&case_space)
        .expect("genesis import remains explicitly ungated");

    assert_eq!(record.history_entry_count, 1);
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("validate imported genesis")
            .valid
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_metadata_only_morphism_advances_history_and_replay() {
    let root = temp_root("append");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let entry = metadata_entry(&case_space);

    store
        .append_morphism(&case_space.case_space_id, entry.clone())
        .expect("append metadata-only morphism");

    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay after append");
    assert_eq!(replay.history.len(), 2);
    assert_eq!(
        replay.current_revision_id,
        id("revision:native-contract-v2")
    );
    assert_eq!(replay.case_space.morphism_log[1], entry);
    let genesis_hash = crate::native_hash::morphism_log_entry_hash(&case_space.morphism_log[0])
        .expect("genesis hash");
    assert_eq!(
        replay.history[1].previous_entry_hash.as_deref(),
        Some(genesis_hash.as_str())
    );
    let second_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &replay.current_revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("second revision snapshot path");
    assert!(!second_snapshot_path.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn periodic_snapshots_replay_more_than_one_interval_to_the_full_fold() {
    let root = temp_root("periodic-fold");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let target_sequence = SNAPSHOT_INTERVAL + 2;
    let expected = append_through_sequence(&store, &case_space.case_space_id, target_sequence);
    let entries = store
        .history_entries(&case_space.case_space_id)
        .expect("periodic history");

    for entry in &entries {
        let snapshot_path = store
            .resolve_snapshot_path(
                &store.relative_snapshot_path(&case_space.case_space_id, &entry.target_revision_id),
                &store.log_path(&case_space.case_space_id),
            )
            .expect("revision snapshot path");
        assert_eq!(
            snapshot_path.exists(),
            snapshot_required(entry.sequence),
            "snapshot policy for sequence {}",
            entry.sequence
        );
    }

    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("fold from nearest periodic snapshot");
    let folded = fold_morphism_log(
        &store.log_path(&case_space.case_space_id),
        &entries,
        |_, _, _, _| Ok(()),
    )
    .expect("fold entire log from empty");

    assert_eq!(replay.case_space, expected);
    assert_eq!(replay.case_space, folded);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fold_reuses_one_known_id_index_across_unsnapshotted_entries() {
    let case_space = fixture_space();
    let mut expected = case_space.clone();
    let mut entries = case_space.morphism_log.clone();
    for _ in 2..=8 {
        let entry = next_metadata_entry(&expected);
        apply_bounded_morphism(Path::new("fold-index-test"), &mut expected, &entry)
            .expect("prepare expected revision");
        expected.morphism_log.push(entry.clone());
        expected.revision = revision_from_entry(&expected.case_space_id, &entry);
        entries.push(entry);
    }

    KNOWN_IDS_CALL_COUNT.with(|count| count.set(0));
    let folded = fold_morphism_log(Path::new("fold-index-test"), &entries, |_, _, _, _| Ok(()))
        .expect("fold with one incremental id index");
    let known_id_builds = KNOWN_IDS_CALL_COUNT.with(std::cell::Cell::get);

    assert_eq!(known_id_builds, 1);
    assert_eq!(folded, expected);
}

#[test]
fn fold_streams_revisions_without_retaining_materialized_history() {
    let case_space = fixture_space();
    let mut expected = case_space.clone();
    let mut entries = case_space.morphism_log.clone();
    for _ in 2..=8 {
        let entry = next_metadata_entry(&expected);
        apply_bounded_morphism(Path::new("streaming-fold-test"), &mut expected, &entry)
            .expect("prepare expected revision");
        expected.morphism_log.push(entry.clone());
        expected.revision = revision_from_entry(&expected.case_space_id, &entry);
        entries.push(entry);
    }
    let mut visited_revision_ids = Vec::new();

    let folded = fold_morphism_log(
        Path::new("streaming-fold-test"),
        &entries,
        |_, entry, _, case_space| {
            assert_eq!(case_space.revision.revision_id, entry.target_revision_id);
            visited_revision_ids.push(entry.target_revision_id.clone());
            Ok(())
        },
    )
    .expect("stream revisions through callback");

    assert_eq!(visited_revision_ids.len(), entries.len());
    assert_eq!(folded, expected);
}

#[test]
fn native_record_names_only_snapshots_that_exist() {
    let root = temp_root("record-snapshot-shape");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let record = store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append unscheduled revision");
    let value = serde_json::to_value(&record).expect("serialize native record");

    assert_eq!(
        value["schema"],
        serde_json::json!("highergraphen.case.native_store.record.v2")
    );
    assert_eq!(
        value["nearest_snapshot_path"],
        value["revisions"][0]["snapshot_path"]
    );
    assert!(value["revisions"][1].get("snapshot_path").is_none());
    assert!(record.nearest_snapshot_path.is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn legacy_snapshot_at_every_revision_remains_an_exact_replay_source() {
    let root = temp_root("legacy-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second revision");
    let expected = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay folded second revision")
        .case_space;
    let snapshot_path = store
        .resolve_snapshot_path(
            &store
                .relative_snapshot_path(&case_space.case_space_id, &expected.revision.revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("legacy extra snapshot path");
    write_json_create_new_without_lock_check(&snapshot_path, &expected)
        .expect("write legacy extra snapshot");

    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay exact legacy snapshot");

    assert_eq!(replay.case_space, expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_rejects_reused_revision_without_corrupting_the_store() {
    let root = temp_root("reused-revision");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second revision");

    let first_revision_id = case_space.revision.revision_id.clone();
    let first_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &first_revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("first snapshot path");
    let first_snapshot_before =
        fs::read(&first_snapshot_path).expect("read first snapshot before rejected append");
    let replay = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay second revision");
    let mut entry = replay.history[1].clone();
    entry.sequence = 3;
    entry.entry_id = id("morphism_log_entry:reused-revision");
    entry.morphism_id = id("morphism:reused-revision");
    entry.source_revision_id = Some(replay.current_revision_id.clone());
    entry.target_revision_id = first_revision_id.clone();
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.source_revision_id = entry.source_revision_id.clone();
    entry.morphism.target_revision_id = entry.target_revision_id.clone();
    entry.previous_entry_hash = Some(
        crate::native_hash::morphism_log_entry_hash(
            replay.history.last().expect("current morphism log entry"),
        )
        .expect("current entry hash"),
    );
    entry.replay_checksum.clear();

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("reusing an earlier revision must fail");

    assert!(matches!(error, NativeStoreError::InvalidMorphism { .. }));
    assert!(error.to_string().contains(&format!(
        "target_revision_id {first_revision_id} already exists in the morphism log"
    )));
    assert_eq!(
        fs::read(&first_snapshot_path).expect("read first snapshot after rejected append"),
        first_snapshot_before
    );
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("store remains valid after rejected revision reuse")
            .valid
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_rejects_an_existing_target_snapshot_without_replacing_it() {
    let root = temp_root("existing-target-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let current = append_through_sequence(&store, &case_space.case_space_id, SNAPSHOT_INTERVAL - 1);
    let entry = next_metadata_entry(&current);
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &entry.target_revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("target snapshot path");
    let snapshot_before = b"pre-existing snapshot sentinel\n";
    fs::write(&snapshot_path, snapshot_before).expect("create pre-existing target snapshot");
    let log_path = store.log_path(&case_space.case_space_id);
    let log_before = fs::read(&log_path).expect("read log before rejected append");

    let error = store
        .append_morphism(&case_space.case_space_id, entry.clone())
        .expect_err("an existing target snapshot must reject the append");

    assert!(matches!(error, NativeStoreError::InvalidMorphism { .. }));
    assert!(error
        .to_string()
        .contains(entry.target_revision_id.as_str()));
    assert_eq!(
        fs::read(&snapshot_path).expect("read pre-existing target snapshot"),
        snapshot_before
    );
    assert_eq!(
        fs::read(&log_path).expect("read log after rejected append"),
        log_before
    );
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("store remains valid after snapshot collision")
            .valid
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_refuses_a_forged_unscheduled_snapshot_before_writing_the_log() {
    let root = temp_root("forged-unscheduled-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let entry = metadata_entry(&case_space);
    assert!(!snapshot_required(entry.sequence));
    let genesis_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("genesis snapshot path");
    let forged_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &entry.target_revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("unscheduled target snapshot path");
    fs::copy(&genesis_snapshot_path, &forged_snapshot_path)
        .expect("forge target snapshot from genesis");
    let forged_snapshot_before =
        fs::read(&forged_snapshot_path).expect("read forged snapshot before append");
    let log_path = store.log_path(&case_space.case_space_id);
    let log_before = fs::read(&log_path).expect("read log before refused append");

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("forged unscheduled snapshot must refuse append before mutation");

    assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
    assert!(error.to_string().contains("embedded morphism_log mismatch"));
    assert_eq!(
        fs::read(&log_path).expect("read log after refused append"),
        log_before
    );
    assert_eq!(
        fs::read(&forged_snapshot_path).expect("read forged snapshot after refused append"),
        forged_snapshot_before
    );
    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history after refused append")
            .len(),
        1
    );
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("store remains valid after refused append")
            .valid
    );
    assert_eq!(
        store
            .rebuild_case_space(&case_space.case_space_id)
            .expect("store remains rebuildable after refused append")
            .revisions[0]
            .snapshot_status,
        NativeSnapshotStatus::Agrees
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_head_rejects_a_forged_unsnapshotted_tail() {
    let root = temp_root("forged-log-tail");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append unscheduled tail");
    let mut forged = store
        .history_entries(&case_space.case_space_id)
        .expect("history before forgery");
    forged[1].actor_id = id("actor:forged");
    forged[1].morphism.metadata["operation_gate"]["actor_id"] = serde_json::json!("actor:forged");
    rewrite_history_without_head(&store, &case_space.case_space_id, &forged);

    let history_error = store
        .history_entries(&case_space.case_space_id)
        .expect_err("history must reject forged tail");
    let replay_error = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect_err("replay must reject forged tail");
    let inspect_error = store
        .inspect_case_space(&case_space.case_space_id)
        .expect_err("inspect must reject forged tail");
    let validation_error = store
        .validate_case_space(&case_space.case_space_id)
        .expect_err("validate must reject forged tail");
    let rebuild_error = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect_err("rebuild must reject forged tail");

    for error in [
        history_error,
        replay_error,
        inspect_error,
        validation_error,
        rebuild_error,
    ] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error.to_string().contains("morphism log head"));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_log_head_refuses_replay_validation_and_rebuild() {
    let root = temp_root("missing-log-head");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    fs::remove_file(store.head_path(&case_space.case_space_id)).expect("remove log head");

    let replay_error = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect_err("replay must require log head");
    let validation_error = store
        .validate_case_space(&case_space.case_space_id)
        .expect_err("validation must require log head");
    let rebuild_error = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect_err("rebuild must require log head");

    for error in [replay_error, validation_error, rebuild_error] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error.to_string().contains("morphism log head is required"));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unscheduled_append_refuses_a_log_without_final_newline() {
    let root = temp_root("missing-final-newline");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let log_path = store.log_path(&case_space.case_space_id);
    let mut without_newline = fs::read(&log_path).expect("read morphism log");
    assert_eq!(without_newline.pop(), Some(b'\n'));
    fs::write(&log_path, &without_newline).expect("strip final newline");
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("missing final newline remains readable before append")
            .valid
    );

    let error = store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect_err("append must refuse missing line delimiter");

    assert!(error
        .to_string()
        .contains("morphism log did not end with a newline before append"));
    assert_eq!(
        fs::read(&log_path).expect("read log after refused append"),
        without_newline
    );
    assert!(
        store
            .validate_case_space(&case_space.case_space_id)
            .expect("refused append leaves prior store readable")
            .valid
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn tampered_snapshot_checksum_fails_replay_and_validation() {
    let root = temp_root("tampered-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("snapshot path");
    let mut snapshot = read_json(&snapshot_path);
    snapshot["case_cells"][0]["lifecycle"] = serde_json::json!("retired");
    write_json_value(&snapshot_path, &snapshot);

    let replay_error = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect_err("tampered snapshot must fail replay");
    let validation_error = store
        .validate_case_space(&case_space.case_space_id)
        .expect_err("tampered snapshot must fail validation");
    let rebuild_error = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect_err("tampered snapshot must fail rebuild");

    for error in [replay_error, validation_error, rebuild_error] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error.to_string().contains("snapshot checksum mismatch"));
        assert!(error
            .to_string()
            .contains(snapshot_path.to_str().expect("utf-8 snapshot path")));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn validation_checks_a_tampered_snapshot_older_than_the_replay_source() {
    let root = temp_root("tampered-older-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let second = store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second revision");
    let second_state = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay second revision")
        .case_space;
    let second_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &second.current_revision_id),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("legacy second snapshot path");
    write_json_create_new_without_lock_check(&second_snapshot_path, &second_state)
        .expect("write legacy extra snapshot");
    store
        .append_morphism(
            &case_space.case_space_id,
            next_metadata_entry(&second_state),
        )
        .expect("append revision after replay source");
    let genesis_snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("genesis snapshot path");
    let mut genesis_snapshot = read_json(&genesis_snapshot_path);
    genesis_snapshot["case_cells"][0]["title"] = serde_json::json!("tampered older snapshot");
    write_json_value(&genesis_snapshot_path, &genesis_snapshot);

    store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("newer snapshot remains a valid replay source");
    let validation_error = store
        .validate_case_space(&case_space.case_space_id)
        .expect_err("validation must inspect older snapshot");
    let rebuild_error = store
        .rebuild_case_space(&case_space.case_space_id)
        .expect_err("rebuild must inspect older snapshot");

    for error in [validation_error, rebuild_error] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error
            .to_string()
            .contains(genesis_snapshot_path.to_str().expect("utf-8 path")));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn snapshot_embedded_morphism_log_must_match_external_prefix() {
    let root = temp_root("tampered-embedded-log");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &case_space.revision.revision_id,
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("snapshot path");
    let mut snapshot = read_json(&snapshot_path);
    snapshot["morphism_log"][0]["morphism"]["metadata"]["tampered"] = serde_json::json!(true);
    write_json_value(&snapshot_path, &snapshot);

    let error = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect_err("embedded log disagreement must fail replay");

    assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
    assert!(error.to_string().contains("embedded morphism_log mismatch"));
    assert!(error
        .to_string()
        .contains(snapshot_path.to_str().expect("utf-8 snapshot path")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn import_rejects_existing_case_space_and_preserves_log() {
    let root = temp_root("duplicate-import");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second history entry");
    let log_path = store.log_path(&case_space.case_space_id);
    let log_before = fs::read_to_string(&log_path).expect("read history before duplicate import");
    let entry_count_before = store
        .history_entries(&case_space.case_space_id)
        .expect("history before duplicate import")
        .len();

    let error = store
        .import_case_space(&case_space)
        .expect_err("duplicate import must fail");

    assert!(matches!(error, NativeStoreError::ExistingCase { .. }));
    assert!(error.to_string().contains(
        store
            .case_dir(&case_space.case_space_id)
            .to_str()
            .expect("utf-8 case path")
    ));
    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history after duplicate import")
            .len(),
        entry_count_before
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read history after duplicate import"),
        log_before
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_fails_while_case_lock_is_held_without_corrupting_history() {
    // This waits out LOCK_WAIT_BUDGET on purpose — nothing releases the lock,
    // so the whole budget elapses. Do not "fix" the runtime by shrinking that
    // constant: it is sized to outlast an ordinary append on a large case
    // space, which is the defect it was raised to close.
    let root = temp_root("held-lock");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let lock_path = store.case_dir(&case_space.case_space_id).join(".lock");
    fs::write(&lock_path, "held by test\n").expect("create held lock");

    let error = store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect_err("append must not pass a held lock");

    assert!(matches!(error, NativeStoreError::LockUnavailable { .. }));
    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history after rejected append")
            .len(),
        1
    );
    assert!(!store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(
                &case_space.case_space_id,
                &id("revision:native-contract-v2"),
            ),
            &store.log_path(&case_space.case_space_id),
        )
        .expect("second snapshot path")
        .exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_refuses_rather_than_breaking_an_aged_case_lock() {
    // ADR 0017 / issue #30: the tool never infers a live lock is abandoned
    // from file age alone. This used to be `append_breaks_stale_case_lock`,
    // asserting the opposite — that an aged lock was broken and the append
    // went through — which is exactly the inference the ADR forbids; the
    // fixture encoded the defect, not a requirement.
    let root = temp_root("aged-lock");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let lock_path = store.case_dir(&case_space.case_space_id).join(".lock");
    let lock_contents = "token=forged-aged-lock\n";
    fs::write(&lock_path, lock_contents).expect("create aged lock");
    OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("open aged lock")
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH))
        .expect("age lock far past the old 60s staleness threshold");

    let error = store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect_err("append must refuse rather than break an aged lock");

    assert!(matches!(error, NativeStoreError::LockUnavailable { .. }));
    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history after refused append")
            .len(),
        1
    );
    assert!(
        lock_path.exists(),
        "a refused acquire must leave the lock file in place"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read lock after refusal"),
        lock_contents,
        "a refused acquire must leave the lock file byte-identical"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn lock_guard_drop_does_not_delete_a_successor_lock() {
    let root = temp_root("successor-lock");
    fs::create_dir_all(&root).expect("create lock directory");
    let guard = CaseLockGuard::acquire(&root).expect("acquire original lock");
    let lock_path = root.join(".lock");
    let successor_token = "token=foreign-successor\n";
    fs::remove_file(&lock_path).expect("simulate stale-lock removal");
    fs::write(&lock_path, successor_token).expect("install successor lock token");

    drop(guard);

    assert_eq!(
        fs::read_to_string(&lock_path).expect("successor lock must remain"),
        successor_token
    );
    assert!(matches!(
        CaseLockGuard::acquire(&root),
        Err(NativeStoreError::LockUnavailable { .. })
    ));
    let _ = fs::remove_dir_all(root);
}

/// ADR 0017's 2026-08-02 amendment (F1): a displaced holder refuses instead
/// of writing. This is the scenario an adversarial review reproduced end to
/// end on a real store — two writers, one `rm`, and a hash-chained log
/// corrupted at the same sequence with both writers reporting success —
/// reduced here to its essential shape: acquire the lock exactly as
/// `append_morphism` does, simulate an operator's `rm` immediately followed
/// by a new holder's `create_new` (a foreign token now owns the file), then
/// call `append_verified_log_entry` directly with that guard and confirm the
/// write never happens: the log stays byte-identical.
///
/// Issue #36: `still_owned()` used to be a hand-placed call the caller made
/// before this write; it is now checked by `append_verified_log_entry` itself
/// as soon as it receives the guard, so there is no longer a separate
/// call-site sequence to reproduce here — passing the displaced guard in is
/// enough.
#[test]
fn a_displaced_holder_refuses_instead_of_appending() {
    let root = temp_root("displaced-write");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let case_dir = store.case_dir(&case_space.case_space_id);
    let log_path = store.log_path(&case_space.case_space_id);
    let log_before = fs::read_to_string(&log_path).expect("read log before displacement");

    let guard = CaseLockGuard::acquire(&case_dir).expect("acquire the lock");
    // The act ADR 0017 documents as the recovery procedure, aimed here at a
    // lock that is still live: an operator's `rm` on what they believe is
    // abandoned, immediately followed by a new holder's `create_new`.
    fs::write(case_dir.join(".lock"), "token=foreign-displacement\n")
        .expect("simulate a displacing rm followed by a new holder's create_new");

    let entry = metadata_entry(&case_space);
    let result: NativeStoreResult<u64> = append_verified_log_entry(&guard, &log_path, &entry);

    let error = result.expect_err("a displaced holder must refuse before appending");
    assert!(
        matches!(error, NativeStoreError::LockUnavailable { .. }),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read log after refusal"),
        log_before,
        "a displaced holder's refusal must leave the log untouched — this is the exact write \
         that produced two silent successes and a hash-chained log corrupted at the same \
         sequence before this fix"
    );

    drop(guard);
    let _ = fs::remove_dir_all(root);
}

/// The same F1 fix, on `rebuild_case_space_inner`'s own durable writes — the
/// review that found the gap in `append_morphism`/`import_case_space` also
/// found it here, on the write team-lead named the dangerous one: the
/// repair-lagging-head path *overwrites* the head with a `latest` computed
/// from a log read taken under a lock this process may no longer hold. A
/// displaced rebuild racing a concurrent append could otherwise write a head
/// naming an earlier entry than the log now contains — an untraceable
/// rollback manufactured out of nothing, the exact shape residual risk 2
/// forbids. Same shape as `a_displaced_holder_refuses_instead_of_appending`:
/// acquire the guard exactly as `rebuild_case_space_inner` does, displace it,
/// then call `write_log_head_owned` directly with that guard and confirm the
/// write never happens.
///
/// Issue #36: as above, `still_owned()` moved from a hand-placed call at the
/// call site into `write_log_head_owned` itself, so passing the displaced
/// guard to that function is the whole reproduction.
#[test]
fn a_displaced_holder_refuses_instead_of_overwriting_the_head() {
    let root = temp_root("displaced-head-write");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let case_dir = store.case_dir(&case_space.case_space_id);
    let head_path = store.head_path(&case_space.case_space_id);
    let head_before = fs::read_to_string(&head_path).expect("read head before displacement");

    let guard = CaseLockGuard::acquire(&case_dir).expect("acquire the lock");
    // The act ADR 0017 documents as the recovery procedure, aimed here at a
    // lock that is still live: an operator's `rm` on what they believe is
    // abandoned, immediately followed by a new holder's `create_new`.
    fs::write(case_dir.join(".lock"), "token=foreign-displacement\n")
        .expect("simulate a displacing rm followed by a new holder's create_new");

    let latest = case_space
        .morphism_log
        .last()
        .expect("fixture morphism log")
        .clone();
    let result: NativeStoreResult<()> = write_log_head_owned(&guard, &head_path, &latest);

    let error = result.expect_err("a displaced holder must refuse before overwriting the head");
    assert!(
        matches!(error, NativeStoreError::LockUnavailable { .. }),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        fs::read_to_string(&head_path).expect("read head after refusal"),
        head_before,
        "a displaced holder's refusal must leave the head untouched"
    );

    drop(guard);
    let _ = fs::remove_dir_all(root);
}

/// Pins the actual `append_morphism` call site the invariant-duplication
/// audit's Finding 2 fixed, rather than the hand-assembled
/// `guard.still_owned().and_then(...)` sequence the two tests above
/// exercise. Those tests would stay green even if a future edit deleted the
/// `still_owned()` call from `append_morphism` itself — neither one calls
/// it. This test does, through
/// `arrange_lock_displacement_before_next_still_owned_check` (a deterministic
/// test-only seam, not a real race: a genuinely racy multi-threaded test
/// would be exactly the load-dependent, timing-sensitive kind issue #32 is
/// working to eliminate from this suite).
///
/// The bug this pins: `write_json_create_new(&snapshot_path, &next)` — a
/// durable write — used to run before `lock.still_owned()?` on the
/// snapshot-scheduled branch, so a displaced holder wrote the snapshot and
/// only then refused. Build the case space up to the sequence just before
/// one is scheduled (`snapshot_required`), arm the displacement on the next
/// append's lock file, and confirm `append_morphism` refuses with no
/// snapshot left behind.
#[test]
fn append_morphism_refuses_before_the_scheduled_snapshot_write_when_displaced() {
    let root = temp_root("displaced-append-morphism-snapshot");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");

    let case_space =
        append_through_sequence(&store, &case_space.case_space_id, SNAPSHOT_INTERVAL - 1);
    let case_dir = store.case_dir(&case_space.case_space_id);
    let log_path = store.log_path(&case_space.case_space_id);
    let log_before = fs::read_to_string(&log_path).expect("read log before displaced append");

    let entry = next_metadata_entry(&case_space);
    assert!(
        snapshot_required(entry.sequence),
        "test setup must land on a snapshot-scheduled sequence"
    );
    let snapshot_path = store
        .resolve_snapshot_path(
            &store.relative_snapshot_path(&case_space.case_space_id, &entry.target_revision_id),
            &log_path,
        )
        .expect("resolve scheduled snapshot path");
    assert!(
        !snapshot_path.exists(),
        "test setup must not already carry the scheduled snapshot"
    );

    arrange_lock_displacement_before_next_still_owned_check(case_dir.join(".lock"));

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("a displaced holder must refuse before its scheduled snapshot write");

    assert!(
        matches!(error, NativeStoreError::LockUnavailable { .. }),
        "unexpected error: {error:?}"
    );
    assert!(
        !snapshot_path.exists(),
        "a displaced holder's refusal must leave no snapshot behind — this is the exact \
         durable write Finding 2 found sitting outside the still_owned obligation"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read log after refused append"),
        log_before,
        "a displaced holder's refusal must leave the log untouched"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn import_rejects_unsupported_case_space_schema() {
    let root = temp_root("bad-schema");
    let store = NativeCaseStore::new(root.clone());
    let mut case_space = fixture_space();
    case_space.schema = "highergraphen.case.space.v0".to_owned();

    let error = store
        .import_case_space(&case_space)
        .expect_err("unsupported schema");
    assert!(matches!(error, NativeStoreError::UnsupportedSchema { .. }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn import_rejects_a_genesis_previous_entry_hash() {
    let root = temp_root("genesis-entry-hash");
    let store = NativeCaseStore::new(root.clone());
    let mut case_space = fixture_space();
    case_space.morphism_log[0].previous_entry_hash = Some("0".repeat(64));

    let error = store
        .import_case_space(&case_space)
        .expect_err("genesis previous entry hash");

    assert!(error
        .to_string()
        .contains("genesis log entry morphism_log_entry:genesis-native-contract must not set previous_entry_hash"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn import_rejects_multi_entry_materialized_log_without_partial_write() {
    let root = temp_root("multi-entry-import");
    let store = NativeCaseStore::new(root.clone());
    let mut case_space = fixture_space();
    let mut second_entry = case_space.morphism_log[0].clone();
    second_entry.sequence = 2;
    second_entry.entry_id = id("morphism_log_entry:second");
    second_entry.morphism_id = id("morphism:second");
    second_entry.source_revision_id = Some(case_space.revision.revision_id.clone());
    second_entry.target_revision_id = id("revision:second");
    second_entry.morphism.morphism_id = second_entry.morphism_id.clone();
    second_entry.morphism.source_revision_id = second_entry.source_revision_id.clone();
    second_entry.morphism.target_revision_id = second_entry.target_revision_id.clone();
    case_space.morphism_log.push(second_entry);

    let error = store
        .import_case_space(&case_space)
        .expect_err("multi-entry imports are not materializable without prior snapshots");

    assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
    assert!(!store.log_path(&case_space.case_space_id).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_rejects_malformed_log_sequence() {
    let root = temp_root("bad-history");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let log_path = store.log_path(&case_space.case_space_id);
    let mut bad_entry = case_space.morphism_log[0].clone();
    bad_entry.sequence = 2;
    fs::write(
        &log_path,
        format!(
            "{}\n",
            serde_json::to_string(&bad_entry).expect("serialize bad entry")
        ),
    )
    .expect("rewrite malformed log");

    let error = store
        .history_entries(&case_space.case_space_id)
        .expect_err("malformed history");
    assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_rejects_payload_and_added_id_mismatch() {
    let root = temp_root("bad-append");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let mut entry = metadata_entry(&case_space);
    entry.entry_id = id("morphism_log_entry:unsupported-payload");
    entry.morphism_id = id("morphism:unsupported-payload");
    entry.target_revision_id = id("revision:unsupported-payload");
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.target_revision_id = entry.target_revision_id.clone();
    entry.morphism.added_ids = vec![id("case:not-materialized")];

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("payload and added ids mismatch");
    assert!(matches!(
        error,
        NativeStoreError::InvalidMorphism { ref reason, .. }
            if reason.contains("added_ids") && reason.contains("payload added_cells")
    ));

    let _ = fs::remove_dir_all(root);
}

/// Issue #155: `native_cli::ops::validate_candidate_morphism` now calls this
/// crate's `require_ids_exist` directly against the candidate it builds, so
/// `morphism propose`/`check`/`apply` all refuse a relation whose own
/// `evidence_ids` names a nonexistent id before any gated call. That guard
/// lives in `native_cli`, though, and `NativeCaseStore::append_morphism` is
/// `pub` — a library caller that never goes through the CLI can still reach
/// this append path directly with a hand-built entry, the same reasoning
/// #157's test above this one already established for relation retirement.
/// This constructs exactly that: a relation added by morphism, with a bogus
/// `evidence_ids` entry, appended straight through the store.
#[test]
fn append_still_rejects_a_relation_whose_evidence_id_does_not_exist() {
    let root = temp_root("append-bogus-evidence");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");

    let mut entry = metadata_entry(&case_space);
    entry.entry_id = id("morphism_log_entry:issue155-bogus-evidence");
    entry.morphism_id = id("morphism:issue155-bogus-evidence");
    entry.target_revision_id = id("revision:issue155-bogus-evidence");
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.morphism_type = CaseMorphismType::Relate;
    entry.morphism.target_revision_id = entry.target_revision_id.clone();
    entry.morphism.added_ids = vec![id("relation:issue155-bogus-evidence")];
    entry.morphism.metadata.insert(
        "payload".to_owned(),
        serde_json::json!({"added_relations": [{
            "id": "relation:issue155-bogus-evidence",
            "relation_type": "covers",
            "relation_strength": "soft",
            "from_id": "case:native-contract-example",
            "to_id": "goal:native-case-contract",
            "evidence_ids": ["evidence:does-not-exist"],
            "source_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }]}),
    );
    // The reducer itself does not check a relation's own `evidence_ids` (only
    // `from_id`/`to_id`), so `apply_morphism` accepts this candidate — the
    // checksum below is real, and the refusal this test expects comes from
    // `require_ids_exist`, run after the reducer, not from a mismatched hash.
    let mut expected = case_space.clone();
    apply_morphism(&mut expected, &entry.morphism).expect("reducer accepts the relation");
    expected.morphism_log.push(entry.clone());
    expected.revision = revision_from_entry(&expected.case_space_id, &entry);
    entry.replay_checksum = case_space_checksum(&expected).expect("checksum");

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("a relation's own evidence_ids must resolve too");
    assert!(matches!(
        error,
        NativeStoreError::InvalidMorphism { ref reason, .. }
            if reason.contains("unknown referenced id evidence:does-not-exist")
    ));

    let _ = fs::remove_dir_all(root);
}

/// Issue #156/#159: the append path every durable mutation reaches used to
/// Debug-dump the evaluator's violation list into the refusal message,
/// leaving `NativeStoreError::NotEvaluable`'s structured `violations` field
/// unread. Fixed by constructing `NotEvaluable` directly at this call site
/// instead of folding it into a string first.
///
/// This drove `morphism apply` through the CLI when it was written. #157
/// closed that path: retiring a relation is now refused at `propose`, before
/// a caller ever reaches `apply`, so the CLI can no longer reach the
/// append-time evaluability check this test exercises. That check is not
/// dead, though — `NativeCaseStore::append_morphism` is `pub`, so a library
/// consumer that never goes through the CLI's `propose`/`check`/`apply`
/// sequence can still call it directly with a hand-built entry and land here.
/// This test does exactly that: it drives `NativeCaseStore` and `apply_morphism`
/// (`native_model.rs`) below the CLI's guard, the way such a caller would,
/// rather than through `run_cli`.
///
/// Reaching the append-time evaluator needs a relation the *log* names, so
/// this adds one by morphism and then retires it — retiring a genesis
/// relation trips an earlier reference check inside `apply_morphism` itself
/// and never reaches this one.
#[test]
fn append_reports_an_unevaluable_result_as_data_not_a_debug_dump() {
    let root = temp_root("append-not-evaluable");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");

    // Add a relation the log will name, exactly as `metadata_entry` builds a
    // valid, correctly gated and hash-chained entry — overridden to carry a
    // `relate` payload instead of the metadata-only one it defaults to.
    let mut add_entry = metadata_entry(&case_space);
    add_entry.entry_id = id("morphism_log_entry:issue156-add");
    add_entry.morphism_id = id("morphism:issue156-add");
    add_entry.target_revision_id = id("revision:issue156-added");
    add_entry.morphism.morphism_id = add_entry.morphism_id.clone();
    add_entry.morphism.morphism_type = CaseMorphismType::Relate;
    add_entry.morphism.target_revision_id = add_entry.target_revision_id.clone();
    add_entry.morphism.added_ids = vec![id("relation:issue156-link")];
    add_entry.morphism.metadata.insert(
        "payload".to_owned(),
        serde_json::json!({"added_relations": [{
            "id": "relation:issue156-link",
            "relation_type": "covers",
            "relation_strength": "soft",
            "from_id": "case:native-contract-example",
            "to_id": "goal:native-case-contract",
            "evidence_ids": [], "source_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }]}),
    );
    let mut expected = case_space.clone();
    apply_morphism(&mut expected, &add_entry.morphism).expect("reducer accepts the addition");
    expected.morphism_log.push(add_entry.clone());
    expected.revision = revision_from_entry(&expected.case_space_id, &add_entry);
    add_entry.replay_checksum = case_space_checksum(&expected).expect("checksum after add");
    store
        .append_morphism(&case_space.case_space_id, add_entry)
        .expect("append add-relation morphism");
    let after_add = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay after add");

    // Retire it. `apply_morphism` — the same reducer `append_morphism` itself
    // runs the candidate through to compute the checksum below — accepts this
    // retirement on its own: retiring a relation only removes it from
    // `case_relations`, and nothing else in *this* candidate space still
    // names it. The dangling reference only exists one level up, in the log:
    // `add_entry.morphism.added_ids` still names `relation:issue156-link`
    // after this retirement removes it, which is exactly what
    // `validate_native_case_space` (run after the reducer, inside
    // `append_morphism_with_authority`) refuses.
    let mut retire_entry = next_metadata_entry(&after_add.case_space);
    retire_entry.entry_id = id("morphism_log_entry:issue156-retire");
    retire_entry.morphism_id = id("morphism:issue156-retire");
    retire_entry.target_revision_id = id("revision:issue156-retired");
    retire_entry.morphism.morphism_id = retire_entry.morphism_id.clone();
    retire_entry.morphism.morphism_type = CaseMorphismType::Retire;
    retire_entry.morphism.target_revision_id = retire_entry.target_revision_id.clone();
    retire_entry.morphism.retired_ids = vec![id("relation:issue156-link")];
    let mut next = after_add.case_space.clone();
    apply_morphism(&mut next, &retire_entry.morphism).expect("reducer accepts the retirement");
    next.morphism_log.push(retire_entry.clone());
    next.revision = revision_from_entry(&next.case_space_id, &retire_entry);
    retire_entry.replay_checksum = case_space_checksum(&next).expect("checksum after retire");

    let error = store
        .append_morphism(&case_space.case_space_id, retire_entry)
        .expect_err("retiring a log-referenced relation leaves it not evaluable");
    let NativeStoreError::NotEvaluable { violations, .. } = &error else {
        panic!("expected NotEvaluable, got {error:?}");
    };
    assert!(!violations.is_empty(), "{error:?}");
    for violation in violations {
        assert!(!violation.field.is_empty(), "{error:?}");
        assert!(!violation.message.is_empty(), "{error:?}");
    }
    let message = error.to_string();
    assert!(
        !message.contains("NativeEvalViolation") && !message.contains("Some(Id("),
        "the message must be prose, not a Debug rendering: {message}"
    );
    assert!(
        !message.contains("imported case space"),
        "nothing was imported on the append path: {message}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_rejects_morphism_that_does_not_advance_revision() {
    let root = temp_root("same-revision-append");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let mut entry = metadata_entry(&case_space);
    entry.entry_id = id("morphism_log_entry:same-revision");
    entry.morphism_id = id("morphism:same-revision");
    entry.target_revision_id = case_space.revision.revision_id.clone();
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.target_revision_id = entry.target_revision_id.clone();

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("same revision append");
    assert!(matches!(error, NativeStoreError::InvalidMorphism { .. }));

    let _ = fs::remove_dir_all(root);
}

/// Issue #39: a lost race between two concurrent appenders makes an entry's
/// `source_revision_id` *and* its `sequence` stale together — both were
/// computed from the same now-superseded read. `validate_append` checks
/// `source_revision_id` first specifically so this benign case is
/// classified `stale_revision` (re-read `current_revision_id` and retry),
/// not `store_integrity` (stop and investigate), even though the sequence
/// disagrees too and would have tripped the older check.
#[test]
fn stale_source_revision_is_reported_as_stale_revision_not_store_integrity() {
    let root = temp_root("stale-source-revision");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append second revision");
    let current = store
        .replay_current_case_space(&case_space.case_space_id)
        .expect("replay current case space");

    // Built against the original (genesis) case space, the same way a
    // concurrent writer that read an earlier snapshot would: its
    // source_revision_id (genesis's revision) is stale, and because
    // `next_metadata_entry` derives sequence from that same stale snapshot,
    // its sequence (2) is stale too — the log is already at length 2, so
    // the next slot is 3. Both go stale for the same reason, which is
    // exactly the shape this fix must classify correctly.
    let stale_entry = next_metadata_entry(&case_space);
    assert_ne!(
        stale_entry.source_revision_id.as_ref(),
        Some(&current.current_revision_id),
        "test setup must build an entry whose source_revision_id is genuinely stale"
    );
    assert_ne!(
        stale_entry.sequence,
        current.history.len() as u64 + 1,
        "test setup must build an entry whose sequence is stale too"
    );

    let error = store
        .append_morphism(&case_space.case_space_id, stale_entry)
        .expect_err("a stale source_revision_id must be refused");

    assert_eq!(
        error.error_code(),
        "stale_revision",
        "unexpected error: {error:?}"
    );
    match &error {
        NativeStoreError::StaleSourceRevision {
            current_revision_id,
            ..
        } => {
            assert_eq!(
                *current_revision_id, current.current_revision_id,
                "the recovery datum must name the real current revision"
            );
        }
        other => panic!("expected StaleSourceRevision, got {other:?}"),
    }

    let _ = fs::remove_dir_all(root);
}

/// The other half of issue #39's fix: when `source_revision_id` agrees with
/// current but `sequence` still disagrees, that is not the benign race
/// above — nothing legitimate produces this shape — so it must keep
/// reporting `store_integrity` exactly as before. This is the test that
/// stops the reordering from becoming a blanket downgrade of every append
/// failure to `stale_revision`.
#[test]
fn sequence_disagreement_with_an_agreeing_source_revision_stays_store_integrity() {
    let root = temp_root("genuine-sequence-mismatch");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");

    // source_revision_id correctly names current (nothing has been
    // appended yet); only sequence is wrong.
    let mut entry = metadata_entry(&case_space);
    assert_eq!(
        entry.source_revision_id.as_ref(),
        Some(&case_space.revision.revision_id),
        "test setup must build an entry whose source_revision_id agrees with current"
    );
    entry.sequence = 99;

    let error = store
        .append_morphism(&case_space.case_space_id, entry)
        .expect_err("a genuine sequence disagreement must still be refused");

    assert!(
        matches!(error, NativeStoreError::InvalidMorphism { .. }),
        "unexpected error: {error:?}"
    );
    assert_eq!(error.error_code(), "store_integrity");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_rejects_a_missing_previous_entry_hash() {
    let root = temp_root("missing-entry-hash");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let entry = metadata_entry(&case_space);
    store
        .append_morphism(&case_space.case_space_id, entry)
        .expect("append valid entry");
    let mut history = store
        .history_entries(&case_space.case_space_id)
        .expect("valid history");
    history[1].previous_entry_hash = None;
    rewrite_history(&store, &case_space.case_space_id, &history);

    let error = store
        .history_entries(&case_space.case_space_id)
        .expect_err("missing previous entry hash");

    assert!(error
        .to_string()
        .contains("log entry morphism_log_entry:metadata-only is missing previous_entry_hash"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn history_rejects_a_wrong_previous_entry_hash() {
    let root = temp_root("wrong-entry-hash");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let entry = metadata_entry(&case_space);
    store
        .append_morphism(&case_space.case_space_id, entry)
        .expect("append valid entry");
    let mut history = store
        .history_entries(&case_space.case_space_id)
        .expect("valid history");
    history[1].previous_entry_hash = Some("0".repeat(64));
    rewrite_history(&store, &case_space.case_space_id, &history);

    let error = store
        .history_entries(&case_space.case_space_id)
        .expect_err("wrong previous entry hash");

    assert!(error
        .to_string()
        .contains("log entry morphism_log_entry:metadata-only has previous_entry_hash"));
    let expected = crate::native_hash::morphism_log_entry_hash(&case_space.morphism_log[0])
        .expect("genesis hash");
    assert!(error.to_string().contains(&format!("expected {expected}")));
    let _ = fs::remove_dir_all(root);
}

fn fixture_space() -> CaseSpace {
    let mut case_space: CaseSpace =
        serde_json::from_str(NATIVE_EXAMPLE).expect("native case space example");
    case_space.revision.checksum.clear();
    for entry in &mut case_space.morphism_log {
        entry.replay_checksum.clear();
    }
    let checksum = case_space_checksum(&case_space).expect("fixture checksum");
    case_space.revision.checksum = checksum.clone();
    case_space
        .morphism_log
        .last_mut()
        .expect("fixture morphism log")
        .replay_checksum = checksum;
    case_space
}

fn metadata_entry(case_space: &CaseSpace) -> MorphismLogEntry {
    assert_eq!(case_space.morphism_log.len(), 1);
    next_metadata_entry(case_space)
}

fn next_metadata_entry(case_space: &CaseSpace) -> MorphismLogEntry {
    let sequence = case_space.morphism_log.len() as u64 + 1;
    let mut entry = case_space.morphism_log[0].clone();
    entry.sequence = sequence;
    entry.entry_id = if sequence == 2 {
        id("morphism_log_entry:metadata-only")
    } else {
        id(&format!("morphism_log_entry:metadata-only-{sequence}"))
    };
    entry.morphism_id = if sequence == 2 {
        id("morphism:metadata-only")
    } else {
        id(&format!("morphism:metadata-only-{sequence}"))
    };
    entry.source_revision_id = Some(case_space.revision.revision_id.clone());
    entry.target_revision_id = id(&format!("revision:native-contract-v{sequence}"));
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.morphism_type = CaseMorphismType::Review;
    entry.morphism.source_revision_id = entry.source_revision_id.clone();
    entry.morphism.target_revision_id = entry.target_revision_id.clone();
    entry.morphism.added_ids = Vec::new();
    entry.morphism.updated_ids = Vec::new();
    entry.morphism.retired_ids = Vec::new();
    entry.morphism.preserved_ids = vec![id("goal:native-case-contract")];
    entry.morphism.review_status = ReviewStatus::Reviewed;
    entry.morphism.metadata = serde_json::Map::new();
    entry.morphism.metadata.insert(
        "operation_gate".to_owned(),
        serde_json::json!({
            "actor_id": "actor:native-mutation-cli",
            // A real operation string: the gate now checks it against the
            // capability's `metadata.operations`, so a synthetic one would only
            // be testing the new refusal.
            "operation": "morphism-apply",
            "operation_scope_id": case_space.case_space_id,
            "audience": ProjectionAudience::Audit,
            "capability_ids": ["capability:durable-mutation"],
            "source_boundary_id": "source_boundary:native-case-management-contract"
        }),
    );
    entry.actor_id = id("actor:native-mutation-cli");
    entry.previous_entry_hash = Some(
        crate::native_hash::morphism_log_entry_hash(
            case_space
                .morphism_log
                .last()
                .expect("genesis morphism log entry"),
        )
        .expect("previous entry hash"),
    );
    entry.replay_checksum.clear();

    let mut expected = case_space.clone();
    expected.morphism_log.push(entry.clone());
    expected.revision = revision_from_entry(&expected.case_space_id, &entry);
    entry.replay_checksum = case_space_checksum(&expected).expect("checksum");
    entry
}

fn append_through_sequence(
    store: &NativeCaseStore,
    case_space_id: &Id,
    target_sequence: u64,
) -> CaseSpace {
    loop {
        let current = store
            .replay_current_case_space(case_space_id)
            .expect("replay before append");
        if current.history.len() as u64 == target_sequence {
            return current.case_space;
        }
        assert!(
            current.history.len() as u64 <= target_sequence,
            "target sequence must not precede current history"
        );
        let entry = next_metadata_entry(&current.case_space);
        store
            .append_morphism(case_space_id, entry)
            .expect("append through target sequence");
    }
}

fn rewrite_history(store: &NativeCaseStore, case_space_id: &Id, history: &[MorphismLogEntry]) {
    rewrite_history_without_head(store, case_space_id, history);
    write_log_head_without_lock_check(
        &store.head_path(case_space_id),
        history.last().expect("history head"),
    )
    .expect("rewrite history head");
}

fn rewrite_history_without_head(
    store: &NativeCaseStore,
    case_space_id: &Id,
    history: &[MorphismLogEntry],
) {
    let mut text = history
        .iter()
        .map(|entry| serde_json::to_string(entry).expect("serialize history entry"))
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    fs::write(store.log_path(case_space_id), text).expect("rewrite history");
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read JSON fixture"))
        .expect("parse JSON fixture")
}

fn write_json_value(path: &std::path::Path, value: &serde_json::Value) {
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).expect("serialize JSON fixture")
        ),
    )
    .expect("write JSON fixture");
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time since epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("casegraphen-native-store-{name}-{nanos}"));
    let _ = fs::remove_dir_all(&root);
    root
}

fn id(value: &str) -> Id {
    Id::new(value).expect("fixture id")
}
