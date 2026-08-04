#![allow(missing_docs)]

use casegraphen::{
    control_plane::{
        ControlPlaneRefusal, ControlPlaneRequest, DecisionDelegate, ResourceDelegate,
        CONTROL_PLANE_NOTIFICATION_SCHEMA,
    },
    mcp_stdio::McpStdioServer,
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

struct TranscriptDelegate {
    calls: Arc<AtomicUsize>,
}

impl DecisionDelegate for TranscriptDelegate {
    fn invoke(&mut self, request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ControlPlaneRefusal::stale(
            request.base_revision_id.as_deref().unwrap_or("missing"),
            "revision:current",
        ))
    }
}

impl ResourceDelegate for TranscriptDelegate {
    fn read_resource(&mut self, uri: &str) -> Result<Value, ControlPlaneRefusal> {
        Ok(json!({"uri": uri, "projection": true}))
    }
}

fn request(id: i64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn initialize(server: &mut McpStdioServer<TranscriptDelegate>) -> Value {
    let response = server
        .handle_line(&request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "transcript-test", "version": "1"}
            }),
        ))
        .unwrap();
    assert!(server
        .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .is_none());
    response
}

#[test]
fn catalog_and_resource_transcript_is_mcp_compatible() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut server = McpStdioServer::new(TranscriptDelegate { calls });
    let initialized = initialize(&mut server);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    assert!(initialized["result"]["capabilities"]["resources"].is_object());
    assert!(initialized["result"]["capabilities"]["tools"].is_object());

    let resources = server
        .handle_line(&request(2, "resources/list", json!({})))
        .unwrap();
    assert!(resources["result"]["resources"]
        .as_array()
        .unwrap()
        .is_empty());
    let templates = server
        .handle_line(&request(20, "resources/templates/list", json!({})))
        .unwrap();
    assert_eq!(
        templates["result"]["resourceTemplates"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    let uri = "casegraphen://spaces/case-1/status";
    let read = server
        .handle_line(&request(3, "resources/read", json!({"uri": uri})))
        .unwrap();
    let text = read["result"]["contents"][0]["text"].as_str().unwrap();
    assert_eq!(serde_json::from_str::<Value>(text).unwrap()["uri"], uri);

    let tools = server
        .handle_line(&request(4, "tools/list", json!({})))
        .unwrap();
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 18);
    let review = tools
        .iter()
        .find(|tool| tool["name"] == "review_accept")
        .unwrap();
    assert!(review["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("base_revision_id")));
    assert!(review["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("caller_declared_audit_context")));
    assert!(!review["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .contains_key("operation_gate"));
}

#[test]
fn stale_tool_refusal_and_reconnect_are_structured_and_idempotent() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut server = McpStdioServer::new(TranscriptDelegate {
        calls: Arc::clone(&calls),
    });
    initialize(&mut server);
    let arguments = json!({
        "request_id": "request:first",
        "idempotency_key": "semantic:review-1",
        "base_revision_id": "revision:observed",
        "caller_declared_audit_context": {
            "declared_actor_id": "actor:test",
            "declared_capability_ids": ["capability:review-declared"],
            "declared_operation_scope_id": "scope:review",
            "declared_audience": "audit",
            "declared_source_boundary_id": "boundary:test"
        },
        "payload": {"review_id": "review:1"}
    });
    let first = server
        .handle_line(&request(
            2,
            "tools/call",
            json!({"name": "review_accept", "arguments": arguments}),
        ))
        .unwrap();
    assert_eq!(
        first["result"]["structuredContent"]["refusal"]["code"],
        "stale_revision"
    );
    assert_eq!(
        first["result"]["structuredContent"]["refusal"]["current_revision_id"],
        "revision:current"
    );
    assert_eq!(
        first["result"]["structuredContent"]["authority_facts"]
            ["canonical_casegraphen_authorization"],
        "not_evaluated"
    );
    assert_eq!(
        first["result"]["transport_authentication"]["authenticated"],
        false
    );

    let mut reconnect = arguments;
    reconnect["request_id"] = json!("request:reconnect");
    let second = server
        .handle_line(&request(
            3,
            "tools/call",
            json!({"name": "review_accept", "arguments": reconnect}),
        ))
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(second["result"]["structuredContent"]["replayed"], true);
    let replay = server
        .handle_line(&request(
            4,
            "casegraphen/replay",
            json!({"after_sequence": 0}),
        ))
        .unwrap();
    assert_eq!(replay["result"]["responses"].as_array().unwrap().len(), 1);
}

#[test]
fn notification_publish_and_cursor_replay_never_authorize() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut server = McpStdioServer::new(TranscriptDelegate { calls });
    initialize(&mut server);
    let publish = server
        .handle_line(&request(
            2,
            "casegraphen/notifications/publish",
            json!({
                "schema": CONTROL_PLANE_NOTIFICATION_SCHEMA,
                "notification_id": "notification:1",
                "sequence": 0,
                "kind": "review_required",
                "subject_uri": "casegraphen://spaces/case-1/reviews",
                "observed_revision_id": "revision:1",
                "payload": {"claim_id": "claim:1"},
                "authorizes_action": true
            }),
        ))
        .unwrap();
    assert_eq!(
        publish["result"]["notification"]["authorizes_action"],
        false
    );
    let replay = server
        .handle_line(&request(
            3,
            "casegraphen/notifications/replay",
            json!({"after_sequence": 0}),
        ))
        .unwrap();
    assert_eq!(
        replay["result"]["notifications"][0]["notification_id"],
        "notification:1"
    );
}

#[test]
fn external_binary_speaks_only_newline_delimited_json_rpc_on_stdout() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let input = child.stdin.as_mut().unwrap();
        writeln!(
            input,
            "{}",
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"}))
        )
        .unwrap();
        writeln!(
            input,
            "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}"
        )
        .unwrap();
        writeln!(input, "{}", request(2, "tools/list", json!({}))).unwrap();
        writeln!(
            input,
            "{}",
            request(
                3,
                "tools/call",
                json!({
                    "name": "lint_execution_topology",
                    "arguments": {
                        "request_id": "request:binary-lint",
                        "idempotency_key": "idempotency:binary-lint",
                        "payload": {
                            "topology_json": include_str!("../schemas/experimental/execution.topology.file-review.example.json")
                        }
                    }
                })
            )
        )
        .unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines = String::from_utf8(output.stdout).unwrap();
    let messages = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 3);
    assert!(messages.iter().all(|message| message["jsonrpc"] == "2.0"));
    assert_eq!(messages[1]["result"]["tools"].as_array().unwrap().len(), 18);
    assert_eq!(messages[2]["result"]["isError"], false);
    assert!(messages[2]["result"]["structuredContent"]["result"]["findings"].is_array());
}

#[test]
fn durable_authenticated_session_replays_after_restart_without_redelegating() {
    let directory =
        std::env::temp_dir().join(format!("casegraphen-mcp-durable-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let state_path: PathBuf = directory.join("state.json");
    let calls = Arc::new(AtomicUsize::new(0));
    let arguments = json!({
        "request_id": "request:durable",
        "idempotency_key": "semantic:durable",
        "base_revision_id": "revision:observed",
        "caller_declared_audit_context": {
            "declared_actor_id": "actor:test",
            "declared_capability_ids": ["capability:review-declared"],
            "declared_operation_scope_id": "scope:review",
            "declared_audience": "audit",
            "declared_source_boundary_id": "boundary:test"
        },
        "payload": {"review_id": "review:1"}
    });
    {
        let mut server = McpStdioServer::new_durable_authenticated(
            TranscriptDelegate {
                calls: Arc::clone(&calls),
            },
            &state_path,
            "token:test".to_owned(),
        )
        .unwrap();
        initialize(&mut server);
        let unauthorized = server
            .handle_line(&request(
                2,
                "tools/call",
                json!({"name":"review_accept", "arguments":arguments.clone()}),
            ))
            .unwrap();
        assert_eq!(unauthorized["error"]["code"], -32001);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let authorized = server
            .handle_line(&request(
                3,
                "tools/call",
                json!({"authorization":"token:test", "name":"review_accept", "arguments":arguments.clone()}),
            ))
            .unwrap();
        assert_eq!(
            authorized["result"]["structuredContent"]["refusal"]["code"],
            "stale_revision"
        );
        assert_eq!(
            authorized["result"]["transport_authentication"]["authenticated"],
            true
        );
        assert_eq!(
            authorized["result"]["transport_authentication"]["canonical_casegraphen_authorization"],
            "not_evaluated"
        );
        assert_eq!(
            authorized["result"]["structuredContent"]["authority_facts"]
                ["caller_declared_audit_context_present"],
            true
        );
    }
    {
        let mut restarted = McpStdioServer::new_durable_authenticated(
            TranscriptDelegate {
                calls: Arc::clone(&calls),
            },
            &state_path,
            "token:test".to_owned(),
        )
        .unwrap();
        initialize(&mut restarted);
        let replay = restarted
            .handle_line(&request(
                4,
                "tools/call",
                json!({"authorization":"token:test", "name":"review_accept", "arguments":arguments}),
            ))
            .unwrap();
        assert_eq!(replay["result"]["structuredContent"]["replayed"], true);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn operational_host_projects_real_store_state_and_compiles_without_a_custom_rust_caller() {
    let directory =
        std::env::temp_dir().join(format!("casegraphen-mcp-host-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    let store = directory.join("store");
    let artifacts = directory.join("artifacts");
    let state = directory.join("protocol-state.json");
    fs::create_dir_all(&directory).unwrap();
    let create = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "space",
            "new",
            "--store",
            store.to_str().unwrap(),
            "--case-space-id",
            "case_space:host-e2e",
            "--space-id",
            "space:host-e2e",
            "--title",
            "Host E2E",
            "--revision-id",
            "revision:host-e2e",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );

    let topology_text =
        include_str!("../schemas/experimental/execution.topology.file-review.example.json");
    let mut verification: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/verification.policy.example.json"
    ))
    .unwrap();
    verification["verification_policy_id"] = json!("verification:independent");
    let topology: Value = serde_json::from_str(topology_text).unwrap();
    let mappings = topology["nodes"].as_array().unwrap().iter().map(|node| json!({
        "node_id": node["node_id"],
        "worker_binding_id": format!("worker_binding:{}", node["node_id"].as_str().unwrap()),
        "success_evidence_requirement_ids": [format!("evidence_requirement:{}", node["node_id"].as_str().unwrap())],
        "allowed_transition_classes": [{
            "morphism_type":"update", "target_cell_types":["work"], "to_lifecycles":["resolved"]
        }]
    })).collect::<Vec<_>>();
    let arguments = json!({
        "request_id":"request:host-compile",
        "idempotency_key":"idempotency:host-compile",
        "base_revision_id":"revision:host-e2e",
        "payload":{
            "topology_json":topology_text,
            "compiler_request":{
                "case_space_id":"case_space:file-review",
                "base_revision_id":"revision:host-e2e",
                "plan_id":"plan:host-e2e",
                "node_plan_mappings":mappings,
                "verification_policies":{"verification:independent":verification},
                "budget_policies":{"budget:small":{"policy_id":"budget:small","max_cost":10}},
                "expansion_policies":{}
            }
        }
    });
    let messages = vec![
        request(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        request(
            2,
            "resources/read",
            json!({
                "authorization":"token:e2e",
                "uri":"casegraphen://spaces/case_space:host-e2e/status"
            }),
        ),
        request(
            3,
            "tools/call",
            json!({
                "authorization":"token:e2e", "name":"compile_deployment_bundle", "arguments":arguments.clone()
            }),
        ),
    ];
    let first = run_operational_host(&state, &store, &artifacts, &messages);
    assert_eq!(first.len(), 3);
    let resource_text = first[1]["result"]["contents"][0]["text"].as_str().unwrap();
    let resource: Value = serde_json::from_str(resource_text).unwrap();
    assert_eq!(resource["current_revision_id"], "revision:host-e2e");
    assert_eq!(first[2]["result"]["isError"], false);
    let bundle = &first[2]["result"]["structuredContent"]["result"];
    assert_eq!(bundle["accepted"], false);
    assert!(PathBuf::from(bundle["bundle_directory"].as_str().unwrap())
        .join("manifest.json")
        .is_file());

    let replay = run_operational_host(
        &state,
        &store,
        &artifacts,
        &[
            request(4, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            request(
                5,
                "tools/call",
                json!({
                    "authorization":"token:e2e", "name":"compile_deployment_bundle", "arguments":arguments
                }),
            ),
        ],
    );
    assert_eq!(replay[1]["result"]["structuredContent"]["replayed"], true);
    fs::remove_dir_all(directory).unwrap();
}

fn run_operational_host(
    state: &std::path::Path,
    store: &std::path::Path,
    artifacts: &std::path::Path,
    messages: &[String],
) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
        .args([
            "--state",
            state.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
            "--artifacts",
            artifacts.to_str().unwrap(),
            "--auth-token-env",
            "CASEGRAPHEN_TEST_MCP_TOKEN",
        ])
        .env("CASEGRAPHEN_TEST_MCP_TOKEN", "token:e2e")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let input = child.stdin.as_mut().unwrap();
        for message in messages {
            writeln!(input, "{message}").unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
