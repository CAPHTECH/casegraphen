use crate::native_model::{
    apply_morphism, genesis_case_space_materialization, CaseSpace, MorphismLogEntry, Revision,
    NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION, NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
};
use crate::native_review::{check_operation_gate, NativeOperationGate};
use higher_graphen_core::Id;
use serde_json::Map;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

mod support;
mod types;
use support::*;
pub use types::*;

const NATIVE_DIRECTORY: &str = "native_case_spaces";

impl NativeCaseStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn import_case_space(
        &self,
        case_space: &CaseSpace,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        require_case_space_contract(&self.root, case_space)?;
        require_importable_materialized_log(&self.root, case_space)?;
        validate_materialized_log(&self.root, case_space)?;
        let latest = latest_entry(&case_space.morphism_log, &self.root)?;
        require_snapshot_checksum(&self.root, case_space, latest)?;
        let folded = fold_morphism_log(&self.root, &case_space.morphism_log)?;
        let reconstructed = folded
            .last()
            .expect("importable materialized log is non-empty");
        require_fold_checksum(&self.root, reconstructed)?;
        if reconstructed.case_space != *case_space {
            return Err(NativeStoreError::ReplayMismatch {
                path: self.root.clone(),
                reason: format!(
                    "genesis morphism does not reconstruct imported case space {} at revision {}",
                    case_space.case_space_id, case_space.revision.revision_id
                ),
            });
        }

        let case_dir = self.case_dir(&case_space.case_space_id);
        fs::create_dir_all(self.native_root()).map_err(|source| NativeStoreError::Io {
            path: self.native_root(),
            source,
        })?;
        let created_case_dir = match fs::create_dir(&case_dir) {
            Ok(()) => true,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(source) => {
                return Err(NativeStoreError::Io {
                    path: case_dir,
                    source,
                });
            }
        };
        if !created_case_dir && !case_dir.is_dir() {
            return Err(NativeStoreError::ExistingCase { path: case_dir });
        }
        let _lock = CaseLockGuard::acquire(&case_dir)?;
        let log_path = self.log_path(&case_space.case_space_id);
        if !created_case_dir || log_path.exists() {
            return Err(NativeStoreError::ExistingCase { path: case_dir });
        }

        let snapshots_dir = case_dir.join("snapshots");
        fs::create_dir_all(&snapshots_dir).map_err(|source| NativeStoreError::Io {
            path: snapshots_dir.clone(),
            source,
        })?;

        let mut snapshot = case_space.clone();
        snapshot.morphism_log = case_space.morphism_log.clone();
        write_json_create_new(
            &self.resolve_snapshot_path(
                &self.relative_snapshot_path(
                    &case_space.case_space_id,
                    &case_space.revision.revision_id,
                ),
                &log_path,
            )?,
            &snapshot,
        )?;

        fs::write(&log_path, "").map_err(|source| NativeStoreError::Io {
            path: log_path.clone(),
            source,
        })?;
        for entry in &case_space.morphism_log {
            append_json_line(&log_path, entry)?;
        }

        self.inspect_case_space(&case_space.case_space_id)
    }

    pub fn append_morphism(
        &self,
        case_space_id: &Id,
        entry: MorphismLogEntry,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        let case_dir = self.case_dir(case_space_id);
        if !case_dir.is_dir() {
            return Err(NativeStoreError::MissingCase {
                case_space_id: case_space_id.clone(),
                path: self.log_path(case_space_id),
            });
        }
        let _lock = CaseLockGuard::acquire(&case_dir)?;
        let replay = self.replay_current_case_space(case_space_id)?;
        let log_path = self.log_path(case_space_id);
        require_valid_operation_gate(&log_path, &replay.case_space, &entry)?;
        validate_append(&log_path, &replay.case_space, &entry, &replay.history)?;

        let mut next = replay.case_space;
        apply_bounded_morphism(&log_path, &mut next, &entry)?;
        next.morphism_log.push(entry.clone());
        next.revision = revision_from_entry(&next.case_space_id, &entry);

        let expected_checksum = case_space_checksum(&next)?;
        if entry.replay_checksum != expected_checksum {
            return Err(NativeStoreError::ReplayMismatch {
                path: log_path,
                reason: format!(
                    "entry {} replay_checksum {} does not match computed {}",
                    entry.entry_id, entry.replay_checksum, expected_checksum
                ),
            });
        }

        let snapshot_path = self.resolve_snapshot_path(
            &self.relative_snapshot_path(&next.case_space_id, &next.revision.revision_id),
            &log_path,
        )?;
        require_snapshot_absent(&log_path, &snapshot_path, &entry.target_revision_id)?;
        if let Err(error) = write_json_create_new(&snapshot_path, &next) {
            if matches!(
                &error,
                NativeStoreError::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::AlreadyExists
            ) {
                return Err(snapshot_already_exists(
                    &log_path,
                    &snapshot_path,
                    &entry.target_revision_id,
                ));
            }
            return Err(error);
        }
        if let Err(error) = append_json_line(&log_path, &entry) {
            match fs::remove_file(&snapshot_path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(NativeStoreError::ReplayMismatch {
                        path: log_path,
                        reason: format!(
                            "failed to append morphism log entry ({error}); failed to roll back snapshot {}: {source}",
                            snapshot_path.display()
                        ),
                    });
                }
            }
            return Err(error);
        }
        self.inspect_case_space(case_space_id)
    }

    pub fn list_case_spaces(&self) -> NativeStoreResult<Vec<NativeCaseSpaceRecord>> {
        let root = self.native_root();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(NativeStoreError::Io { path: root, source }),
        };

        let mut directories = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| NativeStoreError::Io {
                path: root.clone(),
                source,
            })?;
            if entry
                .file_type()
                .map_err(|source| NativeStoreError::Io {
                    path: entry.path(),
                    source,
                })?
                .is_dir()
            {
                directories.push(entry.path());
            }
        }
        directories.sort();

        let mut records = Vec::new();
        for directory in directories {
            records.push(self.inspect_directory(&directory)?);
        }
        records.sort_by_key(|record| record.case_space_id.as_str().to_owned());
        Ok(records)
    }

    pub fn inspect_case_space(
        &self,
        case_space_id: &Id,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        let entries = self.history_entries(case_space_id)?;
        native_record(self, case_space_id, &entries)
    }

    pub fn history_entries(&self, case_space_id: &Id) -> NativeStoreResult<Vec<MorphismLogEntry>> {
        let path = self.log_path(case_space_id);
        if !path.exists() {
            return Err(NativeStoreError::MissingCase {
                case_space_id: case_space_id.clone(),
                path,
            });
        }
        let text = fs::read_to_string(&path).map_err(|source| NativeStoreError::Io {
            path: path.clone(),
            source,
        })?;
        let entries = parse_log_entries(&path, &text)?;
        validate_log_entries(Some(case_space_id), &path, &entries)?;
        Ok(entries)
    }

    pub fn replay_current_case_space(
        &self,
        case_space_id: &Id,
    ) -> NativeStoreResult<NativeCaseSpaceReplay> {
        let entries = self.history_entries(case_space_id)?;
        let latest = latest_entry(&entries, &self.log_path(case_space_id))?;
        let snapshot_path = self.resolve_snapshot_path(
            &self.relative_snapshot_path(&latest.case_space_id, &latest.target_revision_id),
            &self.log_path(case_space_id),
        )?;
        let case_space = read_verified_snapshot(&snapshot_path, latest, &entries)?;
        validate_materialized_log(&snapshot_path, &case_space)?;

        Ok(NativeCaseSpaceReplay {
            schema: NATIVE_CASE_SPACE_REPLAY_SCHEMA.to_owned(),
            schema_version: NATIVE_STORE_SCHEMA_VERSION,
            case_space_id: latest.case_space_id.clone(),
            space_id: case_space.space_id.clone(),
            current_revision_id: latest.target_revision_id.clone(),
            case_space,
            history: entries,
        })
    }

    pub fn validate_case_space(
        &self,
        case_space_id: &Id,
    ) -> NativeStoreResult<NativeCaseSpaceValidation> {
        let replay = self.replay_current_case_space(case_space_id)?;
        let folded = fold_morphism_log(&self.log_path(case_space_id), &replay.history)?;
        let current = folded
            .last()
            .expect("validated morphism history is non-empty");
        require_snapshot_agrees_with_fold(
            &self.log_path(case_space_id),
            &replay.case_space,
            current,
        )?;
        require_fold_checksum(&self.log_path(case_space_id), current)?;
        Ok(NativeCaseSpaceValidation {
            schema: NATIVE_CASE_SPACE_VALIDATION_SCHEMA.to_owned(),
            schema_version: NATIVE_STORE_SCHEMA_VERSION,
            case_space_id: replay.case_space_id,
            current_revision_id: replay.current_revision_id,
            history_entry_count: replay.history.len() as u32,
            valid: true,
        })
    }

    pub fn rebuild_case_space(
        &self,
        case_space_id: &Id,
    ) -> NativeStoreResult<NativeCaseSpaceRebuild> {
        let case_dir = self.case_dir(case_space_id);
        if !case_dir.is_dir() {
            return Err(NativeStoreError::MissingCase {
                case_space_id: case_space_id.clone(),
                path: self.log_path(case_space_id),
            });
        }
        let _lock = CaseLockGuard::acquire(&case_dir)?;
        let entries = self.history_entries(case_space_id)?;
        let log_path = self.log_path(case_space_id);
        let folded = fold_morphism_log(&log_path, &entries)?;
        let mut reports = Vec::with_capacity(folded.len());
        let mut missing = Vec::new();

        for (index, revision) in folded.iter().enumerate() {
            let entry = &entries[index];
            let relative_snapshot_path =
                self.relative_snapshot_path(case_space_id, &entry.target_revision_id);
            let snapshot_path = self.resolve_snapshot_path(&relative_snapshot_path, &log_path)?;
            let snapshot_status = match fs::metadata(&snapshot_path) {
                Ok(_) => {
                    let snapshot =
                        read_verified_snapshot(&snapshot_path, entry, &entries[..=index]).map_err(
                            |error| snapshot_fold_disagreement(&snapshot_path, entry, error),
                        )?;
                    require_snapshot_agrees_with_fold(&snapshot_path, &snapshot, revision)?;
                    NativeSnapshotStatus::Agrees
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    missing.push((snapshot_path, revision.case_space.clone()));
                    NativeSnapshotStatus::Rebuilt
                }
                Err(source) => {
                    return Err(NativeStoreError::Io {
                        path: snapshot_path,
                        source,
                    });
                }
            };
            require_fold_checksum(&log_path, revision)?;
            reports.push(NativeRebuildRevision {
                revision_id: entry.target_revision_id.clone(),
                sequence: entry.sequence,
                snapshot_path: relative_snapshot_path,
                computed_checksum: revision.computed_checksum.clone(),
                replay_checksum: entry.replay_checksum.clone(),
                snapshot_status,
            });
        }

        for (snapshot_path, case_space) in missing {
            write_json_create_new(&snapshot_path, &case_space)?;
        }

        let latest = latest_entry(&entries, &log_path)?;
        Ok(NativeCaseSpaceRebuild {
            schema: NATIVE_CASE_SPACE_REBUILD_SCHEMA.to_owned(),
            schema_version: NATIVE_STORE_SCHEMA_VERSION,
            case_space_id: case_space_id.clone(),
            current_revision_id: latest.target_revision_id.clone(),
            revision_count: reports.len() as u32,
            revisions: reports,
        })
    }

    fn inspect_directory(&self, directory: &Path) -> NativeStoreResult<NativeCaseSpaceRecord> {
        let log_path = directory.join("morphism_log.jsonl");
        let text = fs::read_to_string(&log_path).map_err(|source| NativeStoreError::Io {
            path: log_path.clone(),
            source,
        })?;
        let entries = parse_log_entries(&log_path, &text)?;
        validate_log_entries(None, &log_path, &entries)?;
        let latest = latest_entry(&entries, &log_path)?;
        native_record(self, &latest.case_space_id, &entries)
    }

    fn native_root(&self) -> PathBuf {
        self.root.join(NATIVE_DIRECTORY)
    }

    fn case_dir(&self, case_space_id: &Id) -> PathBuf {
        self.native_root().join(path_segment(case_space_id))
    }

    fn log_path(&self, case_space_id: &Id) -> PathBuf {
        self.case_dir(case_space_id).join("morphism_log.jsonl")
    }

    fn relative_snapshot_path(&self, case_space_id: &Id, revision_id: &Id) -> String {
        format!(
            "{}/{}/snapshots/{}.case.space.json",
            NATIVE_DIRECTORY,
            path_segment(case_space_id),
            path_segment(revision_id)
        )
    }

    fn resolve_snapshot_path(
        &self,
        relative_path: &str,
        log_path: &Path,
    ) -> NativeStoreResult<PathBuf> {
        require_relative_store_path(log_path, relative_path)?;
        Ok(self.root.join(relative_path))
    }
}

