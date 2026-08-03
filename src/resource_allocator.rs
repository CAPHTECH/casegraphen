//! Durable atomic allocator built on append-only, create-new journal events.
//!
//! The pure compatibility/conflict rules remain in [`crate::resource_protocol`].
//! This module owns only serialization, durable replay, idempotency, and the
//! canonical active-reservation snapshot used by an operational host.

use crate::{
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
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
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMPORARY_NONCE: AtomicU64 = AtomicU64::new(0);

pub const RESOURCE_ALLOCATOR_CONFIGURATION_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_configuration.v0";
pub const RESOURCE_ALLOCATOR_EVENT_SCHEMA: &str =
    "casegraphen.experimental.resource.allocator_event.v0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceAllocatorConfiguration {
    pub schema: String,
    pub schema_version: u32,
    pub capacities: Vec<RateLimitCapacity>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceAllocatorEventPayload {
    Reserve {
        topology_content_hash: String,
        base_revision_id: String,
        declaration: ResourceDeclaration,
        reservation: ResourceReservation,
    },
    Disposition {
        base_revision_id: String,
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

#[derive(Default)]
struct ReplayState {
    reservations: Vec<ResourceReservation>,
    declarations: BTreeMap<String, ResourceDeclaration>,
    dispositions: Vec<ReservationDispositionAssertion>,
    events_by_idempotency: BTreeMap<String, ResourceAllocatorEvent>,
    generation: u64,
    last_event_hash: Option<String>,
}

pub struct AtomicResourceAllocator {
    journal_directory: PathBuf,
    capacities: Vec<RateLimitCapacity>,
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

    pub fn reserve(
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

    pub fn disposition(
        &self,
        base_revision_id: &str,
        assertion: ReservationDispositionAssertion,
        idempotency_key: &str,
    ) -> Result<ResourceAllocatorOutcome, ResourceAllocatorError> {
        let payload = ResourceAllocatorEventPayload::Disposition {
            base_revision_id: base_revision_id.to_owned(),
            assertion,
        };
        self.append(idempotency_key, payload, |state, payload| {
            let ResourceAllocatorEventPayload::Disposition { assertion, .. } = payload else {
                unreachable!()
            };
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

    fn replay(&self) -> Result<ReplayState, ResourceAllocatorError> {
        fs::create_dir_all(&self.journal_directory)?;
        let mut paths = fs::read_dir(&self.journal_directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        let mut state = ReplayState::default();
        for path in paths {
            let event: ResourceAllocatorEvent =
                serde_json::from_slice(&fs::read(&path)?).map_err(|error| {
                    ResourceAllocatorError::Integrity(format!("{}: {error}", path.display()))
                })?;
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
            apply_replayed_event(&mut state, &self.capacities, &event)?;
            state.generation = event.sequence;
            state.last_event_hash = Some(event.event_hash.clone());
            state
                .events_by_idempotency
                .insert(event.idempotency_key.clone(), event);
        }
        Ok(state)
    }
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
