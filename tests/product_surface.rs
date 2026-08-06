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

#[test]
fn operational_memory_tools_are_read_only_or_unreviewed_proposals() {
    let directory = temp("memory-boundary");
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();
    let create = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "space",
            "new",
            "--store",
            store.to_str().unwrap(),
            "--case-space-id",
            "case_space:memory-mcp",
            "--space-id",
            "space:memory-mcp",
            "--title",
            "Memory MCP fixture",
            "--revision-id",
            "revision:memory-mcp",
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

    let artifacts = directory.join("artifacts");
    fs::create_dir_all(artifacts.join("memory-sources")).unwrap();
    let source_bytes = b"CaseGraphen does not own LLM execution.\n";
    fs::write(artifacts.join("memory-sources/adr-0002.txt"), source_bytes).unwrap();
    let digest = casegraphen::memory::content_hash(source_bytes);
    let policy = json!({
        "schema":"casegraphen.experimental.memory.policy.v0",
        "policy_id":"memory-policy:mcp",
        "project_id":"casegraphen",
        "actor_grants":[{
            "actor_id":"actor:coding-agent",
            "allowed_audiences":["ai_agent"],
            "allowed_purposes":["code_change"],
            "project_ids":["casegraphen"],
            "max_sensitivity":"internal",
            "max_authority":"project_constraint"
        }],
        "valid_time_required_kinds":["preference","goal","commitment"],
        "hard_conflict_relation_types":["contradicts"],
        "exact_source_escalation":true
    });
    let query = json!({
        "schema":"casegraphen.experimental.memory.query.v0",
        "query_id":"memory-query:mcp",
        "base_revision_id":"revision:memory-mcp",
        "requesting_actor_id":"actor:coding-agent",
        "audience":"ai_agent",
        "purpose":"code_change",
        "risk_class":"normal",
        "as_of":"2026-08-06T00:00:00Z",
        "scope":{"case_space_id":"case_space:memory-mcp","project_id":"casegraphen","actor_ids":[]},
        "memory_kinds":["constraint"],
        "budget":{"max_items":30,"max_tokens":6000},
        "query_text":"runtime boundary",
        "include_historical":false,
        "include_contested":false
    });
    let source_record = json!({
        "schema":"casegraphen.experimental.memory.source_record.v0",
        "source_record_id":"memory-source:mcp",
        "source_kind":"document",
        "content_hash":format!("sha256:{digest}"),
        "captured_at":"2026-08-06T00:00:00Z",
        "origin_actor_id":"actor:architecture-reviewer",
        "source_boundary_id":"source_boundary:repository",
        "authority_origin":"reviewer",
        "sensitivity":"internal",
        "artifact_ref":"docs/adr/0002-graph-engineering-positioning.md"
    });
    let claim = json!({
        "schema":"casegraphen.experimental.memory.claim.v0",
        "claim_id":"memory:runtime-boundary-mcp",
        "memory_kind":"constraint",
        "subject_refs":["repo:CAPHTECH/casegraphen"],
        "statement":{"predicate":"must_not_depend_on","object":"agent-runtime"},
        "scope":{"case_space_id":"case_space:memory-mcp","project_id":"casegraphen","actor_ids":[]},
        "valid_time":{"valid_from":"2026-07-30T00:00:00Z"},
        "source_refs":[format!("artifact:sha256-{digest}")],
        "derivation_actor_id":"actor:memory-proposer",
        "derivation_method":"extraction",
        "model_assertions_are_untrusted":true,
        "provenance_role":"reviewed_architecture_decision",
        "authority_ceiling":"project_constraint",
        "sensitivity":"internal"
    });
    let proposal = json!({
        "case_space_id":"case_space:memory-mcp",
        "source_record":source_record,
        "claim":claim,
        "policy":policy,
        "artifact_path":"memory-sources/adr-0002.txt"
    });
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
                    "name":"memory_query",
                    "arguments":{
                        "request_id":"request:memory-query",
                        "idempotency_key":"idempotency:memory-query",
                        "base_revision_id":"revision:memory-mcp",
                        "payload":{"memory_request":{
                            "case_space_id":"case_space:memory-mcp",
                            "query":query,
                            "policy":policy
                        }}
                    }
                }),
            ),
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"memory_propose_claim",
                    "arguments":{
                        "request_id":"request:memory-propose",
                        "idempotency_key":"idempotency:memory-propose",
                        "base_revision_id":"revision:memory-mcp",
                        "payload":{"memory_proposal":proposal}
                    }
                }),
            ),
            rpc(
                4,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"memory_propose_claim",
                    "arguments":{
                        "request_id":"request:memory-forged-acceptance",
                        "idempotency_key":"idempotency:memory-forged-acceptance",
                        "base_revision_id":"revision:memory-mcp",
                        "payload":{"memory_proposal":{
                            "accepted":true,
                            "case_space_id":"case_space:memory-mcp",
                            "source_record":source_record,
                            "claim":claim,
                            "policy":policy,
                            "artifact_path":"memory-sources/adr-0002.txt"
                        }}
                    }
                }),
            ),
        ],
    );
    let query_result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(query_result["projection"]["read_only"], true);
    assert_eq!(query_result["mutation_performed"], false);
    let proposal_result = &responses[2]["result"]["structuredContent"]["result"];
    assert_eq!(proposal_result["accepted"], false);
    assert_eq!(proposal_result["mutation_performed"], false);
    assert_eq!(
        proposal_result["claim_proposal"]["claim_cell"]["lifecycle"],
        "proposed"
    );
    assert_eq!(
        proposal_result["claim_proposal"]["claim_cell"]["provenance"]["review_status"],
        "unreviewed"
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["refusal"]["code"],
        "invalid_payload"
    );

    let replay = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "space",
            "replay",
            "--store",
            store.to_str().unwrap(),
            "--case-space-id",
            "case_space:memory-mcp",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(replay.status.success());
    let replay: Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(
        replay["result"]["replay"]["current_revision_id"],
        "revision:memory-mcp"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #102's `github observe|refresh|project`: store-free, read-only,
/// and no more capable than that. Proves two things a schema cannot: (1)
/// no file anywhere under the read-only pilot capture directory (or
/// anywhere else the commands could reach) is created or modified by any of
/// the three commands, including a `refresh` that reads a *second* capture
/// directory as its previous-basis input; (2) every output record carries
/// `accepted: false` and `mutation_performed: false`, the same read-only
/// discipline `operational_memory_tools_are_read_only_or_unreviewed_proposals`
/// proves for the Memory Plane MCP tools above.
#[test]
fn github_evidence_commands_never_mutate_the_filesystem() {
    let pilot_dir = root().join("docs/pilots/issue-102");
    let manifest = pilot_dir.join("capture_manifest.v0.json");
    let before = snapshot_directory(&pilot_dir);

    let observe = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["github", "observe", "--manifest"])
        .arg(&manifest)
        .args(["--capture-dir"])
        .arg(&pilot_dir)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(
        observe.status.success(),
        "{}",
        String::from_utf8_lossy(&observe.stderr)
    );
    let observe_result: Value = serde_json::from_slice(&observe.stdout).unwrap();
    assert_eq!(observe_result["result"]["accepted"], false);
    assert_eq!(observe_result["result"]["mutation_performed"], false);

    let refresh = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["github", "refresh", "--manifest"])
        .arg(&manifest)
        .args(["--capture-dir"])
        .arg(&pilot_dir)
        .args(["--previous-manifest"])
        .arg(&manifest)
        .args(["--previous-capture-dir"])
        .arg(&pilot_dir)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(
        refresh.status.success(),
        "{}",
        String::from_utf8_lossy(&refresh.stderr)
    );
    let refresh_result: Value = serde_json::from_slice(&refresh.stdout).unwrap();
    assert_eq!(refresh_result["result"]["accepted"], false);
    assert_eq!(refresh_result["result"]["mutation_performed"], false);

    let project = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["github", "project", "--manifest"])
        .arg(&manifest)
        .args(["--capture-dir"])
        .arg(&pilot_dir)
        .args(["--require-independent-review", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        project.status.success(),
        "{}",
        String::from_utf8_lossy(&project.stderr)
    );
    let project_result: Value = serde_json::from_slice(&project.stdout).unwrap();
    assert_eq!(project_result["result"]["accepted"], false);
    assert_eq!(project_result["result"]["mutation_performed"], false);

    let after = snapshot_directory(&pilot_dir);
    assert_eq!(
        before, after,
        "github observe/refresh/project must not create or modify any file under \
         docs/pilots/issue-102 (including the second, --previous-capture-dir read of \
         the same directory)"
    );
}

/// `(relative path, byte length, modified time)` for every file under
/// `directory`, sorted by path — enough to catch a create, a delete, or an
/// in-place rewrite without hashing every file's bytes on every test run.
fn snapshot_directory(directory: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    fn walk(directory: &Path, root: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                walk(&path, root, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    metadata.len(),
                    metadata.modified().unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(directory, directory, &mut out);
    out.sort();
    out
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
