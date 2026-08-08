//! Transport-neutral state for an external MCP-compatible control plane.
//!
//! This module owns protocol idempotency and replay only. A delegate owns every
//! CaseGraphen, compiler, runtime, verification, and resource decision.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
};

pub const CONTROL_PLANE_REQUEST_SCHEMA: &str = "casegraphen.experimental.control_plane.request.v0";
pub const CONTROL_PLANE_RESPONSE_SCHEMA: &str =
    "casegraphen.experimental.control_plane.response.v0";
pub const CONTROL_PLANE_NOTIFICATION_SCHEMA: &str =
    "casegraphen.experimental.control_plane.notification.v0";
/// Schema identity of the transport-neutral control-plane capability catalog.
pub const CONTROL_PLANE_CATALOG_SCHEMA: &str = "casegraphen.experimental.control_plane.catalog.v0";

pub const RESOURCE_TEMPLATES: &[&str] = &[
    "casegraphen://spaces/{id}/status",
    "casegraphen://spaces/{id}/frontier",
    "casegraphen://spaces/{id}/halts",
    "casegraphen://spaces/{id}/reviews",
    "casegraphen://spaces/{id}/revisions/{revision}",
    "casegraphen://runs/{run_id}",
    "casegraphen://topologies/{topology_id}",
];
pub const TOOLS: &[ControlPlaneTool] = &[
    ControlPlaneTool::ProposeExecutionTopology,
    ControlPlaneTool::LintExecutionTopology,
    ControlPlaneTool::CompileDeploymentBundle,
    ControlPlaneTool::CompileReviewedDeploymentBundle,
    ControlPlaneTool::AttachRuntimeReport,
    ControlPlaneTool::ReconcileRun,
    ControlPlaneTool::ApplyEvidencePacket,
    ControlPlaneTool::ReviewAccept,
    ControlPlaneTool::ReviewReject,
    ControlPlaneTool::Resume,
    ControlPlaneTool::SupersedeDispatch,
    ControlPlaneTool::ReserveResources,
    ControlPlaneTool::ReconcileResources,
    ControlPlaneTool::ReleaseResources,
    ControlPlaneTool::SimulateExecutionTopology,
    ControlPlaneTool::EvaluateExpansionRound,
    ControlPlaneTool::ReconcileStreamingRun,
    ControlPlaneTool::ReconcileVerificationLineage,
    ControlPlaneTool::ProposeTopologyRedesign,
    ControlPlaneTool::MemoryQuery,
    ControlPlaneTool::MemoryExplain,
    ControlPlaneTool::MemoryHistory,
    ControlPlaneTool::MemoryConflicts,
    ControlPlaneTool::MemorySources,
    ControlPlaneTool::MemoryProposeClaim,
    ControlPlaneTool::MemoryProposeSupersession,
    ControlPlaneTool::MemoryProposeRetraction,
    ControlPlaneTool::MemoryProposeProcedure,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneTool {
    ProposeExecutionTopology,
    LintExecutionTopology,
    CompileDeploymentBundle,
    CompileReviewedDeploymentBundle,
    AttachRuntimeReport,
    ReconcileRun,
    ApplyEvidencePacket,
    ReviewAccept,
    ReviewReject,
    Resume,
    SupersedeDispatch,
    ReserveResources,
    ReconcileResources,
    ReleaseResources,
    SimulateExecutionTopology,
    EvaluateExpansionRound,
    ReconcileStreamingRun,
    ReconcileVerificationLineage,
    ProposeTopologyRedesign,
    MemoryQuery,
    MemoryExplain,
    MemoryHistory,
    MemoryConflicts,
    MemorySources,
    MemoryProposeClaim,
    MemoryProposeSupersession,
    MemoryProposeRetraction,
    MemoryProposeProcedure,
}

impl ControlPlaneTool {
    /// Whether the tool consumes or emits a case-bound artifact whose caller-
    /// observed revision must be preserved even though it may not mutate the
    /// acceptance ledger.
    pub fn requires_base_revision(self) -> bool {
        self.changes_managed_state()
            || matches!(
                self,
                Self::CompileDeploymentBundle
                    | Self::CompileReviewedDeploymentBundle
                    | Self::ReconcileRun
                    | Self::ReconcileResources
                    | Self::EvaluateExpansionRound
                    | Self::ReconcileStreamingRun
                    | Self::ReconcileVerificationLineage
                    | Self::ProposeTopologyRedesign
                    | Self::MemoryQuery
                    | Self::MemoryExplain
                    | Self::MemoryHistory
                    | Self::MemoryConflicts
                    | Self::MemorySources
                    | Self::MemoryProposeClaim
                    | Self::MemoryProposeSupersession
                    | Self::MemoryProposeRetraction
                    | Self::MemoryProposeProcedure
            )
    }

    pub fn changes_managed_state(self) -> bool {
        matches!(
            self,
            Self::AttachRuntimeReport
                | Self::ApplyEvidencePacket
                | Self::ReviewAccept
                | Self::ReviewReject
                | Self::Resume
                | Self::SupersedeDispatch
                | Self::ReserveResources
                | Self::ReleaseResources
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    RevisionChanged,
    ReviewRequired,
    ExternalWaitCleared,
    DispatchStalled,
    BudgetExhausted,
    IntegrityRefusal,
    ResourceConflict,
}
pub const NOTIFICATIONS: &[NotificationKind] = &[
    NotificationKind::RevisionChanged,
    NotificationKind::ReviewRequired,
    NotificationKind::ExternalWaitCleared,
    NotificationKind::DispatchStalled,
    NotificationKind::BudgetExhausted,
    NotificationKind::IntegrityRefusal,
    NotificationKind::ResourceConflict,
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallerDeclaredAuditContext {
    pub declared_actor_id: String,
    pub declared_capability_ids: Vec<String>,
    pub declared_operation_scope_id: String,
    pub declared_audience: String,
    pub declared_source_boundary_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneRequest {
    pub schema: String,
    pub request_id: String,
    pub idempotency_key: String,
    pub tool: ControlPlaneTool,
    pub base_revision_id: Option<String>,
    /// Caller attribution only. This is not a validated CaseGraphen operation
    /// gate and never authorizes an acceptance-ledger mutation.
    pub caller_declared_audit_context: Option<CallerDeclaredAuditContext>,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalCasegraphenAuthorization {
    #[default]
    NotEvaluated,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneAuthorityFacts {
    pub caller_declared_audit_context_present: bool,
    pub canonical_casegraphen_authorization: CanonicalCasegraphenAuthorization,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneRefusal {
    pub code: String,
    pub detail: String,
    pub supplied_base_revision_id: Option<String>,
    pub current_revision_id: Option<String>,
    pub suggested_next_operation: String,
}

impl ControlPlaneRefusal {
    pub fn stale(supplied: impl Into<String>, current: impl Into<String>) -> Self {
        Self {
            code: "stale_revision".to_owned(),
            detail: "client-observed base revision is not current".to_owned(),
            supplied_base_revision_id: Some(supplied.into()),
            current_revision_id: Some(current.into()),
            suggested_next_operation: "re_read_state_and_resubmit_explicitly".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneResponse {
    pub schema: String,
    pub sequence: u64,
    pub request_id: String,
    pub idempotency_key: String,
    pub replayed: bool,
    /// Old experimental durable journals omitted these facts. Loading them
    /// defaults conservatively to no caller context and no evaluated authority.
    #[serde(default)]
    pub authority_facts: ControlPlaneAuthorityFacts,
    pub result: Option<Value>,
    pub refusal: Option<ControlPlaneRefusal>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneNotification {
    pub schema: String,
    pub notification_id: String,
    pub sequence: u64,
    pub kind: NotificationKind,
    pub subject_uri: String,
    pub observed_revision_id: Option<String>,
    pub payload: Value,
    pub authorizes_action: bool,
}

pub trait DecisionDelegate {
    fn invoke(&mut self, request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal>;
}

pub trait ResourceDelegate {
    fn read_resource(&mut self, uri: &str) -> Result<Value, ControlPlaneRefusal>;
}

pub fn read_resource(
    uri: &str,
    delegate: &mut impl ResourceDelegate,
) -> Result<Value, ControlPlaneRefusal> {
    if !uri.starts_with("casegraphen://") {
        return Err(local_refusal(
            "unsupported_resource_uri",
            "resource URI must use the casegraphen scheme",
        ));
    }
    let value = delegate.read_resource(uri)?;
    // Layer 2 of ADR 0034, extended to `resources/read` by ADR 0036 (#122):
    // this function is the single chokepoint every resource read flows
    // through before reaching the wire (`mcp_stdio.rs::read_resource_request`
    // calls nowhere else), exactly as `ControlPlaneState::execute` is the
    // chokepoint for `tools/call`. The vocabulary and the comparison
    // (`claim_vocabulary_violation`) are shared with that path verbatim,
    // because the question is identical: does this top-level object claim
    // something only the canonical review morphism may truthfully claim?
    // What differs is what a violation means here — a resource read has no
    // request/response envelope to journal a refusal into, so this refuses
    // the read itself rather than converting a delegate result into an
    // envelope refusal.
    //
    // Unlike `execute`, a non-object top-level value is not itself a
    // violation on this path: `result`/`refusal` exclusivity is a property
    // of the `tools/call` envelope that `resources/read` does not have, so
    // there is no equivalent shape this path must reject. A non-object
    // resource value simply has no top-level key for the vocabulary to
    // apply to, so the check is a no-op rather than a refusal.
    if let Value::Object(object) = &value {
        if let Some(detail) = claim_vocabulary_violation(object) {
            return Err(resource_wire_claim_refusal(&detail));
        }
    }
    Ok(value)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PendingRequest {
    request_digest: String,
    semantic_digest: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ControlPlaneState {
    next_sequence: u64,
    by_request: BTreeMap<String, (String, ControlPlaneResponse)>,
    by_idempotency_key: BTreeMap<String, (String, ControlPlaneResponse)>,
    notifications: BTreeMap<String, (String, ControlPlaneNotification)>,
    pending_by_idempotency_key: BTreeMap<String, PendingRequest>,
}

impl ControlPlaneState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads crash-safe protocol state. Missing files create an empty state;
    /// malformed files are never adopted; and `next_sequence` is recomputed
    /// from the journal rather than taken from the file.
    pub fn load_durable(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
                .and_then(|state| {
                    state.orderable_cursor_journals()?;
                    Ok(state.with_derived_next_sequence())
                }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::new()),
            Err(error) => Err(error),
        }
    }

    /// A sequence that cannot order an entry is a malformed journal, which
    /// `load_durable` already refuses to adopt (issue #135).
    ///
    /// This and the derived counter are guarantees of `load_durable`, not of
    /// the type. `ControlPlaneState` is `pub` and derives `Deserialize`, so a
    /// consumer deserializing one directly gets neither; `src/mcp_stdio.rs` is
    /// this crate's only consumer and goes through `load_durable`. ADR 0034's
    /// open questions record why neither remedy for that was taken.
    ///
    /// Deriving `next_sequence` stops a rolled-back counter from renumbering
    /// *future* responses, but it cannot repair a sequence already written
    /// into an entry, and two such values silently remove an entry from the
    /// reconnect cursor rather than misordering it. `replay_after` filters
    /// strictly `sequence > after`, so a zero is invisible to every cursor
    /// including a fresh client's `after_sequence: 0` — the one value with
    /// that property, and enough to hide this build's own
    /// `noncanonical_journaled_response` refusal from the surface an operator
    /// reconnects on. A repeated sequence is the same harm reached the other
    /// way: a client that advances its cursor past the first entry never sees
    /// the second.
    ///
    /// Neither can be caught in `journaled_response_violation`, which is the
    /// natural-looking home for it: that predicate runs inside
    /// `serve_journaled`, and an entry the cursor filters out never reaches
    /// `serve_journaled` at all. Adoption is the only point that sees the
    /// whole journal, so the check belongs here.
    ///
    /// Only the two journals a cursor reads are checked. `by_idempotency_key`
    /// mirrors `by_request`'s responses under a second key, so its sequences
    /// legitimately repeat those and no cursor reads it; it contributes to the
    /// derivation and nothing else.
    fn orderable_cursor_journals(&self) -> io::Result<()> {
        let responses = self
            .by_request
            .values()
            .map(|(_, response)| ("response", response.sequence));
        let notifications = self
            .notifications
            .values()
            .map(|(_, notification)| ("notification", notification.sequence));
        let mut seen = BTreeMap::new();
        for (kind, sequence) in responses.chain(notifications) {
            if sequence == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "journaled {kind} carries sequence 0, which no build of this crate \
                         ever assigns and which no reconnect cursor can reach"
                    ),
                ));
            }
            if seen.insert((kind, sequence), ()).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "sequence {sequence} is journaled by more than one {kind}, so a \
                         reconnect cursor cannot distinguish them"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// `next_sequence` is derived, so it is recomputed on adoption instead of
    /// trusted (issue #135).
    ///
    /// This closes a limit on the guarantee the rest of this fix adds; it is
    /// not a regression that fix introduced. `sequence` was carried through
    /// replay identically before, and the value is unchanged by that change.
    /// What that change did was make the limit visible: re-deciding what a
    /// journaled response may *say* is worth much less while the value
    /// deciding whether it is served at all is still taken on trust, because
    /// `replay_after` uses `sequence` as both its filter key and its sort key.
    /// A counter rolled back below the journal makes `execute` reissue
    /// numbers the journal already contains, and a client reconnecting past
    /// that point loses a genuine response with no refusal and
    /// `isError: false`. Silent loss is worse than the wrong-content case,
    /// and it can hide this build's own `noncanonical_journaled_response`
    /// refusal from the surface an operator reconnects on.
    ///
    /// The reachability is honest about itself: `next_sequence` has had these
    /// semantics since the control plane's first commit, so no honest build
    /// of any version ever wrote a state whose counter disagrees with its
    /// journal, and two hosts sharing a `--state` path clobber each other
    /// with self-consistent files rather than producing one. Getting here
    /// takes a deliberately edited journal. The reason to derive anyway is
    /// `CLAUDE.md`'s rule rather than a threat model: derived state is never
    /// stored, and #139 removed this same shape from the promotion ledger in
    /// 0.9.1. The counter is recomputable from the journal, so storing it
    /// only creates a second answer to a question with one.
    ///
    /// The recomputation is exact, not a repair. `execute` and
    /// `publish_notification` are the only two writers, both increment before
    /// assigning, and both always journal what they numbered; `local_refusal`
    /// reads the counter without incrementing but its responses are never
    /// journaled. So in every state this crate writes, `next_sequence` equals
    /// the largest journaled sequence, and zero when the journal is empty —
    /// which is exactly what this computes. Taking the maximum with the stored
    /// value instead would preserve a counter forged *forward*, where
    /// `u64::MAX` makes the next increment overflow; deriving outright leaves
    /// no stored value to forge in either direction.
    fn with_derived_next_sequence(mut self) -> Self {
        let journaled = self
            .by_request
            .values()
            .chain(self.by_idempotency_key.values())
            .map(|(_, response)| response.sequence)
            .chain(
                self.notifications
                    .values()
                    .map(|(_, notification)| notification.sequence),
            );
        self.next_sequence = journaled.max().unwrap_or(0);
        self
    }

    /// Executes with a write-ahead pending marker. A crash after a delegated
    /// effect but before its response is durably committed becomes an explicit
    /// ambiguity refusal on restart and is never invoked a second time.
    pub fn execute_durable(
        &mut self,
        request: &ControlPlaneRequest,
        delegate: &mut impl DecisionDelegate,
        state_path: &Path,
    ) -> ControlPlaneResponse {
        let request_digest = digest(request);
        let semantic_digest = request_semantic_digest(request);
        if let Some(pending) = self
            .pending_by_idempotency_key
            .get(&request.idempotency_key)
        {
            return if pending.semantic_digest == semantic_digest
                && pending.request_digest == request_digest
            {
                self.local_refusal(
                    request,
                    "ambiguous_prior_effect",
                    "a previous process may have delegated this request before acknowledgement; operator reconciliation is required",
                )
            } else {
                self.local_refusal(
                    request,
                    "idempotency_key_collision",
                    "a pending idempotency key names different request content",
                )
            };
        }
        if self.by_request.contains_key(&request.request_id)
            || self
                .by_idempotency_key
                .contains_key(&request.idempotency_key)
        {
            return self.execute(request, delegate);
        }

        self.pending_by_idempotency_key.insert(
            request.idempotency_key.clone(),
            PendingRequest {
                request_digest,
                semantic_digest,
            },
        );
        if let Err(error) = self.persist_durable(state_path) {
            self.pending_by_idempotency_key
                .remove(&request.idempotency_key);
            return self.local_refusal(
                request,
                "durable_state_unavailable",
                &format!("request was not delegated because the write-ahead state failed: {error}"),
            );
        }

        let response = self.execute(request, delegate);
        let mut committed = self.clone();
        committed
            .pending_by_idempotency_key
            .remove(&request.idempotency_key);
        match committed.persist_durable(state_path) {
            Ok(()) => {
                *self = committed;
                response
            }
            Err(error) => self.local_refusal(
                request,
                "durable_acknowledgement_failed",
                &format!("delegation outcome was not acknowledged because durable commit failed: {error}"),
            ),
        }
    }

    /// Persists notification/replay state with same-directory atomic rename.
    pub fn persist_durable(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("control-plane.state");
        let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if let Ok(directory) = fs::File::open(parent) {
            directory.sync_all()?;
        }
        Ok(())
    }

    pub fn execute(
        &mut self,
        request: &ControlPlaneRequest,
        delegate: &mut impl DecisionDelegate,
    ) -> ControlPlaneResponse {
        let digest = digest(request);
        let semantic_digest = request_semantic_digest(request);
        if let Some((existing_digest, response)) = self.by_request.get(&request.request_id) {
            return if existing_digest == &digest {
                serve_journaled(response)
            } else {
                self.local_refusal(
                    request,
                    "request_id_collision",
                    "request id names different content",
                )
            };
        }
        if let Some((existing_digest, response)) =
            self.by_idempotency_key.get(&request.idempotency_key)
        {
            if existing_digest != &semantic_digest {
                return self.local_refusal(
                    request,
                    "idempotency_key_collision",
                    "idempotency key names different request content",
                );
            }
            return serve_journaled(response);
        }
        if request.schema != CONTROL_PLANE_REQUEST_SCHEMA {
            return self.local_refusal(request, "unsupported_schema", "unsupported request schema");
        }
        if request.request_id.is_empty() || request.idempotency_key.is_empty() {
            return self.local_refusal(
                request,
                "missing_identity",
                "request and idempotency ids are required",
            );
        }
        if request.tool.changes_managed_state()
            && (request
                .base_revision_id
                .as_deref()
                .map_or(true, str::is_empty)
                || request.caller_declared_audit_context.is_none())
        {
            return self.local_refusal(
                request,
                "explicit_mutation_audit_context_required",
                "state-changing host tools require a client-supplied base revision and caller-declared audit context; this context does not authorize the operation",
            );
        }
        if request.tool.requires_base_revision()
            && request
                .base_revision_id
                .as_deref()
                .map_or(true, str::is_empty)
        {
            return self.local_refusal(
                request,
                "explicit_revision_context_required",
                "this tool requires the exact client-observed base revision",
            );
        }

        self.next_sequence += 1;
        let (result, refusal) = match delegate.invoke(request) {
            Ok(value) => match wire_claim_violation(&value) {
                None => (Some(value), None),
                Some(detail) => (None, Some(wire_claim_refusal(&detail))),
            },
            Err(refusal) => (None, Some(refusal)),
        };
        let response = ControlPlaneResponse {
            schema: CONTROL_PLANE_RESPONSE_SCHEMA.to_owned(),
            sequence: self.next_sequence,
            request_id: request.request_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            replayed: false,
            authority_facts: authority_facts(request),
            result,
            refusal,
        };
        self.by_request.insert(
            request.request_id.clone(),
            (digest.clone(), response.clone()),
        );
        self.by_idempotency_key.insert(
            request.idempotency_key.clone(),
            (semantic_digest, response.clone()),
        );
        response
    }

    /// Returns journaled responses after a reconnect cursor in logical order.
    /// This is a second replay surface, not a debugging view — its output
    /// reaches a caller over `casegraphen/replay` — so it serves through
    /// `serve_journaled` exactly as `execute`'s two replay lookups do.
    pub fn replay_after(&self, sequence: u64) -> Vec<ControlPlaneResponse> {
        let mut responses = self
            .by_request
            .values()
            .map(|(_, response)| response)
            .filter(|response| response.sequence > sequence)
            .map(serve_journaled)
            .collect::<Vec<_>>();
        responses.sort_by_key(|response| response.sequence);
        responses
    }

    /// Returns protocol notifications after a reconnect cursor in logical order.
    pub fn notifications_after(&self, sequence: u64) -> Vec<ControlPlaneNotification> {
        let mut notifications = self
            .notifications
            .values()
            .map(|(_, notification)| serve_journaled_notification(notification))
            .filter(|notification| notification.sequence > sequence)
            .collect::<Vec<_>>();
        notifications.sort_by_key(|notification| notification.sequence);
        notifications
    }

    pub fn publish_notification(
        &mut self,
        mut notification: ControlPlaneNotification,
    ) -> Result<ControlPlaneNotification, ControlPlaneRefusal> {
        force_protocol_owned_facts(&mut notification);
        let content_digest = notification_digest(&notification);
        if let Some((existing_digest, existing)) =
            self.notifications.get(&notification.notification_id)
        {
            return if existing_digest == &content_digest {
                Ok(serve_journaled_notification(existing))
            } else {
                Err(local_refusal(
                    "notification_id_collision",
                    "notification id names different content",
                ))
            };
        }
        self.next_sequence += 1;
        notification.sequence = self.next_sequence;
        let content_digest = notification_digest(&notification);
        self.notifications.insert(
            notification.notification_id.clone(),
            (content_digest, notification.clone()),
        );
        Ok(notification)
    }

    /// Publishes and durably commits a notification before returning it.
    pub fn publish_notification_durable(
        &mut self,
        notification: ControlPlaneNotification,
        state_path: &Path,
    ) -> Result<ControlPlaneNotification, ControlPlaneRefusal> {
        let previous = self.clone();
        let published = self.publish_notification(notification)?;
        if let Err(error) = self.persist_durable(state_path) {
            *self = previous;
            return Err(local_refusal(
                "durable_state_unavailable",
                &format!("notification was not acknowledged because persistence failed: {error}"),
            ));
        }
        Ok(published)
    }

    fn local_refusal(
        &self,
        request: &ControlPlaneRequest,
        code: &str,
        detail: &str,
    ) -> ControlPlaneResponse {
        ControlPlaneResponse {
            schema: CONTROL_PLANE_RESPONSE_SCHEMA.to_owned(),
            sequence: self.next_sequence,
            request_id: request.request_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            replayed: false,
            authority_facts: authority_facts(request),
            result: None,
            refusal: Some(local_refusal(code, detail)),
        }
    }
}

fn local_refusal(code: &str, detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: code.to_owned(),
        detail: detail.to_owned(),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "correct_request_and_retry".to_owned(),
    }
}

/// Layer 2 of ADR 0034: the same seven-key claim vocabulary the response
/// schema pins at layer 1, checked against a delegate's raw result before it
/// is journaled, at the result's top level only. A delegate is per-tool data
/// (`json!` literals at construction sites), not a typed struct field, so
/// this is the one Rust-side place the rule is stated — do not restate it at
/// a call site. Nested occurrences (depth >= 1) are payload semantics a
/// payload's own contract governs, exactly as at layer 1: reads truthfully
/// echo accepted ledger state below the top level, so this check must not
/// see into `result`'s nested structure. Returns a description of the first
/// offending key/value pair found, or `None` if the result carries no
/// forbidden claim.
///
/// Both call sites establish the same precondition: this function only ever
/// sees a value that is about to be, or already is, an envelope's `result`
/// with no accompanying `refusal` — `execute` calls it on a delegate's
/// `Ok(value)` payload and never on the `Err(refusal)` branch, and
/// `journaled_response_violation` calls it only from its `(Some(result),
/// None)` arm. `control_plane.response.v0`'s top-level `result`/`refusal`
/// exclusivity pin admits `result: null` only paired with a non-null
/// `refusal` — the shape `execute` produces from the `Err` branch, which
/// never reaches this function, and the shape
/// `journaled_response_violation` handles in its own `(None, Some(_))` arm
/// without consulting this one. A value here is therefore never that
/// legitimate shape: it would make `result` and `refusal` both serialize to
/// `null` (`Option<Value>`'s `Some(Value::Null)` and `None` are
/// indistinguishable on the wire), which is exactly the state the envelope's
/// `oneOf` forbids. So `Value::Null` here is a violation like any other
/// non-object shape, not an exemption from one — the only value that ever
/// makes it past this check is `Value::Object`.
fn wire_claim_violation(result: &Value) -> Option<String> {
    let object = match result {
        Value::Object(object) => object,
        other => {
            let kind = match other {
                Value::Null => "null",
                Value::Bool(_) => "a boolean",
                Value::Number(_) => "a number",
                Value::String(_) => "a string",
                Value::Array(_) => "an array",
                Value::Object(_) => unreachable!("matched above"),
            };
            return Some(format!(
                "result is {kind}, but a successful delegate result must be an object \
                 (the envelope's null result belongs only to a refusal, and this call \
                 returned no refusal)"
            ));
        }
    };
    claim_vocabulary_violation(object)
}

/// The seven-key claim vocabulary ADR 0034 pins at the top level of a
/// `tools/call` result, shared verbatim with the `resources/read` chokepoint
/// (`read_resource`, above; ADR 0036 / #122). Only this predicate — the
/// vocabulary and the truthful-value comparison — is shared: the two call
/// sites answer the same question ("does this top-level object claim
/// something only the canonical review morphism may truthfully claim?"), but
/// what a violation means differs by call site (an envelope refusal for
/// `tools/call`, a resource-read refusal here), so each keeps its own
/// wrapper and refusal construction rather than a single function trying to
/// serve both.
fn claim_vocabulary_violation(object: &serde_json::Map<String, Value>) -> Option<String> {
    let pinned_values: &[(&str, Value)] = &[
        ("accepted", Value::Bool(false)),
        ("mutation_performed", Value::Bool(false)),
        ("read_only", Value::Bool(true)),
        ("accepted_runtime_output", Value::Bool(false)),
        ("proofs_serialized", Value::Bool(false)),
        ("review_status", Value::String("unreviewed".to_owned())),
        (
            "generated_plan_review_status",
            Value::String("unreviewed".to_owned()),
        ),
    ];
    for (key, pinned) in pinned_values {
        if let Some(actual) = object.get(*key) {
            if actual != pinned {
                return Some(format!("{key} = {actual}, but only {pinned} is truthful"));
            }
        }
    }
    None
}

/// Layer 2's refusal for a claim `wire_claim_violation` catches. This
/// deliberately diverges from `publish_notification` forcing
/// `authorizes_action = false`: a notification is a record this protocol
/// layer constructs and owns outright, but a tool result has an author — the
/// delegate — and rewriting its claim to the truthful value would silently
/// launder the exact condition this check exists to surface, and would let a
/// defective or compromised delegate keep operating behind a protocol layer
/// that cleans up after it. Refusing turns the event into something the
/// caller and the journal both see, and a replay of the same request replays
/// this refusal, never the false claim.
fn wire_claim_refusal(detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: "noncanonical_wire_claim".to_owned(),
        detail: format!("delegate result carried a forbidden top-level wire claim: {detail}"),
        supplied_base_revision_id: None,
        current_revision_id: None,
        // Not `correct_request_and_retry`: the caller did nothing wrong, and
        // retrying will not change the delegate's defect. This is a
        // host-side defect surface (ADR 0034), so the suggested operation
        // says so instead of implying the caller can fix its own input.
        suggested_next_operation: "report_host_defect".to_owned(),
    }
}

/// Resource-read counterpart to `wire_claim_refusal` (ADR 0036 / #122): the
/// vocabulary and comparison are shared via `claim_vocabulary_violation`,
/// but the code is distinct — `noncanonical_wire_claim` names a `tools/call`
/// envelope refusal, and a resource read has no envelope to name, so reusing
/// that code would make the two failure surfaces indistinguishable to a
/// caller that branches on it.
fn resource_wire_claim_refusal(detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: "noncanonical_resource_wire_claim".to_owned(),
        detail: format!("resource read carried a forbidden top-level wire claim: {detail}"),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "report_host_defect".to_owned(),
    }
}

/// Turns a journaled response into a wire response, re-deciding ADR 0034's
/// layers 1 and 2 instead of trusting that some build already did. Layer 3 is
/// not re-decided; see the scope note at the end of this comment.
///
/// ADR 0034 justified checking a delegate result exactly once, at compute
/// time, on the ground that "a replayed response from this host version was
/// checked when first computed". That holds only for *this* version, and
/// `ControlPlaneState` records nothing that establishes it: it persists
/// `next_sequence`, the two response indexes, the notifications and the
/// pending markers, and no marker of the build that wrote them (issue #135).
/// So a response journaled before #120 made a non-object result a violation,
/// or before any later tightening, was served past layer 1 and layer 2 alike,
/// with `replayed: true` as the only signal — and `replayed: true` says a
/// response is a repeat, not that it was never checked.
///
/// Re-deciding here rather than recording an epoch makes the guarantee hold by
/// construction: a non-conforming journaled response becomes unrepresentable
/// on the wire, whatever wrote it. It also needs no new stored state, so the
/// state format is unchanged and nothing is migrated or rejected at startup,
/// and it depends on no value a human has to remember to bump.
///
/// This is the single place a journaled response becomes a wire response —
/// `execute`'s request-id and idempotency-key lookups and `replay_after`'s
/// reconnect cursor all route through it — so adding a fourth replay surface
/// that does not is the way to reintroduce the defect.
///
/// **Scope, stated precisely, because an overstated guarantee is its own
/// defect.** Two things this does not establish:
///
/// *Layer 3 is not re-decided.* ADR 0034's third layer is the per-payload
/// contracts, where each record knows what it must say — the omission half of
/// the defense, and the half that governs claims nested below the top level.
/// On the fresh path those are typed structs the delegate constructs, so the
/// shape is right by construction. A journaled response is an
/// `Option<Value>`, and re-deriving the payload contract from it is not merely
/// a second implementation of layer 3: it is not well defined, because the
/// envelope does not record which tool produced the result (ADR 0034 pins the
/// vocabulary rather than requiring keys for exactly this reason), and
/// `replay_after` does not even have the request to ask. So a nested
/// `claim_proposal.accepted: true` in a journaled result is served, with
/// `isError: false`, exactly as it would be on the fresh path. On the replay
/// path that remains a consumer-side obligation. Giving the envelope tool
/// identity would change that, and is a contract decision, not a fix.
///
/// *A journal is not trustworthy.* Nothing authenticates the state file, so
/// anyone who can write it can still put an arbitrary conforming result in
/// the caller's hands — the seven-key vocabulary constrains what a stored
/// response may *claim*, not what it may say. That is the same guarantee
/// `execute` gives a freshly computed result, which is the point: the journal
/// now buys such a writer nothing a compromised delegate would not, where
/// before it bought them the whole vocabulary. Integrity of the state file
/// itself is a filesystem question — see the `--state` note in
/// `docs/guides/mcp-operational-host.md`.
fn serve_journaled(response: &ControlPlaneResponse) -> ControlPlaneResponse {
    match journaled_response_violation(response) {
        None => ControlPlaneResponse {
            replayed: true,
            ..response.clone()
        },
        // The journal is not rewritten. The refusal is derived from the stored
        // entry on every service, never stored in place of it: overwriting the
        // entry would destroy the record of what was actually served, and a
        // stored refusal would be one more piece of state a later build has to
        // trust rather than re-decide. `replay_after` takes `&self` for the
        // same reason. Identity fields are carried over so the caller can
        // correlate this refusal with the response it replaces.
        Some(detail) => ControlPlaneResponse {
            schema: CONTROL_PLANE_RESPONSE_SCHEMA.to_owned(),
            sequence: response.sequence,
            request_id: response.request_id.clone(),
            idempotency_key: response.idempotency_key.clone(),
            replayed: true,
            authority_facts: response.authority_facts.clone(),
            result: None,
            refusal: Some(journaled_response_refusal(&detail)),
        },
    }
}

/// The response-side contract `execute` establishes for a freshly computed
/// response, restated as a question about a journaled one. Layer 1's envelope
/// pin — the schema identity and `control_plane.response.v0`'s
/// `result`/`refusal` exclusivity `oneOf` — and layer 2's claim vocabulary,
/// via `wire_claim_violation` itself rather than a replay-path copy of it.
///
/// The request-side checks `execute` runs before delegating (schema, identity,
/// mutation audit context, revision context) are deliberately not re-run here.
/// Three independent adversarial passes reached this conclusion, so treat this
/// paragraph as the record of a decision rather than an untested opinion.
///
/// They are questions about an input, and a journaled entry is evidence that
/// the input was already delegated: refusing a replay because today's build
/// asks more of the request would tell the caller nothing happened, when a
/// durable effect may have. That trade is worth taking for a response whose
/// content is false — refusing beats repeating a lie — and is a pure loss for
/// one that is merely old, so the line is drawn at what the response says.
///
/// Re-running them would also buy nothing against the only adversary who
/// could exploit their absence. The replay lookups compare a digest taken
/// over the whole request, and anyone able to write the journal computes that
/// digest themselves, so they can already bind any response to any request
/// they like. Checking the request again on the way out cannot take that
/// back.
fn journaled_response_violation(response: &ControlPlaneResponse) -> Option<String> {
    if response.schema != CONTROL_PLANE_RESPONSE_SCHEMA {
        return Some(format!(
            "envelope schema is {}, but this build serves only {CONTROL_PLANE_RESPONSE_SCHEMA}",
            response.schema
        ));
    }
    match (&response.result, &response.refusal) {
        (Some(result), None) => wire_claim_violation(result),
        (None, Some(_)) => None,
        (Some(_), Some(_)) => {
            Some("envelope carries both a result and a refusal, which its oneOf forbids".to_owned())
        }
        (None, None) => Some(
            "envelope carries neither a result nor a refusal, which its oneOf forbids".to_owned(),
        ),
    }
}

/// Replay counterpart to `wire_claim_refusal`, distinct in code for the reason
/// `resource_wire_claim_refusal` is — a caller that branches on the code must
/// be able to tell the surfaces apart — and because it reports a different
/// fact. `noncanonical_wire_claim` says a delegate produced a forbidden claim
/// just now, and nothing was served. This says a response *already served*
/// carried one, and that this host will not serve it again.
///
/// The detail says both halves out loud, because the operator meeting this is
/// meeting a request that is refused today and succeeded yesterday, and the
/// difference is entirely in this host, not in their request. Two consequences
/// follow that a bare claim refusal would not carry: whatever the earlier
/// response was acted on for is suspect, and the original delegation's effect
/// stands — this refusal withholds a response, it does not undo anything.
/// `report_host_defect` alone would understate that, so the suggested
/// operation names the audit first.
fn journaled_response_refusal(detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: "noncanonical_journaled_response".to_owned(),
        detail: format!(
            "a journaled response for this request does not satisfy this build's response \
             contract and will not be replayed: {detail}. It was journaled, and served, by a \
             host whose contract differed from this one's; any effect of that original \
             delegation stands and is not undone by this refusal."
        ),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "audit_prior_response_and_report_host_defect".to_owned(),
    }
}

/// The two facts `publish_notification` forces rather than accepts from its
/// caller. Serving a notification back out of the journal forces them again,
/// for the reason `serve_journaled` re-checks a response: the journal may have
/// been written by a build that forced something else, or nothing.
///
/// The disposition differs from the response path, and does so for ADR 0034's
/// own stated reason. A notification is a record this protocol layer
/// constructs and owns outright, so overwriting these two fields overrules no
/// author and hides nothing — which is why publishing forces rather than
/// refuses. A response has an author, so its path refuses rather than
/// launders. Keeping each path's disposition means the rule stated at publish
/// time and the rule applied at service time are the same rule, in one place.
fn force_protocol_owned_facts(notification: &mut ControlPlaneNotification) {
    notification.schema = CONTROL_PLANE_NOTIFICATION_SCHEMA.to_owned();
    notification.authorizes_action = false;
}

fn serve_journaled_notification(
    notification: &ControlPlaneNotification,
) -> ControlPlaneNotification {
    let mut served = notification.clone();
    force_protocol_owned_facts(&mut served);
    served
}

fn authority_facts(request: &ControlPlaneRequest) -> ControlPlaneAuthorityFacts {
    ControlPlaneAuthorityFacts {
        caller_declared_audit_context_present: request.caller_declared_audit_context.is_some(),
        canonical_casegraphen_authorization: CanonicalCasegraphenAuthorization::NotEvaluated,
    }
}

fn digest(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("control-plane value serializes");
    format!("{:x}", Sha256::digest(bytes))
}

fn request_semantic_digest(request: &ControlPlaneRequest) -> String {
    digest(&(
        request.schema.as_str(),
        request.tool,
        request.base_revision_id.as_deref(),
        request.caller_declared_audit_context.as_ref(),
        &request.payload,
    ))
}

fn notification_digest(notification: &ControlPlaneNotification) -> String {
    digest(&(
        notification.notification_id.as_str(),
        notification.kind,
        notification.subject_uri.as_str(),
        notification.observed_revision_id.as_deref(),
        &notification.payload,
        false,
    ))
}
