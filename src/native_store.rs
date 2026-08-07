use crate::native_model::{
    apply_morphism, apply_morphism_indexed, genesis_case_space_materialization, CaseSpace,
    MorphismApplicationIndex, MorphismLogEntry, MorphismPayload, Revision,
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
// Snapshot cadence trades disk use against maximum fold depth. The hash-chained
// morphism log remains the source of truth; snapshots are only disposable caches.
const SNAPSHOT_INTERVAL: u64 = 32;

pub(crate) fn is_execution_trace_anchor(
    morphism_type: &crate::native_model::CaseMorphismType,
) -> bool {
    matches!(
        morphism_type,
        crate::native_model::CaseMorphismType::Custom(kind) if kind == "execution_trace_anchor"
    )
}

#[cfg(test)]
thread_local! {
    static KNOWN_IDS_CALL_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
        // The evaluator's own contract, checked at the entry point rather than
        // on every later read: its id index covers the revision, entry, and
        // morphism ids that the store's reference checks do not, so a genesis
        // whose cell collides with one of those imports cleanly and then makes
        // every derived command fail permanently, with no repair path.
        crate::native_eval::validate_native_case_space(case_space).map_err(|error| {
            NativeStoreError::NotEvaluable {
                path: self.root.clone(),
                violations: error.violations,
            }
        })?;
        let latest = latest_entry(&case_space.morphism_log, &self.root)?;
        require_snapshot_checksum(&self.root, case_space, latest)?;
        let reconstructed =
            fold_morphism_log(&self.root, &case_space.morphism_log, |_, _, _, _| Ok(()))?;
        if reconstructed != *case_space {
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
        // Checked before acquiring the lock: this reads only whether a log
        // already exists, nothing the lock protects, and the alternative
        // (checking after acquiring) makes an ordinary re-import of an
        // existing case space pay the full `LOCK_WAIT_BUDGET` before
        // reaching a refusal it could have reached immediately.
        let log_path = self.log_path(&case_space.case_space_id);
        if !created_case_dir || log_path.exists() {
            return Err(NativeStoreError::ExistingCase { path: case_dir });
        }
        let lock = CaseLockGuard::acquire(&case_dir)?;

        // Everything past the create_dir is rolled back on failure. Without
        // this, a write error (a too-long snapshot name, ENOSPC, EACCES) leaves
        // a case directory carrying no log: the case-space id is then burned —
        // reimport is refused as already existing — and `space list` fails for
        // every case space in the store.
        let written = self.write_new_case_space(&lock, case_space, &log_path);
        if written.is_err() {
            let _ = fs::remove_dir_all(&case_dir);
        }
        written?;

        self.inspect_case_space(&case_space.case_space_id)
    }

    /// Takes the case lock as a parameter and checks it (ADR 0017's
    /// 2026-08-02 amendment) once at the top, immediately before its first
    /// durable write — the log-file writes below are a single batch on a
    /// case directory nothing else can be racing (this is the only path that
    /// creates it), so they stay covered by this one entry check rather than
    /// re-checking for each one. The snapshot and head writes route through
    /// `write_json_create_new_owned` / `write_log_head_owned` instead of
    /// calling their raw implementations directly (issue #36): those two
    /// functions' unchecked implementations are private, reachable only
    /// through the checked wrapper or a `#[cfg(test)]`-only escape hatch, so
    /// this is the only way any production code can reach them at all.
    fn write_new_case_space(
        &self,
        lock: &CaseLockGuard,
        case_space: &CaseSpace,
        log_path: &Path,
    ) -> NativeStoreResult<()> {
        lock.still_owned()?;
        let snapshots_dir = self.case_dir(&case_space.case_space_id).join("snapshots");
        fs::create_dir_all(&snapshots_dir).map_err(|source| NativeStoreError::Io {
            path: snapshots_dir.clone(),
            source,
        })?;

        let mut snapshot = case_space.clone();
        snapshot.morphism_log = case_space.morphism_log.clone();
        write_json_create_new_owned(
            lock,
            &self.resolve_snapshot_path(
                &self.relative_snapshot_path(
                    &case_space.case_space_id,
                    &case_space.revision.revision_id,
                ),
                log_path,
            )?,
            &snapshot,
        )?;

        fs::write(log_path, "").map_err(|source| NativeStoreError::Io {
            path: log_path.to_path_buf(),
            source,
        })?;
        for entry in &case_space.morphism_log {
            append_json_line(log_path, entry)?;
        }
        write_log_head_owned(
            lock,
            &self.head_path(&case_space.case_space_id),
            latest_entry(&case_space.morphism_log, log_path)?,
        )?;
        Ok(())
    }

    pub fn append_morphism(
        &self,
        case_space_id: &Id,
        entry: MorphismLogEntry,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        self.append_morphism_with_authority(case_space_id, entry, false)
    }

    /// Appends an execution-trace anchor minted by the canonical run path.
    ///
    /// Trace anchors are consumed as proof that CaseGraphen observed a run.
    /// Keeping this method crate-private prevents public store callers from
    /// turning caller-authored morphisms into tool-minted provenance.
    pub(crate) fn append_execution_trace_anchor(
        &self,
        case_space_id: &Id,
        entry: MorphismLogEntry,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        if !is_execution_trace_anchor(&entry.morphism.morphism_type) {
            return Err(invalid_morphism(
                &self.log_path(case_space_id),
                "append_execution_trace_anchor requires custom:execution_trace_anchor",
            ));
        }
        self.append_morphism_with_authority(case_space_id, entry, true)
    }

    fn append_morphism_with_authority(
        &self,
        case_space_id: &Id,
        entry: MorphismLogEntry,
        allow_execution_trace_anchor: bool,
    ) -> NativeStoreResult<NativeCaseSpaceRecord> {
        let case_dir = self.case_dir(case_space_id);
        if !case_dir.is_dir() {
            return Err(NativeStoreError::MissingCase {
                case_space_id: case_space_id.clone(),
                path: self.log_path(case_space_id),
            });
        }
        let lock = CaseLockGuard::acquire(&case_dir)?;
        let replay = self.replay_current_case_space(case_space_id)?;
        let log_path = self.log_path(case_space_id);
        if is_execution_trace_anchor(&entry.morphism.morphism_type) && !allow_execution_trace_anchor
        {
            return Err(invalid_morphism(
                &log_path,
                "custom:execution_trace_anchor is reserved for the canonical run path",
            ));
        }
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
        // The loader enforces this contract on every read; the writer did not,
        // and the two disagreeing is how a store became unreadable. `retire`
        // physically removes a relation, so retiring one that a projection still
        // names left a dangling reference that `require_ids_exist` refuses —
        // after it had been written. Every read path then failed, including
        // `morphism propose`, so nothing in the CLI could repair it, while
        // `space rebuild` still reported success because the fold verifies
        // checksums rather than the materialization contract. Validate the
        // resulting state against the same rule, before anything is written.
        validate_materialized_log(&log_path, &next)?;
        // And against the *whole* loader contract, not the store's narrower
        // reference check. The import path already does this, for the reason
        // stated above it; applying it there and not here left the same hole
        // open on the path every gated mutation takes. Reproduced three ways
        // through ordinary gated commands: an attached evidence cell whose
        // space_id or title the attach path never inspects, and a `retire` of
        // any relation, which dangles the entries that named it. Each was
        // written, each then failed every derived command permanently, and
        // `space validate` reported `valid: true` on all three because the
        // fold verifies checksums rather than this contract.
        // Issue #156: the same structured violations the import path emits.
        // #145 fixed only `import_case_space`, because it was found through
        // `lift`, verified through `lift`, and tested through `lift` — so this
        // site, which every durable mutation reaches, kept Debug-dumping the
        // violation list into the message with `data` left null. This is the
        // costlier of the two: an operator arrives here having already authored
        // a proposal and paid a gated call.
        crate::native_eval::validate_native_case_space(&next).map_err(|error| {
            NativeStoreError::NotEvaluable {
                path: log_path.clone(),
                violations: error.violations,
            }
        })?;

        let snapshot_path = self.resolve_snapshot_path(
            &self.relative_snapshot_path(&next.case_space_id, &next.revision.revision_id),
            &log_path,
        )?;
        if snapshot_required(entry.sequence) {
            require_snapshot_absent(&log_path, &snapshot_path, &entry.target_revision_id)?;
            // Each durable write below takes `lock` itself and checks
            // `still_owned()` immediately before writing (ADR 0017's
            // 2026-08-02 amendment, issue #36) — a displaced holder must
            // never write any of the three, so each checks on its own rather
            // than trusting one earlier check to still hold.
            if let Err(error) = write_json_create_new_owned(&lock, &snapshot_path, &next) {
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
            let previous_log_len = match append_verified_log_entry(&lock, &log_path, &entry) {
                Ok(previous_log_len) => previous_log_len,
                Err(error) => {
                    remove_snapshot_after_failed_append(&log_path, &snapshot_path, &error)?;
                    return Err(error);
                }
            };
            if let Err(error) = write_log_head_owned(&lock, &self.head_path(case_space_id), &entry)
            {
                truncate_after_failed_append(&log_path, previous_log_len, &error)?;
                remove_snapshot_after_failed_append(&log_path, &snapshot_path, &error)?;
                return Err(error);
            }
        } else {
            let target_snapshot_exists =
                require_existing_snapshot_agrees_with_candidate(&snapshot_path, &entry, &next)?;
            let nearest_snapshot_path = if target_snapshot_exists {
                self.relative_snapshot_path(&next.case_space_id, &next.revision.revision_id)
            } else {
                newest_existing_snapshot_path(self, case_space_id, &next.morphism_log)?
                    .unwrap_or_else(|| {
                        self.relative_snapshot_path(&next.case_space_id, &next.revision.revision_id)
                    })
            };
            let previous_log_len = append_verified_log_entry(&lock, &log_path, &entry)?;
            if let Err(error) = write_log_head_owned(&lock, &self.head_path(case_space_id), &entry)
            {
                truncate_after_failed_append(&log_path, previous_log_len, &error)?;
                return Err(error);
            }
            return native_record_from_materialized(
                self,
                case_space_id,
                &next.morphism_log,
                &next,
                Some(nearest_snapshot_path),
            );
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
        let entries = self.read_history_entries(case_space_id)?;
        require_log_head(
            &self.head_path(case_space_id),
            latest_entry(&entries, &self.log_path(case_space_id))?,
        )?;
        Ok(entries)
    }

    fn read_history_entries(&self, case_space_id: &Id) -> NativeStoreResult<Vec<MorphismLogEntry>> {
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
        let (case_space, _) = replay_case_space(self, case_space_id, &entries)?;

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
        let log_path = self.log_path(case_space_id);
        let folded = fold_morphism_log(
            &log_path,
            &replay.history,
            |index, entry, folded, case_space| {
                require_existing_snapshot_agrees_with_fold(
                    self,
                    case_space_id,
                    &log_path,
                    &replay.history,
                    index,
                    entry,
                    folded,
                    case_space,
                )
            },
        )?;
        if folded != replay.case_space {
            return Err(NativeStoreError::ReplayMismatch {
                path: log_path,
                reason: "full log fold disagrees with replayed current case space".to_owned(),
            });
        }
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
        self.rebuild_case_space_inner(case_space_id, false)
    }

    pub fn rebuild_case_space_adopting_existing_log(
        &self,
        case_space_id: &Id,
    ) -> NativeStoreResult<NativeCaseSpaceRebuild> {
        self.rebuild_case_space_inner(case_space_id, true)
    }

    fn rebuild_case_space_inner(
        &self,
        case_space_id: &Id,
        adopt_existing_log: bool,
    ) -> NativeStoreResult<NativeCaseSpaceRebuild> {
        let case_dir = self.case_dir(case_space_id);
        if !case_dir.is_dir() {
            return Err(NativeStoreError::MissingCase {
                case_space_id: case_space_id.clone(),
                path: self.log_path(case_space_id),
            });
        }
        let lock = CaseLockGuard::acquire(&case_dir)?;
        let entries = if adopt_existing_log {
            self.read_history_entries(case_space_id)?
        } else {
            self.history_entries(case_space_id)?
        };
        let log_path = self.log_path(case_space_id);
        let latest = latest_entry(&entries, &log_path)?;
        let head_path = self.head_path(case_space_id);
        // `true` when the head must be written; `Repair` additionally means a
        // head file is already there and this is the crash case.
        let mut repair_lagging_head = false;
        let adopt_log_head = if adopt_existing_log {
            match fs::metadata(&head_path) {
                Ok(_) => match require_log_head(&head_path, latest) {
                    Ok(()) => false,
                    // A head that lags the log is what a crash between the
                    // append and the head write leaves, and it is the one
                    // disagreement that is safe to repair: the log is intact
                    // and the head names an entry still in it. Refusing it
                    // sent an operator who pressed Ctrl-C to delete the head
                    // by hand — the exact primitive residual risk 2 names as
                    // an untraceable rollback, and indistinguishable from one
                    // afterwards. Every other disagreement still refuses.
                    Err(error) => {
                        require_head_lags_log(&head_path, &entries).map_err(|_| error)?;
                        repair_lagging_head = true;
                        true
                    }
                },
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
                Err(source) => {
                    return Err(NativeStoreError::Io {
                        path: head_path,
                        source,
                    });
                }
            }
        } else {
            false
        };
        let mut reports = Vec::with_capacity(entries.len());
        let mut missing = BTreeSet::new();
        fold_morphism_log(&log_path, &entries, |index, entry, revision, case_space| {
            let relative_snapshot_path =
                self.relative_snapshot_path(case_space_id, &entry.target_revision_id);
            let snapshot_path = self.resolve_snapshot_path(&relative_snapshot_path, &log_path)?;
            let snapshot_status = match fs::metadata(&snapshot_path) {
                Ok(_) => {
                    let snapshot =
                        read_verified_snapshot(&snapshot_path, entry, &entries[..=index]).map_err(
                            |error| snapshot_fold_disagreement(&snapshot_path, entry, error),
                        )?;
                    require_snapshot_agrees_with_fold(
                        &snapshot_path,
                        &snapshot,
                        revision,
                        case_space,
                    )?;
                    NativeSnapshotStatus::Agrees
                }
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    if snapshot_required(entry.sequence) {
                        missing.insert(index);
                        NativeSnapshotStatus::Rebuilt
                    } else {
                        NativeSnapshotStatus::NotScheduled
                    }
                }
                Err(source) => {
                    return Err(NativeStoreError::Io {
                        path: snapshot_path,
                        source,
                    });
                }
            };
            reports.push(NativeRebuildRevision {
                revision_id: entry.target_revision_id.clone(),
                sequence: entry.sequence,
                snapshot_path: relative_snapshot_path,
                computed_checksum: revision.computed_checksum.clone(),
                replay_checksum: entry.replay_checksum.clone(),
                snapshot_status,
            });
            Ok(())
        })?;
        if !missing.is_empty() {
            fold_morphism_log(&log_path, &entries, |index, entry, _, case_space| {
                if missing.contains(&index) {
                    let relative_snapshot_path =
                        self.relative_snapshot_path(case_space_id, &entry.target_revision_id);
                    let snapshot_path =
                        self.resolve_snapshot_path(&relative_snapshot_path, &log_path)?;
                    // Checked by `write_json_create_new_owned` itself (ADR
                    // 0017's 2026-08-02 amendment, issue #36): rebuild's
                    // writes are guarded the same as `append_morphism`'s.
                    write_json_create_new_owned(&lock, &snapshot_path, case_space)?;
                }
                Ok(())
            })?;
        }

        if adopt_log_head {
            if repair_lagging_head {
                // The dangerous one: this *overwrites* the head with
                // `latest`, computed from a log read taken under a lock
                // this process may no longer hold. A displaced rebuild
                // racing a concurrent append could otherwise write a head
                // naming an earlier entry than the log now contains — the
                // untraceable rollback residual risk 2 already names as the
                // thing this store must not produce. Checked by
                // `write_log_head_owned` itself (issue #36).
                write_log_head_owned(&lock, &head_path, latest)?;
            } else {
                write_log_head_create_new(&lock, &head_path, latest)?;
            }
        }
        Ok(NativeCaseSpaceRebuild {
            schema: NATIVE_CASE_SPACE_REBUILD_SCHEMA.to_owned(),
            schema_version: NATIVE_STORE_SCHEMA_VERSION,
            case_space_id: case_space_id.clone(),
            current_revision_id: latest.target_revision_id.clone(),
            revision_count: reports.len() as u32,
            head_adopted: adopt_log_head,
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

    fn head_path(&self, case_space_id: &Id) -> PathBuf {
        self.case_dir(case_space_id).join("morphism_log.head.json")
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

fn snapshot_required(sequence: u64) -> bool {
    sequence == 1 || sequence % SNAPSHOT_INTERVAL == 0
}

/// The unchecked atomic head-rewrite implementation. Private: nothing outside
/// this module may call it directly, and within this module only the two
/// functions below do — `write_log_head_owned` (checked, the only path
/// production code has) and, only under `#[cfg(test)]`,
/// `write_log_head_without_lock_check` (the escape hatch `rewrite_history`
/// uses to forge history state directly, bypassing the store's lock
/// entirely). Issue #36: a `pub`-visible raw function sitting next to a
/// checked wrapper is still a call-site obligation with a longer name — a
/// future write path could still compile a call to the raw one. Keeping it
/// private and reaching it only through those two names means a production
/// write path has nothing unchecked left to call.
fn write_log_head_impl(path: &Path, entry: &MorphismLogEntry) -> NativeStoreResult<()> {
    let head = morphism_log_head(path, entry)?;
    let text = serde_json::to_string_pretty(&head).map_err(|source| NativeStoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    let temporary = path.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        entry.sequence
    ));
    fs::write(&temporary, format!("{text}\n")).map_err(|source| NativeStoreError::Io {
        path: temporary.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(NativeStoreError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

/// The lock-checked variant of `write_log_head_impl` (ADR 0017's 2026-08-02
/// amendment, issue #36): confirms the guard still owns the lock immediately
/// before overwriting the head, rather than relying on a hand-placed
/// `lock.still_owned()?` at the call site that a future write path could
/// omit. This is the one team-lead named the dangerous miss: overwriting the
/// head with a `latest` computed under a lock the process may no longer hold
/// is the untraceable-rollback shape residual risk 2 forbids. This is the
/// only way production code can reach the implementation.
fn write_log_head_owned(
    lock: &CaseLockGuard,
    path: &Path,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    lock.still_owned()?;
    write_log_head_impl(path, entry)
}

/// Test-only escape hatch to the unchecked implementation, for
/// `rewrite_history` — a test fixture that forges history state directly,
/// bypassing the store's lock entirely (it never acquired one). `#[cfg(test)]`
/// makes this genuinely unreachable from production code, not merely
/// unlisted in some allowlist: the symbol does not exist in a non-test
/// build, so a new production write path has no unchecked name left to call
/// by mistake.
#[cfg(test)]
fn write_log_head_without_lock_check(
    path: &Path,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    write_log_head_impl(path, entry)
}

/// Every real caller of this one already holds the lock
/// (`rebuild_case_space_inner` adopting a log with no head yet), so — unlike
/// `write_log_head_impl` above — there is no unlocked caller to preserve an
/// escape hatch for. The guard is threaded straight into the signature, and
/// its one durable write routes through `write_json_create_new_owned`
/// (issue #36) rather than the private, unchecked implementation.
fn write_log_head_create_new(
    lock: &CaseLockGuard,
    path: &Path,
    entry: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    let head = morphism_log_head(path, entry)?;
    match write_json_create_new_owned(lock, path, &head) {
        Err(NativeStoreError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            Err(NativeStoreError::ReplayMismatch {
                path: path.to_owned(),
                reason: "morphism log head appeared during adoption; refusing to overwrite it"
                    .to_owned(),
            })
        }
        result => result,
    }
}

fn morphism_log_head(path: &Path, entry: &MorphismLogEntry) -> NativeStoreResult<MorphismLogHead> {
    let entry_hash = crate::native_hash::morphism_log_entry_hash(entry).map_err(|source| {
        NativeStoreError::Json {
            path: path.to_owned(),
            source,
        }
    })?;
    let head = MorphismLogHead {
        target_revision_id: entry.target_revision_id.clone(),
        entry_hash,
        replay_checksum: entry.replay_checksum.clone(),
    };
    Ok(head)
}

/// Accepts exactly one shape of head/log disagreement: a head naming the
/// entry immediately before the tail, and agreeing with it.
///
/// That is what a crash between `append_verified_log_entry` and
/// `write_log_head` leaves, and it is repairable because the log — the source
/// of record — is whole. The distance is part of the signature, not an
/// incidental detail: `append_morphism` takes one entry and holds the case
/// lock across exactly one append and one head write, and no path appends two
/// entries under one lock — `run --step`'s three appends are three separate
/// calls — so a crash can leave the head behind by one entry and never more.
/// Accepting any lag, as this first did, made the condition wider than the
/// only thing that produces it: saving a head, running one `run --step`, and
/// restoring it left a lag of three that the repair blessed.
///
/// A head naming a revision the log no longer contains is the opposite: the
/// log was truncated under it, which is the tail-rollback signature residual
/// risk 2 exists to catch. A head naming a present revision with a different
/// checksum is a rewrite. Both keep refusing.
fn require_head_lags_log(path: &Path, entries: &[MorphismLogEntry]) -> NativeStoreResult<()> {
    let text = fs::read_to_string(path).map_err(|source| NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!("morphism log head is required and could not be read: {source}"),
    })?;
    let head: MorphismLogHead =
        serde_json::from_str(&text).map_err(|source| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!("morphism log head is malformed: {source}"),
        })?;
    let stale = format!(
        "morphism log head at revision {} does not name the entry immediately before this log's tail",
        head.target_revision_id
    );
    let position = entries
        .iter()
        .position(|entry| entry.target_revision_id == head.target_revision_id)
        .ok_or_else(|| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: stale.clone(),
        })?;
    if position + 2 != entries.len() {
        // Either the head names the tail — so it is not lagging at all, and
        // whatever `require_log_head` refused it for stands — or it lags by
        // more than one entry, which no crash produces.
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: stale,
        });
    }
    let entry = &entries[position];
    let expected_hash = crate::native_hash::morphism_log_entry_hash(entry).map_err(|source| {
        NativeStoreError::Json {
            path: path.to_owned(),
            source,
        }
    })?;
    if head.entry_hash != expected_hash || head.replay_checksum != entry.replay_checksum {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "morphism log head at revision {} disagrees with the entry of that revision",
                head.target_revision_id
            ),
        });
    }
    Ok(())
}

fn require_log_head(path: &Path, latest: &MorphismLogEntry) -> NativeStoreResult<()> {
    let text = fs::read_to_string(path).map_err(|source| NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!("morphism log head is required and could not be read: {source}"),
    })?;
    let head: MorphismLogHead =
        serde_json::from_str(&text).map_err(|source| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!("morphism log head is malformed: {source}"),
        })?;
    let expected_hash = crate::native_hash::morphism_log_entry_hash(latest).map_err(|source| {
        NativeStoreError::Json {
            path: path.to_owned(),
            source,
        }
    })?;
    if head.target_revision_id != latest.target_revision_id
        || head.entry_hash != expected_hash
        || head.replay_checksum != latest.replay_checksum
    {
        return Err(NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: format!(
                "morphism log head is stale or disagrees with tail entry {} at revision {}",
                latest.entry_id, latest.target_revision_id
            ),
        });
    }
    Ok(())
}

