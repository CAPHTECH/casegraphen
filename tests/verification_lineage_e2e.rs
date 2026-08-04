#![allow(missing_docs)]

use casegraphen::{
    native_store::NativeCaseStore,
    verification_policy::{
        derive_native_cli_review_verifier_proof, derive_native_cli_run_producer_proof,
        observe_case_execution_trace, reconcile_declared_lineage, reconcile_verification_policy,
        AnchoredExecutionTraceBytes, CapabilityConstraints, DeclaredProducerLineage,
        DeclaredVerifierRecord, NativeCliRunLineageDerivation, PolicyProvenance,
        VerificationPolicy, VerificationQuorum, VerifierDisposition, VERIFICATION_POLICY_SCHEMA,
    },
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const CASE_SPACE_ID: &str = "case_space:native-case-management-contract";
const SOURCE_BOUNDARY: &str = "source_boundary:native-case-management-contract";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen")
}

fn json_output(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse CLI JSON")
}

fn temp_store() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "casegraphen-lineage-e2e-{}-{nonce}-{count}",
        std::process::id()
    ))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn current_case(store: &Path) -> casegraphen::native_model::CaseSpace {
    NativeCaseStore::new(store.to_path_buf())
        .replay_current_case_space(&higher_graphen_core::Id::new(CASE_SPACE_ID).unwrap())
        .expect("replay current case")
        .case_space
}

fn current_revision(store: &Path) -> String {
    current_case(store).revision.revision_id.to_string()
}

fn mutation_gate_args<'a>(args: &mut Vec<&'a str>, actor: &'a str) {
    args.extend([
        "--actor-id",
        actor,
        "--capability-id",
        "capability:durable-mutation",
        "--operation-scope-id",
        CASE_SPACE_ID,
        "--audience",
        "audit",
        "--source-boundary-id",
        SOURCE_BOUNDARY,
    ]);
}

