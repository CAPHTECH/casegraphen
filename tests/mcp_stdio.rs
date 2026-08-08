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
    assert_eq!(tools.len(), 28);
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
    assert_eq!(messages[1]["result"]["tools"].as_array().unwrap().len(), 28);
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

    // ADR 0034 / issue #120, T6: the exact ControlPlaneResponse this live,
    // spawned `casegraphen-mcp-host` process just emitted for
    // `compile_deployment_bundle` must validate against the tightened
    // envelope, and every envelope forgery family constructed against it
    // must be rejected — except the one that must not be, because it is
    // outside the envelope's declared scope.
    let live_response = &first[2]["result"]["structuredContent"];
    assert!(
        validates_against_control_plane_response_schema(live_response),
        "a real, live compile_deployment_bundle response failed to validate \
         against control_plane.response.v0: {live_response}"
    );

    let mut forged = live_response.clone();
    forged["result"]["accepted"] = json!(true);
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result.accepted: true forgery on a live response must fail validation"
    );

    let mut forged = live_response.clone();
    forged["result"]["mutation_performed"] = json!(true);
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result.mutation_performed: true forgery on a live response must fail validation"
    );

    let mut forged = live_response.clone();
    forged["result"]["read_only"] = json!(false);
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result.read_only: false forgery on a live response must fail validation"
    );

    let mut forged = live_response.clone();
    forged["result"]["review_status"] = json!("accepted");
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result.review_status: \"accepted\" forgery on a live response must fail validation"
    );

    let mut forged = live_response.clone();
    forged["refusal"] = json!({
        "code": "forged_refusal", "detail": "forged", "supplied_base_revision_id": null,
        "current_revision_id": null, "suggested_next_operation": "report_host_defect"
    });
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result and refusal both non-null must fail validation"
    );

    let mut forged = live_response.clone();
    forged["result"] = Value::Null;
    assert!(
        !validates_against_control_plane_response_schema(&forged),
        "result and refusal both null must fail validation"
    );

    // Scope boundary (ADR 0034): a nested `accepted: true` below the top
    // level of `result` is payload semantics, not an envelope-level claim,
    // so the envelope must still accept it. This is the pass case the issue
    // asks to be proven, not assumed.
    let mut nested = live_response.clone();
    nested["result"]["nested_ledger_echo"] = json!({"accepted": true});
    assert!(
        validates_against_control_plane_response_schema(&nested),
        "a nested accepted: true at depth >= 1 must still validate; \
         the pin is top-level only by design"
    );

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