fn native_record(
    store: &NativeCaseStore,
    case_space_id: &Id,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<NativeCaseSpaceRecord> {
    let latest = latest_entry(entries, &store.native_root())?;
    if &latest.case_space_id != case_space_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: store.native_root(),
            reason: format!(
                "history for {case_space_id} ended with case space {}",
                latest.case_space_id
            ),
        });
    }
    let current_snapshot_path =
        store.relative_snapshot_path(case_space_id, &latest.target_revision_id);
    let resolved_snapshot_path =
        store.resolve_snapshot_path(&current_snapshot_path, &store.log_path(case_space_id))?;
    let current_snapshot = read_verified_snapshot(&resolved_snapshot_path, latest, entries)?;

    let revisions = entries
        .iter()
        .map(|entry| NativeRevisionRecord {
            revision_id: entry.target_revision_id.clone(),
            parent_revision_id: entry.source_revision_id.clone(),
            sequence: entry.sequence,
            entry_id: entry.entry_id.clone(),
            morphism_id: entry.morphism_id.clone(),
            snapshot_path: store.relative_snapshot_path(case_space_id, &entry.target_revision_id),
            source_ids: entry.source_ids.clone(),
            replay_checksum: entry.replay_checksum.clone(),
        })
        .collect::<Vec<_>>();

    Ok(NativeCaseSpaceRecord {
        schema: NATIVE_CASE_SPACE_RECORD_SCHEMA.to_owned(),
        schema_version: NATIVE_STORE_SCHEMA_VERSION,
        case_space_id: latest.case_space_id.clone(),
        space_id: current_snapshot.space_id,
        current_revision_id: latest.target_revision_id.clone(),
        case_space_directory: format!("{}/{}", NATIVE_DIRECTORY, path_segment(case_space_id)),
        log_path: format!(
            "{}/{}/morphism_log.jsonl",
            NATIVE_DIRECTORY,
            path_segment(case_space_id)
        ),
        current_snapshot_path,
        revision_count: revisions.len() as u32,
        history_entry_count: entries.len() as u32,
        revisions,
    })
}