fn find_run_files(store: &Path, name: &str) -> Vec<PathBuf> {
    fn visit(path: &Path, name: &str, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(path).expect("read run directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                visit(&path, name, found);
            } else if path.file_name().and_then(|v| v.to_str()) == Some(name) {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    visit(&store.join("runs"), name, &mut found);
    found
}

fn run_step(store: &Path, base_revision: &str, retry: bool) -> Value {
    let mut args = vec![
        "run",
        "--step",
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        CASE_SPACE_ID,
        "--plan-id",
        "plan:lineage-e2e",
        "--base-revision-id",
        base_revision,
        "--actor-id",
        "actor:native-run",
        "--gate-actor-id",
        "actor:native-run",
        "--capability-id",
        "capability:dispatch",
        "--capability-id",
        "capability:native-run-worker",
        "--operation-scope-id",
        CASE_SPACE_ID,
        "--audience",
        "audit",
        "--source-boundary-id",
        SOURCE_BOUNDARY,
        "--enable-worker",
        "shell",
        "--format",
        "json",
    ];
    if retry {
        args.extend(["--retry-step", "step:lineage-e2e"]);
    }
    json_output(&cli(&args))
}

fn review(store: &Path, action: &str, base_revision: &str, claim_id: &str) -> Value {
    let mut args = vec![
        "review",
        action,
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        CASE_SPACE_ID,
        "--target-id",
        claim_id,
        "--reviewer-id",
        "reviewer:lineage-independent",
        "--reason",
        "Independently review exact worker evidence",
        "--base-revision-id",
        base_revision,
        "--format",
        "json",
    ];
    mutation_gate_args(&mut args, "actor:native-evidence-cli");
    json_output(&cli(&args))
}

fn run_lineage_host(store: &Path, calls: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
        .args(["--state"])
        .arg(store.join("lineage-control-plane.state.json"))
        .args(["--store"])
        .arg(store)
        .args(["--artifacts"])
        .arg(store)
        .args(["--auth-token-env", "CASEGRAPHEN_LINEAGE_E2E_TOKEN"])
        .env("CASEGRAPHEN_LINEAGE_E2E_TOKEN", "token:lineage-e2e")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start operational MCP host");
    {
        let input = child.stdin.as_mut().unwrap();
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        for (index, call) in calls.iter().enumerate() {
            writeln!(input, "{}", json!({
                "jsonrpc":"2.0", "id":index + 2, "method":"tools/call",
                "params":{"authorization":"token:lineage-e2e", "name":"reconcile_verification_lineage", "arguments":call}
            })).unwrap();
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for MCP host");
    assert!(
        output.status.success(),
        "MCP host stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn real_cli_run_and_review_derive_live_opaque_lineage_proofs() {
    let store = temp_store();
    fs::create_dir_all(&store).expect("create store");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/casegraphen/native.case.space.example.json");
    let import_revision = "revision:lineage-e2e-import";
    json_output(&cli(&[
        "lift",
        "native",
        "--store",
        store.to_str().unwrap(),
        "--input",
        fixture.to_str().unwrap(),
        "--revision-id",
        import_revision,
        "--format",
        "json",
    ]));

    let mut transition_args = vec![
        "cell",
        "transition",
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        CASE_SPACE_ID,
        "--base-revision-id",
        import_revision,
        "--cell-id",
        "work:review-native-contract",
        "--to",
        "active",
        "--format",
        "json",
    ];
    mutation_gate_args(&mut transition_args, "actor:native-transition-cli");
    json_output(&cli(&transition_args));
    let active_revision = current_revision(&store);

    let binding_path = store.join("lineage.worker.binding.json");
    let first_attempt_marker = store.join("lineage-first-attempt.finished");
    let worker_script = format!(
        "if [ ! -f '{}' ]; then printf 'first-attempt\\n'; : > '{}'; exit 1; fi; printf 'lineage-evidence\\n'; printf 'review-diagnostic\\n' >&2",
        first_attempt_marker.display(),
        first_attempt_marker.display()
    );
    fs::write(
        &binding_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "highergraphen.case.workflow.worker_binding.v1",
            "schema_version": 1,
            "binding_id": "worker_binding:lineage-e2e",
            "worker_kind": "shell",
            "command": "/bin/sh",
            "args": ["-c", worker_script],
            "working_directory": store,
            "resolved_command_path": "/caller/value/is-overwritten",
            "resolved_working_directory": "/caller/value/is-overwritten",
            "command_content_hash": "0".repeat(64),
            "env_allowlist": [],
            "timeout_ms": 5000,
            "capability_ids": ["capability:native-run-worker"],
            "metadata": {}
        }))
        .unwrap(),
    )
    .unwrap();
    json_output(&cli(&[
        "binding",
        "register",
        "--store",
        store.to_str().unwrap(),
        "--input",
        binding_path.to_str().unwrap(),
        "--format",
        "json",
    ]));

    let plan_path = store.join("lineage.execution.plan.json");
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "highergraphen.case.workflow.execution_plan.v1",
            "schema_version": 1,
            "plan_id": "plan:lineage-e2e",
            "case_space_id": CASE_SPACE_ID,
            "base_revision_id": active_revision,
            "steps": [{
                "step_id": "step:lineage-e2e",
                "work_cell_id": "work:review-native-contract",
                "worker_binding_id": "worker_binding:lineage-e2e",
                "success_evidence_requirement_ids": ["evidence:native-schema-json-valid"],
                "allowed_transition_classes": [{
                    "morphism_type": "update",
                    "target_cell_types": ["work"],
                    "to_lifecycles": ["resolved"]
                }]
            }],
            "provenance": {"source": {"kind": "human", "title": "lineage E2E"}, "confidence": 1.0, "review_status": "unreviewed"},
            "review_status": "unreviewed",
            "metadata": {}
        }))
        .unwrap(),
    )
    .unwrap();
    json_output(&cli(&[
        "plan",
        "propose",
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        CASE_SPACE_ID,
        "--input",
        plan_path.to_str().unwrap(),
        "--format",
        "json",
    ]));
    json_output(&cli(&[
        "plan",
        "accept",
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        CASE_SPACE_ID,
        "--plan-id",
        "plan:lineage-e2e",
        "--reviewer-id",
        "reviewer:lineage-plan",
        "--reason",
        "Accept exact lineage plan",
        "--base-revision-id",
        &active_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        CASE_SPACE_ID,
        "--audience",
        "audit",
        "--source-boundary-id",
        SOURCE_BOUNDARY,
        "--format",
        "json",
    ]));
    let accepted_revision = current_revision(&store);

    let first_attempt = run_step(&store, &accepted_revision, false);
    assert_eq!(first_attempt["result"]["status"], json!("step_failed"));
    let retry_base = first_attempt["result"]["trace"]["result_revision_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let run = run_step(&store, &retry_base, true);
    assert_eq!(run["result"]["status"], json!("step_executed"));

    let trace_id = run["result"]["trace"]["trace_id"].as_str().unwrap();
    let trace_paths = find_run_files(&store, "execution.trace.json");
    assert_eq!(trace_paths.len(), 2, "two real retained attempts");
    let trace_path = trace_paths
        .iter()
        .find(|path| {
            serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()["trace_id"]
                == json!(trace_id)
        })
        .expect("successful retry trace")
        .clone();
    let first_trace_path = trace_paths
        .iter()
        .find(|path| *path != &trace_path)
        .expect("failed first-attempt trace")
        .clone();
    let run_dir = trace_path.parent().unwrap();
    let trace_bytes = fs::read(&trace_path).unwrap();
    let report_bytes = fs::read(run_dir.join("worker.report.json")).unwrap();
    let stdout_bytes = fs::read(run_dir.join("stdout")).unwrap();
    let stderr_bytes = fs::read(run_dir.join("stderr")).unwrap();
    let first_run_dir = first_trace_path.parent().unwrap();
    let first_trace_bytes = fs::read(&first_trace_path).unwrap();
    let first_report_bytes = fs::read(first_run_dir.join("worker.report.json")).unwrap();
    let first_stdout_bytes = fs::read(first_run_dir.join("stdout")).unwrap();
    let first_stderr_bytes = fs::read(first_run_dir.join("stderr")).unwrap();
    let trace: Value = serde_json::from_slice(&trace_bytes).unwrap();
    let report: Value = serde_json::from_slice(&report_bytes).unwrap();

    // This replay is intentionally retained before the later review. It
    // proves the producer subject is the run's base revision, while verifier
    // authority may be appended at a later ledger revision.
    let pre_review_case = current_case(&store);
    let claim_id = pre_review_case
        .case_cells
        .iter()
        .find(|cell| {
            cell.metadata
                .get("worker_report_id")
                .and_then(Value::as_str)
                == report["report_id"].as_str()
        })
        .expect("CLI worker evidence claim")
        .id
        .to_string();
    let producer = derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
        case_space: &pre_review_case,
        claim_cell_id: &claim_id,
        worker_report_bytes: &report_bytes,
        execution_trace_bytes: &trace_bytes,
        stdout_bytes: &stdout_bytes,
        stderr_bytes: &stderr_bytes,
    })
    .expect("derive producer from real CLI run");

    let stale_verifier = derive_native_cli_review_verifier_proof(
        &pre_review_case,
        &producer,
        "morphism:review:not-yet-recorded",
    );
    assert!(stale_verifier.is_err());
    let accepted = review(&store, "accept", &current_revision(&store), &claim_id);
    let review_morphism_id = accepted["result"]["entry"]["morphism_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let reviewed_case = current_case(&store);
    let verifier =
        derive_native_cli_review_verifier_proof(&reviewed_case, &producer, &review_morphism_id)
            .expect("derive verifier from normal CLI review");
    let anchor = observe_case_execution_trace(
        &reviewed_case,
        "anchor:lineage-run",
        AnchoredExecutionTraceBytes {
            trace: &trace_bytes,
            worker_report: &report_bytes,
            stdout: &stdout_bytes,
            stderr: &stderr_bytes,
        },
    )
    .expect("observe exact anchored run bytes");
    let policy = VerificationPolicy {
        schema: VERIFICATION_POLICY_SCHEMA.to_owned(),
        verification_policy_id: "verification:lineage-e2e".to_owned(),
        producer_constraints: CapabilityConstraints {
            capability_ids: vec!["capability:native-run-worker".to_owned()],
        },
        verifier_constraints: CapabilityConstraints {
            capability_ids: vec!["capability:durable-mutation".to_owned()],
        },
        actor_must_differ: true,
        lenses: vec!["correctness".to_owned()],
        quorum: VerificationQuorum {
            minimum_accepts: 1,
            total_verifiers: 1,
        },
        required_anchors: vec!["anchor:lineage-run".to_owned()],
        allowed_runtime_attestations: Vec::new(),
        provenance: PolicyProvenance {
            source: "issue-83-e2e".to_owned(),
            created_by: "test".to_owned(),
        },
    };
    let before_reconcile_revision = reviewed_case.revision.revision_id.clone();
    let result = reconcile_verification_policy(
        &reviewed_case,
        &policy,
        &producer,
        std::slice::from_ref(&verifier),
        std::slice::from_ref(&anchor),
    );
    assert!(result.policy_satisfied, "findings: {:?}", result.findings);
    assert_eq!(
        result.ledger_scope.subject_kind,
        casegraphen::verification_policy::LedgerLineageSubjectKind::NativeExecutionTrace
    );
    assert_eq!(
        result.ledger_scope.subject_content_hash,
        sha256(&trace_bytes)
    );
    assert_eq!(result.ledger_scope.topology_content_hash, None);
    assert_eq!(
        current_case(&store).revision.revision_id,
        before_reconcile_revision,
        "policy reconciliation is read-only and cannot append acceptance"
    );
    assert_eq!(
        trace["base_revision_id"],
        json!(retry_base),
        "the producer/verifier subject is the run base, not the later review revision"
    );

    // Exercise the supported product surface as an external MCP process. The
    // host derives the same opaque authority internally and exposes only the
    // read-only policy result.
    let relative = |path: &Path| {
        path.strip_prefix(&store)
            .expect("retained run file under artifact root")
            .to_string_lossy()
            .to_string()
    };
    let lineage_payload = json!({
        "verification_lineage": {
            "case_space_id": CASE_SPACE_ID,
            "claim_cell_id": claim_id,
            "policy": policy.clone(),
            "producer_files": {
                "worker_report_path": relative(&run_dir.join("worker.report.json")),
                "execution_trace_path": relative(&trace_path),
                "stdout_path": relative(&run_dir.join("stdout")),
                "stderr_path": relative(&run_dir.join("stderr"))
            },
            "review_morphism_ids": [review_morphism_id.clone()],
            "anchors": [{"kind":"execution_trace","anchor_id":"anchor:lineage-run"}]
        }
    });
    let host_revision = current_revision(&store);
    let mut duplicate_payload = lineage_payload.clone();
    duplicate_payload["verification_lineage"]["review_morphism_ids"] =
        json!([review_morphism_id.clone(), review_morphism_id.clone()]);
    let mut mixed_attempt_payload = lineage_payload.clone();
    mixed_attempt_payload["verification_lineage"]["producer_files"]["worker_report_path"] =
        json!(relative(&first_run_dir.join("worker.report.json")));
    mixed_attempt_payload["verification_lineage"]["producer_files"]["stdout_path"] =
        json!(relative(&first_run_dir.join("stdout")));
    mixed_attempt_payload["verification_lineage"]["producer_files"]["stderr_path"] =
        json!(relative(&first_run_dir.join("stderr")));
    let responses = run_lineage_host(
        &store,
        &[
            json!({
                "request_id":"request:lineage-positive",
                "idempotency_key":"lineage:positive",
                "base_revision_id":host_revision.clone(),
                "payload":lineage_payload
            }),
            json!({
                "request_id":"request:lineage-duplicate-review",
                "idempotency_key":"lineage:duplicate-review",
                "base_revision_id":host_revision.clone(),
                "payload":duplicate_payload
            }),
            json!({
                "request_id":"request:lineage-mixed-attempt",
                "idempotency_key":"lineage:mixed-attempt",
                "base_revision_id":host_revision.clone(),
                "payload":mixed_attempt_payload
            }),
        ],
    );
    let host_result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(host_result["result"]["policy_satisfied"], true);
    assert_eq!(host_result["proofs_serialized"], false);
    assert_eq!(host_result["read_only"], true);
    assert_eq!(host_result["mutation_performed"], false);
    assert_eq!(host_result["accepted"], false);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["refusal"]["code"],
        "duplicate_or_empty_review_morphism_id"
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["refusal"]["code"],
        "verification_producer_derivation_refused"
    );
    assert_eq!(
        current_revision(&store),
        host_revision,
        "operational lineage reconciliation and refusal are both read-only"
    );

    // A proof that was valid on the reviewed current revision is not valid on
    // the older pre-review replay/fork: its verifier authority does not exist
    // there even though the producer files and subject revision do.
    let stale_revision_result = reconcile_verification_policy(
        &pre_review_case,
        &policy,
        &producer,
        std::slice::from_ref(&verifier),
        std::slice::from_ref(&anchor),
    );
    assert!(!stale_revision_result.policy_satisfied);
    assert!(stale_revision_result.findings.iter().any(|finding| {
        finding.code == "lineage_current_authority_invalid"
            || finding.code == "verifier_review_no_longer_effective"
    }));

    let declarations = reconcile_declared_lineage(
        &policy,
        &DeclaredProducerLineage {
            actor_id: "actor:native-run".to_owned(),
            capability_ids: vec!["capability:native-run-worker".to_owned()],
        },
        &[DeclaredVerifierRecord {
            verifier_report_id: "declared:review".to_owned(),
            actor_id: "actor:native-evidence-cli".to_owned(),
            capability_ids: vec!["capability:durable-mutation".to_owned()],
            disposition: VerifierDisposition::Accept,
            runtime_attestations: Vec::new(),
        }],
    );
    assert!(!declarations.ledger_requirements_satisfied);

    let mut substituted_report = report_bytes.clone();
    substituted_report.push(b' ');
    assert!(
        derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
            case_space: &reviewed_case,
            claim_cell_id: &claim_id,
            worker_report_bytes: &substituted_report,
            execution_trace_bytes: &trace_bytes,
            stdout_bytes: &stdout_bytes,
            stderr_bytes: &stderr_bytes,
        })
        .is_err()
    );
    // Do not simulate an attempt mismatch by editing identifiers: mix the
    // actual retained worker report/streams from the failed first attempt with
    // the exact anchored successful retry trace, and vice versa.
    assert!(
        derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
            case_space: &reviewed_case,
            claim_cell_id: &claim_id,
            worker_report_bytes: &first_report_bytes,
            execution_trace_bytes: &trace_bytes,
            stdout_bytes: &first_stdout_bytes,
            stderr_bytes: &first_stderr_bytes,
        })
        .is_err()
    );
    assert!(
        derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
            case_space: &reviewed_case,
            claim_cell_id: &claim_id,
            worker_report_bytes: &report_bytes,
            execution_trace_bytes: &first_trace_bytes,
            stdout_bytes: &stdout_bytes,
            stderr_bytes: &stderr_bytes,
        })
        .is_err()
    );
    let mut substituted_trace: Value = serde_json::from_slice(&trace_bytes).unwrap();
    substituted_trace["trace_id"] = json!("execution_trace:cross-attempt");
    let substituted_trace = serde_json::to_vec(&substituted_trace).unwrap();
    assert!(
        derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
            case_space: &reviewed_case,
            claim_cell_id: &claim_id,
            worker_report_bytes: &report_bytes,
            execution_trace_bytes: &substituted_trace,
            stdout_bytes: &stdout_bytes,
            stderr_bytes: &stderr_bytes,
        })
        .is_err()
    );
    assert!(
        derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
            case_space: &reviewed_case,
            claim_cell_id: "evidence:native-schema-json-valid",
            worker_report_bytes: &report_bytes,
            execution_trace_bytes: &trace_bytes,
            stdout_bytes: &stdout_bytes,
            stderr_bytes: &stderr_bytes,
        })
        .is_err()
    );

    review(&store, "reopen", &current_revision(&store), &claim_id);
    let reopened = current_case(&store);
    let reopened_result = reconcile_verification_policy(
        &reopened,
        &policy,
        &producer,
        std::slice::from_ref(&verifier),
        std::slice::from_ref(&anchor),
    );
    assert!(!reopened_result.policy_satisfied);

    let accepted_again = review(&store, "accept", &current_revision(&store), &claim_id);
    let current = current_case(&store);
    let verifier_again = derive_native_cli_review_verifier_proof(
        &current,
        &producer,
        accepted_again["result"]["entry"]["morphism_id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    // Capability grants are source-boundary administered and cannot be
    // rewritten by a normal durable mutation. Model the next authoritative
    // source snapshot after retirement from the fully CLI-produced replay;
    // no proof constructor or morphism is fabricated, and reconciliation must
    // re-check the retained opaque proof against this changed grant state.
    let mut retired = current_case(&store);
    retired
        .case_cells
        .iter_mut()
        .find(|cell| cell.id.as_str() == "capability:durable-mutation")
        .unwrap()
        .lifecycle = casegraphen::native_model::CaseCellLifecycle::Retired;
    let retired_result =
        reconcile_verification_policy(&retired, &policy, &producer, &[verifier_again], &[anchor]);
    assert!(!retired_result.policy_satisfied);
    assert!(retired_result
        .findings
        .iter()
        .any(|finding| finding.code == "lineage_current_operation_gate_invalid"));

    fs::remove_dir_all(store).expect("remove E2E store");
}
