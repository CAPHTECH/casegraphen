#![allow(missing_docs)]

use casegraphen::{
    control_plane::{
        ControlPlaneNotification, ControlPlaneRefusal, ControlPlaneRequest, ControlPlaneState,
        ControlPlaneTool, DecisionDelegate, NotificationKind, OperationGateInput,
        CONTROL_PLANE_NOTIFICATION_SCHEMA, CONTROL_PLANE_REQUEST_SCHEMA, NOTIFICATIONS,
        RESOURCE_TEMPLATES, TOOLS,
    },
    execution_topology::parse_execution_topology,
    graph_lint::lint_execution_topology,
};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

struct CountingDelegate {
    calls: usize,
    stale: bool,
}

impl DecisionDelegate for CountingDelegate {
    fn invoke(&mut self, request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal> {
        self.calls += 1;
        if self.stale {
            return Err(ControlPlaneRefusal::stale(
                request.base_revision_id.as_deref().unwrap_or("missing"),
                "revision:current",
            ));
        }
        if request.tool == ControlPlaneTool::LintExecutionTopology {
            let topology = parse_execution_topology(
                request.payload["topology_json"]
                    .as_str()
                    .expect("topology JSON string"),
            )
            .expect("typed topology");
            return Ok(serde_json::to_value(lint_execution_topology(&topology)).unwrap());
        }
        Ok(json!({"delegated": true}))
    }
}

fn gate() -> OperationGateInput {
    OperationGateInput {
        actor_id: "actor:test".to_owned(),
        capability_ids: vec!["capability:test".to_owned()],
        operation_scope_id: "scope:test".to_owned(),
        audience: "audit".to_owned(),
        source_boundary_id: "boundary:test".to_owned(),
    }
}

fn request(tool: ControlPlaneTool, id: &str) -> ControlPlaneRequest {
    ControlPlaneRequest {
        schema: CONTROL_PLANE_REQUEST_SCHEMA.to_owned(),
        request_id: id.to_owned(),
        idempotency_key: format!("idempotency:{id}"),
        tool,
        base_revision_id: tool
            .changes_managed_state()
            .then(|| "revision:observed".to_owned()),
        operation_gate: tool.changes_managed_state().then(gate),
        payload: json!({}),
    }
}

#[test]
fn catalog_contains_the_required_mcp_compatible_surface() {
    assert_eq!(RESOURCE_TEMPLATES.len(), 7);
    assert_eq!(TOOLS.len(), 12);
    assert_eq!(NOTIFICATIONS.len(), 7);
    for required in [
        "casegraphen://spaces/{id}/status",
        "casegraphen://spaces/{id}/frontier",
        "casegraphen://runs/{run_id}",
        "casegraphen://topologies/{topology_id}",
    ] {
        assert!(RESOURCE_TEMPLATES.contains(&required));
    }
    let tools = serde_json::to_value(TOOLS).unwrap();
    assert!(tools
        .as_array()
        .unwrap()
        .contains(&json!("reserve_resources")));
    assert!(tools
        .as_array()
        .unwrap()
        .contains(&json!("apply_evidence_packet")));
}

#[test]
fn wire_schema_catalog_is_exactly_compatible_with_the_rust_catalog() {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("schemas/experimental/control_plane.catalog.v0.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema["properties"]["resources"]["const"],
        serde_json::to_value(RESOURCE_TEMPLATES).unwrap()
    );
    assert_eq!(
        schema["properties"]["tools"]["const"],
        serde_json::to_value(TOOLS).unwrap()
    );
    assert_eq!(
        schema["properties"]["notifications"]["const"],
        serde_json::to_value(NOTIFICATIONS).unwrap()
    );
    let output = Command::new("python3")
        .args(["-m", "jsonschema", "-i"])
        .arg(root().join("schemas/experimental/control_plane.catalog.v0.example.json"))
        .arg(root().join("schemas/experimental/control_plane.catalog.v0.schema.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for stem in [
        "control_plane.request.v0",
        "control_plane.response.v0",
        "control_plane.notification.v0",
    ] {
        let output = Command::new("python3")
            .args(["-m", "jsonschema", "-i"])
            .arg(root().join(format!("schemas/experimental/{stem}.example.json")))
            .arg(root().join(format!("schemas/experimental/{stem}.schema.json")))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{stem}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn every_state_changing_tool_requires_explicit_revision_and_gate() {
    for &tool in TOOLS.iter().filter(|tool| tool.changes_managed_state()) {
        let mut request = request(tool, &format!("request:{tool:?}"));
        request.base_revision_id = None;
        request.operation_gate = None;
        let mut state = ControlPlaneState::new();
        let mut delegate = CountingDelegate {
            calls: 0,
            stale: false,
        };
        let response = state.execute(&request, &mut delegate);
        assert_eq!(
            response.refusal.as_ref().unwrap().code,
            "explicit_mutation_context_required"
        );
        assert_eq!(delegate.calls, 0);
    }
}

#[test]
fn reconnect_and_new_request_ids_replay_one_semantic_ingest_or_reservation() {
    for tool in [
        ControlPlaneTool::AttachRuntimeReport,
        ControlPlaneTool::ApplyEvidencePacket,
        ControlPlaneTool::ReserveResources,
    ] {
        let mut state = ControlPlaneState::new();
        let mut delegate = CountingDelegate {
            calls: 0,
            stale: false,
        };
        let first = request(tool, "request:first");
        let first_response = state.execute(&first, &mut delegate);
        let same_response = state.execute(&first, &mut delegate);
        let mut reconnect = first.clone();
        reconnect.request_id = "request:reconnect".to_owned();
        let reconnect_response = state.execute(&reconnect, &mut delegate);
        assert_eq!(delegate.calls, 1);
        assert!(!first_response.replayed);
        assert!(same_response.replayed);
        assert!(reconnect_response.replayed);
        assert_eq!(state.replay_after(0).len(), 1);
    }
}

#[test]
fn stale_revision_is_a_structured_delegated_refusal() {
    let mut state = ControlPlaneState::new();
    let mut delegate = CountingDelegate {
        calls: 0,
        stale: true,
    };
    let response = state.execute(
        &request(ControlPlaneTool::ReviewAccept, "request:stale"),
        &mut delegate,
    );
    let refusal = response.refusal.unwrap();
    assert_eq!(refusal.code, "stale_revision");
    assert_eq!(
        refusal.supplied_base_revision_id.as_deref(),
        Some("revision:observed")
    );
    assert_eq!(
        refusal.current_revision_id.as_deref(),
        Some("revision:current")
    );
}

#[test]
fn mcp_lint_and_cli_lint_are_the_same_decision() {
    let input = root().join("schemas/experimental/execution.topology.file-review.example.json");
    let topology_json = fs::read_to_string(&input).unwrap();
    let mut request = request(ControlPlaneTool::LintExecutionTopology, "request:lint");
    request.payload = json!({"topology_json": topology_json});
    let mut state = ControlPlaneState::new();
    let mut delegate = CountingDelegate {
        calls: 0,
        stale: false,
    };
    let mcp = state.execute(&request, &mut delegate).result.unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["graph", "lint", "--input"])
        .arg(input)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(mcp, cli);
}

#[test]
fn notifications_are_idempotent_and_never_authorize_action() {
    let mut state = ControlPlaneState::new();
    let notification = ControlPlaneNotification {
        schema: CONTROL_PLANE_NOTIFICATION_SCHEMA.to_owned(),
        notification_id: "notification:review".to_owned(),
        sequence: 0,
        kind: NotificationKind::ReviewRequired,
        subject_uri: "casegraphen://spaces/case-1/reviews".to_owned(),
        observed_revision_id: Some("revision:1".to_owned()),
        payload: json!({"target_id": "claim:1"}),
        authorizes_action: true,
    };
    let first = state.publish_notification(notification.clone()).unwrap();
    let second = state.publish_notification(notification).unwrap();
    assert!(!first.authorizes_action);
    assert_eq!(first, second);
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