fn validate_append(
    path: &Path,
    current: &CaseSpace,
    entry: &MorphismLogEntry,
    existing_entries: &[MorphismLogEntry],
) -> NativeStoreResult<()> {
    require_log_entry_contract(path, entry)?;
    require_entry_morphism_match(path, entry)?;
    if entry.case_space_id != current.case_space_id {
        return Err(invalid_morphism(
            path,
            format!(
                "entry case_space_id {} does not match {}",
                entry.case_space_id, current.case_space_id
            ),
        ));
    }
    if entry.sequence != existing_entries.len() as u64 + 1 {
        return Err(invalid_morphism(
            path,
            format!("entry sequence must be {}", existing_entries.len() + 1),
        ));
    }
    require_previous_entry_hash(path, entry, existing_entries.last())?;
    if entry.source_revision_id.as_ref() != Some(&current.revision.revision_id) {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "entry source_revision_id {:?} does not match current revision {}",
                entry.source_revision_id, current.revision.revision_id
            ),
        });
    }
    if existing_entries
        .iter()
        .any(|existing| existing.target_revision_id == entry.target_revision_id)
    {
        return Err(invalid_morphism(
            path,
            format!(
                "target_revision_id {} already exists in the morphism log",
                entry.target_revision_id
            ),
        ));
    }
    if existing_entries
        .iter()
        .any(|existing| existing.entry_id == entry.entry_id)
    {
        return Err(invalid_morphism(
            path,
            format!("duplicate log entry {}", entry.entry_id),
        ));
    }
    if existing_entries
        .iter()
        .any(|existing| existing.morphism_id == entry.morphism_id)
    {
        return Err(invalid_morphism(
            path,
            format!("duplicate morphism {}", entry.morphism_id),
        ));
    }
    Ok(())
}

