#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    resource_protocol::{
        declaration_grants, ResourceDeclaration, ResourceReservation, RuntimeResourceAllocation,
        RESOURCE_ALLOCATION_SCHEMA, RESOURCE_DECLARATION_SCHEMA, RESOURCE_RESERVATION_SCHEMA,
        RUNTIME_ALLOCATION_TRUST_BOUNDARY,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[test]
fn operational_host_reserves_then_reconciles_a_resource_bearing_run_to_review() {
    let directory = temp("reconcile");
    let mut topology_value: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    topology_value["nodes"] = json!([topology_value["nodes"][0].clone()]);
    topology_value["edges"] = json!([]);
    let topology_json = serde_json::to_string(&topology_value).unwrap();
    let topology = parse_execution_topology(&topology_json).unwrap();
    let topology_hash = execution_topology_content_hash(&topology).unwrap();
    let node = &topology.nodes[0];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:e2e-review-a".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: topology_hash.clone(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: "reservation:e2e-review-a".to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: "attempt:e2e-review-a:1".to_owned(),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let grant = &reservation.grants[0];
    let allocation = RuntimeResourceAllocation {
        schema: RESOURCE_ALLOCATION_SCHEMA.to_owned(),
        schema_version: 0,
        allocation_id: "allocation:e2e-review-a".to_owned(),
        reservation_id: reservation.reservation_id.clone(),
        attempt_id: reservation.attempt_id.clone(),
        resource_id: grant.resource_id.clone(),
        mode: grant.mode,
        rate_limit_group: grant.rate_limit_group.clone(),
        rate_limit_units: grant.rate_limit_units,
        workspace_strategy: grant.workspace_strategy,
        network_scope: grant.network_scope.clone(),
        secret_scope: grant.secret_scope.clone(),
        worktree_id: None,
        trust_boundary: RUNTIME_ALLOCATION_TRUST_BOUNDARY.to_owned(),
    };

    let reserve = tool_call(
        "reserve_resources",
        "reserve",
        Some("revision:e2e"),
        json!({
            "topology_json":topology_json,
            "resource_request":{"declaration":declaration,"reservation":reservation}
        }),
        true,
    );
    let reserve_responses = run_host(&directory, &[reserve]);
    assert_eq!(reserve_responses[1]["result"]["isError"], false);
    assert_eq!(
        reserve_responses[1]["result"]["structuredContent"]["result"]["allocator_generation"],
        1
    );

    let artifact_bytes = b"review complete";
    let artifact_digest = format!("{:x}", Sha256::digest(artifact_bytes));
    let artifact_id = format!("artifact:sha256-{artifact_digest}");
    let artifact_record = json!({"kind":"artifact","artifact_id":artifact_id,"media_type":"text/plain","content":"review complete"});
    let report = json!({
        "schema":"casegraphen.experimental.runtime.node_report.v0","schema_version":0,
        "report_id":"runtime_report:e2e-review-a","runtime_graph_id":topology.topology_id,
        "runtime_graph_content_hash":topology_hash,"node_id":node.node_id,
        "attempt_id":"attempt:e2e-review-a:1","retry_of_attempt_id":null,"round_id":"round:1","parent_node_ids":[],
        "input_artifact_ids":[],"output_artifact_ids":[artifact_id],
        "expected_output_schema_id":node.outputs[0].schema_id,"actual_output_schema_id":node.outputs[0].schema_id,
        "started_at":"2026-08-03T00:00:00Z","finished_at":"2026-08-03T00:00:01Z","status":"succeeded","failure_kind":null,
        "runtime_identity":{"runtime_name":"e2e","runtime_version":"1","adapter_name":"jsonl","adapter_version":"1"},
        "reported_model":null,"reported_context_id":null,"token_usage":null,"cost":null,
        "resource_allocations":[{"resource_id":grant.resource_id,"mode":"read","allocation_id":"allocation:e2e-review-a"}],
        "worktree_id":null,"commit_sha":null,"verifier_report_ids":[],
        "trust_boundary":"runtime_reported_untrusted_until_independently_validated_and_reviewed"
    });
    let runtime_jsonl = format!(
        "{}\n{}",
        artifact_record,
        json!({"kind":"node_report","report":report})
    );
    let expectation_bundle = json!({
        "schema":"casegraphen.experimental.runtime.resource_expectation_bundle.v0","schema_version":0,
        "topology_content_hash":topology_hash,"case_revision_id":"revision:e2e",
        "expectations":[{"node_id":node.node_id,"attempt_id":"attempt:e2e-review-a:1","declaration":declaration,
            "reservation":reservation,"allocations":[allocation],"disposition_evidence":[]}]
    });
    let reconcile = tool_call(
        "reconcile_run",
        "reconcile",
        Some("revision:e2e"),
        json!({
            "topology_json":topology_json,"runtime_jsonl":runtime_jsonl,"resource_expectation_bundle":expectation_bundle
        }),
        false,
    );
    let responses = run_host(&directory, &[reconcile]);
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["accepted"], false);
    assert_eq!(result["reconciliation_complete"], true);
    assert_eq!(result["halt"], "needs_review");
    assert!(result["proposals"]
        .as_array()
        .is_some_and(|values| !values.is_empty()));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reserve_resources_rejects_caller_supplied_allocator_state() {
    let directory = temp("forged-state");
    let mut topology_value: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    topology_value["nodes"] = json!([topology_value["nodes"][0].clone()]);
    topology_value["edges"] = json!([]);
    let topology_json = serde_json::to_string(&topology_value).unwrap();
    let topology = parse_execution_topology(&topology_json).unwrap();
    let node = &topology.nodes[0];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:forged-state".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: "reservation:forged-state".to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: "attempt:forged-state".to_owned(),
        granted_at: "2026-08-03T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let call = tool_call(
        "reserve_resources",
        "forged-state",
        Some("revision:e2e"),
        json!({
            "topology_json":topology_json,
            "resource_request":{
                "declaration":declaration,
                "reservation":reservation,
                "existing_reservations":[],
                "rate_limit_capacities":[]
            }
        }),
        true,
    );
    let responses = run_host(&directory, &[call]);
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_eq!(
        responses[1]["result"]["structuredContent"]["refusal"]["code"],
        "invalid_payload"
    );
    fs::remove_dir_all(directory).unwrap();
}

