//! Durable atomic allocator built on append-only, create-new journal events.
//!
//! The pure compatibility/conflict rules remain in [`crate::resource_protocol`].
//! This module owns only serialization, durable replay, idempotency, and the
//! canonical active-reservation snapshot used by an operational host.

use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
    graph_compiler::ReviewedDeploymentAuthority,
    native_hash::sha256_hex,
    resource_protocol::{
        grant_topology_reservation, reservation_is_active, RateLimitCapacity,
        ReservationAssertionKind, ReservationDispositionAssertion, ResourceDeclaration,
        ResourceReservation,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMPORARY_NONCE: AtomicU64 = AtomicU64::new(0);

pub const RESOURCE_ALLOCATOR_CONFIGURATION_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_configuration.v0";
pub const RESOURCE_ALLOCATOR_EVENT_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_event.v0";
pub const RESOURCE_ALLOCATOR_CHECKPOINT_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_checkpoint.v0";
pub const RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_retention_policy.v0";
pub const RESOURCE_ALLOCATOR_COMPACTION_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_compaction.v0";
pub const REVIEWED_DEPLOYMENT_RESERVATION_BINDING_SCHEMA: &str =
    "casegraphen.experimental.resource.reviewed_deployment_binding.v0";

const ALLOCATOR_IDENTITY_SCHEMA: &str = "casegraphen.experimental.resource.allocator_identity.v0";
const IDENTITY_FILE: &str = ".allocator-identity";
const CHECKPOINT_DIRECTORY: &str = "checkpoints";
const ARCHIVE_DIRECTORY: &str = "archive";
const COMPACTION_DIRECTORY: &str = "compactions";

pub fn resource_declaration_content_hash(declaration: &ResourceDeclaration) -> String {
    digest(declaration)
}

/// Durable projection of an opaque reviewed-deployment authority. Callers may
/// read this audit record, but only the graph compiler can mint the proof from
/// which an operational reservation creates it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedDeploymentReservationBinding {
    pub schema: String,
    pub schema_version: u32,
    pub claim_cell_id: String,
    pub accepted_review_id: String,
    pub reviewed_topology_hash: String,
    pub policy_manifest_hash: String,
    pub deployment_bundle_hash: String,
    pub accepted_review_revision: String,
    pub case_space_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub resource_declaration_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorConfiguration {
    pub schema: String,
    pub schema_version: u32,
    pub capacities: Vec<RateLimitCapacity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorCheckpointState {
    pub reservations: Vec<ResourceReservation>,
    pub declarations: BTreeMap<String, ResourceDeclaration>,
    pub dispositions: Vec<ReservationDispositionAssertion>,
    pub events_by_idempotency: BTreeMap<String, ResourceAllocatorEvent>,
}

/// Accelerator derived from an exact journal prefix. The event journal and
/// its content-addressed archive remain the audit authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorCheckpoint {
    pub schema: String,
    pub schema_version: u32,
    pub allocator_instance_id: String,
    pub configuration_hash: String,
    pub covered_event_count: u64,
    pub last_event_sequence: u64,
    pub terminal_event_hash: String,
    pub covered_journal_prefix_hash: String,
    pub state: ResourceAllocatorCheckpointState,
    pub checkpoint_content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorRetentionPolicy {
    pub schema: String,
    pub schema_version: u32,
    pub retain_active_event_count: u64,
}

/// Refuse unsupported retention-policy wire values before host startup.
pub fn validate_resource_allocator_retention_policy(
    policy: &ResourceAllocatorRetentionPolicy,
) -> Result<(), ResourceAllocatorError> {
    validate_retention_policy(policy)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorCompactionRecord {
    pub schema: String,
    pub schema_version: u32,
    pub allocator_instance_id: String,
    pub configuration_hash: String,
    pub checkpoint_content_hash: String,
    pub checkpoint_sequence: u64,
    pub retention_policy: ResourceAllocatorRetentionPolicy,
    pub retention_policy_hash: String,
    pub archived_event_sequences: Vec<u64>,
    pub archived_event_hashes: Vec<String>,
    pub compaction_content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResourceAllocatorCompactionOutcome {
    pub record: ResourceAllocatorCompactionRecord,
    pub archived_event_count: u64,
    pub active_event_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceAllocatorIdentity {
    schema: String,
    schema_version: u32,
    allocator_instance_id: String,
    journal_location_hash: String,
    identity_content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceAllocatorEventPayload {
    Reserve {
        topology_content_hash: String,
        base_revision_id: String,
        #[serde(default)]
        reviewed_deployment: Option<ReviewedDeploymentReservationBinding>,
        declaration: ResourceDeclaration,
        reservation: ResourceReservation,
    },
    Disposition {
        base_revision_id: String,
        #[serde(default)]
        reviewed_deployment: Option<ReviewedDeploymentReservationBinding>,
        assertion: ReservationDispositionAssertion,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorEvent {
    pub schema: String,
    pub schema_version: u32,
    pub sequence: u64,
    pub prior_event_hash: Option<String>,
    pub idempotency_key: String,
    pub operation_digest: String,
    pub event_hash: String,
    pub payload: ResourceAllocatorEventPayload,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResourceAllocatorSnapshot {
    pub generation: u64,
    pub last_event_hash: Option<String>,
    pub active_reservations: Vec<ResourceReservation>,
    pub dispositions: Vec<ReservationDispositionAssertion>,
    pub capacities: Vec<RateLimitCapacity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResourceAllocatorOutcome {
    pub replayed: bool,
    pub event: ResourceAllocatorEvent,
    pub snapshot: ResourceAllocatorSnapshot,
}

#[derive(Debug)]
pub enum ResourceAllocatorError {
    Io(io::Error),
    InvalidConfiguration(String),
    Integrity(String),
    IdempotencyCollision,
    Refused(serde_json::Value),
}

impl std::fmt::Display for ResourceAllocatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidConfiguration(detail) | Self::Integrity(detail) => {
                formatter.write_str(detail)
            }
            Self::IdempotencyCollision => {
                formatter.write_str("idempotency key names different allocator content")
            }
            Self::Refused(findings) => write!(formatter, "resource allocation refused: {findings}"),
        }
    }
}

impl std::error::Error for ResourceAllocatorError {}

impl From<io::Error> for ResourceAllocatorError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
struct ReplayState {
    reservations: Vec<ResourceReservation>,
    declarations: BTreeMap<String, ResourceDeclaration>,
    dispositions: Vec<ReservationDispositionAssertion>,
    events_by_idempotency: BTreeMap<String, ResourceAllocatorEvent>,
    generation: u64,
    last_event_hash: Option<String>,
}

/// Opaque evidence that one checkpoint was compared with a complete replay of
/// the authoritative active and archived journal bytes.
pub struct VerifiedResourceAllocatorCheckpoint {
    allocator_instance_id: String,
    configuration_hash: String,
    checkpoint_content_hash: String,
    checkpoint_sequence: u64,
}

pub struct AtomicResourceAllocator {
    journal_directory: PathBuf,
    capacities: Vec<RateLimitCapacity>,
}

/// Journal-backed evaluator for allocator mechanics that deliberately carries
/// no deployment authority. It is separate from the operational allocator so
/// a library caller cannot accidentally use an unreviewed grant as an
/// operational reservation.
pub struct UnreviewedResourceJournal {
    allocator: AtomicResourceAllocator,
}

impl UnreviewedResourceJournal {
    pub fn new(
        journal_directory: impl Into<PathBuf>,
        configuration: ResourceAllocatorConfiguration,
    ) -> Result<Self, ResourceAllocatorError> {
        Ok(Self {
            allocator: AtomicResourceAllocator::new(journal_directory, configuration)?,
        })
    }

    pub fn reserve(
        &self,
        topology: &ExecutionTopology,
        base_revision_id: &str,
        declaration: ResourceDeclaration,
        reservation: ResourceReservation,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        self.allocator.reserve_unreviewed(
            topology,
            base_revision_id,
            declaration,
            reservation,
            idempotency_key,
        )
    }

    pub fn disposition(
        &self,
        base_revision_id: &str,
        assertion: ReservationDispositionAssertion,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        self.allocator
            .disposition_unreviewed(base_revision_id, assertion, idempotency_key)
    }

    pub fn snapshot(&self) -> Result<ResourceAllocatorSnapshot, ResourceAllocatorError> {
        self.allocator.snapshot()
    }

    pub fn create_checkpoint(&self) -> Result<ResourceAllocatorCheckpoint, ResourceAllocatorError> {
        self.allocator.create_checkpoint()
    }

    pub fn verify_checkpoint(
        &self,
    ) -> Result<VerifiedResourceAllocatorCheckpoint, ResourceAllocatorError> {
        self.allocator.verify_latest_checkpoint()
    }

    pub fn compact(
        &self,
        policy: &ResourceAllocatorRetentionPolicy,
        verified: &VerifiedResourceAllocatorCheckpoint,
    ) -> Result<ResourceAllocatorCompactionOutcome, ResourceAllocatorError> {
        self.allocator.compact(policy, verified)
    }

    pub fn full_replay_snapshot(
        &self,
    ) -> Result<ResourceAllocatorSnapshot, ResourceAllocatorError> {
        self.allocator.full_replay_snapshot()
    }
}

impl AtomicResourceAllocator {
    pub fn new(
        journal_directory: impl Into<PathBuf>,
        configuration: ResourceAllocatorConfiguration,
    ) -> Result<Self, ResourceAllocatorError> {
        validate_configuration(&configuration)?;
        Ok(Self {
            journal_directory: journal_directory.into(),
            capacities: configuration.capacities,
        })
    }

    fn reserve_unreviewed(
        &self,
        topology: &ExecutionTopology,
        base_revision_id: &str,
        declaration: ResourceDeclaration,
        reservation: ResourceReservation,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        let topology_hash =
            execution_topology_content_hash(topology).expect("typed execution topology serializes");
        let payload = ResourceAllocatorEventPayload::Reserve {
            topology_content_hash: topology_hash,
            base_revision_id: base_revision_id.to_owned(),
            reviewed_deployment: None,
            declaration,
            reservation,
        };
        self.append(idempotency_key, payload, |state, payload| {
            let ResourceAllocatorEventPayload::Reserve {
                declaration,
                reservation,
                ..
            } = payload
            else {
                unreachable!()
            };
            grant_topology_reservation(
                topology,
                declaration,
                reservation,
                &state.reservations,
                &state.dispositions,
                &self.capacities,
            )
            .map(|_| ())
            .map_err(|findings| {
                ResourceAllocatorError::Refused(
                    serde_json::to_value(findings).expect("findings serialize"),
                )
            })
        })
    }

    pub fn reserve_reviewed(
        &self,
        topology: &ExecutionTopology,
        authority: &ReviewedDeploymentAuthority,
        base_revision_id: &str,
        declaration: ResourceDeclaration,
        reservation: ResourceReservation,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        let topology_hash =
            execution_topology_content_hash(topology).expect("typed execution topology serializes");
        if topology_hash != authority.topology_content_hash()
            || base_revision_id != authority.accepted_review_revision_id()
        {
            return Err(ResourceAllocatorError::Integrity(
                "reviewed deployment authority does not match topology or accepted revision"
                    .to_owned(),
            ));
        }
        let binding = ReviewedDeploymentReservationBinding {
            schema: REVIEWED_DEPLOYMENT_RESERVATION_BINDING_SCHEMA.to_owned(),
            schema_version: 0,
            claim_cell_id: authority.claim_cell_id().to_owned(),
            accepted_review_id: authority.accepted_review_id().to_owned(),
            reviewed_topology_hash: authority.topology_content_hash().to_owned(),
            policy_manifest_hash: authority.policy_manifest_content_hash().to_owned(),
            deployment_bundle_hash: authority.deployment_bundle_hash().to_owned(),
            accepted_review_revision: authority.accepted_review_revision_id().to_owned(),
            case_space_id: authority.case_space_id().to_owned(),
            node_id: declaration.node_id.clone(),
            attempt_id: reservation.attempt_id.clone(),
            resource_declaration_hash: resource_declaration_content_hash(&declaration),
        };
        let payload = ResourceAllocatorEventPayload::Reserve {
            topology_content_hash: topology_hash,
            base_revision_id: base_revision_id.to_owned(),
            reviewed_deployment: Some(binding),
            declaration,
            reservation,
        };
        self.append(idempotency_key, payload, |state, payload| {
            let ResourceAllocatorEventPayload::Reserve {
                declaration,
                reservation,
                reviewed_deployment,
                ..
            } = payload
            else {
                unreachable!()
            };
            let binding = reviewed_deployment.as_ref().ok_or_else(|| {
                ResourceAllocatorError::Integrity(
                    "reviewed reservation is missing deployment authority".to_owned(),
                )
            })?;
            if binding.node_id != declaration.node_id
                || binding.attempt_id != reservation.attempt_id
                || binding.resource_declaration_hash
                    != resource_declaration_content_hash(declaration)
            {
                return Err(ResourceAllocatorError::Integrity(
                    "reviewed reservation binding does not match declaration or attempt".to_owned(),
                ));
            }
            grant_topology_reservation(
                topology,
                declaration,
                reservation,
                &state.reservations,
                &state.dispositions,
                &self.capacities,
            )
            .map(|_| ())
            .map_err(|findings| {
                ResourceAllocatorError::Refused(
                    serde_json::to_value(findings).expect("findings serialize"),
                )
            })
        })
    }

    fn disposition_unreviewed(
        &self,
        base_revision_id: &str,
        assertion: ReservationDispositionAssertion,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        let payload = ResourceAllocatorEventPayload::Disposition {
            base_revision_id: base_revision_id.to_owned(),
            reviewed_deployment: None,
            assertion,
        };
        self.append(idempotency_key, payload, |state, payload| {
            let ResourceAllocatorEventPayload::Disposition { assertion, .. } = payload else {
                unreachable!()
            };
            validate_disposition(state, assertion)
        })
    }

    pub fn disposition_reviewed(
        &self,
        base_revision_id: &str,
        assertion: ReservationDispositionAssertion,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        let state = self.replay()?;
        let reviewed_deployment = state
            .events_by_idempotency
            .values()
            .find_map(|event| {
                let ResourceAllocatorEventPayload::Reserve {
                    reviewed_deployment,
                    reservation,
                    ..
                } = &event.payload
                else {
                    return None;
                };
                (reservation.reservation_id == assertion.reservation_id
                    && reservation.attempt_id == assertion.attempt_id)
                    .then_some(reviewed_deployment.clone())
                    .flatten()
            })
            .ok_or_else(|| {
                ResourceAllocatorError::Integrity(
                    "resource disposition target has no reviewed deployment authority".to_owned(),
                )
            })?;
        let payload = ResourceAllocatorEventPayload::Disposition {
            base_revision_id: base_revision_id.to_owned(),
            reviewed_deployment: Some(reviewed_deployment),
            assertion,
        };
        self.append(idempotency_key, payload, |state, payload| {
            let ResourceAllocatorEventPayload::Disposition {
                assertion,
                reviewed_deployment,
                ..
            } = payload
            else {
                unreachable!()
            };
            let expected = state.events_by_idempotency.values().find_map(|event| {
                let ResourceAllocatorEventPayload::Reserve {
                    reviewed_deployment,
                    reservation,
                    ..
                } = &event.payload
                else {
                    return None;
                };
                (reservation.reservation_id == assertion.reservation_id
                    && reservation.attempt_id == assertion.attempt_id)
                    .then_some(reviewed_deployment.as_ref())
                    .flatten()
            });
            if expected != reviewed_deployment.as_ref() {
                return Err(ResourceAllocatorError::Integrity(
                    "resource disposition authority differs from the canonical reservation"
                        .to_owned(),
                ));
            }
            if assertion.kind == ReservationAssertionKind::Supersede {
                let superseding = assertion
                    .superseding_reservation_id
                    .as_deref()
                    .expect("validated by resource protocol");
                let superseding_is_reviewed = state.events_by_idempotency.values().any(|event| {
                    matches!(
                        &event.payload,
                        ResourceAllocatorEventPayload::Reserve {
                            reviewed_deployment: Some(_),
                            reservation,
                            ..
                        } if reservation.reservation_id == superseding
                    )
                });
                if !superseding_is_reviewed {
                    return Err(ResourceAllocatorError::Integrity(
                        "superseding reservation has no reviewed deployment authority".to_owned(),
                    ));
                }
                let superseding_binding = state.events_by_idempotency.values().find_map(|event| {
                    let ResourceAllocatorEventPayload::Reserve {
                        reviewed_deployment,
                        reservation,
                        ..
                    } = &event.payload
                    else {
                        return None;
                    };
                    (reservation.reservation_id == superseding)
                        .then_some(reviewed_deployment.as_ref())
                        .flatten()
                });
                if !superseding_binding.is_some_and(|candidate| {
                    candidate.case_space_id
                        == reviewed_deployment
                            .as_ref()
                            .expect("reviewed disposition has authority")
                            .case_space_id
                        && candidate.deployment_bundle_hash
                            == reviewed_deployment
                                .as_ref()
                                .expect("reviewed disposition has authority")
                                .deployment_bundle_hash
                        && candidate.reviewed_topology_hash
                            == reviewed_deployment
                                .as_ref()
                                .expect("reviewed disposition has authority")
                                .reviewed_topology_hash
                        && candidate.policy_manifest_hash
                            == reviewed_deployment
                                .as_ref()
                                .expect("reviewed disposition has authority")
                                .policy_manifest_hash
                }) {
                    return Err(ResourceAllocatorError::Integrity(
                        "superseding reservation belongs to another reviewed deployment".to_owned(),
                    ));
                }
            }
            validate_disposition(state, assertion)
        })
    }

    pub fn snapshot(&self) -> Result<ResourceAllocatorSnapshot, ResourceAllocatorError> {
        let state = self.replay()?;
        Ok(snapshot(&state, &self.capacities))
    }

    pub fn contains_exact_reservation(
        &self,
        declaration: &ResourceDeclaration,
        reservation: &ResourceReservation,
    ) -> Result<bool, ResourceAllocatorError> {
        let state = self.replay()?;
        Ok(state
            .declarations
            .get(&declaration.declaration_id)
            .is_some_and(|candidate| candidate == declaration)
            && state
                .reservations
                .iter()
                .any(|candidate| candidate == reservation))
    }

    pub fn reviewed_reservation_binding(
        &self,
        declaration: &ResourceDeclaration,
        reservation: &ResourceReservation,
    ) -> Result<Option<ReviewedDeploymentReservationBinding>, ResourceAllocatorError> {
        let state = self.replay()?;
        Ok(state.events_by_idempotency.values().find_map(|event| {
            let ResourceAllocatorEventPayload::Reserve {
                reviewed_deployment,
                declaration: candidate_declaration,
                reservation: candidate_reservation,
                ..
            } = &event.payload
            else {
                return None;
            };
            (candidate_declaration == declaration && candidate_reservation == reservation)
                .then_some(reviewed_deployment.clone())
                .flatten()
        }))
    }

    pub fn reviewed_reservation_binding_by_identity(
        &self,
        reservation_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ReviewedDeploymentReservationBinding>, ResourceAllocatorError> {
        let state = self.replay()?;
        Ok(state.events_by_idempotency.values().find_map(|event| {
            let ResourceAllocatorEventPayload::Reserve {
                reviewed_deployment,
                reservation,
                ..
            } = &event.payload
            else {
                return None;
            };
            (reservation.reservation_id == reservation_id && reservation.attempt_id == attempt_id)
                .then_some(reviewed_deployment.clone())
                .flatten()
        }))
    }

    pub fn contains_disposition(
        &self,
        assertion: &ReservationDispositionAssertion,
    ) -> Result<bool, ResourceAllocatorError> {
        Ok(self
            .replay()?
            .dispositions
            .iter()
            .any(|candidate| candidate == assertion))
    }

    fn append(
        &self,
        idempotency_key: &str,
        payload: ResourceAllocatorEventPayload,
        validate: impl Fn(
            &ReplayState,
            &ResourceAllocatorEventPayload,
        ) -> Result<(), ResourceAllocatorError>,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        if idempotency_key.is_empty() {
            return Err(ResourceAllocatorError::Integrity(
                "empty allocator idempotency key".to_owned(),
            ));
        }
        fs::create_dir_all(&self.journal_directory)?;
        let operation_digest = digest(&(idempotency_key, &payload));
        loop {
            let state = self.replay()?;
            if let Some(existing) = state.events_by_idempotency.get(idempotency_key) {
                if existing.operation_digest != operation_digest {
                    return Err(ResourceAllocatorError::IdempotencyCollision);
                }
                return Ok(ResourceAllocatorOutcome {
                    replayed: true,
                    event: existing.clone(),
                    snapshot: snapshot(&state, &self.capacities),
                });
            }
            validate(&state, &payload)?;
            let sequence = state.generation + 1;
            let mut event = ResourceAllocatorEvent {
                schema: RESOURCE_ALLOCATOR_EVENT_SCHEMA.to_owned(),
                schema_version: 0,
                sequence,
                prior_event_hash: state.last_event_hash.clone(),
                idempotency_key: idempotency_key.to_owned(),
                operation_digest: operation_digest.clone(),
                event_hash: String::new(),
                payload: payload.clone(),
            };
            event.event_hash = event_hash(&event);
            let path = self.journal_directory.join(format!("{sequence:020}.json"));
            let temporary = self.journal_directory.join(format!(
                ".pending-{}-{}.tmp",
                std::process::id(),
                TEMPORARY_NONCE.fetch_add(1, Ordering::Relaxed)
            ));
            let bytes = serde_json::to_vec(&event)
                .map_err(|error| ResourceAllocatorError::Integrity(error.to_string()))?;
            let mut temporary_file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            if let Err(error) = temporary_file
                .write_all(&bytes)
                .and_then(|_| temporary_file.sync_all())
            {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
            drop(temporary_file);
            match fs::hard_link(&temporary, &path) {
                Ok(()) => {
                    let _ = fs::remove_file(&temporary);
                    if let Ok(directory) = fs::File::open(&self.journal_directory) {
                        directory.sync_all()?;
                    }
                    let committed = self.replay()?;
                    return Ok(ResourceAllocatorOutcome {
                        replayed: false,
                        event,
                        snapshot: snapshot(&committed, &self.capacities),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&temporary);
                    continue;
                }
                Err(error) => {
                    let _ = fs::remove_file(&temporary);
                    return Err(error.into());
                }
            }
        }
    }

    /// Publish a checkpoint only after deriving it by complete journal replay.
    pub fn create_checkpoint(&self) -> Result<ResourceAllocatorCheckpoint, ResourceAllocatorError> {
        let identity = self.ensure_identity()?;
        let state = self.full_replay()?;
        if state.generation == 0 {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint requires at least one committed event".to_owned(),
            ));
        }
        let mut checkpoint = checkpoint_from_state(
            &state,
            &identity.allocator_instance_id,
            &self.configuration_hash(),
        );
        checkpoint.checkpoint_content_hash = checkpoint_hash(&checkpoint);
        let directory = self.journal_directory.join(CHECKPOINT_DIRECTORY);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "{:020}-{}.json",
            checkpoint.last_event_sequence, checkpoint.checkpoint_content_hash
        ));
        publish_create_new_json(&directory, &path, &checkpoint)?;
        let published: ResourceAllocatorCheckpoint = read_json(&path)?;
        if published != checkpoint {
            return Err(ResourceAllocatorError::Integrity(
                "published allocator checkpoint bytes disagree".to_owned(),
            ));
        }
        self.validate_checkpoint(&published, &identity)?;
        Ok(published)
    }

    /// Compare the newest checkpoint against an independent complete replay.
    pub fn verify_latest_checkpoint(
        &self,
    ) -> Result<VerifiedResourceAllocatorCheckpoint, ResourceAllocatorError> {
        let identity = self.ensure_identity()?;
        let checkpoint = self.load_latest_checkpoint(&identity)?.ok_or_else(|| {
            ResourceAllocatorError::Integrity("allocator checkpoint is absent".to_owned())
        })?;
        let full = self.full_replay()?;
        let checkpoint_state = replay_state_from_checkpoint(&checkpoint);
        if full.generation < checkpoint.last_event_sequence
            || !state_prefix_equals(&full, &checkpoint_state)
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint differs from full journal replay".to_owned(),
            ));
        }
        Ok(VerifiedResourceAllocatorCheckpoint {
            allocator_instance_id: identity.allocator_instance_id,
            configuration_hash: self.configuration_hash(),
            checkpoint_content_hash: checkpoint.checkpoint_content_hash,
            checkpoint_sequence: checkpoint.last_event_sequence,
        })
    }

    /// Archive a checkpoint-covered prefix. Archive bytes remain authoritative
    /// and therefore preserve full replay and audit recovery.
    pub fn compact(
        &self,
        policy: &ResourceAllocatorRetentionPolicy,
        verified: &VerifiedResourceAllocatorCheckpoint,
    ) -> Result<ResourceAllocatorCompactionOutcome, ResourceAllocatorError> {
        validate_retention_policy(policy)?;
        let identity = self.ensure_identity()?;
        let current = self.load_latest_checkpoint(&identity)?.ok_or_else(|| {
            ResourceAllocatorError::Integrity("allocator checkpoint is absent".to_owned())
        })?;
        if identity.allocator_instance_id != verified.allocator_instance_id
            || self.configuration_hash() != verified.configuration_hash
            || current.checkpoint_content_hash != verified.checkpoint_content_hash
            || current.last_event_sequence != verified.checkpoint_sequence
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator compaction proof is stale or belongs to another journal".to_owned(),
            ));
        }
        let cutoff = verified
            .checkpoint_sequence
            .saturating_sub(policy.retain_active_event_count);
        let active = event_paths(&self.journal_directory)?;
        let archive_directory = self.journal_directory.join(ARCHIVE_DIRECTORY);
        fs::create_dir_all(&archive_directory)?;
        let mut archived = Vec::new();
        for (sequence, source) in active.iter().filter(|(sequence, _)| **sequence <= cutoff) {
            let event: ResourceAllocatorEvent = read_json(source)?;
            if event.sequence != *sequence || event.event_hash != event_hash(&event) {
                return Err(ResourceAllocatorError::Integrity(format!(
                    "allocator compaction source is invalid at {}",
                    source.display()
                )));
            }
            let destination = archive_directory.join(event_file_name(*sequence));
            match fs::hard_link(source, &destination) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if fs::read(source)? != fs::read(&destination)? {
                        return Err(ResourceAllocatorError::Integrity(
                            "allocator archive sequence contains different bytes".to_owned(),
                        ));
                    }
                }
                Err(error) => return Err(error.into()),
            }
            archived.push((*sequence, event.event_hash));
        }
        if let Ok(directory) = fs::File::open(&archive_directory) {
            directory.sync_all()?;
        }
        let mut record = ResourceAllocatorCompactionRecord {
            schema: RESOURCE_ALLOCATOR_COMPACTION_SCHEMA.to_owned(),
            schema_version: 0,
            allocator_instance_id: verified.allocator_instance_id.clone(),
            configuration_hash: verified.configuration_hash.clone(),
            checkpoint_content_hash: verified.checkpoint_content_hash.clone(),
            checkpoint_sequence: verified.checkpoint_sequence,
            retention_policy: policy.clone(),
            retention_policy_hash: digest(policy),
            archived_event_sequences: archived.iter().map(|(sequence, _)| *sequence).collect(),
            archived_event_hashes: archived.iter().map(|(_, hash)| hash.clone()).collect(),
            compaction_content_hash: String::new(),
        };
        record.compaction_content_hash = compaction_hash(&record);
        let compaction_directory = self.journal_directory.join(COMPACTION_DIRECTORY);
        fs::create_dir_all(&compaction_directory)?;
        let record_path = compaction_directory.join(format!(
            "{:020}-{}.json",
            record.checkpoint_sequence, record.compaction_content_hash
        ));
        publish_create_new_json(&compaction_directory, &record_path, &record)?;
        validate_compaction_record(&record, policy, verified)?;

        // Deletion begins only after every archive link and the content-bound
        // compaction record are durable. A crash leaves either duplicates or
        // an archived-only prefix; both full-replay paths are equivalent.
        for (sequence, source) in active.iter().filter(|(sequence, _)| **sequence <= cutoff) {
            let archived_path = archive_directory.join(event_file_name(*sequence));
            if !archived_path.is_file() || fs::read(source)? != fs::read(&archived_path)? {
                return Err(ResourceAllocatorError::Integrity(
                    "allocator archive verification failed before active-prefix removal".to_owned(),
                ));
            }
            fs::remove_file(source)?;
        }
        if let Ok(directory) = fs::File::open(&self.journal_directory) {
            directory.sync_all()?;
        }
        Ok(ResourceAllocatorCompactionOutcome {
            record,
            archived_event_count: archived.len() as u64,
            active_event_count: event_paths(&self.journal_directory)?.len() as u64,
        })
    }

    pub fn full_replay_snapshot(
        &self,
    ) -> Result<ResourceAllocatorSnapshot, ResourceAllocatorError> {
        let state = self.full_replay()?;
        Ok(snapshot(&state, &self.capacities))
    }

    fn replay(&self) -> Result<ReplayState, ResourceAllocatorError> {
        fs::create_dir_all(&self.journal_directory)?;
        let identity = self.ensure_identity()?;
        self.validate_published_compactions(&identity, false)?;
        let checkpoint = self.load_latest_checkpoint(&identity)?;
        let mut state = checkpoint
            .as_ref()
            .map(replay_state_from_checkpoint)
            .unwrap_or_default();
        let covered = checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.last_event_sequence);
        if let Some(checkpoint) = &checkpoint {
            self.require_covered_prefix_present(checkpoint)?;
        }
        for (sequence, path) in event_paths(&self.journal_directory)? {
            if sequence <= covered {
                continue;
            }
            let event: ResourceAllocatorEvent = read_json(&path)?;
            apply_checked_event(&mut state, &self.capacities, event, &path)?;
        }
        Ok(state)
    }

    fn full_replay(&self) -> Result<ReplayState, ResourceAllocatorError> {
        fs::create_dir_all(&self.journal_directory)?;
        let identity = self.ensure_identity()?;
        self.validate_published_compactions(&identity, true)?;
        let mut combined = event_paths(&self.journal_directory)?;
        let archived = event_paths(&self.journal_directory.join(ARCHIVE_DIRECTORY))?;
        for (sequence, path) in archived {
            if let Some(active) = combined.get(&sequence) {
                if fs::read(active)? != fs::read(&path)? {
                    return Err(ResourceAllocatorError::Integrity(format!(
                        "allocator active/archive bytes disagree at sequence {sequence}"
                    )));
                }
            } else {
                combined.insert(sequence, path);
            }
        }
        let mut state = ReplayState::default();
        for (_, path) in combined {
            let event: ResourceAllocatorEvent = read_json(&path)?;
            apply_checked_event(&mut state, &self.capacities, event, &path)?;
        }
        Ok(state)
    }

    fn configuration_hash(&self) -> String {
        digest(&ResourceAllocatorConfiguration {
            schema: RESOURCE_ALLOCATOR_CONFIGURATION_SCHEMA.to_owned(),
            schema_version: 0,
            capacities: self.capacities.clone(),
        })
    }

    fn ensure_identity(&self) -> Result<ResourceAllocatorIdentity, ResourceAllocatorError> {
        fs::create_dir_all(&self.journal_directory)?;
        let path = self.journal_directory.join(IDENTITY_FILE);
        if path.exists() {
            return validate_identity(read_json(&path)?, &self.journal_directory);
        }
        let nonce = TEMPORARY_NONCE.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ResourceAllocatorError::Integrity(error.to_string()))?
            .as_nanos();
        let allocator_instance_id = format!(
            "allocator:{}",
            digest(&(
                self.journal_directory.to_string_lossy(),
                std::process::id(),
                now,
                nonce
            ))
        );
        let mut identity = ResourceAllocatorIdentity {
            schema: ALLOCATOR_IDENTITY_SCHEMA.to_owned(),
            schema_version: 0,
            allocator_instance_id,
            journal_location_hash: journal_location_hash(&self.journal_directory)?,
            identity_content_hash: String::new(),
        };
        identity.identity_content_hash = identity_hash(&identity);
        let temporary = self.journal_directory.join(format!(
            ".pending-identity-{}-{nonce}.tmp",
            std::process::id()
        ));
        write_synced_new(
            &temporary,
            &serde_json::to_vec(&identity)
                .map_err(|error| ResourceAllocatorError::Integrity(error.to_string()))?,
        )?;
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error.into());
            }
        }
        let _ = fs::remove_file(&temporary);
        if let Ok(directory) = fs::File::open(&self.journal_directory) {
            directory.sync_all()?;
        }
        validate_identity(read_json(&path)?, &self.journal_directory)
    }

    fn load_latest_checkpoint(
        &self,
        identity: &ResourceAllocatorIdentity,
    ) -> Result<Option<ResourceAllocatorCheckpoint>, ResourceAllocatorError> {
        let directory = self.journal_directory.join(CHECKPOINT_DIRECTORY);
        if !directory.exists() {
            return Ok(None);
        }
        let mut checkpoints = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let checkpoint: ResourceAllocatorCheckpoint = read_json(&path)?;
            self.validate_checkpoint(&checkpoint, identity)?;
            let expected_name = format!(
                "{:020}-{}.json",
                checkpoint.last_event_sequence, checkpoint.checkpoint_content_hash
            );
            if path.file_name().and_then(|value| value.to_str()) != Some(&expected_name) {
                return Err(ResourceAllocatorError::Integrity(
                    "allocator checkpoint filename is not content addressed".to_owned(),
                ));
            }
            checkpoints.push(checkpoint);
        }
        checkpoints.sort_by_key(|checkpoint| checkpoint.last_event_sequence);
        if checkpoints.windows(2).any(|pair| {
            pair[0].last_event_sequence == pair[1].last_event_sequence
                && pair[0].checkpoint_content_hash != pair[1].checkpoint_content_hash
        }) {
            return Err(ResourceAllocatorError::Integrity(
                "ambiguous allocator checkpoints cover the same sequence".to_owned(),
            ));
        }
        Ok(checkpoints.pop())
    }

    fn validate_checkpoint(
        &self,
        checkpoint: &ResourceAllocatorCheckpoint,
        identity: &ResourceAllocatorIdentity,
    ) -> Result<(), ResourceAllocatorError> {
        if checkpoint.schema != RESOURCE_ALLOCATOR_CHECKPOINT_SCHEMA
            || checkpoint.schema_version != 0
            || checkpoint.allocator_instance_id != identity.allocator_instance_id
            || checkpoint.configuration_hash != self.configuration_hash()
            || checkpoint.covered_event_count != checkpoint.last_event_sequence
            || checkpoint.checkpoint_content_hash != checkpoint_hash(checkpoint)
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint identity, configuration, or content hash mismatch".to_owned(),
            ));
        }
        let state = validate_checkpoint_index(checkpoint)?;
        if state.generation != checkpoint.last_event_sequence
            || state.last_event_hash.as_deref() != Some(&checkpoint.terminal_event_hash)
            || journal_prefix_hash(&state) != checkpoint.covered_journal_prefix_hash
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint does not describe its exact journal prefix".to_owned(),
            ));
        }
        Ok(())
    }

    fn require_covered_prefix_present(
        &self,
        checkpoint: &ResourceAllocatorCheckpoint,
    ) -> Result<(), ResourceAllocatorError> {
        let active = event_paths(&self.journal_directory)?;
        let archived = event_paths(&self.journal_directory.join(ARCHIVE_DIRECTORY))?;
        let expected_by_sequence = checkpoint
            .state
            .events_by_idempotency
            .values()
            .map(|event| (event.sequence, event))
            .collect::<BTreeMap<_, _>>();
        for sequence in 1..=checkpoint.last_event_sequence {
            let path = active
                .get(&sequence)
                .or_else(|| archived.get(&sequence))
                .ok_or_else(|| {
                    ResourceAllocatorError::Integrity(format!(
                        "allocator journal prefix is truncated at sequence {sequence}"
                    ))
                })?;
            let observed: ResourceAllocatorEvent = read_json(path)?;
            let expected = expected_by_sequence
                .get(&sequence)
                .copied()
                .ok_or_else(|| {
                    ResourceAllocatorError::Integrity(
                        "allocator checkpoint event index is incomplete".to_owned(),
                    )
                })?;
            if observed != *expected || observed.event_hash != event_hash(&observed) {
                return Err(ResourceAllocatorError::Integrity(format!(
                    "allocator journal prefix was substituted or reordered at sequence {sequence}"
                )));
            }
        }
        let terminal_path = active
            .get(&checkpoint.last_event_sequence)
            .or_else(|| archived.get(&checkpoint.last_event_sequence))
            .ok_or_else(|| {
                ResourceAllocatorError::Integrity(
                    "allocator checkpoint terminal event is absent".to_owned(),
                )
            })?;
        let terminal: ResourceAllocatorEvent = read_json(terminal_path)?;
        if terminal.event_hash != checkpoint.terminal_event_hash
            || terminal.event_hash != event_hash(&terminal)
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint terminal event was substituted".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_published_compactions(
        &self,
        identity: &ResourceAllocatorIdentity,
        verify_archive_bytes: bool,
    ) -> Result<(), ResourceAllocatorError> {
        let directory = self.journal_directory.join(COMPACTION_DIRECTORY);
        if !directory.exists() {
            return Ok(());
        }
        let archive = event_paths(&self.journal_directory.join(ARCHIVE_DIRECTORY))?;
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record: ResourceAllocatorCompactionRecord = read_json(&path)?;
            if record.schema != RESOURCE_ALLOCATOR_COMPACTION_SCHEMA
                || record.schema_version != 0
                || record.allocator_instance_id != identity.allocator_instance_id
                || record.configuration_hash != self.configuration_hash()
                || record.retention_policy_hash != digest(&record.retention_policy)
                || validate_retention_policy(&record.retention_policy).is_err()
                || record.archived_event_sequences.len() != record.archived_event_hashes.len()
                || record
                    .archived_event_sequences
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || record.archived_event_sequences.iter().any(|sequence| {
                    *sequence
                        > record
                            .checkpoint_sequence
                            .saturating_sub(record.retention_policy.retain_active_event_count)
                })
                || record.compaction_content_hash != compaction_hash(&record)
            {
                return Err(ResourceAllocatorError::Integrity(format!(
                    "invalid allocator compaction record {}",
                    path.display()
                )));
            }
            let checkpoint_path = self
                .journal_directory
                .join(CHECKPOINT_DIRECTORY)
                .join(format!(
                    "{:020}-{}.json",
                    record.checkpoint_sequence, record.checkpoint_content_hash
                ));
            let checkpoint: ResourceAllocatorCheckpoint =
                read_json(&checkpoint_path).map_err(|_| {
                    ResourceAllocatorError::Integrity(
                        "allocator compaction references an absent checkpoint".to_owned(),
                    )
                })?;
            self.validate_checkpoint(&checkpoint, identity)?;
            let expected_name = format!(
                "{:020}-{}.json",
                record.checkpoint_sequence, record.compaction_content_hash
            );
            if path.file_name().and_then(|value| value.to_str()) != Some(&expected_name) {
                return Err(ResourceAllocatorError::Integrity(
                    "allocator compaction filename is not content addressed".to_owned(),
                ));
            }
            for (sequence, expected_hash) in record
                .archived_event_sequences
                .iter()
                .zip(&record.archived_event_hashes)
            {
                let archived_path = archive.get(sequence).ok_or_else(|| {
                    ResourceAllocatorError::Integrity(format!(
                        "allocator compaction archive is truncated at sequence {sequence}"
                    ))
                })?;
                if verify_archive_bytes {
                    let event: ResourceAllocatorEvent = read_json(archived_path)?;
                    if event.sequence != *sequence
                        || event.event_hash != *expected_hash
                        || event.event_hash != event_hash(&event)
                    {
                        return Err(ResourceAllocatorError::Integrity(
                            "allocator compaction archive event was substituted".to_owned(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn event_file_name(sequence: u64) -> String {
    format!("{sequence:020}.json")
}

fn event_paths(directory: &Path) -> Result<BTreeMap<u64, PathBuf>, ResourceAllocatorError> {
    if !directory.exists() {
        return Ok(BTreeMap::new());
    }
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            return Err(ResourceAllocatorError::Integrity(
                "allocator journal filename is not UTF-8".to_owned(),
            ));
        };
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.len() != 20 || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ResourceAllocatorError::Integrity(format!(
                "unexpected allocator journal JSON file {}",
                path.display()
            )));
        }
        let sequence = stem.parse::<u64>().map_err(|error| {
            ResourceAllocatorError::Integrity(format!("{}: {error}", path.display()))
        })?;
        if sequence == 0 || result.insert(sequence, path.clone()).is_some() {
            return Err(ResourceAllocatorError::Integrity(
                "duplicate or zero allocator journal sequence".to_owned(),
            ));
        }
    }
    Ok(result)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ResourceAllocatorError> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| ResourceAllocatorError::Integrity(format!("{}: {error}", path.display())))
}

fn write_synced_new(path: &Path, bytes: &[u8]) -> Result<(), ResourceAllocatorError> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn publish_create_new_json<T: Serialize>(
    directory: &Path,
    destination: &Path,
    value: &T,
) -> Result<(), ResourceAllocatorError> {
    let temporary = directory.join(format!(
        ".pending-{}-{}.tmp",
        std::process::id(),
        TEMPORARY_NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ResourceAllocatorError::Integrity(error.to_string()))?;
    write_synced_new(&temporary, &bytes)?;
    match fs::hard_link(&temporary, destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::read(destination)? != bytes {
                let _ = fs::remove_file(&temporary);
                return Err(ResourceAllocatorError::Integrity(
                    "content-addressed allocator publication collision".to_owned(),
                ));
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    let _ = fs::remove_file(&temporary);
    if let Ok(handle) = fs::File::open(directory) {
        handle.sync_all()?;
    }
    Ok(())
}

fn apply_checked_event(
    state: &mut ReplayState,
    capacities: &[RateLimitCapacity],
    event: ResourceAllocatorEvent,
    path: &Path,
) -> Result<(), ResourceAllocatorError> {
    if event.schema != RESOURCE_ALLOCATOR_EVENT_SCHEMA
        || event.schema_version != 0
        || event.sequence != state.generation + 1
        || event.prior_event_hash != state.last_event_hash
        || event.event_hash != event_hash(&event)
    {
        return Err(ResourceAllocatorError::Integrity(format!(
            "allocator journal integrity failure at {}",
            path.display()
        )));
    }
    if state
        .events_by_idempotency
        .contains_key(&event.idempotency_key)
    {
        return Err(ResourceAllocatorError::Integrity(
            "duplicate allocator idempotency key in journal".to_owned(),
        ));
    }
    apply_replayed_event(state, capacities, &event)?;
    state.generation = event.sequence;
    state.last_event_hash = Some(event.event_hash.clone());
    state
        .events_by_idempotency
        .insert(event.idempotency_key.clone(), event);
    Ok(())
}

fn checkpoint_from_state(
    state: &ReplayState,
    allocator_instance_id: &str,
    configuration_hash: &str,
) -> ResourceAllocatorCheckpoint {
    ResourceAllocatorCheckpoint {
        schema: RESOURCE_ALLOCATOR_CHECKPOINT_SCHEMA.to_owned(),
        schema_version: 0,
        allocator_instance_id: allocator_instance_id.to_owned(),
        configuration_hash: configuration_hash.to_owned(),
        covered_event_count: state.generation,
        last_event_sequence: state.generation,
        terminal_event_hash: state
            .last_event_hash
            .clone()
            .expect("non-empty checkpoint has a terminal event"),
        covered_journal_prefix_hash: journal_prefix_hash(state),
        state: checkpoint_state(state),
        checkpoint_content_hash: String::new(),
    }
}

fn checkpoint_state(state: &ReplayState) -> ResourceAllocatorCheckpointState {
    ResourceAllocatorCheckpointState {
        reservations: state.reservations.clone(),
        declarations: state.declarations.clone(),
        dispositions: state.dispositions.clone(),
        events_by_idempotency: state.events_by_idempotency.clone(),
    }
}

fn replay_state_from_checkpoint(checkpoint: &ResourceAllocatorCheckpoint) -> ReplayState {
    ReplayState {
        reservations: checkpoint.state.reservations.clone(),
        declarations: checkpoint.state.declarations.clone(),
        dispositions: checkpoint.state.dispositions.clone(),
        events_by_idempotency: checkpoint.state.events_by_idempotency.clone(),
        generation: checkpoint.last_event_sequence,
        last_event_hash: Some(checkpoint.terminal_event_hash.clone()),
    }
}

fn validate_checkpoint_index(
    checkpoint: &ResourceAllocatorCheckpoint,
) -> Result<ReplayState, ResourceAllocatorError> {
    let mut events = checkpoint
        .state
        .events_by_idempotency
        .values()
        .cloned()
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.sequence);
    if events.len() as u64 != checkpoint.covered_event_count {
        return Err(ResourceAllocatorError::Integrity(
            "allocator checkpoint event index is incomplete".to_owned(),
        ));
    }
    let mut derived = ReplayState::default();
    for event in events {
        if event.schema != RESOURCE_ALLOCATOR_EVENT_SCHEMA
            || event.schema_version != 0
            || event.sequence != derived.generation + 1
            || event.prior_event_hash != derived.last_event_hash
            || event.event_hash != event_hash(&event)
            || derived
                .events_by_idempotency
                .contains_key(&event.idempotency_key)
        {
            return Err(ResourceAllocatorError::Integrity(
                "allocator checkpoint event index is not an exact hash chain".to_owned(),
            ));
        }
        match &event.payload {
            ResourceAllocatorEventPayload::Reserve {
                declaration,
                reservation,
                ..
            } => {
                derived
                    .declarations
                    .insert(declaration.declaration_id.clone(), declaration.clone());
                derived.reservations.push(reservation.clone());
            }
            ResourceAllocatorEventPayload::Disposition { assertion, .. } => {
                derived.dispositions.push(assertion.clone());
            }
        }
        derived.generation = event.sequence;
        derived.last_event_hash = Some(event.event_hash.clone());
        derived
            .events_by_idempotency
            .insert(event.idempotency_key.clone(), event);
    }
    if checkpoint_state(&derived) != checkpoint.state {
        return Err(ResourceAllocatorError::Integrity(
            "allocator checkpoint derived state disagrees with indexed events".to_owned(),
        ));
    }
    Ok(derived)
}

fn state_prefix_equals(full: &ReplayState, prefix: &ReplayState) -> bool {
    let events = full
        .events_by_idempotency
        .iter()
        .filter(|(_, event)| event.sequence <= prefix.generation)
        .map(|(key, event)| (key.clone(), event.clone()))
        .collect::<BTreeMap<_, _>>();
    events == prefix.events_by_idempotency
        && prefix.last_event_hash
            == events
                .values()
                .max_by_key(|event| event.sequence)
                .map(|event| event.event_hash.clone())
}

fn journal_prefix_hash(state: &ReplayState) -> String {
    let mut events = state.events_by_idempotency.values().collect::<Vec<_>>();
    events.sort_by_key(|event| event.sequence);
    digest(
        &events
            .iter()
            .map(|event| (event.sequence, event.event_hash.as_str()))
            .collect::<Vec<_>>(),
    )
}

fn checkpoint_hash(checkpoint: &ResourceAllocatorCheckpoint) -> String {
    let mut value = checkpoint.clone();
    value.checkpoint_content_hash.clear();
    digest(&value)
}

fn compaction_hash(record: &ResourceAllocatorCompactionRecord) -> String {
    let mut value = record.clone();
    value.compaction_content_hash.clear();
    digest(&value)
}

fn identity_hash(identity: &ResourceAllocatorIdentity) -> String {
    let mut value = identity.clone();
    value.identity_content_hash.clear();
    digest(&value)
}

fn validate_identity(
    identity: ResourceAllocatorIdentity,
    journal_directory: &Path,
) -> Result<ResourceAllocatorIdentity, ResourceAllocatorError> {
    if identity.schema != ALLOCATOR_IDENTITY_SCHEMA
        || identity.schema_version != 0
        || identity.allocator_instance_id.is_empty()
        || identity.journal_location_hash != journal_location_hash(journal_directory)?
        || identity.identity_content_hash != identity_hash(&identity)
    {
        return Err(ResourceAllocatorError::Integrity(
            "allocator journal identity is invalid".to_owned(),
        ));
    }
    Ok(identity)
}

fn journal_location_hash(directory: &Path) -> Result<String, ResourceAllocatorError> {
    let canonical = fs::canonicalize(directory)?;
    Ok(digest(&canonical.to_string_lossy()))
}

fn validate_retention_policy(
    policy: &ResourceAllocatorRetentionPolicy,
) -> Result<(), ResourceAllocatorError> {
    if policy.schema != RESOURCE_ALLOCATOR_RETENTION_POLICY_SCHEMA || policy.schema_version != 0 {
        return Err(ResourceAllocatorError::Integrity(
            "unsupported allocator retention policy".to_owned(),
        ));
    }
    Ok(())
}

fn validate_compaction_record(
    record: &ResourceAllocatorCompactionRecord,
    policy: &ResourceAllocatorRetentionPolicy,
    verified: &VerifiedResourceAllocatorCheckpoint,
) -> Result<(), ResourceAllocatorError> {
    if record.schema != RESOURCE_ALLOCATOR_COMPACTION_SCHEMA
        || record.schema_version != 0
        || record.allocator_instance_id != verified.allocator_instance_id
        || record.configuration_hash != verified.configuration_hash
        || record.checkpoint_content_hash != verified.checkpoint_content_hash
        || record.checkpoint_sequence != verified.checkpoint_sequence
        || record.retention_policy != *policy
        || record.retention_policy_hash != digest(policy)
        || record.archived_event_sequences.len() != record.archived_event_hashes.len()
        || record
            .archived_event_sequences
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || record.compaction_content_hash != compaction_hash(record)
    {
        return Err(ResourceAllocatorError::Integrity(
            "allocator compaction record is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn apply_replayed_event(
    state: &mut ReplayState,
    capacities: &[RateLimitCapacity],
    event: &ResourceAllocatorEvent,
) -> Result<(), ResourceAllocatorError> {
    match &event.payload {
        ResourceAllocatorEventPayload::Reserve {
            declaration,
            reservation,
            ..
        } => {
            // Replay cannot reconstruct the topology, but the event records a
            // grant that was topology-validated before its atomic append. The
            // protocol-level conflict/capacity decision is re-run here.
            crate::resource_protocol::grant_reservation(
                declaration,
                reservation,
                &state.reservations,
                &state.dispositions,
                capacities,
            )
            .map_err(|findings| {
                ResourceAllocatorError::Integrity(format!(
                    "committed reservation no longer replays: {}",
                    serde_json::to_string(&findings).expect("findings serialize")
                ))
            })?;
            state
                .declarations
                .insert(declaration.declaration_id.clone(), declaration.clone());
            state.reservations.push(reservation.clone());
        }
        ResourceAllocatorEventPayload::Disposition { assertion, .. } => {
            validate_disposition(state, assertion)?;
            state.dispositions.push(assertion.clone());
        }
    }
    Ok(())
}

fn validate_disposition(
    state: &ReplayState,
    assertion: &ReservationDispositionAssertion,
) -> Result<(), ResourceAllocatorError> {
    let reservation = state
        .reservations
        .iter()
        .find(|reservation| {
            reservation.reservation_id == assertion.reservation_id
                && reservation.attempt_id == assertion.attempt_id
        })
        .ok_or_else(|| {
            ResourceAllocatorError::Integrity(
                "disposition target is not a canonical reservation".to_owned(),
            )
        })?;
    if !reservation_is_active(reservation, &state.dispositions) {
        return Err(ResourceAllocatorError::Integrity(
            "reservation is already inactive".to_owned(),
        ));
    }
    let mut assertions = state.dispositions.clone();
    assertions.push(assertion.clone());
    if reservation_is_active(reservation, &assertions) {
        return Err(ResourceAllocatorError::Integrity(
            "invalid disposition assertion".to_owned(),
        ));
    }
    if assertion.kind == ReservationAssertionKind::Supersede {
        let superseding = assertion
            .superseding_reservation_id
            .as_deref()
            .expect("validated by protocol");
        if !state.reservations.iter().any(|candidate| {
            candidate.reservation_id == superseding
                && reservation_is_active(candidate, &state.dispositions)
        }) {
            return Err(ResourceAllocatorError::Integrity(
                "superseding reservation is not active in canonical state".to_owned(),
            ));
        }
    }
    Ok(())
}

fn snapshot(state: &ReplayState, capacities: &[RateLimitCapacity]) -> ResourceAllocatorSnapshot {
    let active_reservations = state
        .reservations
        .iter()
        .filter(|reservation| reservation_is_active(reservation, &state.dispositions))
        .cloned()
        .collect();
    ResourceAllocatorSnapshot {
        generation: state.generation,
        last_event_hash: state.last_event_hash.clone(),
        active_reservations,
        dispositions: state.dispositions.clone(),
        capacities: capacities.to_vec(),
    }
}

fn validate_configuration(
    configuration: &ResourceAllocatorConfiguration,
) -> Result<(), ResourceAllocatorError> {
    if configuration.schema != RESOURCE_ALLOCATOR_CONFIGURATION_SCHEMA
        || configuration.schema_version != 0
    {
        return Err(ResourceAllocatorError::InvalidConfiguration(
            "unsupported allocator configuration schema".to_owned(),
        ));
    }
    let mut groups = BTreeSet::new();
    for capacity in &configuration.capacities {
        if capacity.schema != crate::resource_protocol::RATE_LIMIT_CAPACITY_SCHEMA
            || capacity.schema_version != 0
            || capacity.capacity == 0
            || !groups.insert(capacity.group_id.as_str())
        {
            return Err(ResourceAllocatorError::InvalidConfiguration(
                "invalid or duplicate rate-limit capacity".to_owned(),
            ));
        }
    }
    Ok(())
}

fn event_hash(event: &ResourceAllocatorEvent) -> String {
    digest(&(
        event.schema.as_str(),
        event.schema_version,
        event.sequence,
        event.prior_event_hash.as_deref(),
        event.idempotency_key.as_str(),
        event.operation_digest.as_str(),
        &event.payload,
    ))
}

fn digest(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("typed allocator content serializes");
    sha256_hex(&bytes)
}