fn require_snapshot_absent(
    log_path: &Path,
    snapshot_path: &Path,
    revision_id: &Id,
) -> NativeStoreResult<()> {
    match fs::metadata(snapshot_path) {
        Ok(_) => Err(snapshot_already_exists(
            log_path,
            snapshot_path,
            revision_id,
        )),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(NativeStoreError::Io {
            path: snapshot_path.to_owned(),
            source,
        }),
    }
}

fn snapshot_already_exists(
    log_path: &Path,
    snapshot_path: &Path,
    revision_id: &Id,
) -> NativeStoreError {
    invalid_morphism(
        log_path,
        format!(
            "snapshot for target_revision_id {revision_id} already exists at {}",
            snapshot_path.display()
        ),
    )
}

fn validate_log_entries(
    expected_case_space_id: Option<&Id>,
    path: &Path,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<()> {
    if entries.is_empty() {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "morphism log is empty".to_owned(),
        });
    }

    let mut seen_entries = BTreeSet::new();
    let mut seen_morphisms = BTreeSet::new();
    let mut previous_revision_id: Option<Id> = None;
    for (index, entry) in entries.iter().enumerate() {
        require_log_entry_contract(path, entry)?;
        require_entry_morphism_match(path, entry)?;
        require_previous_entry_hash(path, entry, index.checked_sub(1).map(|i| &entries[i]))?;
        if let Some(expected_id) = expected_case_space_id {
            if &entry.case_space_id != expected_id {
                return Err(NativeStoreError::ReplayMismatch {
                    path: path.to_owned(),
                    reason: format!(
                        "log entry {} belongs to {}, expected {}",
                        entry.entry_id, entry.case_space_id, expected_id
                    ),
                });
            }
        }
        if entry.sequence != index as u64 + 1 {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "log entry {} has sequence {}, expected {}",
                    entry.entry_id,
                    entry.sequence,
                    index + 1
                ),
            });
        }
        if index == 0 {
            if entry.source_revision_id.is_some() {
                return Err(NativeStoreError::ReplayMismatch {
                    path: path.to_owned(),
                    reason: "first morphism log entry must not set source_revision_id".to_owned(),
                });
            }
        } else if entry.source_revision_id.as_ref() != previous_revision_id.as_ref() {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "log entry {} has source_revision_id {:?}, expected {:?}",
                    entry.entry_id, entry.source_revision_id, previous_revision_id
                ),
            });
        }
        if !seen_entries.insert(entry.entry_id.clone()) {
            return Err(invalid_morphism(
                path,
                format!("duplicate log entry {}", entry.entry_id),
            ));
        }
        if !seen_morphisms.insert(entry.morphism_id.clone()) {
            return Err(invalid_morphism(
                path,
                format!("duplicate morphism {}", entry.morphism_id),
            ));
        }
        previous_revision_id = Some(entry.target_revision_id.clone());
    }
    Ok(())
}

