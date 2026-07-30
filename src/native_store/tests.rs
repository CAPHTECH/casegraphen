use super::*;
use crate::native_model::{CaseMorphismType, CaseSpace};
use higher_graphen_core::{Id, ReviewStatus};
use std::{
    fs::{self, FileTimes, OpenOptions},
    path::PathBuf,
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

    for error in [replay_error, validation_error] {
        assert!(matches!(error, NativeStoreError::ReplayMismatch { .. }));
        assert!(error.to_string().contains("snapshot checksum mismatch"));
        assert!(error
            .to_string()
            .contains(snapshot_path.to_str().expect("utf-8 snapshot path")));
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
fn append_breaks_stale_case_lock() {
    let root = temp_root("stale-lock");
    let store = NativeCaseStore::new(root.clone());
    let case_space = fixture_space();
    store
        .import_case_space(&case_space)
        .expect("import native case space");
    let lock_path = store.case_dir(&case_space.case_space_id).join(".lock");
    fs::write(&lock_path, "stale lock\n").expect("create stale lock");
    OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("open stale lock")
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH))
        .expect("age stale lock");

    store
        .append_morphism(&case_space.case_space_id, metadata_entry(&case_space))
        .expect("append after breaking stale lock");

    assert_eq!(
        store
            .history_entries(&case_space.case_space_id)
            .expect("history after stale lock")
            .len(),
        2
    );
    assert!(!lock_path.exists());
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
    let mut entry = case_space.morphism_log[0].clone();
    entry.sequence = 2;
    entry.entry_id = id("morphism_log_entry:metadata-only");
    entry.morphism_id = id("morphism:metadata-only");
    entry.source_revision_id = Some(case_space.revision.revision_id.clone());
    entry.target_revision_id = id("revision:native-contract-v2");
    entry.morphism.morphism_id = entry.morphism_id.clone();
    entry.morphism.morphism_type = CaseMorphismType::Review;
    entry.morphism.source_revision_id = entry.source_revision_id.clone();
    entry.morphism.target_revision_id = entry.target_revision_id.clone();
    entry.morphism.added_ids = Vec::new();
    entry.morphism.updated_ids = Vec::new();
    entry.morphism.retired_ids = Vec::new();
    entry.morphism.preserved_ids = vec![id("goal:native-case-contract")];
    entry.morphism.review_status = ReviewStatus::Reviewed;
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

fn rewrite_history(store: &NativeCaseStore, case_space_id: &Id, history: &[MorphismLogEntry]) {
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
