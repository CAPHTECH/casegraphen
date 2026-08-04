#![allow(missing_docs)]

use casegraphen::{
    control_plane::{
        CallerDeclaredAuditContext, ControlPlaneNotification, ControlPlaneRefusal,
        ControlPlaneRequest, ControlPlaneState, ControlPlaneTool, DecisionDelegate,
        NotificationKind, CONTROL_PLANE_NOTIFICATION_SCHEMA, CONTROL_PLANE_REQUEST_SCHEMA,
        NOTIFICATIONS, RESOURCE_TEMPLATES, TOOLS,
    },
    execution_topology::parse_execution_topology,
    graph_lint::lint_execution_topology,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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

fn audit_context() -> CallerDeclaredAuditContext {
    CallerDeclaredAuditContext {
        declared_actor_id: "actor:test".to_owned(),
        declared_capability_ids: vec!["capability:test-declared".to_owned()],
        declared_operation_scope_id: "scope:test".to_owned(),
        declared_audience: "audit".to_owned(),
        declared_source_boundary_id: "boundary:test".to_owned(),
    }
}

fn request(tool: ControlPlaneTool, id: &str) -> ControlPlaneRequest {
    ControlPlaneRequest {
        schema: CONTROL_PLANE_REQUEST_SCHEMA.to_owned(),
        request_id: id.to_owned(),
        idempotency_key: format!("idempotency:{id}"),
        tool,
        base_revision_id: tool
            .requires_base_revision()
            .then(|| "revision:observed".to_owned()),
        caller_declared_audit_context: tool.changes_managed_state().then(audit_context),
        payload: json!({}),
    }
}

#[test]
fn catalog_contains_the_required_mcp_compatible_surface() {
    assert_eq!(RESOURCE_TEMPLATES.len(), 7);
    assert_eq!(TOOLS.len(), 18);
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
    for workflow in [
        "compile_deployment_bundle",
        "compile_reviewed_deployment_bundle",
        "reconcile_run",
        "reconcile_resources",
        "simulate_execution_topology",
        "evaluate_expansion_round",
        "reconcile_streaming_run",
        "propose_topology_redesign",
    ] {
        assert!(tools.as_array().unwrap().contains(&json!(workflow)));
    }
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
fn every_state_changing_tool_requires_revision_and_caller_declared_audit_context() {
    for &tool in TOOLS.iter().filter(|tool| tool.changes_managed_state()) {
        let mut request = request(tool, &format!("request:{tool:?}"));
        request.base_revision_id = None;
        request.caller_declared_audit_context = None;
        let mut state = ControlPlaneState::new();
        let mut delegate = CountingDelegate {
            calls: 0,
            stale: false,
        };
        let response = state.execute(&request, &mut delegate);
        assert_eq!(
            response.refusal.as_ref().unwrap().code,
            "explicit_mutation_audit_context_required"
        );
        assert_eq!(delegate.calls, 0);
    }
}

#[test]
fn caller_declared_capabilities_are_recorded_but_never_claim_canonical_authorization() {
    let mut state = ControlPlaneState::new();
    let mut delegate = CountingDelegate {
        calls: 0,
        stale: false,
    };
    let response = state.execute(
        &request(ControlPlaneTool::AttachRuntimeReport, "request:audit-only"),
        &mut delegate,
    );
    assert_eq!(delegate.calls, 1);
    assert!(
        response
            .authority_facts
            .caller_declared_audit_context_present
    );
    assert_eq!(
        serde_json::to_value(response.authority_facts.canonical_casegraphen_authorization).unwrap(),
        json!("not_evaluated")
    );
}

#[test]
fn legacy_durable_responses_load_without_inventing_authority() {
    let legacy = json!({
        "schema": "casegraphen.experimental.control_plane.response.v0",
        "sequence": 1,
        "request_id": "request:legacy",
        "idempotency_key": "idempotency:legacy",
        "replayed": false,
        "result": {"legacy": true},
        "refusal": null
    });
    let response: casegraphen::control_plane::ControlPlaneResponse =
        serde_json::from_value(legacy).unwrap();
    assert!(
        !response
            .authority_facts
            .caller_declared_audit_context_present
    );
    assert_eq!(
        serde_json::to_value(response.authority_facts.canonical_casegraphen_authorization).unwrap(),
        json!("not_evaluated")
    );
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

struct AckSabotageDelegate {
    state_path: PathBuf,
    pending_bytes: Vec<u8>,
    calls: usize,
}

impl DecisionDelegate for AckSabotageDelegate {
    fn invoke(&mut self, _request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal> {
        self.calls += 1;
        self.pending_bytes = fs::read(&self.state_path).expect("capture pending journal");
        fs::remove_file(&self.state_path).expect("remove journal before acknowledgement");
        fs::create_dir(&self.state_path).expect("make acknowledgement rename fail");
        Ok(json!({"delegated": true}))
    }
}

#[test]
fn durable_restart_never_duplicates_an_ambiguous_delegated_effect() {
    let directory = std::env::temp_dir().join(format!(
        "casegraphen-control-plane-durable-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let state_path = directory.join("state.json");
    let mut state = ControlPlaneState::new();
    let mut delegate = AckSabotageDelegate {
        state_path: state_path.clone(),
        pending_bytes: Vec::new(),
        calls: 0,
    };
    let request = request(ControlPlaneTool::AttachRuntimeReport, "request:ambiguous");
    let first = state.execute_durable(&request, &mut delegate, &state_path);
    assert_eq!(delegate.calls, 1);
    assert_eq!(
        first.refusal.unwrap().code,
        "durable_acknowledgement_failed"
    );

    fs::remove_dir(&state_path).unwrap();
    fs::write(&state_path, &delegate.pending_bytes).unwrap();
    let mut restarted = ControlPlaneState::load_durable(&state_path).unwrap();
    let mut should_not_run = CountingDelegate {
        calls: 0,
        stale: false,
    };
    let replay = restarted.execute_durable(&request, &mut should_not_run, &state_path);
    assert_eq!(should_not_run.calls, 0);
    assert_eq!(replay.refusal.unwrap().code, "ambiguous_prior_effect");
    fs::remove_dir_all(directory).unwrap();
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