fn require_importable_materialized_log(
    path: &Path,
    case_space: &CaseSpace,
) -> NativeStoreResult<()> {
    if case_space.morphism_log.len() != 1 {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "native import requires a single materialized genesis log entry; append later morphisms through the native store".to_owned(),
        });
    }

    let entry = &case_space.morphism_log[0];
    require_log_entry_contract(path, entry)?;
    require_entry_morphism_match(path, entry)?;
    require_previous_entry_hash(path, entry, None)?;
    if entry.sequence != 1 {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "first morphism log entry has sequence {}, expected 1",
                entry.sequence
            ),
        });
    }
    if entry.source_revision_id.is_some() {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "first morphism log entry must not set source_revision_id".to_owned(),
        });
    }
    Ok(())
}

fn validate_materialized_log(path: &Path, case_space: &CaseSpace) -> NativeStoreResult<()> {
    if case_space.morphism_log.is_empty() {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "case space morphism_log is empty".to_owned(),
        });
    }
    let latest = case_space
        .morphism_log
        .last()
        .expect("empty log checked before latest access");
    if case_space.revision.case_space_id != case_space.case_space_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "revision case_space_id {} does not match {}",
                case_space.revision.case_space_id, case_space.case_space_id
            ),
        });
    }
    if latest.case_space_id != case_space.case_space_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "latest log case_space_id {} does not match {}",
                latest.case_space_id, case_space.case_space_id
            ),
        });
    }
    if latest.target_revision_id != case_space.revision.revision_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "latest log target_revision_id {} does not match revision {}",
                latest.target_revision_id, case_space.revision.revision_id
            ),
        });
    }
    if latest.replay_checksum != case_space.revision.checksum {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "latest replay_checksum {} does not match revision checksum {}",
                latest.replay_checksum, case_space.revision.checksum
            ),
        });
    }
    require_ids_exist(path, case_space)?;
    Ok(())
}

fn apply_bounded_morphism(
    path: &Path,
    case_space: &mut CaseSpace,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    let morphism = &entry.morphism;
    apply_morphism(case_space, morphism)
        .map_err(|error| invalid_morphism(path, error.to_string()))?;
    require_referenced_ids_exist(path, case_space, &morphism.preserved_ids)?;
    require_referenced_ids_exist(path, case_space, &morphism.evidence_ids)?;
    Ok(())
}

struct FoldedRevision {
    entry_id: Id,
    revision_id: Id,
    replay_checksum: String,
    computed_checksum: String,
    case_space: CaseSpace,
}