fn tool_call(
    name: &str,
    suffix: &str,
    revision: Option<&str>,
    payload: Value,
    mutation: bool,
) -> String {
    let mut arguments = json!({"request_id":format!("request:{suffix}"),"idempotency_key":format!("idem:{suffix}"),"payload":payload});
    if let Some(revision) = revision {
        arguments["base_revision_id"] = json!(revision);
    }
    if mutation {
        arguments["caller_declared_audit_context"] = json!({
            "declared_actor_id":"actor:e2e","declared_capability_ids":["capability:resource"],
            "declared_operation_scope_id":"scope:resource","declared_audience":"audit","declared_source_boundary_id":"boundary:e2e"
        });
    }
    json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"authorization":"token:e2e-resource","name":name,"arguments":arguments}}).to_string()
}

fn temp(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "casegraphen-resource-host-e2e-{label}-{}",
        std::process::id()
    ));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(&path).unwrap();
    path
}

fn run_host(directory: &Path, calls: &[String]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
        .args(["--state"])
        .arg(directory.join("state.json"))
        .args(["--store"])
        .arg(directory.join("store"))
        .args(["--artifacts"])
        .arg(directory.join("artifacts"))
        .args(["--auth-token-env", "CASEGRAPHEN_TEST_RESOURCE_TOKEN"])
        .env("CASEGRAPHEN_TEST_RESOURCE_TOKEN", "token:e2e-resource")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let input = child.stdin.as_mut().unwrap();
        writeln!(input,"{}",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        for call in calls {
            writeln!(input, "{call}").unwrap();
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
