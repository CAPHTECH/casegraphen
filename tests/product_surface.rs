#![allow(missing_docs)]

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    graph_simulation::{simulate_execution_topology, GraphSimulationRequest},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn inventory_conformance_is_a_release_gate() {
    let output = Command::new("python3")
        .arg(root().join("scripts/product-surface-conformance.py"))
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_and_operational_mcp_share_the_exact_lint_report_boundary() {
    let topology_path =
        root().join("schemas/experimental/execution.topology.file-review.example.json");
    let topology_json = fs::read_to_string(&topology_path).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["graph", "lint", "--input"])
        .arg(&topology_path)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let cli_report: Value = serde_json::from_slice(&cli.stdout).unwrap();

    let directory = temp("lint-parity");
    let responses = run_host(
        &directory,
        &[
            rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            rpc(
                2,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"lint_execution_topology",
                    "arguments":{
                        "request_id":"request:lint-parity",
                        "idempotency_key":"idempotency:lint-parity",
                        "payload":{"topology_json":topology_json}
                    }
                }),
            ),
        ],
    );
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["lint"], cli_report);
    assert_eq!(result["accepted"], false);
    assert_eq!(result["review_status"], "unreviewed");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unsupported_acceptance_mutation_fails_closed_at_the_host() {
    let directory = temp("refusal");
    let responses = run_host(
        &directory,
        &[
            rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            rpc(
                2,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"review_accept",
                    "arguments":{
                        "request_id":"request:no-host-accept",
                        "idempotency_key":"idempotency:no-host-accept",
                        "base_revision_id":"revision:observed",
                        "caller_declared_audit_context":{
                            "declared_actor_id":"actor:reviewer",
                            "declared_capability_ids":["capability:review-declared"],
                            "declared_operation_scope_id":"scope:review",
                            "declared_audience":"audit",
                            "declared_source_boundary_id":"boundary:mcp"
                        },
                        "payload":{}
                    }
                }),
            ),
        ],
    );
    let response = &responses[1]["result"]["structuredContent"];
    assert_eq!(response["result"], Value::Null);
    assert_eq!(
        response["refusal"]["code"],
        "unsupported_operational_host_tool"
    );
    assert_eq!(responses[1]["result"]["isError"], true);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn operational_simulation_equals_the_canonical_library_report() {
    let topology_json = fs::read_to_string(
        root().join("schemas/experimental/execution.topology.file-review.example.json"),
    )
    .unwrap();
    let topology = parse_execution_topology(&topology_json).unwrap();
    let mut request: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("schemas/experimental/graph_simulation.request.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    request["topology_content_hash"] = json!(execution_topology_content_hash(&topology).unwrap());
    let typed_request: GraphSimulationRequest = serde_json::from_value(request.clone()).unwrap();
    let canonical =
        serde_json::to_value(simulate_execution_topology(&topology, &typed_request).unwrap())
            .unwrap();

    let directory = temp("simulation-parity");
    let responses = run_host(
        &directory,
        &[
            rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            rpc(
                2,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"simulate_execution_topology",
                    "arguments":{
                        "request_id":"request:simulation-parity",
                        "idempotency_key":"idempotency:simulation-parity",
                        "payload":{"topology_json":topology_json,"simulation_request":request}
                    }
                }),
            ),
        ],
    );
    assert_eq!(
        responses[1]["result"]["structuredContent"]["result"],
        canonical
    );
    assert_eq!(canonical["routing_proposal"]["review_status"], "unreviewed");
    fs::remove_dir_all(directory).unwrap();
}

fn rpc(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}).to_string()
}

fn temp(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "casegraphen-product-surface-{label}-{}",
        std::process::id()
    ));
    if path.exists() {
        fs::remove_dir_all(&path).unwrap();
    }
    fs::create_dir_all(&path).unwrap();
    path
}

fn run_host(directory: &Path, messages: &[String]) -> Vec<Value> {
    let state = directory.join("state.json");
    let store = directory.join("store");
    let artifacts = directory.join("artifacts");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(&artifacts).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
        .args(["--state"])
        .arg(state)
        .args(["--store"])
        .arg(store)
        .args(["--artifacts"])
        .arg(artifacts)
        .args(["--auth-token-env", "CASEGRAPHEN_TEST_SURFACE_TOKEN"])
        .env("CASEGRAPHEN_TEST_SURFACE_TOKEN", "token:surface")
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