fn fold_morphism_log(
    path: &Path,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<Vec<FoldedRevision>> {
    let genesis = entries
        .first()
        .ok_or_else(|| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "morphism log is empty".to_owned(),
        })?;
    let materialization = genesis_case_space_materialization(&genesis.morphism)
        .map_err(|error| invalid_morphism(path, error.to_string()))?;
    let genesis_revision_metadata = materialization.revision_metadata;
    let mut case_space = CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: genesis.case_space_id.clone(),
        space_id: materialization.space_id,
        case_cells: Vec::new(),
        case_relations: Vec::new(),
        morphism_log: Vec::new(),
        projections: materialization.projections,
        revision: revision_from_entry(&genesis.case_space_id, genesis),
        close_policy_id: materialization.close_policy_id,
        metadata: materialization.metadata,
    };

    let mut revisions = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        apply_bounded_morphism(path, &mut case_space, entry).map_err(|error| {
            NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "cannot fold morphism {} for revision {}: {error}",
                    entry.morphism_id, entry.target_revision_id
                ),
            }
        })?;
        case_space.morphism_log.push(entry.clone());
        case_space.revision = revision_from_entry(&case_space.case_space_id, entry);
        if index == 0 {
            case_space.revision.metadata = genesis_revision_metadata.clone();
        }
        require_ids_exist(path, &case_space)?;
        let computed_checksum = case_space_checksum(&case_space)?;
        revisions.push(FoldedRevision {
            entry_id: entry.entry_id.clone(),
            revision_id: entry.target_revision_id.clone(),
            replay_checksum: entry.replay_checksum.clone(),
            computed_checksum,
            case_space: case_space.clone(),
        });
    }
    Ok(revisions)
}

fn require_fold_checksum(path: &Path, folded: &FoldedRevision) -> NativeStoreResult<()> {
    if folded.computed_checksum == folded.replay_checksum {
        return Ok(());
    }
    Err(NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!(
            "revision {} disagrees with folded log at entry {}: computed checksum {}, replay_checksum {}",
            folded.revision_id,
            folded.entry_id,
            folded.computed_checksum,
            folded.replay_checksum
        ),
    })
}

fn require_snapshot_agrees_with_fold(
    path: &Path,
    snapshot: &CaseSpace,
    folded: &FoldedRevision,
) -> NativeStoreResult<()> {
    if snapshot == &folded.case_space {
        return Ok(());
    }
    Err(NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!(
            "snapshot for revision {} disagrees with folded morphism log at entry {}",
            folded.revision_id, folded.entry_id
        ),
    })
}

fn snapshot_fold_disagreement(
    path: &Path,
    entry: &MorphismLogEntry,
    error: NativeStoreError,
) -> NativeStoreError {
    NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!(
            "snapshot for revision {} cannot agree with folded morphism log: {error}",
            entry.target_revision_id
        ),
    }
}

fn require_valid_operation_gate(
    path: &Path,
    case_space: &CaseSpace,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    let value = entry
        .morphism
        .metadata
        .get("operation_gate")
        .ok_or_else(|| {
            invalid_morphism(
                path,
                format!(
                    "morphism {} is missing required metadata.operation_gate",
                    entry.morphism_id
                ),
            )
        })?;
    let gate: NativeOperationGate = serde_json::from_value(value.clone()).map_err(|error| {
        invalid_morphism(
            path,
            format!(
                "morphism {} has malformed metadata.operation_gate: {error}",
                entry.morphism_id
            ),
        )
    })?;
    if gate.operation.trim().is_empty() {
        return Err(invalid_morphism(
            path,
            format!(
                "morphism {} metadata.operation_gate.operation must not be empty",
                entry.morphism_id
            ),
        ));
    }
    if entry.actor_id != gate.actor_id {
        return Err(invalid_morphism(
            path,
            format!(
                "morphism {} metadata.operation_gate.actor_id {} does not match log actor_id {}",
                entry.morphism_id, gate.actor_id, entry.actor_id
            ),
        ));
    }
    check_operation_gate(case_space, &gate, &gate.operation).map_err(|error| {
        invalid_morphism(
            path,
            format!(
                "morphism {} has invalid metadata.operation_gate: {error}",
                entry.morphism_id
            ),
        )
    })
}

fn revision_from_entry(case_space_id: &Id, entry: &MorphismLogEntry) -> Revision {
    Revision {
        revision_id: entry.target_revision_id.clone(),
        case_space_id: case_space_id.clone(),
        applied_entry_ids: vec![entry.entry_id.clone()],
        applied_morphism_ids: vec![entry.morphism_id.clone()],
        checksum: entry.replay_checksum.clone(),
        parent_revision_id: entry.source_revision_id.clone(),
        created_at: entry.recorded_at.clone(),
        source_ids: entry.source_ids.clone(),
        metadata: Map::new(),
    }
}

