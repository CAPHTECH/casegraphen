//! Transport-neutral state for an external MCP-compatible control plane.
//!
//! This module owns protocol idempotency and replay only. A delegate owns every
//! CaseGraphen, compiler, runtime, verification, and resource decision.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const CONTROL_PLANE_REQUEST_SCHEMA: &str = "casegraphen.experimental.control_plane.request.v0";
pub const CONTROL_PLANE_RESPONSE_SCHEMA: &str =
    "casegraphen.experimental.control_plane.response.v0";
pub const CONTROL_PLANE_NOTIFICATION_SCHEMA: &str =
    "casegraphen.experimental.control_plane.notification.v0";

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
    ControlPlaneTool::AttachRuntimeReport,
    ControlPlaneTool::ReconcileRun,
    ControlPlaneTool::ApplyEvidencePacket,
    ControlPlaneTool::ReviewAccept,
    ControlPlaneTool::ReviewReject,
    ControlPlaneTool::Resume,
    ControlPlaneTool::SupersedeDispatch,
    ControlPlaneTool::ReserveResources,
    ControlPlaneTool::ReleaseResources,
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneTool {
    ProposeExecutionTopology,
    LintExecutionTopology,
    CompileDeploymentBundle,
    AttachRuntimeReport,
    ReconcileRun,
    ApplyEvidencePacket,
    ReviewAccept,
    ReviewReject,
    Resume,
    SupersedeDispatch,
    ReserveResources,
    ReleaseResources,
}

impl ControlPlaneTool {
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
pub struct OperationGateInput {
    pub actor_id: String,
    pub capability_ids: Vec<String>,
    pub operation_scope_id: String,
    pub audience: String,
    pub source_boundary_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneRequest {
    pub schema: String,
    pub request_id: String,
    pub idempotency_key: String,
    pub tool: ControlPlaneTool,
    pub base_revision_id: Option<String>,
    pub operation_gate: Option<OperationGateInput>,
    pub payload: Value,
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
    delegate.read_resource(uri)
}

#[derive(Default)]
pub struct ControlPlaneState {
    next_sequence: u64,
    by_request: BTreeMap<String, (String, ControlPlaneResponse)>,
    by_idempotency_key: BTreeMap<String, (String, ControlPlaneResponse)>,
    notifications: BTreeMap<String, (String, ControlPlaneNotification)>,
}

impl ControlPlaneState {
    pub fn new() -> Self {
        Self::default()
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
                replay(response)
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
            return replay(response);
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
                || request.operation_gate.is_none())
        {
            return self.local_refusal(
                request,
                "explicit_mutation_context_required",
                "state-changing tools require client-supplied base revision and operation gate",
            );
        }

        self.next_sequence += 1;
        let (result, refusal) = match delegate.invoke(request) {
            Ok(value) => (Some(value), None),
            Err(refusal) => (None, Some(refusal)),
        };
        let response = ControlPlaneResponse {
            schema: CONTROL_PLANE_RESPONSE_SCHEMA.to_owned(),
            sequence: self.next_sequence,
            request_id: request.request_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            replayed: false,
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

    pub fn replay_after(&self, sequence: u64) -> Vec<ControlPlaneResponse> {
        let mut responses = self
            .by_request
            .values()
            .map(|(_, response)| response)
            .filter(|response| response.sequence > sequence)
            .cloned()
            .collect::<Vec<_>>();
        responses.sort_by_key(|response| response.sequence);
        responses
    }

    /// Returns protocol notifications after a reconnect cursor in logical order.
    pub fn notifications_after(&self, sequence: u64) -> Vec<ControlPlaneNotification> {
        let mut notifications = self
            .notifications
            .values()
            .map(|(_, notification)| notification)
            .filter(|notification| notification.sequence > sequence)
            .cloned()
            .collect::<Vec<_>>();
        notifications.sort_by_key(|notification| notification.sequence);
        notifications
    }

    pub fn publish_notification(
        &mut self,
        mut notification: ControlPlaneNotification,
    ) -> Result<ControlPlaneNotification, ControlPlaneRefusal> {
        notification.authorizes_action = false;
        notification.schema = CONTROL_PLANE_NOTIFICATION_SCHEMA.to_owned();
        let content_digest = notification_digest(&notification);
        if let Some((existing_digest, existing)) =
            self.notifications.get(&notification.notification_id)
        {
            return if existing_digest == &content_digest {
                Ok(existing.clone())
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

fn replay(response: &ControlPlaneResponse) -> ControlPlaneResponse {
    ControlPlaneResponse {
        replayed: true,
        ..response.clone()
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
        request.operation_gate.as_ref(),
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
