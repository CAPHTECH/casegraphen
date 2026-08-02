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
    io::Write,
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
    assert_eq!(tools.len(), 12);
    let review = tools
        .iter()
        .find(|tool| tool["name"] == "review_accept")
        .unwrap();
    assert!(review["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("base_revision_id")));
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
        "operation_gate": {
            "actor_id": "actor:test",
            "capability_ids": ["capability:review"],
            "operation_scope_id": "scope:review",
            "audience": "audit",
            "source_boundary_id": "boundary:test"
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
    assert_eq!(messages[1]["result"]["tools"].as_array().unwrap().len(), 12);
    assert_eq!(messages[2]["result"]["isError"], false);
    assert!(messages[2]["result"]["structuredContent"]["result"]["findings"].is_array());
}