fn require_case_space_contract(path: &Path, case_space: &CaseSpace) -> NativeStoreResult<()> {
    if case_space.schema != NATIVE_CASE_SPACE_SCHEMA {
        return Err(NativeStoreError::UnsupportedSchema {
            path: path.to_owned(),
            actual: case_space.schema.clone(),
            expected: NATIVE_CASE_SPACE_SCHEMA,
        });
    }
    if case_space.schema_version != NATIVE_CASE_SPACE_SCHEMA_VERSION {
        return Err(NativeStoreError::UnsupportedVersion {
            path: path.to_owned(),
            actual: case_space.schema_version,
            expected: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn require_log_entry_contract(path: &Path, entry: &MorphismLogEntry) -> NativeStoreResult<()> {
    if entry.schema != NATIVE_MORPHISM_LOG_ENTRY_SCHEMA {
        return Err(NativeStoreError::UnsupportedSchema {
            path: path.to_owned(),
            actual: entry.schema.clone(),
            expected: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
        });
    }
    if entry.schema_version != NATIVE_CASE_SPACE_SCHEMA_VERSION {
        return Err(NativeStoreError::UnsupportedVersion {
            path: path.to_owned(),
            actual: entry.schema_version,
            expected: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn require_entry_morphism_match(path: &Path, entry: &MorphismLogEntry) -> NativeStoreResult<()> {
    if entry.morphism_id != entry.morphism.morphism_id {
        return Err(invalid_morphism(
            path,
            format!(
                "entry morphism_id {} does not match payload {}",
                entry.morphism_id, entry.morphism.morphism_id
            ),
        ));
    }
    if entry.source_revision_id != entry.morphism.source_revision_id {
        return Err(invalid_morphism(
            path,
            "entry source_revision_id does not match morphism payload",
        ));
    }
    if entry.target_revision_id != entry.morphism.target_revision_id {
        return Err(invalid_morphism(
            path,
            "entry target_revision_id does not match morphism payload",
        ));
    }
    Ok(())
}

fn require_previous_entry_hash(
    path: &Path,
    entry: &MorphismLogEntry,
    previous: Option<&MorphismLogEntry>,
) -> NativeStoreResult<()> {
    let Some(previous) = previous else {
        if entry.previous_entry_hash.is_some() {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "genesis log entry {} must not set previous_entry_hash",
                    entry.entry_id
                ),
            });
        }
        return Ok(());
    };

    let expected = crate::native_hash::morphism_log_entry_hash(previous).map_err(|source| {
        NativeStoreError::Json {
            path: path.to_owned(),
            source,
        }
    })?;
    match entry.previous_entry_hash.as_deref() {
        None => Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "log entry {} is missing previous_entry_hash; expected {} for predecessor {}",
                entry.entry_id, expected, previous.entry_id
            ),
        }),
        Some(actual) if actual != expected => Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "log entry {} has previous_entry_hash {}, expected {} for predecessor {}",
                entry.entry_id, actual, expected, previous.entry_id
            ),
        }),
        Some(_) => Ok(()),
    }
}

fn require_case_space_matches_entry(
    path: &Path,
    case_space: &CaseSpace,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    require_case_space_contract(path, case_space)?;
    if case_space.case_space_id != entry.case_space_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "case_space_id {} does not match log entry {}",
                case_space.case_space_id, entry.case_space_id
            ),
        });
    }
    if case_space.revision.revision_id != entry.target_revision_id {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "revision {} does not match log target {}",
                case_space.revision.revision_id, entry.target_revision_id
            ),
        });
    }
    if case_space.revision.checksum != entry.replay_checksum {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "revision checksum {} does not match replay checksum {}",
                case_space.revision.checksum, entry.replay_checksum
            ),
        });
    }
    Ok(())
}

fn require_snapshot_checksum(
    path: &Path,
    case_space: &CaseSpace,
    authoritative_entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    let computed = snapshot_checksum(path, case_space)?;
    require_computed_snapshot_checksum(path, case_space, authoritative_entry, &computed)
}

fn snapshot_checksum(path: &Path, case_space: &CaseSpace) -> NativeStoreResult<String> {
    crate::native_hash::case_space_checksum(case_space).map_err(|source| NativeStoreError::Json {
        path: path.to_owned(),
        source,
    })
}

fn require_computed_snapshot_checksum(
    path: &Path,
    case_space: &CaseSpace,
    authoritative_entry: &MorphismLogEntry,
    computed: &str,
) -> NativeStoreResult<()> {
    if computed != case_space.revision.checksum || computed != authoritative_entry.replay_checksum {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "snapshot checksum mismatch: computed {computed}, stored revision.checksum {}, authoritative replay_checksum {}",
                case_space.revision.checksum, authoritative_entry.replay_checksum
            ),
        });
    }
    Ok(())
}