/// Issue #165: before this, every one of the 28 catalog tools published the
/// identical description and an unconstrained `"payload": {}`, even though
/// ten registered `casegraphen.experimental.mcp.*_input.v0` contracts were
/// already enforced server-side for seventeen of them. Driven against the
/// real, live `casegraphen-mcp-host`, not asserted in the abstract: every
/// tool gets a distinct description, each of the seventeen contracted tools
/// publishes the exact schema id `invoke` deserializes against (so the
/// published contract and the enforced one cannot silently drift apart),
/// and the eleven tools with no registered type (five that always refuse,
/// six that parse `payload` ad hoc) stay honestly unconstrained.
#[test]
fn tools_list_publishes_the_real_contracted_payload_schemas_and_distinct_descriptions() {
    let directory = std::env::temp_dir().join(format!(
        "casegraphen-mcp-host-tools-list-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    let store = directory.join("store");
    let artifacts = directory.join("artifacts");
    let state = directory.join("protocol-state.json");
    fs::create_dir_all(&directory).unwrap();

    let responses = run_operational_host(
        &state,
        &store,
        &artifacts,
        &[
            request(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            request(2, "tools/list", json!({"authorization":"token:e2e"})),
        ],
    );
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 28);

    let descriptions: std::collections::HashSet<&str> = tools
        .iter()
        .map(|tool| tool["description"].as_str().unwrap())
        .collect();
    assert_eq!(
        descriptions.len(),
        28,
        "every tool must publish a distinct description: {tools:?}"
    );

    let payload_schema = |name: &str| -> Value {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} not in catalog: {tools:?}"))["inputSchema"]
            ["properties"]["payload"]
            .clone()
    };

    // Representative of the seventeen: the published `$ref` names the exact
    // schema id `invoke` deserializes `payload.compiler_request` against.
    let compile = payload_schema("compile_deployment_bundle");
    assert_eq!(
        compile["properties"]["compiler_request"]["$ref"],
        "casegraphen.experimental.mcp.proposal_compiler_input.v0",
        "{compile}"
    );
    assert_eq!(
        compile["required"],
        json!(["topology_json", "compiler_request"]),
        "{compile}"
    );

    // A memory-read tool shares one contract across five tool names.
    assert_eq!(
        payload_schema("memory_query")["properties"]["memory_request"]["$ref"],
        "casegraphen.experimental.mcp.memory_read_input.v0"
    );

    // A memory-proposal tool: distinct wrapper key and contract from reads.
    assert_eq!(
        payload_schema("memory_propose_supersession")["properties"]["memory_proposal"]["$ref"],
        "casegraphen.experimental.mcp.memory_proposal_input.v0"
    );

    // The eleven tools with no registered payload type stay unconstrained:
    // publishing a schema for a shape nothing enforces would be a claim
    // this delegate cannot back up.
    for name in [
        "reconcile_run",
        "propose_execution_topology",
        "review_accept",
    ] {
        assert_eq!(
            payload_schema(name),
            json!({}),
            "{name} has no registered contract and must stay unconstrained"
        );
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn resources_read_classifies_pure_echoes_and_claim_bearing_projections_and_rejects_a_forged_claim()
{
    // ADR 0036 / #122: prove, against a real spawned host and a real store,
    // both halves of the classification — the four pure-echo resources still
    // read cleanly (the regression the top-level-only pin exists to avoid),
    // and the three claim-bearing resources carry the contracted
    // `resource_projection.v0` shape, which rejects a forged top-level
    // `accepted: true` when validated live rather than in the abstract.
    let directory = std::env::temp_dir().join(format!(
        "casegraphen-mcp-host-resource-read-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    let store = directory.join("store");
    let artifacts = directory.join("artifacts");
    let state = directory.join("protocol-state.json");
    fs::create_dir_all(&directory).unwrap();

    let case_space_id = "case_space:resource-read-e2e";
    let revision_id = "revision:resource-read-e2e";
    let create = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "space",
            "new",
            "--store",
            store.to_str().unwrap(),
            "--case-space-id",
            case_space_id,
            "--space-id",
            "space:resource-read-e2e",
            "--title",
            "Resource Read E2E",
            "--revision-id",
            revision_id,
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

    // Claim-bearing artifact files, written the way an external runtime
    // would write them — outside every CaseGraphen mutation path, exactly
    // as `read_external_projection` expects to find them.
    let run_id = "run:resource-read-e2e";
    let topology_id = "topology:resource-read-e2e";
    for (subdirectory, identity_field, id) in [
        ("halts", "case_space_id", case_space_id),
        ("runs", "run_id", run_id),
        ("topologies", "topology_id", topology_id),
    ] {
        let subdirectory_path = artifacts.join(subdirectory);
        fs::create_dir_all(&subdirectory_path).unwrap();
        fs::write(
            subdirectory_path.join(format!("{id}.json")),
            serde_json::to_vec(&json!({identity_field: id, "halt": "needs_review"})).unwrap(),
        )
        .unwrap();
    }

    let messages = vec![
        request(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
        request(
            2,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/status")}),
        ),
        request(
            3,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/frontier")}),
        ),
        request(
            4,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/reviews")}),
        ),
        request(
            5,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/revisions/{revision_id}")}),
        ),
        request(
            6,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/halts")}),
        ),
        request(
            7,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://runs/{run_id}")}),
        ),
        request(
            8,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://topologies/{topology_id}")}),
        ),
        // Issue #168: `status` and `reviews` must default to a bounded
        // summary and still make the complete evaluation reachable via
        // `?detail=full`; an unrecognized `detail` value must refuse rather
        // than silently falling back to one of the two.
        request(
            9,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/status?detail=full")}),
        ),
        request(
            10,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/reviews?detail=full")}),
        ),
        request(
            11,
            "resources/read",
            json!({"authorization":"token:e2e", "uri":format!("casegraphen://spaces/{case_space_id}/status?detail=bogus")}),
        ),
    ];
    let responses = run_operational_host(&state, &store, &artifacts, &messages);
    assert_eq!(responses.len(), 11);

    // The four pure-echo resources read cleanly: no refusal, real ledger
    // content, and none of them ever surfaces a top-level claim key — the
    // classification this ADR relies on, checked here rather than assumed.
    let status = resource_content(&responses[1]);
    assert_eq!(status["case_space_id"], case_space_id, "{status}");
    assert!(status.get("refusal").is_none(), "{status}");
    assert_pure_echo("status", &status);
    // #168 default: bounded summary, not the whole evaluation.
    assert!(status.get("evaluation").is_none(), "{status}");
    assert!(status.get("assurance").is_some(), "{status}");
    assert!(status.get("progress").is_some(), "{status}");
    assert!(status.get("frontier_cell_ids").is_some(), "{status}");

    let frontier = resource_content(&responses[2]);
    assert_eq!(frontier["case_space_id"], case_space_id, "{frontier}");
    assert!(frontier.get("refusal").is_none(), "{frontier}");
    assert_pure_echo("frontier", &frontier);

    let reviews = resource_content(&responses[3]);
    assert_eq!(reviews["case_space_id"], case_space_id, "{reviews}");
    assert!(reviews.get("refusal").is_none(), "{reviews}");
    assert_pure_echo("reviews", &reviews);
    // #168: `reviews` measured larger than `status` and had the identical
    // no-projection defect, so it gets the identical fix.
    assert!(reviews.get("review_gaps").is_none(), "{reviews}");
    assert!(reviews.get("reviewed_cells").is_none(), "{reviews}");
    assert!(reviews.get("review_gap_ids").is_some(), "{reviews}");
    assert!(reviews.get("reviewed_cell_ids").is_some(), "{reviews}");

    // Every field the default summary dropped stays reachable at
    // `?detail=full` — this is a default change, not a removal.
    let status_full = resource_content(&responses[8]);
    assert!(status_full.get("refusal").is_none(), "{status_full}");
    assert_pure_echo("status?detail=full", &status_full);
    assert!(status_full.get("evaluation").is_some(), "{status_full}");
    assert_eq!(
        status_full["evaluation"]["assurance"], status["assurance"],
        "the full evaluation must agree with the summary it was derived from"
    );

    let reviews_full = resource_content(&responses[9]);
    assert!(reviews_full.get("refusal").is_none(), "{reviews_full}");
    assert_pure_echo("reviews?detail=full", &reviews_full);
    assert!(reviews_full.get("review_gaps").is_some(), "{reviews_full}");
    assert!(
        reviews_full.get("reviewed_cells").is_some(),
        "{reviews_full}"
    );

    let bad_detail = resource_content(&responses[10]);
    assert_eq!(
        bad_detail["refusal"]["code"], "invalid_resource_detail",
        "{bad_detail}"
    );

    let revision = resource_content(&responses[4]);
    assert_eq!(revision["revision_id"], revision_id, "{revision}");
    assert!(revision.get("refusal").is_none(), "{revision}");
    assert_pure_echo("revisions/{revision}", &revision);

    // The three claim-bearing resources carry the contracted
    // `resource_projection.v0` shape and validate against the shipped
    // schema; a forged top-level `accepted: true` on the same live response
    // fails that validation, proven by construction, not by inspection.
    for (response, identity_field, id) in [
        (&responses[5], "case_space_id", case_space_id),
        (&responses[6], "run_id", run_id),
        (&responses[7], "topology_id", topology_id),
    ] {
        let projection = resource_content(response);
        assert_eq!(
            projection["schema"], "casegraphen.experimental.control_plane.resource_projection.v0",
            "{projection}"
        );
        assert_eq!(projection["accepted"], false, "{projection}");
        assert_eq!(projection["projection"][identity_field], id, "{projection}");
        assert!(
            validates_against_resource_projection_schema(&projection),
            "a real, live resource-read projection failed to validate against \
             control_plane.resource_projection.v0: {projection}"
        );

        let mut forged = projection.clone();
        forged["accepted"] = json!(true);
        assert!(
            !validates_against_resource_projection_schema(&forged),
            "accepted: true forgery on a live resource-read response must fail validation"
        );
    }

    fs::remove_dir_all(directory).unwrap();
}

/// The seven-key claim vocabulary `claim_vocabulary_violation`
/// (`src/control_plane.rs`) checks at the top level of a response — kept in
/// sync with that Rust list by inspection, since this asserts the absence of
/// the whole vocabulary rather than any particular truthful/forbidden value.
const CLAIM_VOCABULARY_KEYS: [&str; 7] = [
    "accepted",
    "mutation_performed",
    "read_only",
    "accepted_runtime_output",
    "proofs_serialized",
    "review_status",
    "generated_plan_review_status",
];

/// Makes `product-surface.v0.json`'s `pure_echo` classification executable
/// (ADR 0036 / #122). That classification is the entire reason a top-level
/// vocabulary pin is safe on `resources/read` at all: the pin *permits* any
/// of these keys at their truthful value (`accepted: false` is truthful, not
/// forbidden), so if a pure-echo resource ever grows one of them, the
/// layer-2 pin would let it through silently. A failure here does not mean
/// this resource is broken — it means the resource is no longer a pure
/// echo, and the fix is to reclassify it in `product-surface.v0.json` (and
/// very likely contract it), not to relax this assertion.
fn assert_pure_echo(name: &str, value: &Value) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{name} resource content is not an object: {value}"));
    for key in CLAIM_VOCABULARY_KEYS {
        assert!(
            !object.contains_key(key),
            "{name} carries top-level {key:?}: {value} — this resource is no longer a \
             pure echo, and its classification in product-surface.v0.json is now wrong"
        );
    }
}

/// Parses the JSON string an MCP `resources/read` response wraps in
/// `contents[0].text`.
fn resource_content(response: &Value) -> Value {
    let text = response["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("resource content missing: {response}"));
    serde_json::from_str(text).unwrap()
}

/// ADR 0036 / #117 pattern: validate a real, live resource-read projection
/// against the shipped contract rather than asserting about the schema in
/// the abstract.
fn validates_against_resource_projection_schema(instance: &Value) -> bool {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "casegraphen-resource-projection-{}-{nonce}.json",
        std::process::id()
    ));
    fs::write(&file, serde_json::to_vec(instance).unwrap()).expect("write instance");
    let status = Command::new("python3")
        .args(["-m", "jsonschema", "-i"])
        .arg(&file)
        .arg(root().join("schemas/experimental/control_plane.resource_projection.v0.schema.json"))
        .status()
        .expect("run python3 -m jsonschema");
    let _ = fs::remove_file(&file);
    status.success()
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

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// ADR 0034 / #117 pattern: validate a real, live response against the
/// shipped contract rather than asserting about the schema in the abstract.
fn validates_against_control_plane_response_schema(instance: &Value) -> bool {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "casegraphen-control-plane-response-{}-{nonce}.json",
        std::process::id()
    ));
    fs::write(&file, serde_json::to_vec(instance).unwrap()).expect("write instance");
    let status = Command::new("python3")
        .args(["-m", "jsonschema", "-i"])
        .arg(&file)
        .arg(root().join("schemas/experimental/control_plane.response.v0.schema.json"))
        .status()
        .expect("run python3 -m jsonschema");
    let _ = fs::remove_file(&file);
    status.success()
}