fn remove_snapshot_after_failed_append(
    log_path: &Path,
    snapshot_path: &Path,
    append_error: &NativeStoreError,
) -> NativeStoreResult<()> {
    match fs::remove_file(snapshot_path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(NativeStoreError::ReplayMismatch {
            path: log_path.to_owned(),
            reason: format!(
                "failed to append morphism log entry ({append_error}); failed to roll back snapshot {}: {source}",
                snapshot_path.display()
            ),
        }),
    }
}

fn replay_case_space(
    store: &NativeCaseStore,
    case_space_id: &Id,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<(CaseSpace, Option<String>)> {
    let log_path = store.log_path(case_space_id);
    let latest = latest_entry(entries, &log_path)?;
    let mut nearest_snapshot = None;

    for index in (0..entries.len()).rev() {
        let entry = &entries[index];
        let relative_snapshot_path =
            store.relative_snapshot_path(case_space_id, &entry.target_revision_id);
        let snapshot_path = store.resolve_snapshot_path(&relative_snapshot_path, &log_path)?;
        match fs::metadata(&snapshot_path) {
            Ok(_) => {
                let case_space = read_verified_snapshot(&snapshot_path, entry, &entries[..=index])?;
                validate_materialized_log(&snapshot_path, &case_space)?;
                nearest_snapshot = Some((case_space, index + 1, relative_snapshot_path));
                break;
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(NativeStoreError::Io {
                    path: snapshot_path,
                    source,
                });
            }
        }
    }

    let (mut case_space, first_unapplied, nearest_snapshot_path, genesis_revision_metadata) =
        match nearest_snapshot {
            Some((case_space, first_unapplied, snapshot_path)) => {
                (case_space, first_unapplied, Some(snapshot_path), None)
            }
            None => {
                let (case_space, revision_metadata) =
                    empty_case_space_from_genesis(&log_path, &entries[0])?;
                (case_space, 0, None, Some(revision_metadata))
            }
        };
    let mut validation = ReplayValidationState::new(&case_space);

    for (index, entry) in entries.iter().enumerate().skip(first_unapplied) {
        apply_replayed_entry(
            &log_path,
            &mut case_space,
            &mut validation,
            entry,
            (index == 0)
                .then_some(genesis_revision_metadata.as_ref())
                .flatten(),
        )?;
    }
    validate_materialized_log(&log_path, &case_space)?;

    // A selected snapshot has already been checksummed by read_verified_snapshot.
    // Only a state that folded beyond it needs one final checksum here.
    if first_unapplied < entries.len() {
        require_replayed_checksum(&log_path, &case_space, latest)?;
    }

    Ok((case_space, nearest_snapshot_path))
}

fn newest_existing_snapshot_path(
    store: &NativeCaseStore,
    case_space_id: &Id,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<Option<String>> {
    let log_path = store.log_path(case_space_id);
    for entry in entries.iter().rev() {
        let relative_snapshot_path =
            store.relative_snapshot_path(case_space_id, &entry.target_revision_id);
        let snapshot_path = store.resolve_snapshot_path(&relative_snapshot_path, &log_path)?;
        match fs::metadata(&snapshot_path) {
            Ok(_) => return Ok(Some(relative_snapshot_path)),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(NativeStoreError::Io {
                    path: snapshot_path,
                    source,
                });
            }
        }
    }
    Ok(None)
}

fn native_record(
    store: &NativeCaseStore,
    case_space_id: &Id,
    entries: &[MorphismLogEntry],
) -> NativeStoreResult<NativeCaseSpaceRecord> {
    require_log_head(
        &store.head_path(case_space_id),
        latest_entry(entries, &store.log_path(case_space_id))?,
    )?;
    let (current_case_space, nearest_snapshot_path) =
        replay_case_space(store, case_space_id, entries)?;
    native_record_from_materialized(
        store,
        case_space_id,
        entries,
        &current_case_space,
        nearest_snapshot_path,
    )
}

fn native_record_from_materialized(
    store: &NativeCaseStore,
    case_space_id: &Id,
    entries: &[MorphismLogEntry],
    current_case_space: &CaseSpace,
    nearest_snapshot_path: Option<String>,
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

    let revisions = entries
        .iter()
        .map(|entry| {
            let relative_snapshot_path =
                store.relative_snapshot_path(case_space_id, &entry.target_revision_id);
            let resolved_snapshot_path = store
                .resolve_snapshot_path(&relative_snapshot_path, &store.log_path(case_space_id))?;
            let snapshot_path = match fs::metadata(&resolved_snapshot_path) {
                Ok(_) => Some(relative_snapshot_path),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
                Err(source) => {
                    return Err(NativeStoreError::Io {
                        path: resolved_snapshot_path,
                        source,
                    });
                }
            };
            Ok(NativeRevisionRecord {
                revision_id: entry.target_revision_id.clone(),
                parent_revision_id: entry.source_revision_id.clone(),
                sequence: entry.sequence,
                entry_id: entry.entry_id.clone(),
                morphism_id: entry.morphism_id.clone(),
                snapshot_path,
                source_ids: entry.source_ids.clone(),
                replay_checksum: entry.replay_checksum.clone(),
            })
        })
        .collect::<NativeStoreResult<Vec<_>>>()?;

    Ok(NativeCaseSpaceRecord {
        schema: NATIVE_CASE_SPACE_RECORD_SCHEMA.to_owned(),
        schema_version: NATIVE_STORE_SCHEMA_VERSION,
        case_space_id: latest.case_space_id.clone(),
        space_id: current_case_space.space_id.clone(),
        current_revision_id: latest.target_revision_id.clone(),
        case_space_directory: format!("{}/{}", NATIVE_DIRECTORY, path_segment(case_space_id)),
        log_path: format!(
            "{}/{}/morphism_log.jsonl",
            NATIVE_DIRECTORY,
            path_segment(case_space_id)
        ),
        nearest_snapshot_path,
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
    // Issue #39: the caller's own assertion about the world — which
    // revision this entry was built against — is checked before the
    // sequence and `previous_entry_hash` checks below. Under a benign lost
    // race between two concurrent appenders (both read the log at the same
    // length, one wins the lock, the other's precomputed entry is now
    // stale), `source_revision_id`, `sequence`, and `previous_entry_hash`
    // all go stale together — they are not three independent signals, they
    // are the same staleness read three different ways. Checking an
    // internal invariant (sequence) first reported the less informative
    // answer: `store_integrity` ("stop, your store may be corrupt") when
    // the truth was `stale_revision` ("re-read current_revision_id and
    // retry"), a one-line recovery. Only once `source_revision_id` agrees
    // does a sequence or hash disagreement mean anything other than this
    // same stale read, and it keeps meaning exactly what it always meant.
    if entry.source_revision_id.as_ref() != Some(&current.revision.revision_id) {
        return Err(NativeStoreError::StaleSourceRevision {
            path: path.to_owned(),
            source_revision_id: entry.source_revision_id.clone(),
            current_revision_id: current.revision.revision_id.clone(),
        });
    }
    if entry.sequence != existing_entries.len() as u64 + 1 {
        return Err(invalid_morphism(
            path,
            format!("entry sequence must be {}", existing_entries.len() + 1),
        ));
    }
    require_previous_entry_hash(path, entry, existing_entries.last())?;
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

fn require_existing_snapshot_agrees_with_candidate(
    snapshot_path: &Path,
    entry: &MorphismLogEntry,
    candidate: &CaseSpace,
) -> NativeStoreResult<bool> {
    match fs::metadata(snapshot_path) {
        Ok(_) => {
            let snapshot = read_verified_snapshot(snapshot_path, entry, &candidate.morphism_log)?;
            validate_materialized_log(snapshot_path, &snapshot)?;
            if snapshot == *candidate {
                return Ok(true);
            }
            Err(NativeStoreError::ReplayMismatch {
                path: snapshot_path.to_owned(),
                reason: format!(
                    "pre-existing snapshot for unscheduled revision {} disagrees with candidate morphism log",
                    entry.target_revision_id
                ),
            })
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
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
    if !morphism.preserved_ids.is_empty() {
        require_referenced_ids_exist(path, case_space, &morphism.preserved_ids)?;
    }
    if !morphism.evidence_ids.is_empty() {
        require_referenced_ids_exist(path, case_space, &morphism.evidence_ids)?;
    }
    Ok(())
}

fn empty_case_space_from_genesis(
    path: &Path,
    genesis: &MorphismLogEntry,
) -> NativeStoreResult<(CaseSpace, Map<String, serde_json::Value>)> {
    let materialization = genesis_case_space_materialization(&genesis.morphism)
        .map_err(|error| invalid_morphism(path, error.to_string()))?;
    let revision_metadata = materialization.revision_metadata;
    let case_space = CaseSpace {
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
    Ok((case_space, revision_metadata))
}

fn apply_replayed_entry(
    path: &Path,
    case_space: &mut CaseSpace,
    validation: &mut ReplayValidationState,
    entry: &MorphismLogEntry,
    revision_metadata: Option<&Map<String, serde_json::Value>>,
) -> NativeStoreResult<()> {
    apply_morphism_indexed(
        case_space,
        &entry.morphism,
        &mut validation.application_index,
    )
    .map_err(|error| NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!(
            "cannot fold morphism {} for revision {}: {error}",
            entry.morphism_id, entry.target_revision_id
        ),
    })?;
    validation.update_materialized_ids(case_space, entry);
    validation.require_entry_references(path, case_space, entry)?;
    let previous_revision_id = case_space.revision.revision_id.clone();
    case_space.morphism_log.push(entry.clone());
    case_space.revision = revision_from_entry(&case_space.case_space_id, entry);
    if let Some(metadata) = revision_metadata {
        case_space.revision.metadata = metadata.clone();
    }
    validation.known_ids.remove(&previous_revision_id);
    validation.known_ids.insert(entry.entry_id.clone());
    validation.known_ids.insert(entry.morphism_id.clone());
    validation
        .known_ids
        .insert(entry.target_revision_id.clone());
    Ok(())
}

struct ReplayValidationState {
    known_ids: BTreeSet<Id>,
    application_index: MorphismApplicationIndex,
}

impl ReplayValidationState {
    fn new(case_space: &CaseSpace) -> Self {
        Self {
            known_ids: known_ids(case_space),
            application_index: MorphismApplicationIndex::new(case_space),
        }
    }

    fn update_materialized_ids(&mut self, case_space: &CaseSpace, entry: &MorphismLogEntry) {
        self.known_ids
            .extend(entry.morphism.added_ids.iter().cloned());
        for retired_id in &entry.morphism.retired_ids {
            let remains_materialized = case_space
                .case_cells
                .iter()
                .any(|cell| &cell.id == retired_id)
                || case_space
                    .case_relations
                    .iter()
                    .any(|relation| &relation.id == retired_id);
            if !remains_materialized {
                self.known_ids.remove(retired_id);
            }
        }
    }

    fn require_entry_references(
        &self,
        path: &Path,
        case_space: &CaseSpace,
        entry: &MorphismLogEntry,
    ) -> NativeStoreResult<()> {
        let morphism = &entry.morphism;
        require_referenced_ids_if_any(path, &self.known_ids, &morphism.preserved_ids)?;
        require_referenced_ids_if_any(path, &self.known_ids, &morphism.evidence_ids)?;

        let payload: MorphismPayload = match morphism.metadata.get("payload") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| invalid_morphism(path, error.to_string()))?,
            None => MorphismPayload::default(),
        };
        for relation in payload
            .added_relations
            .iter()
            .chain(&payload.updated_relations)
        {
            require_referenced_ids(
                path,
                &self.known_ids,
                &[relation.from_id.clone(), relation.to_id.clone()],
            )?;
            require_referenced_ids_if_any(path, &self.known_ids, &relation.evidence_ids)?;
        }
        if entry.sequence == 1 {
            for projection in &case_space.projections {
                require_referenced_ids_if_any(
                    path,
                    &self.known_ids,
                    &projection.represented_cell_ids,
                )?;
                require_referenced_ids_if_any(
                    path,
                    &self.known_ids,
                    &projection.represented_relation_ids,
                )?;
                require_referenced_ids_if_any(path, &self.known_ids, &projection.omitted_cell_ids)?;
                require_referenced_ids_if_any(
                    path,
                    &self.known_ids,
                    &projection.omitted_relation_ids,
                )?;
            }
        }
        Ok(())
    }
}

struct FoldedRevision {
    entry_id: Id,
    revision_id: Id,
    replay_checksum: String,
    computed_checksum: String,
}

fn fold_morphism_log<F>(
    path: &Path,
    entries: &[MorphismLogEntry],
    mut visit: F,
) -> NativeStoreResult<CaseSpace>
where
    F: FnMut(usize, &MorphismLogEntry, &FoldedRevision, &CaseSpace) -> NativeStoreResult<()>,
{
    let genesis = entries
        .first()
        .ok_or_else(|| NativeStoreError::ReplayMismatch {
            path: path.to_owned(),
            reason: "morphism log is empty".to_owned(),
        })?;
    let (mut case_space, genesis_revision_metadata) = empty_case_space_from_genesis(path, genesis)?;
    let mut validation = ReplayValidationState::new(&case_space);

    for (index, entry) in entries.iter().enumerate() {
        apply_replayed_entry(
            path,
            &mut case_space,
            &mut validation,
            entry,
            (index == 0).then_some(&genesis_revision_metadata),
        )?;
        let computed_checksum = case_space_checksum(&case_space)?;
        let folded = FoldedRevision {
            entry_id: entry.entry_id.clone(),
            revision_id: entry.target_revision_id.clone(),
            replay_checksum: entry.replay_checksum.clone(),
            computed_checksum,
        };
        visit(index, entry, &folded, &case_space)?;
        require_fold_checksum(path, &folded)?;
    }
    Ok(case_space)
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

fn require_replayed_checksum(
    path: &Path,
    case_space: &CaseSpace,
    target: &MorphismLogEntry,
) -> NativeStoreResult<()> {
    let computed = case_space_checksum(case_space)?;
    if computed == target.replay_checksum {
        return Ok(());
    }
    Err(NativeStoreError::ReplayMismatch {
        path: path.to_owned(),
        reason: format!(
            "revision {} disagrees with replayed morphism log at entry {}: computed checksum {}, replay_checksum {}",
            target.target_revision_id, target.entry_id, computed, target.replay_checksum
        ),
    })
}

fn require_snapshot_agrees_with_fold(
    path: &Path,
    snapshot: &CaseSpace,
    folded: &FoldedRevision,
    case_space: &CaseSpace,
) -> NativeStoreResult<()> {
    if snapshot == case_space {
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

#[allow(clippy::too_many_arguments)]
fn require_existing_snapshot_agrees_with_fold(
    store: &NativeCaseStore,
    case_space_id: &Id,
    log_path: &Path,
    entries: &[MorphismLogEntry],
    index: usize,
    entry: &MorphismLogEntry,
    folded: &FoldedRevision,
    case_space: &CaseSpace,
) -> NativeStoreResult<()> {
    let relative_snapshot_path =
        store.relative_snapshot_path(case_space_id, &entry.target_revision_id);
    let snapshot_path = store.resolve_snapshot_path(&relative_snapshot_path, log_path)?;
    match fs::metadata(&snapshot_path) {
        Ok(_) => {
            let snapshot = read_verified_snapshot(&snapshot_path, entry, &entries[..=index])
                .map_err(|error| snapshot_fold_disagreement(&snapshot_path, entry, error))?;
            require_snapshot_agrees_with_fold(&snapshot_path, &snapshot, folded, case_space)
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(NativeStoreError::Io {
            path: snapshot_path,
            source,
        }),
    }
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

fn require_referenced_ids_if_any(
    path: &Path,
    ids: &BTreeSet<Id>,
    references: &[Id],
) -> NativeStoreResult<()> {
    if references.is_empty() {
        return Ok(());
    }
    require_referenced_ids(path, ids, references)
}

fn known_ids(case_space: &CaseSpace) -> BTreeSet<Id> {
    #[cfg(test)]
    KNOWN_IDS_CALL_COUNT.with(|count| count.set(count.get() + 1));
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
