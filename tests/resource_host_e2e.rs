#![allow(missing_docs)]

use casegraphen::{
    deployment_policy::{deployment_policy_manifest, deployment_policy_manifest_content_hash},
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
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[test]
fn operational_host_reserves_then_reconciles_a_resource_bearing_run_to_review() {
    let directory = temp("reconcile");
    let ReviewedDeploymentFixture {
        topology_json,
        topology_path,
        policy_path,
        accepted_revision,
        bundle_hash,
        compiler_payload: _,
    } = reviewed_deployment(&directory);
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
        Some(&accepted_revision),
        json!({
            "resource_request":{
                "deployment_authority":{
                    "case_space_id":native_case_space_id(),
                    "claim_cell_id":"evidence:execution-topology",
                    "deployment_bundle_hash":bundle_hash
                },
                "declaration":declaration,"reservation":reservation
            }
        }),
        true,
    );
    let reserve_responses = run_host(&directory, &[reserve]);
    assert_eq!(reserve_responses[1]["result"]["isError"], false);
    assert_eq!(
        reserve_responses[1]["result"]["structuredContent"]["result"]["allocator_generation"],
        1
    );
    let reviewed_binding = reserve_responses[1]["result"]["structuredContent"]["result"]
        ["allocator_event"]["payload"]["reviewed_deployment"]
        .clone();
    assert_eq!(reviewed_binding["deployment_bundle_hash"], bundle_hash);
    assert!(
        validates_against_control_plane_response_schema(
            &reserve_responses[1]["result"]["structuredContent"]
        ),
        "a real, live reserve_resources response failed to validate \
         against control_plane.response.v0"
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
        "topology_content_hash":topology_hash,"case_revision_id":accepted_revision,
        "expectations":[{"node_id":node.node_id,"attempt_id":"attempt:e2e-review-a:1","reviewed_deployment":reviewed_binding,"declaration":declaration,
            "reservation":reservation,"allocations":[allocation],"disposition_evidence":[]}]
    });
    let reconcile = tool_call(
        "reconcile_run",
        "reconcile",
        Some(&accepted_revision),
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
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live reconcile_run response failed to validate against control_plane.response.v0"
    );

    if let Ok(path) = std::env::var("CASEGRAPHEN_REVIEWED_RESOURCE_PILOT_REPORT") {
        let retained = json!({
            "schema":"casegraphen.experimental.reviewed_resource_pilot.report.v0",
            "passed":true,
            "accepted":false,
            "halt":"needs_review",
            "topology_content_hash":topology_hash,
            "reviewed_deployment_hash":bundle_hash,
            "accepted_review_revision_id":accepted_revision,
            "reviewed_deployment_binding":reviewed_binding,
            "reconciliation_complete":result["reconciliation_complete"],
            "proposal_count":result["proposals"].as_array().map_or(0, Vec::len),
            "failure_disposition":"review_proposals_only"
        });
        fs::write(path, serde_json::to_vec_pretty(&retained).unwrap()).unwrap();
    }

    let reopened = gated_cli(
        &directory.join("store"),
        &[
            "topology-review",
            "reopen",
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:execution-topology",
            "--input",
            topology_path.to_str().unwrap(),
            "--policy-manifest",
            policy_path.to_str().unwrap(),
            "--reviewer-id",
            "reviewer:resource",
            "--reason",
            "Advance the ledger while preserving cleanup authority.",
            "--base-revision-id",
            &accepted_revision,
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    let reopened: Value = serde_json::from_slice(&reopened.stdout).unwrap();
    let current_revision = reopened["result"]["record"]["current_revision_id"]
        .as_str()
        .unwrap();
    let assertion = json!({
        "schema":"casegraphen.experimental.resource.reservation_disposition.v0",
        "schema_version":0,
        "assertion_id":"assertion:e2e-review-a:release",
        "reservation_id":"reservation:e2e-review-a",
        "attempt_id":"attempt:e2e-review-a:1",
        "kind":"release",
        "asserted_by":"operator:e2e",
        "reason":"runtime attempt completed and cleanup was independently requested",
        "superseding_reservation_id":null
    });
    let stale_release = tool_call(
        "release_resources",
        "release-stale",
        Some(&accepted_revision),
        json!({"resource_disposition":{"assertion":assertion.clone()}}),
        true,
    );
    let stale = run_host(&directory, &[stale_release]);
    assert_eq!(stale[1]["result"]["isError"], true);
    assert_eq!(
        stale[1]["result"]["structuredContent"]["refusal"]["code"],
        "stale_revision"
    );
    assert!(
        validates_against_control_plane_response_schema(&stale[1]["result"]["structuredContent"]),
        "a real, live release_resources refusal failed to validate against control_plane.response.v0"
    );
    let release = tool_call(
        "release_resources",
        "release-current",
        Some(current_revision),
        json!({"resource_disposition":{"assertion":assertion}}),
        true,
    );
    let released = run_host(&directory, &[release]);
    assert_eq!(released[1]["result"]["isError"], false, "{released:?}");
    assert_eq!(
        released[1]["result"]["structuredContent"]["result"]["allocator_generation"],
        2
    );
    assert!(
        validates_against_control_plane_response_schema(
            &released[1]["result"]["structuredContent"]
        ),
        "a real, live release_resources response failed to validate against control_plane.response.v0"
    );
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

#[test]
fn reviewed_compile_and_reservation_fail_closed_on_stale_or_tampered_authority() {
    let directory = temp("reviewed-negative");
    let reviewed = reviewed_deployment(&directory);
    let mut wrong_claim_payload = reviewed.compiler_payload.clone();
    wrong_claim_payload["compiler_request"]["claim_cell_id"] =
        json!("evidence:native-schema-json-valid");
    let wrong_claim = tool_call(
        "compile_reviewed_deployment_bundle",
        "compile-wrong-claim",
        Some(&reviewed.accepted_revision),
        wrong_claim_payload,
        false,
    );
    let wrong_claim = run_host(&directory, &[wrong_claim]);
    assert_eq!(
        wrong_claim[1]["result"]["structuredContent"]["refusal"]["code"],
        "reviewed_compilation_authority_refused"
    );
    let mut substituted_policy = reviewed.compiler_payload.clone();
    substituted_policy["compiler_request"]["budget_policies"]["budget:small"]["max_cost"] =
        json!(999);
    let substituted = tool_call(
        "compile_reviewed_deployment_bundle",
        "compile-substituted-policy",
        Some(&reviewed.accepted_revision),
        substituted_policy,
        false,
    );
    let substituted = run_host(&directory, &[substituted]);
    assert_eq!(
        substituted[1]["result"]["structuredContent"]["refusal"]["code"],
        "reviewed_compilation_refused"
    );
    let reopened = gated_cli(
        &directory.join("store"),
        &[
            "topology-review",
            "reopen",
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:execution-topology",
            "--input",
            reviewed.topology_path.to_str().unwrap(),
            "--policy-manifest",
            reviewed.policy_path.to_str().unwrap(),
            "--reviewer-id",
            "reviewer:resource",
            "--reason",
            "Invalidate deployment authority for negative coverage.",
            "--base-revision-id",
            &reviewed.accepted_revision,
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    let reopened: Value = serde_json::from_slice(&reopened.stdout).unwrap();
    let current_revision = reopened["result"]["record"]["current_revision_id"]
        .as_str()
        .unwrap();
    let stale_compile = tool_call(
        "compile_reviewed_deployment_bundle",
        "compile-stale",
        Some(&reviewed.accepted_revision),
        reviewed.compiler_payload.clone(),
        false,
    );
    let stale = run_host(&directory, &[stale_compile]);
    assert_eq!(
        stale[1]["result"]["structuredContent"]["refusal"]["code"],
        "stale_revision"
    );

    let bundle_topology = directory
        .join("artifacts/bundles")
        .join(&reviewed.bundle_hash)
        .join("execution.topology.json");
    fs::write(&bundle_topology, b"{\"tampered\":true}").unwrap();
    let topology = parse_execution_topology(&reviewed.topology_json).unwrap();
    let node = &topology.nodes[0];
    let declaration = ResourceDeclaration {
        schema: RESOURCE_DECLARATION_SCHEMA.to_owned(),
        schema_version: 0,
        declaration_id: "declaration:tampered-bundle".to_owned(),
        runtime_graph_id: topology.topology_id.clone(),
        runtime_graph_content_hash: execution_topology_content_hash(&topology).unwrap(),
        node_id: node.node_id.clone(),
        claims: node.resource_claims.clone(),
    };
    let reservation = ResourceReservation {
        schema: RESOURCE_RESERVATION_SCHEMA.to_owned(),
        schema_version: 0,
        reservation_id: "reservation:tampered-bundle".to_owned(),
        declaration_id: declaration.declaration_id.clone(),
        attempt_id: "attempt:tampered-bundle".to_owned(),
        granted_at: "2026-08-04T00:00:00Z".to_owned(),
        grants: declaration_grants(&declaration),
    };
    let reserve = tool_call(
        "reserve_resources",
        "tampered-bundle",
        Some(current_revision),
        json!({"resource_request":{
            "deployment_authority":{
                "case_space_id":native_case_space_id(),
                "claim_cell_id":"evidence:execution-topology",
                "deployment_bundle_hash":reviewed.bundle_hash
            },
            "declaration":declaration,
            "reservation":reservation
        }}),
        true,
    );
    let refused = run_host(&directory, &[reserve]);
    assert_eq!(refused[1]["result"]["isError"], true);
    assert_eq!(
        refused[1]["result"]["structuredContent"]["refusal"]["code"],
        "deployment_bundle_integrity_failure"
    );
    assert_store_survives_refusals(&directory.join("store"));
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

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// ADR 0034 / #117 pattern: validate a real, live response against the
/// shipped contract rather than asserting about the schema in the abstract.
fn validates_against_control_plane_response_schema(instance: &Value) -> bool {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "casegraphen-resource-host-schema-check-{}-{nonce}.json",
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

struct ReviewedDeploymentFixture {
    topology_json: String,
    topology_path: PathBuf,
    policy_path: PathBuf,
    accepted_revision: String,
    bundle_hash: String,
    compiler_payload: Value,
}

fn reviewed_deployment(directory: &Path) -> ReviewedDeploymentFixture {
    let store = directory.join("store");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/casegraphen/native.case.space.example.json");
    let lift = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(["lift", "native", "--store"])
        .arg(&store)
        .args(["--input"])
        .arg(&fixture)
        .args([
            "--revision-id",
            "revision:resource-reviewed-base",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        lift.status.success(),
        "{}",
        String::from_utf8_lossy(&lift.stderr)
    );

    let mut topology_value: Value = serde_json::from_str(include_str!(
        "../schemas/experimental/execution.topology.file-review.example.json"
    ))
    .unwrap();
    topology_value["topology_id"] = json!("topology:resource-reviewed");
    topology_value["case_space_id"] = json!(native_case_space_id());
    topology_value["nodes"] = json!([topology_value["nodes"][0].clone()]);
    topology_value["edges"] = json!([]);
    let topology_json = serde_json::to_string(&topology_value).unwrap();
    let topology = parse_execution_topology(&topology_json).unwrap();
    let topology_hash = execution_topology_content_hash(&topology).unwrap();

    let verification_policies = topology
        .verification_policy_ids
        .iter()
        .map(|id| {
            let mut value: Value = serde_json::from_str(include_str!(
                "../schemas/experimental/verification.policy.example.json"
            ))
            .unwrap();
            value["verification_policy_id"] = json!(id);
            (id.clone(), value)
        })
        .collect::<BTreeMap<_, _>>();
    let budget_policies = topology
        .budget_policy_ids
        .iter()
        .map(|id| (id.clone(), json!({"policy_id":id,"max_cost":10})))
        .collect::<BTreeMap<_, _>>();
    let expansion_policies = BTreeMap::new();
    let policy_manifest = deployment_policy_manifest(
        &topology,
        &topology_hash,
        &verification_policies,
        &budget_policies,
        &expansion_policies,
    );
    let policy_manifest_hash = deployment_policy_manifest_content_hash(&policy_manifest).unwrap();
    let topology_path = directory.join("execution.topology.json");
    let topology_bytes = serde_json::to_vec_pretty(&topology_value).unwrap();
    fs::write(&topology_path, &topology_bytes).unwrap();
    let policy_path = directory.join("deployment-policy-manifest.json");
    fs::write(
        &policy_path,
        serde_json::to_vec_pretty(&policy_manifest).unwrap(),
    )
    .unwrap();
    let artifact_hash = format!("{:x}", Sha256::digest(&topology_bytes));
    let artifact_id = format!("artifact:sha256-{artifact_hash}");
    let mut claim: Value = serde_json::from_str(include_str!(
        "../schemas/casegraphen/native.case.space.example.json"
    ))
    .unwrap();
    let mut claim = claim["case_cells"][3].take();
    claim["id"] = json!("evidence:execution-topology");
    claim["title"] = json!("Reviewed resource deployment topology");
    claim["lifecycle"] = json!("active");
    claim["provenance"]["review_status"] = json!("unreviewed");
    claim["metadata"] = json!({
        "evidence_boundary":"inferred",
        "topology_id":topology.topology_id,
        "execution_topology_content_hash":topology_hash,
        "artifact_id":artifact_id,
        "case_space_id":native_case_space_id(),
        "policy_manifest_content_hash":policy_manifest_hash
    });
    let claim_path = directory.join("execution-topology-claim.json");
    fs::write(&claim_path, serde_json::to_vec_pretty(&claim).unwrap()).unwrap();

    let attach = gated_cli(
        &store,
        &[
            "evidence",
            "attach",
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:resource-reviewed-base",
            "--input",
            claim_path.to_str().unwrap(),
            "--artifact",
            topology_path.to_str().unwrap(),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    let attached: Value = serde_json::from_slice(&attach.stdout).unwrap();
    let attached_revision = attached["result"]["record"]["current_revision_id"]
        .as_str()
        .unwrap();
    let review = gated_cli(
        &store,
        &[
            "topology-review",
            "accept",
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:execution-topology",
            "--input",
            topology_path.to_str().unwrap(),
            "--policy-manifest",
            policy_path.to_str().unwrap(),
            "--reviewer-id",
            "reviewer:resource",
            "--reason",
            "Reviewed exact resource deployment topology and policies.",
            "--base-revision-id",
            attached_revision,
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    let reviewed: Value = serde_json::from_slice(&review.stdout).unwrap();
    let accepted_revision = reviewed["result"]["record"]["current_revision_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mappings = topology
        .nodes
        .iter()
        .map(|node| json!({
            "node_id":node.node_id,
            "worker_binding_id":format!("worker_binding:{}",node.node_id),
            "success_evidence_requirement_ids":[format!("evidence_requirement:{}",node.node_id)],
            "allowed_transition_classes":[{"morphism_type":"update","target_cell_types":["work"],"to_lifecycles":["resolved"]}]
        }))
        .collect::<Vec<_>>();
    let compiler_payload = json!({
        "topology_json":topology_json,
        "compiler_request":{
            "case_space_id":native_case_space_id(),
            "claim_cell_id":"evidence:execution-topology",
            "plan_id":"plan:resource-reviewed",
            "node_plan_mappings":mappings,
            "verification_policies":verification_policies,
            "budget_policies":budget_policies,
            "expansion_policies":expansion_policies
        }
    });
    let compile = tool_call(
        "compile_reviewed_deployment_bundle",
        "reviewed-compile",
        Some(&accepted_revision),
        compiler_payload.clone(),
        false,
    );
    let responses = run_host(directory, &[compile]);
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["deployment_authority"], "reviewed");
    assert_eq!(result["manifest"]["mode"], "reviewed");
    assert_eq!(
        result["manifest"]["accepted_review_revision_id"],
        accepted_revision
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live compile_reviewed_deployment_bundle response failed to validate \
         against control_plane.response.v0"
    );
    ReviewedDeploymentFixture {
        topology_json,
        topology_path,
        policy_path,
        accepted_revision,
        bundle_hash: result["manifest_content_hash"].as_str().unwrap().to_owned(),
        compiler_payload,
    }
}

fn gated_cli(store: &Path, args: &[&str], actor_id: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_casegraphen"));
    command.args(args).args(["--store"]).arg(store).args([
        "--actor-id",
        actor_id,
        "--capability-id",
        "capability:durable-mutation",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
    ]);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn native_case_space_id() -> &'static str {
    "case_space:native-case-management-contract"
}

fn assert_store_survives_refusals(store: &Path) {
    for operation in ["validate", "rebuild"] {
        let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
            .args(["space", operation, "--store"])
            .arg(store)
            .args([
                "--case-space-id",
                native_case_space_id(),
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "space {operation} failed after refused authority inputs: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
