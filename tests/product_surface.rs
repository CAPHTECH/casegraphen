#![allow(missing_docs)]

//! Issue #129 added the tests below marked `Tier 1`/`Tier 2`/`Tier 3`/`Tier 4`
//! in their doc comments, closing the gap where seventeen of twenty-eight MCP
//! catalog tools had no real-host test anywhere. The tier is load-bearing,
//! not decoration — it says what kind of test this is, and simplifying one
//! down to another tier's shape silently drops what it was proving:
//!
//! - **Tier 1**: the tool's logic exists *only* inline in
//!   `casegraphen-mcp-host.rs` (an evidence write, a proposal's own guard, an
//!   unimplemented-tool refusal). The e2e test is the sole verification that
//!   code path has, or will ever have. Do not shrink it to "call it, check
//!   `isError`" — it must assert on the actual claim the tool carries
//!   (emitted bytes, a file on disk, a specific refusal code).
//! - **Tier 2**: the core algorithm is already unit-tested at the library
//!   level (`dynamic_expansion.rs`, `streaming_reconciliation.rs`,
//!   `topology_redesign.rs`, `runtime_integration.rs`). The e2e test's job is
//!   narrower — prove the host's wire marshaling doesn't mangle it — so a
//!   parity comparison against the library call is the right shape here, not
//!   a defect if it looks thinner than a Tier 1 test.
//! - **Tier 3**: read-only tools whose own filtering logic
//!   (`query_memory`, contested-claim handling) is unit-tested in
//!   `memory_plane.rs`. What's untested is the host's per-tool flag-setting
//!   and output reshaping, so the bar is proving genuine divergence between
//!   sibling tools on the same fixture, not re-proving the filter itself.
//! - **Tier 4**: two tool names resolve to the literal same match arm. The
//!   risk is someone splitting that arm later and fixing only one branch —
//!   a comment noting "identical to X" would not catch that, since comments
//!   don't fail. These tests drive both names against the same input and
//!   assert identical output, so a future split breaks the test unless both
//!   branches stay correct.
//!
//! See issue #129 for the full ranking and reasoning behind the tier
//! assignments. `memory_propose_supersession` (the 17th originally-listed
//! tool) is covered by `operational_memory_relation_proposal_is_unreviewed_and_contracted`
//! above, from #120.