fn require_embedded_log_matches_prefix(
    path: &Path,
    embedded: &[MorphismLogEntry],
    authoritative_prefix: &[MorphismLogEntry],
) -> NativeStoreResult<()> {
    if embedded.len() != authoritative_prefix.len() {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "embedded morphism_log mismatch: snapshot has {} entries, authoritative prefix has {}",
                embedded.len(),
                authoritative_prefix.len()
            ),
        });
    }
    for (index, (embedded_entry, authoritative_entry)) in
        embedded.iter().zip(authoritative_prefix).enumerate()
    {
        if embedded_entry.entry_id != authoritative_entry.entry_id {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "embedded morphism_log mismatch at sequence {}: entry id {} does not match authoritative {}",
                    index + 1,
                    embedded_entry.entry_id,
                    authoritative_entry.entry_id
                ),
            });
        }
        let embedded_hash =
            crate::native_hash::morphism_log_entry_hash(embedded_entry).map_err(|source| {
                NativeStoreError::Json {
                    path: path.to_owned(),
                    source,
                }
            })?;
        let authoritative_hash = crate::native_hash::morphism_log_entry_hash(authoritative_entry)
            .map_err(|source| NativeStoreError::Json {
            path: path.to_owned(),
            source,
        })?;
        if embedded_hash != authoritative_hash {
            return Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: format!(
                    "embedded morphism_log mismatch at entry {}: hash {embedded_hash} does not match authoritative {authoritative_hash}",
                    embedded_entry.entry_id
                ),
            });
        }
    }
    Ok(())
}

fn require_ids_exist(path: &Path, case_space: &CaseSpace) -> NativeStoreResult<()> {
    let ids = known_ids(case_space);
    for relation in &case_space.case_relations {
        require_referenced_ids(
            path,
            &ids,
            &[relation.from_id.clone(), relation.to_id.clone()],
        )?;
        require_referenced_ids(path, &ids, &relation.evidence_ids)?;
    }
    for projection in &case_space.projections {
        require_referenced_ids(path, &ids, &projection.represented_cell_ids)?;
        require_referenced_ids(path, &ids, &projection.represented_relation_ids)?;
        require_referenced_ids(path, &ids, &projection.omitted_cell_ids)?;
        require_referenced_ids(path, &ids, &projection.omitted_relation_ids)?;
    }
    Ok(())
}

fn require_referenced_ids_exist(
    path: &Path,
    case_space: &CaseSpace,
    references: &[Id],
) -> NativeStoreResult<()> {
    require_referenced_ids(path, &known_ids(case_space), references)
}

fn require_referenced_ids(
    path: &Path,
    ids: &BTreeSet<Id>,
    references: &[Id],
) -> NativeStoreResult<()> {
    for id in references {
        if !ids.contains(id) {
            return Err(invalid_morphism(
                path,
                format!("unknown referenced id {id}"),
            ));
        }
    }
    Ok(())
}

fn known_ids(case_space: &CaseSpace) -> BTreeSet<Id> {
    case_space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            case_space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .chain(
            case_space
                .projections
                .iter()
                .map(|projection| projection.projection_id.clone()),
        )
        .chain(
            case_space
                .morphism_log
                .iter()
                .flat_map(|entry| [entry.entry_id.clone(), entry.morphism_id.clone()]),
        )
        .chain([case_space.revision.revision_id.clone()])
        .collect()
}

fn read_verified_snapshot(
    path: &Path,
    authoritative_entry: &MorphismLogEntry,
    authoritative_prefix: &[MorphismLogEntry],
) -> NativeStoreResult<CaseSpace> {
    let text = fs::read_to_string(path).map_err(|source| NativeStoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    let case_space: CaseSpace =
        serde_json::from_str(&text).map_err(|source| NativeStoreError::Json {
            path: path.to_owned(),
            source,
        })?;
    require_case_space_contract(path, &case_space)?;
    let computed_checksum = snapshot_checksum(path, &case_space)?;
    require_embedded_log_matches_prefix(path, &case_space.morphism_log, authoritative_prefix)?;
    require_computed_snapshot_checksum(path, &case_space, authoritative_entry, &computed_checksum)?;
    require_case_space_matches_entry(path, &case_space, authoritative_entry)?;
    Ok(case_space)
}

#[cfg(test)]
mod tests;
