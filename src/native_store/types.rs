use crate::native_model::{CaseSpace, MorphismLogEntry};
use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type NativeStoreResult<T> = Result<T, NativeStoreError>;

pub const NATIVE_CASE_SPACE_RECORD_SCHEMA: &str = "highergraphen.case.native_store.record.v2";
pub const NATIVE_CASE_SPACE_REPLAY_SCHEMA: &str = "highergraphen.case.native_store.replay.v1";
pub const NATIVE_CASE_SPACE_REBUILD_SCHEMA: &str = "highergraphen.case.native_store.rebuild.v1";
pub const NATIVE_CASE_SPACE_VALIDATION_SCHEMA: &str =
    "highergraphen.case.native_store.validation.v1";
pub const NATIVE_STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub enum NativeStoreError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedSchema {
        path: PathBuf,
        actual: String,
        expected: &'static str,
    },
    UnsupportedVersion {
        path: PathBuf,
        actual: u32,
        expected: u32,
    },
    MissingCase {
        case_space_id: Id,
        path: PathBuf,
    },
    ExistingCase {
        path: PathBuf,
    },
    LockUnavailable {
        path: PathBuf,
        reason: String,
    },
    ReplayMismatch {
        path: PathBuf,
        reason: String,
    },
    InvalidMorphism {
        path: PathBuf,
        reason: String,
    },
    /// A durable-write entry's `source_revision_id` — what its caller
    /// asserted the case space's current revision was when the entry was
    /// built — no longer names `current_revision_id`. Issue #39: kept
    /// distinct from `ReplayMismatch`/`InvalidMorphism` (`store_integrity`,
    /// "stop and check your store") even though all three can fire from the
    /// same benign lost race between two concurrent appenders — a
    /// disagreeing `source_revision_id` means the caller's own basis for
    /// this write moved, which is `stale_revision`'s "re-read
    /// `current_revision_id` and retry", not a store-corruption signal.
    /// `validate_append` (`src/native_store.rs`) checks this before the
    /// sequence and `previous_entry_hash` checks specifically so this
    /// classification wins the race against the internal-invariant checks
    /// that go stale for the same underlying reason.
    StaleSourceRevision {
        path: PathBuf,
        source_revision_id: Option<Id>,
        current_revision_id: Id,
    },
}

pub struct NativeCaseStore {
    pub(crate) root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCaseSpaceRecord {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub space_id: Id,
    pub current_revision_id: Id,
    pub case_space_directory: String,
    pub log_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_snapshot_path: Option<String>,
    pub revision_count: u32,
    pub history_entry_count: u32,
    pub revisions: Vec<NativeRevisionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRevisionRecord {
    pub revision_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<Id>,
    pub sequence: u64,
    pub entry_id: Id,
    pub morphism_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
    pub source_ids: Vec<Id>,
    pub replay_checksum: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCaseSpaceReplay {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub space_id: Id,
    pub current_revision_id: Id,
    pub case_space: CaseSpace,
    // Folding state, not report content: these are the same entries as
    // `case_space.morphism_log`, and `space history` is the command that
    // answers for the log alone (ADR 0011).
    #[serde(skip_serializing)]
    pub history: Vec<MorphismLogEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCaseSpaceRebuild {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub current_revision_id: Id,
    pub revision_count: u32,
    pub head_adopted: bool,
    pub revisions: Vec<NativeRebuildRevision>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRebuildRevision {
    pub revision_id: Id,
    pub sequence: u64,
    pub snapshot_path: String,
    pub computed_checksum: String,
    pub replay_checksum: String,
    pub snapshot_status: NativeSnapshotStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSnapshotStatus {
    Agrees,
    Rebuilt,
    NotScheduled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCaseSpaceValidation {
    pub schema: String,
    pub schema_version: u32,
    pub case_space_id: Id,
    pub current_revision_id: Id,
    pub history_entry_count: u32,
    pub valid: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MorphismLogHead {
    pub target_revision_id: Id,
    pub entry_hash: String,
    pub replay_checksum: String,
}

impl NativeStoreError {
    /// The stable, machine-readable classification for this refusal.
    ///
    /// Derived from the variant alone — never from `to_string()` — so the
    /// mapping cannot drift from what the variant already distinguishes.
    /// `ReplayMismatch` and `InvalidMorphism` share `store_integrity`: both
    /// mean the on-disk log disagrees with what replaying it (or the
    /// checksum it carries) produces, which calls for the same response
    /// (stop and investigate, do not retry). `UnsupportedSchema` and
    /// `UnsupportedVersion` share `unsupported_schema` for the same reason:
    /// both mean this build cannot read the file's declared shape. `Io` and
    /// `Json` share `store_io`: a lower-level filesystem or parse failure
    /// reading or writing a store file, distinct from the higher-level
    /// integrity and shape checks above it. `StaleSourceRevision` gets its
    /// own `stale_revision` code (issue #39) rather than joining
    /// `ReplayMismatch`/`InvalidMorphism`: it can fire from the same benign
    /// lost race that would otherwise trip those two, but it means the
    /// caller's own basis for the write moved, not that the store disagrees
    /// with itself — the same distinction `NativeCliError::StaleRevision`
    /// already draws one layer up, for the CLI's own `--base-revision`.
    pub(crate) fn error_code(&self) -> &'static str {
        match self {
            Self::Io { .. } | Self::Json { .. } => "store_io",
            Self::UnsupportedSchema { .. } | Self::UnsupportedVersion { .. } => {
                "unsupported_schema"
            }
            Self::MissingCase { .. } => "missing_case_space",
            Self::ExistingCase { .. } => "existing_case_space",
            Self::LockUnavailable { .. } => "lock_unavailable",
            Self::ReplayMismatch { .. } | Self::InvalidMorphism { .. } => "store_integrity",
            Self::StaleSourceRevision { .. } => "stale_revision",
        }
    }
}

impl std::fmt::Display for NativeStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Json { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::UnsupportedSchema {
                path,
                actual,
                expected,
            } => write!(
                formatter,
                "{}: unsupported schema {actual:?}; expected {expected:?}",
                path.display()
            ),
            Self::UnsupportedVersion {
                path,
                actual,
                expected,
            } => write!(
                formatter,
                "{}: unsupported schema version {actual}; expected {expected}",
                path.display()
            ),
            Self::MissingCase {
                case_space_id,
                path,
            } => write!(
                formatter,
                "{}: missing native case space {case_space_id}",
                path.display()
            ),
            Self::ExistingCase { path } => {
                write!(
                    formatter,
                    "{}: native case space already exists",
                    path.display()
                )
            }
            Self::LockUnavailable { path, reason } => {
                write!(formatter, "{}: {reason}", path.display())
            }
            Self::ReplayMismatch { path, reason } => {
                write!(formatter, "{}: {reason}", path.display())
            }
            Self::InvalidMorphism { path, reason } => {
                write!(formatter, "{}: {reason}", path.display())
            }
            Self::StaleSourceRevision {
                path,
                source_revision_id,
                current_revision_id,
            } => write!(
                formatter,
                "{}: entry source_revision_id {source_revision_id:?} does not match current \
                 revision {current_revision_id}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for NativeStoreError {}