use casegraphen::{
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    graph_simulation::{simulate_execution_topology, GraphSimulationRequest},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Fixture builders for the `memory_conflicts`/`memory_sources`/
/// `memory_explain`/`memory_history` e2e tests below (issue #129).
///
/// `query_memory` only grants `MemoryStatus::Accepted` to a claim whose
/// source is an immutable, content-addressed `custom:artifact` cell reached
/// by an accepted `derives_from` relation (`memory/query.rs::sources_are_immutable`).
/// `native_model.rs::require_artifact_cell_entered_via_attach` refuses to let
/// *any* morphism — including genesis/lift — mint a `custom:artifact` cell
/// directly: "genesis is not exempt here: an artifact is a fact about one
/// file, not a source boundary a human authored and can vouch for as a
/// whole." So an e2e fixture with a genuinely *accepted* memory claim cannot
/// be hand-assembled and lifted; it has to be built through the real
/// `evidence attach` → `review accept` pipeline, the same path a live case
/// would use. These helpers drive that pipeline through the real CLI (never
/// touching the store format directly) and hand back the revision id the
/// host should be pointed at.
const MEMORY_E2E_CASE_SPACE_ID: &str = "case_space:native-case-management-contract";
const MEMORY_E2E_ACTOR_ID: &str = "actor:native-evidence-cli";
const MEMORY_E2E_MUTATION_ACTOR_ID: &str = "actor:native-mutation-cli";
const MEMORY_E2E_AS_OF: &str = "2026-08-06T00:00:00Z";

fn memory_e2e_gate_args() -> [&'static str; 8] {
    [
        "--capability-id",
        "capability:durable-mutation",
        "--operation-scope-id",
        MEMORY_E2E_CASE_SPACE_ID,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
    ]
}

fn memory_e2e_run(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn memory_e2e_revision(report: &Value) -> String {
    report["result"]["record"]["current_revision_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Lifts the shipped, already-valid `native.case.space.example.json`
/// (genesis morphism log intact, `capability:durable-mutation` already
/// present) and returns `(store_dir, initial_revision_id)`.
fn memory_e2e_lift(directory: &Path) -> (PathBuf, String) {
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();
    let revision_id = "revision:memory-e2e-genesis".to_owned();
    memory_e2e_run(&[
        "lift",
        "native",
        "--store",
        store.to_str().unwrap(),
        "--input",
        root()
            .join("schemas/casegraphen/native.case.space.example.json")
            .to_str()
            .unwrap(),
        "--revision-id",
        &revision_id,
        "--format",
        "json",
    ]);
    (store, revision_id)
}

/// Attaches `claim` as a real, gated `evidence attach` (with a real
/// `--artifact` file, so its content-addressed source cell is genuine) and
/// promotes it to `accepted` with a real `review accept`. Returns the
/// resulting revision id: the claim is now genuinely `MemoryStatus::Accepted`
/// when queried, not merely shaped like one.
fn memory_e2e_attach_and_accept_claim(
    directory: &Path,
    store: &Path,
    base_revision_id: &str,
    claim: &Value,
    source_bytes: &[u8],
) -> String {
    let claim_id = claim["claim_id"].as_str().unwrap().to_owned();
    let digest = format!("{:x}", Sha256::digest(source_bytes));
    let artifact_path = directory.join(format!("{}.artifact", claim_id.replace(':', "_")));
    fs::write(&artifact_path, source_bytes).unwrap();

    // `sources_are_immutable` requires the claim's own `source_refs` to name
    // this exact artifact id — set after the digest is known, the same as
    // the real `evidence attach --artifact` flow computes it after reading
    // the file, not before.
    let mut claim = claim.clone();
    claim["source_refs"] = json!([format!("artifact:sha256-{digest}")]);
    let claim_id = claim_id.as_str();

    let source_record = json!({
        "schema":"casegraphen.experimental.memory.source_record.v0",
        "source_record_id":format!("memory-source:{claim_id}"),
        "source_kind":"artifact",
        "content_hash":format!("sha256:{digest}"),
        "captured_at":MEMORY_E2E_AS_OF,
        "origin_actor_id":"actor:fixture-reviewer",
        "source_boundary_id":"source_boundary:test",
        "authority_origin":"reviewer",
        "sensitivity":"internal",
        "artifact_ref":format!("fixture:{claim_id}")
    });
    let claim_cell = json!({
        "id":claim_id,
        "cell_type":"evidence",
        "space_id":"space:higher-graphen-casegraphen",
        "title":claim_id,
        "lifecycle":"active",
        "source_ids":["source:native-design-doc"],
        "structure_ids":[],
        "provenance":{"confidence":1.0,"review_status":"unreviewed","source":{"kind":"human"}},
        "metadata":{"memory_claim":claim,"memory_source_records":[source_record]}
    });
    let claim_cell_path = directory.join(format!("{}.cell.json", claim_id.replace(':', "_")));
    fs::write(&claim_cell_path, serde_json::to_vec(&claim_cell).unwrap()).unwrap();

    let mut attach_args = vec![
        "evidence".to_owned(),
        "attach".to_owned(),
        "--store".to_owned(),
        store.to_str().unwrap().to_owned(),
        "--case-space-id".to_owned(),
        MEMORY_E2E_CASE_SPACE_ID.to_owned(),
        "--base-revision-id".to_owned(),
        base_revision_id.to_owned(),
        "--input".to_owned(),
        claim_cell_path.to_str().unwrap().to_owned(),
        "--artifact".to_owned(),
        artifact_path.to_str().unwrap().to_owned(),
        "--actor-id".to_owned(),
        MEMORY_E2E_ACTOR_ID.to_owned(),
    ];
    attach_args.extend(memory_e2e_gate_args().iter().map(|arg| arg.to_string()));
    attach_args.push("--format".to_owned());
    attach_args.push("json".to_owned());
    let attach_report = memory_e2e_run(&attach_args.iter().map(String::as_str).collect::<Vec<_>>());
    let attached_revision = memory_e2e_revision(&attach_report);

    let mut accept_args = vec![
        "review".to_owned(),
        "accept".to_owned(),
        "--store".to_owned(),
        store.to_str().unwrap().to_owned(),
        "--case-space-id".to_owned(),
        MEMORY_E2E_CASE_SPACE_ID.to_owned(),
        "--target-id".to_owned(),
        claim_id.to_owned(),
        "--reviewer-id".to_owned(),
        "reviewer:human".to_owned(),
        "--reason".to_owned(),
        "e2e fixture acceptance".to_owned(),
        "--base-revision-id".to_owned(),
        attached_revision,
        "--evidence-id".to_owned(),
        claim_id.to_owned(),
        "--actor-id".to_owned(),
        MEMORY_E2E_ACTOR_ID.to_owned(),
    ];
    accept_args.extend(memory_e2e_gate_args().iter().map(|arg| arg.to_string()));
    accept_args.push("--format".to_owned());
    accept_args.push("json".to_owned());
    let accept_report = memory_e2e_run(&accept_args.iter().map(String::as_str).collect::<Vec<_>>());
    memory_e2e_revision(&accept_report)
}

/// Adds an accepted `CaseRelation` between two already-accepted cells through
/// a real, gated, generic `morphism propose` → `morphism apply` — the same
/// mechanism `tests/command.rs::native_morphism_propose_check_apply_and_reject_flow`
/// uses. `contradicts`/hard is the only relation shape these tests need.
fn memory_e2e_add_hard_conflict_relation(
    directory: &Path,
    store: &Path,
    base_revision_id: &str,
    relation_id: &str,
    from_claim_id: &str,
    to_claim_id: &str,
) -> String {
    let morphism_id = format!("morphism:{relation_id}");
    let morphism = json!({
        "morphism_id":morphism_id,
        "morphism_type":"update",
        "source_revision_id":base_revision_id,
        "target_revision_id":format!("revision:{relation_id}"),
        "added_ids":[relation_id],
        "updated_ids":[],
        "retired_ids":[],
        "preserved_ids":[],
        "violated_invariant_ids":[],
        "review_status":"accepted",
        "evidence_ids":[],
        "source_ids":["source:native-cli-test"],
        "metadata":{
            "payload":{
                "added_cells":[],
                "added_relations":[{
                    "id":relation_id,
                    "relation_type":"contradicts",
                    "relation_strength":"hard",
                    "from_id":from_claim_id,
                    "to_id":to_claim_id,
                    "evidence_ids":[],
                    "source_ids":["source:native-cli-test"],
                    "provenance":{"confidence":1.0,"review_status":"accepted","source":{"kind":"human"}},
                    "metadata":{}
                }]
            }
        }
    });
    let morphism_path = directory.join(format!("{}.morphism.json", relation_id.replace(':', "_")));
    fs::write(&morphism_path, serde_json::to_vec(&morphism).unwrap()).unwrap();

    memory_e2e_run(&[
        "morphism",
        "propose",
        "--store",
        store.to_str().unwrap(),
        "--case-space-id",
        MEMORY_E2E_CASE_SPACE_ID,
        "--input",
        morphism_path.to_str().unwrap(),
        "--format",
        "json",
    ]);

    let mut apply_args = vec![
        "morphism".to_owned(),
        "apply".to_owned(),
        "--store".to_owned(),
        store.to_str().unwrap().to_owned(),
        "--case-space-id".to_owned(),
        MEMORY_E2E_CASE_SPACE_ID.to_owned(),
        "--morphism-id".to_owned(),
        morphism_id,
        "--base-revision-id".to_owned(),
        base_revision_id.to_owned(),
        "--reviewer-id".to_owned(),
        "reviewer:human".to_owned(),
        "--reason".to_owned(),
        "e2e hard-conflict fixture".to_owned(),
        "--actor-id".to_owned(),
        MEMORY_E2E_MUTATION_ACTOR_ID.to_owned(),
    ];
    apply_args.extend(memory_e2e_gate_args().iter().map(|arg| arg.to_string()));
    apply_args.push("--format".to_owned());
    apply_args.push("json".to_owned());
    let apply_report = memory_e2e_run(&apply_args.iter().map(String::as_str).collect::<Vec<_>>());
    memory_e2e_revision(&apply_report)
}

fn memory_e2e_claim(claim_id: &str, predicate: &str, object: &str) -> Value {
    json!({
        "schema":"casegraphen.experimental.memory.claim.v0",
        "claim_id":claim_id,
        "memory_kind":"constraint",
        "subject_refs":["repo:CAPHTECH/casegraphen"],
        "statement":{"predicate":predicate,"object":object},
        "scope":{"case_space_id":MEMORY_E2E_CASE_SPACE_ID,"project_id":"casegraphen","actor_ids":[]},
        "valid_time":{"valid_from":"2026-01-01T00:00:00Z"},
        "derivation_actor_id":"actor:memory-proposer",
        "derivation_method":"extraction",
        "model_assertions_are_untrusted":true,
        "provenance_role":"reviewed_architecture_decision",
        "authority_ceiling":"project_constraint",
        "sensitivity":"internal"
    })
}

fn memory_e2e_policy() -> Value {
    json!({
        "schema":"casegraphen.experimental.memory.policy.v0",
        "policy_id":"memory-policy:e2e",
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
    })
}

fn memory_e2e_query(revision_id: &str, claim_id: Option<&str>) -> Value {
    json!({
        "case_space_id":MEMORY_E2E_CASE_SPACE_ID,
        "query":{
            "schema":"casegraphen.experimental.memory.query.v0",
            "query_id":"memory-query:e2e",
            "base_revision_id":revision_id,
            "requesting_actor_id":"actor:coding-agent",
            "audience":"ai_agent",
            "purpose":"code_change",
            "risk_class":"normal",
            "as_of":MEMORY_E2E_AS_OF,
            "scope":{"case_space_id":MEMORY_E2E_CASE_SPACE_ID,"project_id":"casegraphen","actor_ids":[]},
            "memory_kinds":["constraint"],
            "budget":{"max_items":30,"max_tokens":6000},
            "query_text":"runtime boundary",
            "include_historical":false,
            "include_contested":false
        },
        "policy":memory_e2e_policy(),
        "claim_id":claim_id
    })
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
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live lint_execution_topology response failed to validate \
         against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 4: `propose_execution_topology` shares the *exact* match
/// arm with `lint_execution_topology` above
/// (`LintExecutionTopology | ProposeExecutionTopology => { ... }`), and
/// neither `requires_base_revision()` nor `changes_managed_state()`
/// distinguishes them either. Same reasoning as the memory_explain/history
/// pairing: the risk isn't that the shared arm is wrong, it's that someone
/// splits it later and only fixes one branch. Driving both tool names against
/// the identical payload and asserting identical output makes that split
/// break this test instead of merely reading fine in review.
#[test]
fn operational_propose_and_lint_execution_topology_produce_the_identical_shape() {
    let topology_json = fs::read_to_string(
        root().join("schemas/experimental/execution.topology.file-review.example.json"),
    )
    .unwrap();
    let directory = temp("propose-lint-parity");
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
                        "request_id":"request:propose-lint-parity-lint",
                        "idempotency_key":"idempotency:propose-lint-parity-lint",
                        "payload":{"topology_json":topology_json}
                    }
                }),
            ),
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"propose_execution_topology",
                    "arguments":{
                        "request_id":"request:propose-lint-parity-propose",
                        "idempotency_key":"idempotency:propose-lint-parity-propose",
                        "payload":{"topology_json":topology_json}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    assert_eq!(responses[2]["result"]["isError"], false, "{responses:?}");
    let lint_result = &responses[1]["result"]["structuredContent"]["result"];
    let propose_result = &responses[2]["result"]["structuredContent"]["result"];
    assert_eq!(
        lint_result, propose_result,
        "the shared arm must keep both tools' output identical"
    );
    assert_eq!(propose_result["accepted"], false);
    assert_eq!(propose_result["review_status"], "unreviewed");
    assert!(
        validates_against_control_plane_response_schema(
            &responses[2]["result"]["structuredContent"]
        ),
        "a real, live propose_execution_topology response failed to validate \
         against control_plane.response.v0"
    );
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
    // Issue #166: the refusal must say the outcome is permanent for this
    // release and name where the operation actually lives, and
    // `suggested_next_operation` must not suggest a retry that can never
    // succeed on this host.
    let refusal = &response["refusal"];
    assert_ne!(
        refusal["suggested_next_operation"], "inspect_host_state_and_retry_explicitly",
        "{refusal}"
    );
    let detail = refusal["detail"].as_str().unwrap();
    assert!(detail.contains("permanent"), "{detail}");
    assert!(detail.contains("casegraphen review accept"), "{detail}");
    assert!(
        validates_against_control_plane_response_schema(response),
        "a real, live refusal response failed to validate against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129: `review_accept`'s sibling mutation tools — `apply_evidence_packet`,
/// `review_reject`, `supersede_dispatch`, and `resume` (the 17th untested tool;
/// the issue's own enumeration named 16, but `resume` shares the identical
/// unimplemented arm and had zero real-host coverage anywhere) — had never been
/// driven through the real binary at all. All five names fall into the same
/// explicit refusal arm in `casegraphen-mcp-host.rs` (no longer a `_` wildcard;
/// see the match's comment for why), so a change that silently narrows that
/// arm for exactly one of them — implementing it without the gating this crate
/// requires — would pass every test that only exercises `review_accept`. This
/// drives each of the four remaining names by itself and pins the exact
/// refusal code and schema validity per tool name, not just "some" refusal.
#[test]
fn remaining_unsupported_mutation_tools_fail_closed_at_the_host() {
    let directory = temp("refusal-remaining");
    let audit_context = json!({
        "declared_actor_id":"actor:reviewer",
        "declared_capability_ids":["capability:review-declared"],
        "declared_operation_scope_id":"scope:review",
        "declared_audience":"audit",
        "declared_source_boundary_id":"boundary:mcp"
    });
    let tools = [
        ("apply_evidence_packet", "casegraphen packet apply"),
        ("review_reject", "casegraphen review reject"),
        (
            "supersede_dispatch",
            "casegraphen run/operate --supersede-trace",
        ),
        ("resume", "casegraphen packet resume"),
    ];
    let mut messages = vec![
        rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
    ];
    for (index, (tool, _cli_command)) in tools.iter().enumerate() {
        messages.push(rpc(
            2 + index as u64,
            "tools/call",
            json!({
                "authorization":"token:surface",
                "name":tool,
                "arguments":{
                    "request_id":format!("request:no-host-{tool}"),
                    "idempotency_key":format!("idempotency:no-host-{tool}"),
                    "base_revision_id":"revision:observed",
                    "caller_declared_audit_context":audit_context,
                    "payload":{}
                }
            }),
        ));
    }
    let responses = run_host(&directory, &messages);
    for (index, (tool, cli_command)) in tools.iter().enumerate() {
        let response = &responses[index + 1]["result"]["structuredContent"];
        assert_eq!(
            response["result"],
            Value::Null,
            "{tool} unexpectedly returned a result"
        );
        assert_eq!(
            response["refusal"]["code"], "unsupported_operational_host_tool",
            "{tool} did not fail closed with the shared refusal code"
        );
        assert_eq!(
            responses[index + 1]["result"]["isError"],
            true,
            "{tool} did not report isError"
        );
        // Issue #166: same permanence/no-retry checks as the sibling
        // `review_accept` test, driven per tool name so a fix that only
        // updates one of the five cannot pass silently.
        let refusal = &response["refusal"];
        assert_ne!(
            refusal["suggested_next_operation"], "inspect_host_state_and_retry_explicitly",
            "{tool}: {refusal}"
        );
        let detail = refusal["detail"].as_str().unwrap();
        assert!(detail.contains("permanent"), "{tool}: {detail}");
        assert!(detail.contains(cli_command), "{tool}: {detail}");
        assert!(
            validates_against_control_plane_response_schema(response),
            "a real, live {tool} refusal response failed to validate against \
             control_plane.response.v0"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 1: unlike every tool above, `attach_runtime_report`'s
/// content-addressed write — the `create_new` file open, the same-bytes
/// idempotency check, and the forced `accepted:false`/`unreviewed` output —
/// lives only inline in `casegraphen-mcp-host.rs`. No library-level unit test
/// exercises it; this e2e test is the only verification that code path has,
/// or will ever have. Do not shrink this into a "call it, check isError"
/// test: it must keep checking the actual bytes on disk.
#[test]
fn operational_attach_runtime_report_is_content_addressed_and_forced_unreviewed() {
    let directory = temp("attach-runtime-report");
    let record = "{\"schema\":\"casegraphen.experimental.runtime.node_report.v0\",\"report_id\":\"runtime_report:e2e:1\"}\n";
    let other_record = "{\"schema\":\"casegraphen.experimental.runtime.node_report.v0\",\"report_id\":\"runtime_report:e2e:2\"}\n";
    let digest = format!("{:x}", Sha256::digest(record.as_bytes()));
    let other_digest = format!("{:x}", Sha256::digest(other_record.as_bytes()));
    assert_ne!(digest, other_digest);

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
                    "name":"attach_runtime_report",
                    "arguments":{
                        "request_id":"request:attach-first",
                        "idempotency_key":"idempotency:attach-first",
                        "base_revision_id":"revision:observed",
                        "caller_declared_audit_context":{
                            "declared_actor_id":"actor:runtime-adapter",
                            "declared_capability_ids":["capability:attach-declared"],
                            "declared_operation_scope_id":"scope:attach",
                            "declared_audience":"audit",
                            "declared_source_boundary_id":"boundary:mcp"
                        },
                        "payload":{"jsonl_record":record}
                    }
                }),
            ),
            // Same bytes, a fresh request/idempotency key: the write path must
            // be idempotent on content, not merely on idempotency key.
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"attach_runtime_report",
                    "arguments":{
                        "request_id":"request:attach-repeat",
                        "idempotency_key":"idempotency:attach-repeat",
                        "base_revision_id":"revision:observed",
                        "caller_declared_audit_context":{
                            "declared_actor_id":"actor:runtime-adapter",
                            "declared_capability_ids":["capability:attach-declared"],
                            "declared_operation_scope_id":"scope:attach",
                            "declared_audience":"audit",
                            "declared_source_boundary_id":"boundary:mcp"
                        },
                        "payload":{"jsonl_record":record}
                    }
                }),
            ),
            rpc(
                4,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"attach_runtime_report",
                    "arguments":{
                        "request_id":"request:attach-other",
                        "idempotency_key":"idempotency:attach-other",
                        "base_revision_id":"revision:observed",
                        "caller_declared_audit_context":{
                            "declared_actor_id":"actor:runtime-adapter",
                            "declared_capability_ids":["capability:attach-declared"],
                            "declared_operation_scope_id":"scope:attach",
                            "declared_audience":"audit",
                            "declared_source_boundary_id":"boundary:mcp"
                        },
                        "payload":{"jsonl_record":other_record}
                    }
                }),
            ),
        ],
    );
    for index in [1, 2] {
        let result = &responses[index]["result"]["structuredContent"]["result"];
        assert_eq!(result["artifact_id"], format!("artifact:sha256-{digest}"));
        assert_eq!(result["content_hash"], format!("sha256:{digest}"));
        assert_eq!(result["accepted"], false);
        assert_eq!(result["review_status"], "unreviewed");
        assert_eq!(responses[index]["result"]["isError"], false);
        assert!(
            validates_against_control_plane_response_schema(
                &responses[index]["result"]["structuredContent"]
            ),
            "a real, live attach_runtime_report response failed to validate \
             against control_plane.response.v0"
        );
    }
    let other_result = &responses[3]["result"]["structuredContent"]["result"];
    assert_eq!(
        other_result["artifact_id"],
        format!("artifact:sha256-{other_digest}")
    );
    assert_eq!(
        other_result["content_hash"],
        format!("sha256:{other_digest}")
    );

    // The claim is the bytes on disk, not the tool's self-report.
    let first_path = directory
        .join("artifacts/runtime-ingest")
        .join(format!("sha256-{digest}.jsonl"));
    assert_eq!(fs::read_to_string(&first_path).unwrap(), record);
    let other_path = directory
        .join("artifacts/runtime-ingest")
        .join(format!("sha256-{other_digest}.jsonl"));
    assert_eq!(fs::read_to_string(&other_path).unwrap(), other_record);

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
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live simulate_execution_topology response failed to validate \
         against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 2: `reconcile_resources` delegates to
/// `reconcile_resource_allocations`, which is well covered at the library
/// level (`runtime_integration.rs`) — the gap here is purely the host's
/// payload marshaling, not the algorithm. Reuses the shipped
/// `mcp.resource_reconciliation_input.v0.example.json` wire fixture directly
/// (it is already a schema-valid `payload.resource_reconciliation`) and
/// compares the host's `reconciliation` field against calling the same
/// library function directly with the same typed inputs, the same parity
/// pattern as `operational_simulation_equals_the_canonical_library_report`
/// above.
#[test]
fn operational_reconcile_resources_equals_the_canonical_library_report() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("schemas/experimental/mcp.resource_reconciliation_input.v0.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let declaration: casegraphen::resource_protocol::ResourceDeclaration =
        serde_json::from_value(fixture["declaration"].clone()).unwrap();
    let reservation: casegraphen::resource_protocol::ResourceReservation =
        serde_json::from_value(fixture["reservation"].clone()).unwrap();
    let allocations: Vec<casegraphen::resource_protocol::RuntimeResourceAllocation> =
        serde_json::from_value(fixture["allocations"].clone()).unwrap();
    let canonical = serde_json::to_value(
        casegraphen::resource_protocol::reconcile_resource_allocations(
            &declaration,
            &reservation,
            &allocations,
        ),
    )
    .unwrap();

    let directory = temp("reconcile-resources-parity");
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
                    "name":"reconcile_resources",
                    "arguments":{
                        "request_id":"request:reconcile-resources-parity",
                        "idempotency_key":"idempotency:reconcile-resources-parity",
                        "base_revision_id":"revision:observed",
                        "payload":{"resource_reconciliation":fixture}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["reconciliation"], canonical);
    assert_eq!(result["accepted_runtime_output"], false);
    assert_eq!(result["base_revision_id"], "revision:observed");
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live reconcile_resources response failed to validate \
         against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 2: `propose_topology_redesign`'s core algorithm
/// (`propose_redesign`) is unit-tested extensively in `topology_redesign.rs`;
/// the host-layer gap is whether the wire payload actually reaches it intact
/// and the response keeps `accepted:false`/`unreviewed` forced. Builds the
/// same old/proposed topology pair `topology_redesign.rs::topologies()` uses
/// and compares the host's `proposal` field against calling `propose_redesign`
/// directly with the same typed input.
#[test]
fn operational_propose_topology_redesign_equals_the_canonical_library_report() {
    let topology_json = fs::read_to_string(
        root().join("schemas/experimental/execution.topology.file-review.example.json"),
    )
    .unwrap();
    let old = parse_execution_topology(&topology_json).unwrap();
    let mut proposed = old.clone();
    proposed.nodes[0].purpose.push_str(" with focused routing");
    proposed.budget_policy_ids.push("budget:focused".into());
    let proposed_json = serde_json::to_string(&proposed).unwrap();

    let redesign_request = json!({
        "evidence":{
            "audit_artifact_ids":[format!("artifact:sha256-{}", "a".repeat(64))],
            "integration_proposal_ids":[format!("proposal:sha256-{}", "2".repeat(64))],
            "expansion_proposal_ids":[format!("proposal:sha256-{}", "3".repeat(64))]
        },
        "expected_impact":[{
            "metric":"latency_ms",
            "expected_direction":"decrease",
            "estimated_delta":-90.0,
            "rationale":"simulation comparison"
        }],
        "uncertainty":["runtime latency calibration may drift"],
        "information_loss":["none observed; exact node diff retained"],
        "reviewer_authority":{
            "authority_policy_id":"authority:topology-review",
            "required_capability_ids":["capability:review"]
        },
        "simulation":{
            "input_artifact_id":format!("artifact:sha256-{}", "b".repeat(64)),
            "old_report_artifact_id":format!("artifact:sha256-{}", "c".repeat(64)),
            "proposed_report_artifact_id":format!("artifact:sha256-{}", "d".repeat(64))
        }
    });
    let typed_input: casegraphen::topology_redesign::RedesignProposalInput =
        serde_json::from_value(redesign_request.clone()).unwrap();
    let canonical = serde_json::to_value(
        casegraphen::topology_redesign::propose_redesign(&old, &proposed, typed_input).unwrap(),
    )
    .unwrap();

    let directory = temp("redesign-parity");
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
                    "name":"propose_topology_redesign",
                    "arguments":{
                        "request_id":"request:redesign-parity",
                        "idempotency_key":"idempotency:redesign-parity",
                        "base_revision_id":"revision:observed",
                        "payload":{
                            "topology_json":topology_json,
                            "proposed_topology_json":proposed_json,
                            "redesign_request":redesign_request
                        }
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["proposal"], canonical);
    assert_eq!(result["accepted"], false);
    assert_eq!(result["review_status"], "unreviewed");
    assert_eq!(canonical["review_status"], "unreviewed");
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live propose_topology_redesign response failed to validate \
         against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 2: `evaluate_expansion_round` drives `ExpansionController`,
/// which is heavily unit-tested in `dynamic_expansion.rs` — the host-layer gap
/// is purely wire marshaling (`ExpansionEvaluationInput` deserialization, the
/// begin/process/finish attempt sequencing, and the forced
/// `accepted:false`/`unreviewed` envelope). Reuses the same policy and
/// topology fixtures `dynamic_expansion.rs` builds its controller from and
/// asserts on the real halt/decision shape the host emits.
#[test]
fn operational_evaluate_expansion_round_reports_real_controller_decisions() {
    let topology_json = fs::read_to_string(
        root().join("schemas/experimental/execution.topology.file-review.example.json"),
    )
    .unwrap();
    let topology = parse_execution_topology(&topology_json).unwrap();
    let policy: Value = serde_json::from_str(
        &fs::read_to_string(root().join("schemas/experimental/expansion.policy.example.json"))
            .unwrap(),
    )
    .unwrap();

    let mut added_node = serde_json::to_value(&topology.nodes[0]).unwrap();
    added_node["node_id"] = json!("node:expansion-candidate");
    added_node["work_cell_id"] = json!("work:expansion-candidate");
    added_node["idempotency_key"] = json!("expand:expansion-candidate");
    let candidate = json!({
        "candidate_schema_id":"schema:bug-candidate",
        "dedupe_values":{"file":"src/a.rs","symbol":"handler","failure_signature":"panic-1"},
        "requested_disposition":"accept_for_proposal",
        "topology_patch":{
            "schema":"casegraphen.experimental.topology.patch.v0",
            "schema_version":0,
            "added_nodes":[added_node],
            "removed_node_ids":[],
            "updated_nodes":[],
            "added_edges":[],
            "removed_edge_ids":[]
        }
    });
    let expansion_round = json!({
        "policy":policy,
        "attempt_id":"attempt:evaluate-expansion-round-e2e",
        "rounds":[{
            "candidates":[candidate],
            "accounted_round_cost":1.5,
            "accounted_round_latency_ms":4000
        }]
    });

    let directory = temp("expansion-round");
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
                    "name":"evaluate_expansion_round",
                    "arguments":{
                        "request_id":"request:expansion-round-e2e",
                        "idempotency_key":"idempotency:expansion-round-e2e",
                        "base_revision_id":"revision:observed",
                        "payload":{"topology_json":topology_json,"expansion_round":expansion_round}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["accepted"], false);
    assert_eq!(result["review_status"], "unreviewed");
    assert_eq!(result["base_revision_id"], "revision:observed");
    let rounds = result["rounds"].as_array().expect("rounds array");
    assert_eq!(rounds.len(), 1);
    // A real accepted-for-proposal candidate halts the round for review —
    // proves the controller's disposition-to-halt wiring actually ran through
    // the host, not a passthrough.
    assert_eq!(rounds[0]["halt"], "needs_review");
    let decisions = rounds[0]["decisions"].as_array().expect("decisions array");
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["disposition"], "accepted_for_proposal");
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live evaluate_expansion_round response failed to validate \
         against control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 2 (the most complex of the batch): `reconcile_streaming_run`
/// composes `derive_streaming_acceptance`, `GenericJsonlReconciler`,
/// `derive_streaming_resource_permits`, and `reconcile_stream` behind an exact
/// case-revision check against the real store. Every one of those pieces is
/// separately unit-tested in `streaming_reconciliation.rs`; what's untested is
/// whether the host actually threads a real lifted case space and a real
/// topology through that composition without losing the chain. Deliberately
/// minimal (no resource claims, no stream events): the point is to prove the
/// wire path reaches every stage and stays revision-exact, not to re-prove
/// the streaming algorithm.
#[test]
fn operational_reconcile_streaming_run_reaches_the_real_store_and_topology() {
    let directory = temp("streaming-run");
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();
    let revision_id = "revision:streaming-run-e2e";
    let lift = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "lift",
            "native",
            "--store",
            store.to_str().unwrap(),
            "--input",
        ])
        .arg(root().join("schemas/casegraphen/native.case.space.example.json"))
        .args(["--revision-id", revision_id, "--format", "json"])
        .output()
        .unwrap();
    assert!(
        lift.status.success(),
        "{}",
        String::from_utf8_lossy(&lift.stderr)
    );

    let case_space_id = "case_space:native-case-management-contract";
    let mut topology = parse_execution_topology(
        &fs::read_to_string(
            root().join("schemas/experimental/execution.topology.file-review.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    topology.case_space_id = case_space_id.to_owned();
    for node in &mut topology.nodes {
        node.resource_claims.clear();
    }
    let topology_json = serde_json::to_string(&topology).unwrap();
    let expectation =
        casegraphen::runtime_protocol::derive_runtime_graph_expectation(&topology).unwrap();
    let expectation_json = serde_json::to_value(&expectation).unwrap();

    let streaming_run = json!({
        "case_space_id":case_space_id,
        "expectation":expectation_json,
        "runtime_jsonl":"",
        "run_closed":false
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
                    "name":"reconcile_streaming_run",
                    "arguments":{
                        "request_id":"request:streaming-run-e2e",
                        "idempotency_key":"idempotency:streaming-run-e2e",
                        "base_revision_id":revision_id,
                        "payload":{"topology_json":topology_json,"streaming_run":streaming_run}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    // No findings: proves acceptance derivation from the real replayed case
    // space and resource-permit derivation both actually ran and agreed,
    // rather than the host short-circuiting before reaching either.
    assert_eq!(result["findings"], json!([]));
    assert_eq!(result["status"], "collecting");
    let mut unfinished = result["unfinished_node_ids"]
        .as_array()
        .expect("unfinished_node_ids array")
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    unfinished.sort();
    let mut expected_node_ids = topology
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    expected_node_ids.sort();
    assert_eq!(unfinished, expected_node_ids);
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live reconcile_streaming_run response failed to validate \
         against control_plane.response.v0"
    );

    // A stale revision must be refused before any of that composition runs.
    let stale_responses = run_host(
        &directory,
        &[
            rpc(1, "initialize", json!({"protocolVersion":"2025-06-18"})),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            rpc(
                2,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"reconcile_streaming_run",
                    "arguments":{
                        "request_id":"request:streaming-run-stale",
                        "idempotency_key":"idempotency:streaming-run-stale",
                        "base_revision_id":"revision:not-the-real-one",
                        "payload":{"topology_json":topology_json,"streaming_run":streaming_run}
                    }
                }),
            ),
        ],
    );
    assert_eq!(
        stale_responses[1]["result"]["isError"], true,
        "{stale_responses:?}"
    );
    assert_eq!(
        stale_responses[1]["result"]["structuredContent"]["refusal"]["code"],
        "stale_revision"
    );

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

    // ADR 0034 / issue #120, T6: live envelope validation for memory_query
    // and memory_propose_claim, plus T4's memory.claim_proposal.v0 contract
    // validated against the nested `claim_proposal` a real MCP
    // memory_propose_claim response actually emits.
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_query response failed to validate against control_plane.response.v0"
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[2]["result"]["structuredContent"]
        ),
        "a real, live memory_propose_claim response failed to validate \
         against control_plane.response.v0"
    );
    assert!(
        validates_against_schema(
            &proposal_result["claim_proposal"],
            "memory.claim_proposal.v0.schema.json"
        ),
        "a real, live MCP memory_propose_claim claim_proposal failed to validate \
         against memory.claim_proposal.v0: {}",
        proposal_result["claim_proposal"]
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

/// Finding 2 of the adversarial-execution-reviewer's pass on ADR 0034:
/// `relation_proposal` crosses the wire nested inside `memory_propose_supersession`
/// / `memory_propose_retraction` results and had no payload contract — found
/// by the original #120 audit, then dropped from the ADR's Decision and
/// deferral accounting. `memory.relation_proposal.v0` closes that, pinning
/// `accepted` and `review_status` `const` + `required` (#117's pattern).
/// Proven against the real, live host, not a hand-typed instance: the target
/// claim is a real evidence cell lifted into the store precisely so
/// `memory_propose_supersession` has something legitimate to name.
#[test]
fn operational_memory_relation_proposal_is_unreviewed_and_contracted() {
    let directory = temp("memory-relation");
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();

    let mut fixture: Value = serde_json::from_str(include_str!(
        "../schemas/casegraphen/native.case.space.example.json"
    ))
    .unwrap();
    let case_space_id = fixture["case_space_id"].as_str().unwrap().to_owned();
    let space_id = fixture["space_id"].as_str().unwrap().to_owned();
    let target_cell_id = "evidence:memory-claim-to-supersede";
    fixture["case_cells"].as_array_mut().unwrap().push(json!({
        "id": target_cell_id,
        "cell_type": "evidence",
        "space_id": space_id,
        "title": "Memory claim: existing runtime-boundary constraint",
        "lifecycle": "accepted",
        "source_ids": [],
        "structure_ids": [],
        "provenance": {"source": {"kind": "document"}, "confidence": 1.0, "review_status": "accepted"},
        "metadata": {"memory_claim": {"claim_id": "memory:existing-runtime-boundary"}}
    }));
    let fixture_path = directory.join("fixture.json");
    fs::write(&fixture_path, serde_json::to_vec(&fixture).unwrap()).unwrap();

    let revision_id = "revision:memory-relation-e2e";
    let lift = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "lift",
            "native",
            "--store",
            store.to_str().unwrap(),
            "--input",
            fixture_path.to_str().unwrap(),
            "--revision-id",
            revision_id,
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

    let artifacts = directory.join("artifacts");
    fs::create_dir_all(artifacts.join("memory-sources")).unwrap();
    let source_bytes = b"CaseGraphen supersedes stale memory with a reviewed replacement.\n";
    fs::write(artifacts.join("memory-sources/adr-0002.txt"), source_bytes).unwrap();
    let digest = casegraphen::memory::content_hash(source_bytes);
    let policy = json!({
        "schema":"casegraphen.experimental.memory.policy.v0",
        "policy_id":"memory-policy:relation-e2e",
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
    let source_record = json!({
        "schema":"casegraphen.experimental.memory.source_record.v0",
        "source_record_id":"memory-source:relation-e2e",
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
        "claim_id":"memory:runtime-boundary-relation-e2e",
        "memory_kind":"constraint",
        "subject_refs":["repo:CAPHTECH/casegraphen"],
        "statement":{"predicate":"must_not_depend_on","object":"agent-runtime"},
        "scope":{"case_space_id":case_space_id,"project_id":"casegraphen","actor_ids":[]},
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
        "case_space_id":case_space_id,
        "source_record":source_record,
        "claim":claim,
        "policy":policy,
        "artifact_path":"memory-sources/adr-0002.txt",
        "target_claim_id":target_cell_id
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
                    "name":"memory_propose_supersession",
                    "arguments":{
                        "request_id":"request:memory-relation-e2e",
                        "idempotency_key":"idempotency:memory-relation-e2e",
                        "base_revision_id":revision_id,
                        "payload":{"memory_proposal":proposal}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    let relation_proposal = &result["relation_proposal"];
    assert_eq!(relation_proposal["relation_type"], "supersedes");
    assert_eq!(relation_proposal["accepted"], false);
    assert_eq!(relation_proposal["review_status"], "unreviewed");
    assert_eq!(relation_proposal["to_id"], target_cell_id);
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_propose_supersession response failed to validate \
         against control_plane.response.v0"
    );
    assert!(
        validates_against_schema(relation_proposal, "memory.relation_proposal.v0.schema.json"),
        "a real, live MemoryRelationProposal failed to validate against \
         memory.relation_proposal.v0: {relation_proposal}"
    );

    let shipped_example: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("schemas/experimental/memory.relation_proposal.v0.example.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        relation_proposal, &shipped_example,
        "the shipped example must be this exact real output, not a hand-typed instance"
    );

    let mut forged_accepted = relation_proposal.clone();
    forged_accepted["accepted"] = json!(true);
    assert!(
        !validates_against_schema(&forged_accepted, "memory.relation_proposal.v0.schema.json"),
        "an accepted: true forgery must fail schema validation"
    );
    let mut forged_status = relation_proposal.clone();
    forged_status["review_status"] = json!("accepted");
    assert!(
        !validates_against_schema(&forged_status, "memory.relation_proposal.v0.schema.json"),
        "a review_status: \"accepted\" forgery must fail schema validation"
    );
    let mut omitted_accepted = relation_proposal.clone();
    omitted_accepted.as_object_mut().unwrap().remove("accepted");
    assert!(
        !validates_against_schema(&omitted_accepted, "memory.relation_proposal.v0.schema.json"),
        "omitting accepted must also fail schema validation (const alone is evadable by omission)"
    );
    let mut omitted_status = relation_proposal.clone();
    omitted_status
        .as_object_mut()
        .unwrap()
        .remove("review_status");
    assert!(
        !validates_against_schema(&omitted_status, "memory.relation_proposal.v0.schema.json"),
        "omitting review_status must also fail schema validation"
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 1: `memory_propose_retraction` shares `memory_proposal_tool`
/// with the now-covered `memory_propose_supersession` above, but the two
/// diverge in exactly the field a consumer acts on — `relation_type`
/// (`"retracts"` vs `"supersedes"`). That divergence was unverified: nothing
/// proved the retraction branch actually emits `"retracts"` rather than
/// silently reusing the supersession branch's output. Mirrors the
/// supersession test's fixture and forged-claim rejections; the shipped
/// `memory.relation_proposal.v0.example.json` is pinned to the supersession
/// case above, so this test validates schema conformance directly instead of
/// byte-comparing against that example.
#[test]
fn operational_memory_propose_retraction_is_unreviewed_and_contracted() {
    let directory = temp("memory-retraction");
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();

    let mut fixture: Value = serde_json::from_str(include_str!(
        "../schemas/casegraphen/native.case.space.example.json"
    ))
    .unwrap();
    let case_space_id = fixture["case_space_id"].as_str().unwrap().to_owned();
    let space_id = fixture["space_id"].as_str().unwrap().to_owned();
    let target_cell_id = "evidence:memory-claim-to-retract";
    fixture["case_cells"].as_array_mut().unwrap().push(json!({
        "id": target_cell_id,
        "cell_type": "evidence",
        "space_id": space_id,
        "title": "Memory claim: existing runtime-boundary constraint",
        "lifecycle": "accepted",
        "source_ids": [],
        "structure_ids": [],
        "provenance": {"source": {"kind": "document"}, "confidence": 1.0, "review_status": "accepted"},
        "metadata": {"memory_claim": {"claim_id": "memory:existing-runtime-boundary-to-retract"}}
    }));
    let fixture_path = directory.join("fixture.json");
    fs::write(&fixture_path, serde_json::to_vec(&fixture).unwrap()).unwrap();

    let revision_id = "revision:memory-retraction-e2e";
    let lift = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "lift",
            "native",
            "--store",
            store.to_str().unwrap(),
            "--input",
            fixture_path.to_str().unwrap(),
            "--revision-id",
            revision_id,
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

    let artifacts = directory.join("artifacts");
    fs::create_dir_all(artifacts.join("memory-sources")).unwrap();
    let source_bytes = b"CaseGraphen retracts stale memory that no longer holds.\n";
    fs::write(artifacts.join("memory-sources/adr-0002.txt"), source_bytes).unwrap();
    let digest = casegraphen::memory::content_hash(source_bytes);
    let policy = json!({
        "schema":"casegraphen.experimental.memory.policy.v0",
        "policy_id":"memory-policy:retraction-e2e",
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
    let source_record = json!({
        "schema":"casegraphen.experimental.memory.source_record.v0",
        "source_record_id":"memory-source:retraction-e2e",
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
        "claim_id":"memory:runtime-boundary-retraction-e2e",
        "memory_kind":"constraint",
        "subject_refs":["repo:CAPHTECH/casegraphen"],
        "statement":{"predicate":"must_not_depend_on","object":"agent-runtime"},
        "scope":{"case_space_id":case_space_id,"project_id":"casegraphen","actor_ids":[]},
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
        "case_space_id":case_space_id,
        "source_record":source_record,
        "claim":claim,
        "policy":policy,
        "artifact_path":"memory-sources/adr-0002.txt",
        "target_claim_id":target_cell_id
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
                    "name":"memory_propose_retraction",
                    "arguments":{
                        "request_id":"request:memory-retraction-e2e",
                        "idempotency_key":"idempotency:memory-retraction-e2e",
                        "base_revision_id":revision_id,
                        "payload":{"memory_proposal":proposal}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    let relation_proposal = &result["relation_proposal"];
    assert_eq!(relation_proposal["relation_type"], "retracts");
    assert_eq!(relation_proposal["accepted"], false);
    assert_eq!(relation_proposal["review_status"], "unreviewed");
    assert_eq!(relation_proposal["to_id"], target_cell_id);
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_propose_retraction response failed to validate \
         against control_plane.response.v0"
    );
    assert!(
        validates_against_schema(relation_proposal, "memory.relation_proposal.v0.schema.json"),
        "a real, live MemoryRelationProposal failed to validate against \
         memory.relation_proposal.v0: {relation_proposal}"
    );

    let mut forged_accepted = relation_proposal.clone();
    forged_accepted["accepted"] = json!(true);
    assert!(
        !validates_against_schema(&forged_accepted, "memory.relation_proposal.v0.schema.json"),
        "an accepted: true forgery must fail schema validation"
    );
    let mut forged_status = relation_proposal.clone();
    forged_status["review_status"] = json!("accepted");
    assert!(
        !validates_against_schema(&forged_status, "memory.relation_proposal.v0.schema.json"),
        "a review_status: \"accepted\" forgery must fail schema validation"
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 1: `memory_propose_procedure` shares `memory_proposal_tool`
/// with `memory_propose_claim`, but carries a guard nothing else exercises —
/// the host refuses with `memory_kind_mismatch` when the proposed claim's
/// `memory_kind` isn't `procedure`. That guard exists only in
/// `casegraphen-mcp-host.rs`; this proves it actually fires through the real
/// binary, and that a correctly-kinded claim still proposes cleanly.
#[test]
fn operational_memory_propose_procedure_enforces_its_kind_guard() {
    let directory = temp("memory-procedure");
    let store = directory.join("store");
    fs::create_dir_all(&store).unwrap();
    let create = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args([
            "space",
            "new",
            "--store",
            store.to_str().unwrap(),
            "--case-space-id",
            "case_space:memory-procedure-mcp",
            "--space-id",
            "space:memory-procedure-mcp",
            "--title",
            "Memory procedure MCP fixture",
            "--revision-id",
            "revision:memory-procedure-mcp",
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
    let source_bytes = b"Run `sh scripts/static-analysis.sh` before proposing a change is done.\n";
    fs::write(artifacts.join("memory-sources/procedure.txt"), source_bytes).unwrap();
    let digest = casegraphen::memory::content_hash(source_bytes);
    let policy = json!({
        "schema":"casegraphen.experimental.memory.policy.v0",
        "policy_id":"memory-policy:procedure-mcp",
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
    let source_record = json!({
        "schema":"casegraphen.experimental.memory.source_record.v0",
        "source_record_id":"memory-source:procedure-mcp",
        "source_kind":"document",
        "content_hash":format!("sha256:{digest}"),
        "captured_at":"2026-08-06T00:00:00Z",
        "origin_actor_id":"actor:architecture-reviewer",
        "source_boundary_id":"source_boundary:repository",
        "authority_origin":"reviewer",
        "sensitivity":"internal",
        "artifact_ref":"CLAUDE.md"
    });
    let procedure_claim = json!({
        "schema":"casegraphen.experimental.memory.claim.v0",
        "claim_id":"memory:static-analysis-gate",
        "memory_kind":"procedure",
        "subject_refs":["repo:CAPHTECH/casegraphen"],
        "statement":{"predicate":"run_before_proposing_done","object":"sh scripts/static-analysis.sh"},
        "scope":{"case_space_id":"case_space:memory-procedure-mcp","project_id":"casegraphen","actor_ids":[]},
        "valid_time":{"valid_from":"2026-07-30T00:00:00Z"},
        "source_refs":[format!("artifact:sha256-{digest}")],
        "derivation_actor_id":"actor:memory-proposer",
        "derivation_method":"extraction",
        "model_assertions_are_untrusted":true,
        "provenance_role":"reviewed_architecture_decision",
        "authority_ceiling":"project_constraint",
        "sensitivity":"internal"
    });
    let mut wrong_kind_claim = procedure_claim.clone();
    wrong_kind_claim["claim_id"] = json!("memory:static-analysis-gate-wrong-kind");
    wrong_kind_claim["memory_kind"] = json!("constraint");
    let base_proposal = json!({
        "case_space_id":"case_space:memory-procedure-mcp",
        "source_record":source_record,
        "policy":policy,
        "artifact_path":"memory-sources/procedure.txt"
    });
    let mut correct_proposal = base_proposal.clone();
    correct_proposal["claim"] = procedure_claim;
    let mut wrong_kind_proposal = base_proposal;
    wrong_kind_proposal["claim"] = wrong_kind_claim;

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
                    "name":"memory_propose_procedure",
                    "arguments":{
                        "request_id":"request:memory-procedure-correct",
                        "idempotency_key":"idempotency:memory-procedure-correct",
                        "base_revision_id":"revision:memory-procedure-mcp",
                        "payload":{"memory_proposal":correct_proposal}
                    }
                }),
            ),
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"memory_propose_procedure",
                    "arguments":{
                        "request_id":"request:memory-procedure-wrong-kind",
                        "idempotency_key":"idempotency:memory-procedure-wrong-kind",
                        "base_revision_id":"revision:memory-procedure-mcp",
                        "payload":{"memory_proposal":wrong_kind_proposal}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let correct_result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(correct_result["accepted"], false);
    assert_eq!(correct_result["review_status"], "unreviewed");
    assert_eq!(
        correct_result["claim_proposal"]["claim_cell"]["lifecycle"],
        "proposed"
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_propose_procedure response failed to validate \
         against control_plane.response.v0"
    );

    assert_eq!(responses[2]["result"]["isError"], true, "{responses:?}");
    assert_eq!(
        responses[2]["result"]["structuredContent"]["refusal"]["code"], "memory_kind_mismatch",
        "a non-procedure claim through memory_propose_procedure must be refused \
         with memory_kind_mismatch, not silently accepted or refused for a \
         different reason: {responses:?}"
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 3 (read-only; `query_memory`'s own filtering, including
/// contested-claim handling, is already unit-tested at the library level in
/// `memory_plane.rs` — what's untested here is the host's per-tool flag-
/// setting and output reshaping): `memory_conflicts` forces
/// `include_contested` before calling `query_memory`, then filters the
/// projection down to contested/hard-conflict items. Proves that divergence
/// against `memory_query`'s default (`include_contested:false`) output on
/// the exact same fixture: a claim with a hard `contradicts` relation must be
/// invisible to `memory_query` but present in `memory_conflicts`'s `items`
/// and `contested_claim_ids`.
#[test]
fn operational_memory_conflicts_exposes_what_memory_query_excludes() {
    let directory = temp("memory-conflicts");
    let (store, r0) = memory_e2e_lift(&directory);
    let r1 = memory_e2e_attach_and_accept_claim(
        &directory,
        &store,
        &r0,
        &memory_e2e_claim(
            "memory:uncontested",
            "must_preserve_acceptance_boundary",
            "runtime output remains untrusted",
        ),
        b"CaseGraphen keeps runtime output untrusted by construction.\n",
    );
    let r2 = memory_e2e_attach_and_accept_claim(
        &directory,
        &store,
        &r1,
        &memory_e2e_claim("memory:conflict-a", "must_not_depend_on", "agent-runtime"),
        b"CaseGraphen does not own LLM execution.\n",
    );
    let r3 = memory_e2e_attach_and_accept_claim(
        &directory,
        &store,
        &r2,
        &memory_e2e_claim("memory:conflict-b", "may_depend_on", "agent-runtime"),
        b"CaseGraphen may depend on agent runtime for orchestration.\n",
    );
    let r4 = memory_e2e_add_hard_conflict_relation(
        &directory,
        &store,
        &r3,
        "relation:hard-conflict-e2e",
        "memory:conflict-a",
        "memory:conflict-b",
    );

    let memory_request = memory_e2e_query(&r4, None);
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
                        "request_id":"request:memory-conflicts-query",
                        "idempotency_key":"idempotency:memory-conflicts-query",
                        "base_revision_id":r4,
                        "payload":{"memory_request":memory_request}
                    }
                }),
            ),
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"memory_conflicts",
                    "arguments":{
                        "request_id":"request:memory-conflicts",
                        "idempotency_key":"idempotency:memory-conflicts",
                        "base_revision_id":r4,
                        "payload":{"memory_request":memory_request}
                    }
                }),
            ),
        ],
    );

    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let query_result = &responses[1]["result"]["structuredContent"]["result"];
    let query_ids = query_result["projection"]["selected_claim_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        query_ids.contains(&"memory:uncontested"),
        "memory_query must still surface the uncontested claim: {query_ids:?}"
    );
    assert!(
        !query_ids.contains(&"memory:conflict-a") && !query_ids.contains(&"memory:conflict-b"),
        "memory_query's default include_contested:false must exclude contested \
         claims from selected_claim_ids: {query_ids:?}"
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_query response failed to validate against \
         control_plane.response.v0"
    );

    assert_eq!(responses[2]["result"]["isError"], false, "{responses:?}");
    let conflicts_result = &responses[2]["result"]["structuredContent"]["result"];
    assert_eq!(conflicts_result["read_only"], true);
    assert_eq!(conflicts_result["mutation_performed"], false);
    assert_eq!(conflicts_result["accepted"], false);
    let contested_ids = conflicts_result["contested_claim_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(contested_ids.contains(&"memory:conflict-a"));
    assert!(contested_ids.contains(&"memory:conflict-b"));
    let conflict_item_ids = conflicts_result["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["claim_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        conflict_item_ids.contains(&"memory:conflict-a")
            && conflict_item_ids.contains(&"memory:conflict-b"),
        "memory_conflicts must surface the contested claims memory_query hid: \
         {conflict_item_ids:?}"
    );
    assert!(
        !conflict_item_ids.contains(&"memory:uncontested"),
        "memory_conflicts must filter out the uncontested claim: {conflict_item_ids:?}"
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[2]["result"]["structuredContent"]
        ),
        "a real, live memory_conflicts response failed to validate against \
         control_plane.response.v0"
    );

    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 3: `memory_sources` reshapes the same underlying
/// projection into a distinct output — `source_refs`/`source_records` for one
/// named claim — rather than `memory_query`'s `projection`. Proves the real
/// `SourceRecord` attached to the fixture claim comes back intact through the
/// host, keyed by the claim's own `source_refs`.
#[test]
fn operational_memory_sources_returns_the_real_source_records() {
    let directory = temp("memory-sources");
    let (store, r0) = memory_e2e_lift(&directory);
    let source_bytes = b"CaseGraphen keeps memory sources content-addressed.\n";
    let digest = format!("{:x}", Sha256::digest(source_bytes));
    let r1 = memory_e2e_attach_and_accept_claim(
        &directory,
        &store,
        &r0,
        &memory_e2e_claim(
            "memory:sourced",
            "must_preserve_acceptance_boundary",
            "runtime output remains untrusted",
        ),
        source_bytes,
    );

    let memory_request = memory_e2e_query(&r1, Some("memory:sourced"));
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
                    "name":"memory_sources",
                    "arguments":{
                        "request_id":"request:memory-sources",
                        "idempotency_key":"idempotency:memory-sources",
                        "base_revision_id":r1,
                        "payload":{"memory_request":memory_request}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    let result = &responses[1]["result"]["structuredContent"]["result"];
    assert_eq!(result["claim_id"], "memory:sourced");
    assert_eq!(result["read_only"], true);
    assert_eq!(result["mutation_performed"], false);
    assert_eq!(
        result["source_refs"],
        json!([format!("artifact:sha256-{digest}")])
    );
    let records = result["source_records"].as_array().expect("source_records");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]["source_record_id"],
        "memory-source:memory:sourced"
    );
    assert_eq!(records[0]["content_hash"], format!("sha256:{digest}"));
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_sources response failed to validate against \
         control_plane.response.v0"
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Issue #129, Tier 3: `memory_explain` and `memory_history` are the exact
/// same match arm in `casegraphen-mcp-host.rs`
/// (`MemoryHistory | MemoryExplain => { ... }`) and set the identical query
/// flags. The risk in a shared arm is never that the shared code is wrong —
/// it's that someone splits the arm later and gives the two tools different
/// behaviour without meaning to. A single test per tool with a comment noting
/// "covered by the identical path" would not catch that: the comment doesn't
/// fail when the split happens. Driving both here and asserting their outputs
/// are identical does — splitting the arm without keeping both tools correct
/// breaks this test.
#[test]
fn operational_memory_explain_and_memory_history_produce_the_identical_shape() {
    let directory = temp("memory-explain-history");
    let (store, r0) = memory_e2e_lift(&directory);
    let r1 = memory_e2e_attach_and_accept_claim(
        &directory,
        &store,
        &r0,
        &memory_e2e_claim(
            "memory:explained",
            "must_preserve_acceptance_boundary",
            "runtime output remains untrusted",
        ),
        b"CaseGraphen explains its memory claims from real evidence.\n",
    );

    let memory_request = memory_e2e_query(&r1, Some("memory:explained"));
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
                    "name":"memory_explain",
                    "arguments":{
                        "request_id":"request:memory-explain",
                        "idempotency_key":"idempotency:memory-explain",
                        "base_revision_id":r1,
                        "payload":{"memory_request":memory_request}
                    }
                }),
            ),
            rpc(
                3,
                "tools/call",
                json!({
                    "authorization":"token:surface",
                    "name":"memory_history",
                    "arguments":{
                        "request_id":"request:memory-history",
                        "idempotency_key":"idempotency:memory-history",
                        "base_revision_id":r1,
                        "payload":{"memory_request":memory_request}
                    }
                }),
            ),
        ],
    );
    assert_eq!(responses[1]["result"]["isError"], false, "{responses:?}");
    assert_eq!(responses[2]["result"]["isError"], false, "{responses:?}");
    let explain_result = &responses[1]["result"]["structuredContent"]["result"];
    let history_result = &responses[2]["result"]["structuredContent"]["result"];
    assert_eq!(
        explain_result, history_result,
        "the shared arm must keep both tools' output identical"
    );
    assert_eq!(explain_result["claim_id"], "memory:explained");
    assert!(explain_result["item"].is_object());
    assert_eq!(explain_result["read_only"], true);
    assert_eq!(explain_result["mutation_performed"], false);
    assert_eq!(explain_result["accepted"], false);
    assert!(
        validates_against_control_plane_response_schema(
            &responses[1]["result"]["structuredContent"]
        ),
        "a real, live memory_explain response failed to validate against \
         control_plane.response.v0"
    );
    assert!(
        validates_against_control_plane_response_schema(
            &responses[2]["result"]["structuredContent"]
        ),
        "a real, live memory_history response failed to validate against \
         control_plane.response.v0"
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

/// ADR 0034 / #117 pattern: validate a real, live response against the
/// shipped contract rather than asserting about the schema in the abstract.
fn validates_against_schema(instance: &Value, schema_file: &str) -> bool {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!(
        "casegraphen-product-surface-schema-check-{}-{nonce}.json",
        std::process::id()
    ));
    fs::write(&file, serde_json::to_vec(instance).unwrap()).expect("write instance");
    let status = Command::new("python3")
        .args(["-m", "jsonschema", "-i"])
        .arg(&file)
        .arg(root().join(format!("schemas/experimental/{schema_file}")))
        .status()
        .expect("run python3 -m jsonschema");
    let _ = fs::remove_file(&file);
    status.success()
}

fn validates_against_control_plane_response_schema(instance: &Value) -> bool {
    validates_against_schema(instance, "control_plane.response.v0.schema.json")
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
