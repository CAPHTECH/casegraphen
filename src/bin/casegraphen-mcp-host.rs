//! Durable, authenticated external host for the CaseGraphen MCP boundary.

use casegraphen::{
    control_plane::{
        ControlPlaneRefusal, ControlPlaneRequest, ControlPlaneTool, DecisionDelegate,
        ResourceDelegate,
    },
    dynamic_expansion::{ExpansionCandidate, ExpansionController, ExpansionPolicy},
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    graph_compiler::{
        compile_execution_topology, reviewed_compilation_mode, reviewed_deployment_authority,
        verify_deployment_bundle, BundleArtifact, BundleManifest, CompilationMode,
        CompilationTarget, CompilerRequest, DeploymentBundle, NodePlanMapping,
        VerifiedDeploymentBundle,
    },
    graph_lint::lint_execution_topology,
    graph_simulation::{simulate_execution_topology, GraphSimulationRequest},
    mcp_stdio::{serve_stdio, McpStdioServer},
    memory::{
        build_claim_proposal, query_memory, source_records_for_claim, validate_memory_claim,
        validate_memory_policy, validate_memory_proposal, MemoryClaim, MemoryKind, MemoryPolicy,
        MemoryQuery, MemoryRelationKind, MemoryRelationProposal, SourceRecord,
        MEMORY_RELATION_PROPOSAL_SCHEMA,
    },
    native_eval::evaluate_native_case,
    native_model::RelationStrength,
    native_store::NativeCaseStore,
    resource_allocator::{
        validate_resource_allocator_retention_policy, AtomicResourceAllocator,
        ResourceAllocatorConfiguration, ResourceAllocatorRetentionPolicy,
    },
    resource_protocol::{
        reconcile_resource_allocations, ReservationDispositionAssertion, ResourceDeclaration,
        ResourceReservation, RuntimeResourceAllocation,
    },
    runtime_integration::{
        GenericJsonlReconciler, ResourceExpectationBundle, RuntimeResourceExpectation,
    },
    runtime_protocol::{RuntimeGraphExpectation, RuntimeNodeReport},
    streaming_reconciliation::{
        derive_streaming_acceptance, derive_streaming_resource_permits, reconcile_stream,
        RuntimeStreamEvent, StreamingReconciliationInput,
    },
    topology_redesign::{propose_redesign, RedesignProposalInput},
    verification_policy::{
        derive_native_cli_review_verifier_proof, derive_native_cli_run_producer_proof,
        observe_case_artifact, observe_case_execution_trace, reconcile_verification_policy,
        AnchoredExecutionTraceBytes, NativeCliRunLineageDerivation, ToolObservedAnchorProof,
        VerificationPolicy,
    },
};
use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
};

struct HostConfiguration {
    state_path: PathBuf,
    store_path: PathBuf,
    artifact_path: PathBuf,
    resource_journal_path: PathBuf,
    resource_configuration_path: Option<PathBuf>,
    resource_retention_policy_path: Option<PathBuf>,
    resource_checkpoint_interval: Option<u64>,
    authorization_token: String,
}

struct OperationalDelegate {
    store_path: PathBuf,
    artifact_path: PathBuf,
    allocator: AtomicResourceAllocator,
    resource_retention_policy: Option<ResourceAllocatorRetentionPolicy>,
    resource_checkpoint_interval: Option<u64>,
}

impl OperationalDelegate {
    fn maintain_resource_journal(
        &self,
        generation: u64,
        replayed: bool,
    ) -> Result<Value, ControlPlaneRefusal> {
        let (Some(policy), Some(interval)) = (
            self.resource_retention_policy.as_ref(),
            self.resource_checkpoint_interval,
        ) else {
            return Ok(Value::Null);
        };
        if replayed || generation == 0 || generation % interval != 0 {
            return Ok(Value::Null);
        }
        let checkpoint = self
            .allocator
            .create_checkpoint()
            .map_err(allocator_refusal)?;
        let proof = self
            .allocator
            .verify_latest_checkpoint()
            .map_err(allocator_refusal)?;
        let compaction = self
            .allocator
            .compact(policy, &proof)
            .map_err(allocator_refusal)?;
        Ok(json!({
            "checkpoint_content_hash": checkpoint.checkpoint_content_hash,
            "checkpoint_sequence": checkpoint.last_event_sequence,
            "compaction_content_hash": compaction.record.compaction_content_hash,
            "archived_event_count": compaction.archived_event_count,
            "active_event_count": compaction.active_event_count
        }))
    }
}

// Issue #118: every type below is deserialized directly out of a
// `tools/call` payload (see the `payload_value`/`serde_json::from_value`
// call sites that name it) and therefore has a published contract under
// `schemas/experimental/mcp.*.v0.schema.json`. `Serialize` is derived only
// so `tests/experimental_schema_conformance.rs` can round-trip a shipped
// example through the Rust type and re-validate it against that schema; the
// host itself never serializes these. `scripts/experimental-schema-conformance.py
// --check` fails closed if a type here has no matching entry in
// `schemas/experimental/contracts.v0.json`.
/// Contract for [`ProposalCompilerInput`], the `compile_deployment_bundle` payload.
pub const PROPOSAL_COMPILER_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.proposal_compiler_input.v0";
/// Contract for [`ReviewedCompilerInput`], the `compile_reviewed_deployment_bundle` payload.
pub const REVIEWED_COMPILER_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.reviewed_compiler_input.v0";
/// Contract for [`ResourceReservationInput`], the `reserve_resources` payload.
pub const RESOURCE_RESERVATION_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.resource_reservation_input.v0";
/// Contract for [`ResourceDispositionInput`], the `release_resources` payload.
pub const RESOURCE_DISPOSITION_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.resource_disposition_input.v0";
/// Contract for [`ResourceReconciliationInput`], the `reconcile_resources` payload.
pub const RESOURCE_RECONCILIATION_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.resource_reconciliation_input.v0";
/// Contract for [`ExpansionEvaluationInput`], the `evaluate_expansion_round` payload.
pub const EXPANSION_EVALUATION_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.expansion_evaluation_input.v0";
/// Contract for [`StreamingRunInput`], the `reconcile_streaming_run` payload.
pub const STREAMING_RUN_INPUT_SCHEMA: &str = "casegraphen.experimental.mcp.streaming_run_input.v0";
/// Contract for [`VerificationLineageInput`], the `reconcile_verification_lineage` payload.
pub const VERIFICATION_LINEAGE_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.verification_lineage_input.v0";
/// Contract for [`MemoryReadInput`], the `memory_query`/`memory_explain`/
/// `memory_history`/`memory_conflicts`/`memory_sources` payload.
pub const MEMORY_READ_INPUT_SCHEMA: &str = "casegraphen.experimental.mcp.memory_read_input.v0";
/// Contract for [`MemoryProposalInput`], the `memory_propose_claim`/
/// `memory_propose_supersession`/`memory_propose_retraction`/
/// `memory_propose_procedure` payload.
pub const MEMORY_PROPOSAL_INPUT_SCHEMA: &str =
    "casegraphen.experimental.mcp.memory_proposal_input.v0";

/// External input for the `compile_deployment_bundle` tool.
/// Contract: [`PROPOSAL_COMPILER_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProposalCompilerInput {
    case_space_id: String,
    base_revision_id: String,
    plan_id: String,
    node_plan_mappings: Vec<NodePlanMapping>,
    #[serde(default)]
    verification_policies: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    budget_policies: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    expansion_policies: std::collections::BTreeMap<String, Value>,
}

/// External input for the `compile_reviewed_deployment_bundle` tool.
/// Contract: [`REVIEWED_COMPILER_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewedCompilerInput {
    case_space_id: String,
    claim_cell_id: String,
    plan_id: String,
    node_plan_mappings: Vec<NodePlanMapping>,
    #[serde(default)]
    verification_policies: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    budget_policies: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    expansion_policies: std::collections::BTreeMap<String, Value>,
}

/// External input for the `reserve_resources` tool.
/// Contract: [`RESOURCE_RESERVATION_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceReservationInput {
    deployment_authority: ReviewedDeploymentReference,
    declaration: ResourceDeclaration,
    reservation: ResourceReservation,
}

/// Nested only in [`ResourceReservationInput::deployment_authority`]; not an
/// independent contract (see `mcp.resource_reservation_input.v0.schema.json`'s
/// `$defs.reviewed_deployment_reference`).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewedDeploymentReference {
    case_space_id: String,
    claim_cell_id: String,
    deployment_bundle_hash: String,
}

/// External input for the `release_resources` tool.
/// Contract: [`RESOURCE_DISPOSITION_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceDispositionInput {
    assertion: ReservationDispositionAssertion,
}

/// External input for the `reconcile_resources` tool.
/// Contract: [`RESOURCE_RECONCILIATION_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ResourceReconciliationInput {
    declaration: ResourceDeclaration,
    reservation: ResourceReservation,
    allocations: Vec<RuntimeResourceAllocation>,
}

/// External input for the `evaluate_expansion_round` tool.
/// Contract: [`EXPANSION_EVALUATION_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExpansionEvaluationInput {
    policy: ExpansionPolicy,
    attempt_id: String,
    rounds: Vec<ExpansionRoundStep>,
}

/// Nested only in [`ExpansionEvaluationInput::rounds`]; not an independent
/// contract (see `mcp.expansion_evaluation_input.v0.schema.json`'s
/// `$defs.expansion_round_step`).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExpansionRoundStep {
    candidates: Vec<ExpansionCandidate>,
    accounted_round_cost: f64,
    accounted_round_latency_ms: u64,
}

/// External input for the `reconcile_streaming_run` tool.
/// Contract: [`STREAMING_RUN_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StreamingRunInput {
    case_space_id: String,
    expectation: RuntimeGraphExpectation,
    #[serde(default)]
    events: Vec<RuntimeStreamEvent>,
    #[serde(default)]
    terminal_reports: Vec<RuntimeNodeReport>,
    #[serde(default)]
    resource_expectations: Vec<RuntimeResourceExpectation>,
    runtime_jsonl: String,
    run_closed: bool,
}

/// External input for the `reconcile_verification_lineage` tool.
/// Contract: [`VERIFICATION_LINEAGE_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VerificationLineageInput {
    case_space_id: String,
    claim_cell_id: String,
    policy: VerificationPolicy,
    producer_files: RetainedLineageFiles,
    review_morphism_ids: Vec<String>,
    #[serde(default)]
    anchors: Vec<VerificationAnchorInput>,
}

/// Nested only in [`VerificationLineageInput::producer_files`]; not an
/// independent contract (see `mcp.verification_lineage_input.v0.schema.json`'s
/// `$defs.retained_lineage_files`).
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RetainedLineageFiles {
    worker_report_path: String,
    execution_trace_path: String,
    stdout_path: String,
    stderr_path: String,
}

/// Nested only in [`VerificationLineageInput::anchors`]; not an independent
/// contract (see `mcp.verification_lineage_input.v0.schema.json`'s
/// `$defs.verification_anchor_input`). `CaseArtifact`'s meaning — what it
/// simultaneously checks against the ledger, the artifact id, and the hash
/// of the real bytes at `artifact_path` — lives in that `$defs` entry's
/// description, not only in `verification_policy::observe_case_artifact`.
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum VerificationAnchorInput {
    ExecutionTrace {
        anchor_id: String,
    },
    CaseArtifact {
        anchor_id: String,
        artifact_id: String,
        artifact_path: String,
    },
}

/// External input for the `memory_query`/`memory_explain`/`memory_history`/
/// `memory_conflicts`/`memory_sources` tools.
/// Contract: [`MEMORY_READ_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MemoryReadInput {
    case_space_id: String,
    query: MemoryQuery,
    policy: MemoryPolicy,
    #[serde(default)]
    claim_id: Option<String>,
}

/// External input for the `memory_propose_claim`/`memory_propose_supersession`/
/// `memory_propose_retraction`/`memory_propose_procedure` tools.
/// Contract: [`MEMORY_PROPOSAL_INPUT_SCHEMA`].
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MemoryProposalInput {
    case_space_id: String,
    source_record: SourceRecord,
    claim: MemoryClaim,
    policy: MemoryPolicy,
    artifact_path: String,
    #[serde(default)]
    target_claim_id: Option<String>,
}

impl DecisionDelegate for OperationalDelegate {
    fn invoke(&mut self, request: &ControlPlaneRequest) -> Result<Value, ControlPlaneRefusal> {
        match request.tool {
            ControlPlaneTool::LintExecutionTopology
            | ControlPlaneTool::ProposeExecutionTopology => {
                let topology = topology_from_payload(&request.payload)?;
                let lint = lint_execution_topology(&topology);
                Ok(json!({
                    "topology_id": topology.topology_id,
                    "topology_content_hash": execution_topology_content_hash(&topology)
                        .expect("typed topology hashes"),
                    "review_status": "unreviewed",
                    "accepted": false,
                    "lint": lint,
                }))
            }
            ControlPlaneTool::AttachRuntimeReport => {
                let record = required_string(&request.payload, "jsonl_record")?;
                let digest = format!("{:x}", Sha256::digest(record.as_bytes()));
                let directory = self.artifact_path.join("runtime-ingest");
                fs::create_dir_all(&directory).map_err(io_refusal)?;
                let path = directory.join(format!("sha256-{digest}.jsonl"));
                match fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                {
                    Ok(mut file) => {
                        use std::io::Write;
                        file.write_all(record.as_bytes()).map_err(io_refusal)?;
                        file.sync_all().map_err(io_refusal)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let existing = fs::read(&path).map_err(io_refusal)?;
                        if existing != record.as_bytes() {
                            return Err(refusal(
                                "artifact_hash_collision",
                                "stored runtime record bytes disagree",
                            ));
                        }
                    }
                    Err(error) => return Err(io_refusal(error)),
                }
                Ok(json!({
                    "artifact_id": format!("artifact:sha256-{digest}"),
                    "content_hash": format!("sha256:{digest}"),
                    "accepted": false,
                    "review_status": "unreviewed"
                }))
            }
            ControlPlaneTool::CompileDeploymentBundle => {
                let topology = topology_from_payload(&request.payload)?;
                let input: ProposalCompilerInput = serde_json::from_value(
                    request
                        .payload
                        .get("compiler_request")
                        .cloned()
                        .ok_or_else(|| {
                            refusal("invalid_payload", "payload.compiler_request is required")
                        })?,
                )
                .map_err(|error| refusal("invalid_compiler_request", &error.to_string()))?;
                require_matching_revision(request, &input.base_revision_id)?;
                let compiler_request = CompilerRequest {
                    mode: CompilationMode::Proposal,
                    target: CompilationTarget::GenericJsonlV0,
                    case_space_id: input.case_space_id,
                    base_revision_id: input.base_revision_id,
                    plan_id: input.plan_id,
                    node_plan_mappings: input.node_plan_mappings,
                    verification_policies: input.verification_policies,
                    budget_policies: input.budget_policies,
                    expansion_policies: input.expansion_policies,
                };
                let bundle =
                    compile_execution_topology(&topology, &compiler_request).map_err(|report| {
                        refusal(
                            "compilation_refused",
                            &serde_json::to_string(&report).expect("compiler report serializes"),
                        )
                    })?;
                persist_bundle(&self.artifact_path, bundle, false)
            }
            ControlPlaneTool::CompileReviewedDeploymentBundle => {
                let topology = topology_from_payload(&request.payload)?;
                let input: ReviewedCompilerInput =
                    payload_value(&request.payload, "compiler_request")?;
                let expected_revision = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "reviewed compilation requires the client-observed accepted review revision",
                    )
                })?;
                let case_space_id = safe_id(&input.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != expected_revision {
                    return Err(ControlPlaneRefusal::stale(
                        expected_revision,
                        replay.current_revision_id.to_string(),
                    ));
                }
                let mode = reviewed_compilation_mode(&replay.case_space, &input.claim_cell_id)
                    .map_err(|finding| {
                        findings_refusal("reviewed_compilation_authority_refused", &finding)
                    })?;
                let compiler_request = CompilerRequest {
                    mode,
                    target: CompilationTarget::GenericJsonlV0,
                    case_space_id: input.case_space_id,
                    base_revision_id: expected_revision.to_owned(),
                    plan_id: input.plan_id,
                    node_plan_mappings: input.node_plan_mappings,
                    verification_policies: input.verification_policies,
                    budget_policies: input.budget_policies,
                    expansion_policies: input.expansion_policies,
                };
                let bundle =
                    compile_execution_topology(&topology, &compiler_request).map_err(|report| {
                        findings_refusal("reviewed_compilation_refused", report.as_ref())
                    })?;
                persist_bundle(&self.artifact_path, bundle, true)
            }
            ControlPlaneTool::ReconcileRun => {
                let topology = topology_from_payload(&request.payload)?;
                let jsonl = required_string(&request.payload, "runtime_jsonl")?;
                let base_revision_id = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "reconcile_run requires the client-observed revision",
                    )
                })?;
                let mut reconciler = GenericJsonlReconciler::new();
                reconciler.ingest_jsonl(jsonl);
                let report = if let Some(bundle) =
                    request.payload.get("resource_expectation_bundle")
                {
                    let bundle: ResourceExpectationBundle = serde_json::from_value(bundle.clone())
                        .map_err(|error| {
                            refusal("invalid_resource_expectation_bundle", &error.to_string())
                        })?;
                    let expectations =
                        bundle
                            .validate(&topology, base_revision_id)
                            .map_err(|findings| {
                                findings_refusal("resource_expectation_bundle_refused", &findings)
                            })?;
                    for entry in &bundle.expectations {
                        if !self
                            .allocator
                            .contains_exact_reservation(&entry.declaration, &entry.reservation)
                            .map_err(allocator_refusal)?
                        {
                            return Err(refusal("noncanonical_resource_reservation", "resource expectation does not name an exact allocator journal reservation"));
                        }
                        let journaled_authority = self
                            .allocator
                            .reviewed_reservation_binding(&entry.declaration, &entry.reservation)
                            .map_err(allocator_refusal)?;
                        if journaled_authority.as_ref() != entry.reviewed_deployment.as_ref() {
                            return Err(refusal(
                                "noncanonical_reviewed_deployment_reservation",
                                "resource expectation does not retain the allocator journal deployment authority",
                            ));
                        }
                        for assertion in &entry.disposition_evidence {
                            if !self
                                .allocator
                                .contains_disposition(assertion)
                                .map_err(allocator_refusal)?
                            {
                                return Err(refusal("noncanonical_resource_disposition", "resource disposition evidence is absent from allocator journal"));
                            }
                        }
                    }
                    let allocation_jsonl = bundle.allocation_jsonl();
                    if !allocation_jsonl.is_empty() {
                        reconciler.ingest_jsonl(&allocation_jsonl);
                    }
                    reconciler.reconcile_with_resources(&topology, base_revision_id, &expectations)
                } else {
                    reconciler.reconcile(&topology, base_revision_id)
                };
                serde_json::to_value(report)
                    .map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ControlPlaneTool::ReconcileVerificationLineage => {
                let input: VerificationLineageInput =
                    payload_value(&request.payload, "verification_lineage")?;
                let expected_revision = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "verification lineage reconciliation requires the client-observed revision",
                    )
                })?;
                let case_space_id = safe_id(&input.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != expected_revision {
                    return Err(ControlPlaneRefusal::stale(
                        expected_revision,
                        replay.current_revision_id.to_string(),
                    ));
                }

                let report_bytes = read_confined_artifact(
                    &self.artifact_path,
                    &input.producer_files.worker_report_path,
                )?;
                let trace_bytes = read_confined_artifact(
                    &self.artifact_path,
                    &input.producer_files.execution_trace_path,
                )?;
                let stdout_bytes =
                    read_confined_artifact(&self.artifact_path, &input.producer_files.stdout_path)?;
                let stderr_bytes =
                    read_confined_artifact(&self.artifact_path, &input.producer_files.stderr_path)?;
                let producer =
                    derive_native_cli_run_producer_proof(NativeCliRunLineageDerivation {
                        case_space: &replay.case_space,
                        claim_cell_id: &input.claim_cell_id,
                        worker_report_bytes: &report_bytes,
                        execution_trace_bytes: &trace_bytes,
                        stdout_bytes: &stdout_bytes,
                        stderr_bytes: &stderr_bytes,
                    })
                    .map_err(|findings| {
                        findings_refusal("verification_producer_derivation_refused", &findings)
                    })?;

                let mut review_ids = BTreeSet::new();
                let mut verifiers = Vec::with_capacity(input.review_morphism_ids.len());
                for review_morphism_id in &input.review_morphism_ids {
                    if review_morphism_id.is_empty()
                        || !review_ids.insert(review_morphism_id.as_str())
                    {
                        return Err(refusal(
                            "duplicate_or_empty_review_morphism_id",
                            "review morphism ids must be unique and non-empty; one review cannot satisfy multiple quorum slots",
                        ));
                    }
                    verifiers.push(
                        derive_native_cli_review_verifier_proof(
                            &replay.case_space,
                            &producer,
                            review_morphism_id,
                        )
                        .map_err(|findings| {
                            findings_refusal("verification_verifier_derivation_refused", &findings)
                        })?,
                    );
                }

                let mut anchor_ids = BTreeSet::new();
                let mut anchors: Vec<ToolObservedAnchorProof> =
                    Vec::with_capacity(input.anchors.len());
                for anchor in input.anchors {
                    let anchor_id = match &anchor {
                        VerificationAnchorInput::ExecutionTrace { anchor_id }
                        | VerificationAnchorInput::CaseArtifact { anchor_id, .. } => anchor_id,
                    };
                    if anchor_id.is_empty() || !anchor_ids.insert(anchor_id.clone()) {
                        return Err(refusal(
                            "duplicate_or_empty_anchor_id",
                            "tool-observed anchor ids must be unique and non-empty",
                        ));
                    }
                    let proof = match anchor {
                        VerificationAnchorInput::ExecutionTrace { anchor_id } => {
                            observe_case_execution_trace(
                                &replay.case_space,
                                &anchor_id,
                                AnchoredExecutionTraceBytes {
                                    trace: &trace_bytes,
                                    worker_report: &report_bytes,
                                    stdout: &stdout_bytes,
                                    stderr: &stderr_bytes,
                                },
                            )
                        }
                        VerificationAnchorInput::CaseArtifact {
                            anchor_id,
                            artifact_id,
                            artifact_path,
                        } => {
                            let artifact_bytes =
                                read_confined_artifact(&self.artifact_path, &artifact_path)?;
                            observe_case_artifact(
                                &replay.case_space,
                                &anchor_id,
                                &artifact_id,
                                &artifact_bytes,
                            )
                        }
                    }
                    .map_err(|findings| {
                        findings_refusal("verification_anchor_derivation_refused", &findings)
                    })?;
                    anchors.push(proof);
                }

                let result = reconcile_verification_policy(
                    &replay.case_space,
                    &input.policy,
                    &producer,
                    &verifiers,
                    &anchors,
                );
                Ok(json!({
                    "case_space_id": input.case_space_id,
                    "observed_revision_id": replay.current_revision_id,
                    "result": result,
                    "proofs_serialized": false,
                    "read_only": true,
                    "mutation_performed": false,
                    "accepted": false
                }))
            }
            ControlPlaneTool::ReserveResources => {
                let input: ResourceReservationInput =
                    payload_value(&request.payload, "resource_request")?;
                let base_revision_id = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "resource allocation requires a base revision",
                    )
                })?;
                let case_space_id = safe_id(&input.deployment_authority.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != base_revision_id {
                    return Err(ControlPlaneRefusal::stale(
                        base_revision_id,
                        replay.current_revision_id.to_string(),
                    ));
                }
                let bundle = load_verified_bundle(
                    &self.artifact_path,
                    &input.deployment_authority.deployment_bundle_hash,
                )?;
                let authority = reviewed_deployment_authority(
                    &replay.case_space,
                    &input.deployment_authority.claim_cell_id,
                    &bundle,
                )
                .map_err(|finding| {
                    findings_refusal("reviewed_deployment_authority_refused", &finding)
                })?;
                let outcome = self
                    .allocator
                    .reserve_reviewed_bounded(
                        bundle.topology(),
                        &authority,
                        base_revision_id,
                        input.declaration,
                        input.reservation,
                        &request.idempotency_key,
                    )
                    .map_err(allocator_refusal)?;
                let allocator_maintenance =
                    self.maintain_resource_journal(outcome.snapshot.generation, outcome.replayed)?;
                Ok(json!({
                    "topology_content_hash": execution_topology_content_hash(bundle.topology()).expect("typed topology hashes"),
                    "base_revision_id": request.base_revision_id,
                    "allocator_event": outcome.event,
                    "allocator_generation": outcome.snapshot.generation,
                    "active_reservation_count": outcome.snapshot.active_reservation_count,
                    "replayed": outcome.replayed,
                    "allocator_maintenance": allocator_maintenance,
                    "accepted_runtime_output": false
                }))
            }
            ControlPlaneTool::ReleaseResources => {
                let input: ResourceDispositionInput =
                    payload_value(&request.payload, "resource_disposition")?;
                let base_revision_id = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "resource disposition requires a base revision",
                    )
                })?;
                let binding = self
                    .allocator
                    .reviewed_reservation_binding_by_identity(
                        &input.assertion.reservation_id,
                        &input.assertion.attempt_id,
                    )
                    .map_err(allocator_refusal)?
                    .ok_or_else(|| {
                        refusal(
                            "reviewed_deployment_authority_required",
                            "resource disposition target has no reviewed deployment authority",
                        )
                    })?;
                let case_space_id = safe_id(&binding.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != base_revision_id {
                    return Err(ControlPlaneRefusal::stale(
                        base_revision_id,
                        replay.current_revision_id.to_string(),
                    ));
                }
                let outcome = self
                    .allocator
                    .disposition_reviewed_bounded(
                        base_revision_id,
                        input.assertion,
                        &request.idempotency_key,
                    )
                    .map_err(allocator_refusal)?;
                let allocator_maintenance =
                    self.maintain_resource_journal(outcome.snapshot.generation, outcome.replayed)?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "allocator_event": outcome.event,
                    "allocator_generation": outcome.snapshot.generation,
                    "active_reservation_count": outcome.snapshot.active_reservation_count,
                    "replayed": outcome.replayed,
                    "allocator_maintenance": allocator_maintenance,
                    "accepted_runtime_output": false
                }))
            }
            ControlPlaneTool::ReconcileResources => {
                let input: ResourceReconciliationInput =
                    payload_value(&request.payload, "resource_reconciliation")?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "reconciliation": reconcile_resource_allocations(
                    &input.declaration,
                    &input.reservation,
                    &input.allocations,
                    ),
                    "accepted_runtime_output": false
                }))
            }
            ControlPlaneTool::SimulateExecutionTopology => {
                let topology = topology_from_payload(&request.payload)?;
                let simulation: GraphSimulationRequest =
                    payload_value(&request.payload, "simulation_request")?;
                let report = simulate_execution_topology(&topology, &simulation)
                    .map_err(|findings| findings_refusal("simulation_refused", &findings))?;
                serde_json::to_value(report)
                    .map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ControlPlaneTool::EvaluateExpansionRound => {
                let topology = topology_from_payload(&request.payload)?;
                let input: ExpansionEvaluationInput =
                    payload_value(&request.payload, "expansion_round")?;
                if input.rounds.is_empty() {
                    return Err(refusal(
                        "expansion_refused",
                        "expansion_round.rounds must contain at least one bounded round",
                    ));
                }
                let mut controller = ExpansionController::new(input.policy, &topology)
                    .map_err(|findings| findings_refusal("expansion_refused", &findings))?;
                controller
                    .begin_attempt(&input.attempt_id, &topology)
                    .map_err(|finding| findings_refusal("expansion_refused", &[finding]))?;
                let mut results = Vec::new();
                for round in input.rounds {
                    let result = controller
                        .process_round(
                            &input.attempt_id,
                            round.candidates,
                            round.accounted_round_cost,
                            round.accounted_round_latency_ms,
                        )
                        .map_err(|finding| findings_refusal("expansion_refused", &[finding]))?;
                    let terminal = !matches!(
                        result.halt,
                        casegraphen::dynamic_expansion::ExpansionHalt::Continue
                    );
                    results.push(result);
                    if terminal {
                        break;
                    }
                }
                controller
                    .finish_attempt(&input.attempt_id)
                    .map_err(|finding| findings_refusal("expansion_refused", &[finding]))?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "rounds": results,
                    "accepted": false,
                    "review_status": "unreviewed"
                }))
            }
            ControlPlaneTool::ReconcileStreamingRun => {
                let topology = topology_from_payload(&request.payload)?;
                let input: StreamingRunInput = payload_value(&request.payload, "streaming_run")?;
                let expected_revision = request.base_revision_id.as_deref().ok_or_else(|| {
                    refusal(
                        "explicit_revision_required",
                        "streaming reconciliation requires an exact case revision",
                    )
                })?;
                let case_space_id = safe_id(&input.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != expected_revision {
                    return Err(ControlPlaneRefusal::stale(
                        expected_revision,
                        replay.current_revision_id.to_string(),
                    ));
                }
                let acceptance = derive_streaming_acceptance(&replay.case_space, &topology)
                    .map_err(|finding| {
                        findings_refusal("streaming_acceptance_refused", &[finding])
                    })?;
                let mut reconciler = GenericJsonlReconciler::new();
                reconciler.ingest_jsonl(&input.runtime_jsonl);
                let integration = reconciler.reconcile_with_resources(
                    &topology,
                    expected_revision,
                    &input.resource_expectations,
                );
                let permits = derive_streaming_resource_permits(
                    &topology,
                    &input.resource_expectations,
                    &integration,
                    &acceptance,
                )
                .map_err(|findings| findings_refusal("streaming_resource_refused", &findings))?;
                let observed_artifacts = reconciler.artifact_observations();
                serde_json::to_value(reconcile_stream(StreamingReconciliationInput {
                    topology: &topology,
                    expectation: &input.expectation,
                    events: &input.events,
                    terminal_reports: &input.terminal_reports,
                    observed_artifacts: &observed_artifacts,
                    expected_case_revision_id: expected_revision,
                    resource_permits: Some(&permits),
                    acceptance: Some(&acceptance),
                    run_closed: input.run_closed,
                }))
                .map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ControlPlaneTool::ProposeTopologyRedesign => {
                let old = topology_from_payload(&request.payload)?;
                let proposed_json = required_string(&request.payload, "proposed_topology_json")?;
                let proposed = parse_execution_topology(proposed_json)
                    .map_err(|findings| findings_refusal("invalid_proposed_topology", &findings))?;
                let input: RedesignProposalInput =
                    payload_value(&request.payload, "redesign_request")?;
                let proposal = propose_redesign(&old, &proposed, input)
                    .map_err(|findings| findings_refusal("redesign_refused", &findings))?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "proposal": proposal,
                    "accepted": false,
                    "review_status": "unreviewed"
                }))
            }
            ControlPlaneTool::MemoryQuery
            | ControlPlaneTool::MemoryExplain
            | ControlPlaneTool::MemoryHistory
            | ControlPlaneTool::MemoryConflicts
            | ControlPlaneTool::MemorySources => memory_read_tool(self, request),
            ControlPlaneTool::MemoryProposeClaim
            | ControlPlaneTool::MemoryProposeSupersession
            | ControlPlaneTool::MemoryProposeRetraction
            | ControlPlaneTool::MemoryProposeProcedure => memory_proposal_tool(self, request),
            // Naming every remaining variant instead of `_` keeps this match
            // exhaustive: a tool added to `TOOLS` fails to compile here until
            // someone decides what it does, instead of silently landing in
            // this refusal and reading as a deliberate decision it never was.
            ControlPlaneTool::ApplyEvidencePacket
            | ControlPlaneTool::ReviewAccept
            | ControlPlaneTool::ReviewReject
            | ControlPlaneTool::Resume
            | ControlPlaneTool::SupersedeDispatch => {
                Err(unsupported_operational_host_tool_refusal(request.tool))
            }
        }
    }

    /// Publishes the seventeen registered `casegraphen.experimental.mcp.*_input.v0`
    /// contracts this delegate actually deserializes `payload` fields into
    /// (#165). Each `$ref` names the same `..._SCHEMA` constant `invoke`
    /// deserializes against, so the published contract and the enforced one
    /// share one source and cannot drift apart. The remaining eleven tools
    /// (five that always refuse, six that pull fields out of `payload` ad
    /// hoc with no registered type) keep the trait's unconstrained default:
    /// publishing a schema for a shape nothing enforces would be a claim
    /// this delegate cannot back up.
    fn payload_schema(&self, tool: ControlPlaneTool) -> Value {
        match tool {
            ControlPlaneTool::CompileDeploymentBundle => json!({
                "type": "object",
                "required": ["topology_json", "compiler_request"],
                "properties": {
                    "topology_json": topology_json_property(),
                    "compiler_request": {"$ref": PROPOSAL_COMPILER_INPUT_SCHEMA}
                }
            }),
            ControlPlaneTool::CompileReviewedDeploymentBundle => json!({
                "type": "object",
                "required": ["topology_json", "compiler_request"],
                "properties": {
                    "topology_json": topology_json_property(),
                    "compiler_request": {"$ref": REVIEWED_COMPILER_INPUT_SCHEMA}
                }
            }),
            ControlPlaneTool::ReserveResources => json!({
                "type": "object",
                "required": ["resource_request"],
                "properties": {"resource_request": {"$ref": RESOURCE_RESERVATION_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::ReleaseResources => json!({
                "type": "object",
                "required": ["resource_disposition"],
                "properties": {"resource_disposition": {"$ref": RESOURCE_DISPOSITION_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::ReconcileResources => json!({
                "type": "object",
                "required": ["resource_reconciliation"],
                "properties": {"resource_reconciliation": {"$ref": RESOURCE_RECONCILIATION_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::EvaluateExpansionRound => json!({
                "type": "object",
                "required": ["topology_json", "expansion_round"],
                "properties": {
                    "topology_json": topology_json_property(),
                    "expansion_round": {"$ref": EXPANSION_EVALUATION_INPUT_SCHEMA}
                }
            }),
            ControlPlaneTool::ReconcileStreamingRun => json!({
                "type": "object",
                "required": ["topology_json", "streaming_run"],
                "properties": {
                    "topology_json": topology_json_property(),
                    "streaming_run": {"$ref": STREAMING_RUN_INPUT_SCHEMA}
                }
            }),
            ControlPlaneTool::ReconcileVerificationLineage => json!({
                "type": "object",
                "required": ["verification_lineage"],
                "properties": {"verification_lineage": {"$ref": VERIFICATION_LINEAGE_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::MemoryQuery
            | ControlPlaneTool::MemoryExplain
            | ControlPlaneTool::MemoryHistory
            | ControlPlaneTool::MemoryConflicts
            | ControlPlaneTool::MemorySources => json!({
                "type": "object",
                "required": ["memory_request"],
                "properties": {"memory_request": {"$ref": MEMORY_READ_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::MemoryProposeClaim
            | ControlPlaneTool::MemoryProposeSupersession
            | ControlPlaneTool::MemoryProposeRetraction
            | ControlPlaneTool::MemoryProposeProcedure => json!({
                "type": "object",
                "required": ["memory_proposal"],
                "properties": {"memory_proposal": {"$ref": MEMORY_PROPOSAL_INPUT_SCHEMA}}
            }),
            ControlPlaneTool::ProposeExecutionTopology
            | ControlPlaneTool::LintExecutionTopology
            | ControlPlaneTool::AttachRuntimeReport
            | ControlPlaneTool::ReconcileRun
            | ControlPlaneTool::SimulateExecutionTopology
            | ControlPlaneTool::ProposeTopologyRedesign
            | ControlPlaneTool::ApplyEvidencePacket
            | ControlPlaneTool::ReviewAccept
            | ControlPlaneTool::ReviewReject
            | ControlPlaneTool::Resume
            | ControlPlaneTool::SupersedeDispatch => json!({}),
        }
    }
}

/// The `topology_json` property shared by every payload schema below that
/// requires one. Not itself a registered contract: `topology_json` carries
/// JSON text, not a nested object, and `topology_from_payload` parses it with
/// `parse_execution_topology` rather than `serde_json::from_value`, so there
/// is no deserialized Rust type here to own a `..._SCHEMA` constant the
/// `mcp_input_type_without_contract` gate would look for. The referenced
/// schema id documents what the text must parse as without this transport
/// layer validating it as embedded JSON.
fn topology_json_property() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "JSON text parsed as casegraphen.experimental.execution.topology.v0"
    })
}

fn memory_read_tool(
    delegate: &OperationalDelegate,
    request: &ControlPlaneRequest,
) -> Result<Value, ControlPlaneRefusal> {
    let mut input: MemoryReadInput = payload_value(&request.payload, "memory_request")?;
    require_matching_revision(request, &input.query.base_revision_id)?;
    let replay = replay_exact_memory_case(delegate, request, &input.case_space_id)?;
    match request.tool {
        ControlPlaneTool::MemoryHistory | ControlPlaneTool::MemoryExplain => {
            input.query.include_historical = true;
            input.query.include_contested = true;
        }
        ControlPlaneTool::MemoryConflicts => input.query.include_contested = true,
        ControlPlaneTool::MemoryQuery | ControlPlaneTool::MemorySources => {}
        _ => unreachable!("memory read helper called for proposal tool"),
    }
    let projection = query_memory(&replay.case_space, &input.query, &input.policy)
        .map_err(|findings| findings_refusal("memory_query_refused", &findings))?;
    match request.tool {
        ControlPlaneTool::MemoryQuery => Ok(json!({
            "projection": projection,
            "read_only": true,
            "mutation_performed": false,
            "accepted": false
        })),
        ControlPlaneTool::MemoryConflicts => Ok(json!({
            "base_revision_id": projection.base_revision_id,
            "contested_claim_ids": projection.contested_claim_ids,
            "items": projection.items.into_iter().filter(|item| {
                item.hard_conflict
                    || item.status == casegraphen::memory::MemoryStatus::Contested
            }).collect::<Vec<_>>(),
            "losses": projection.losses,
            "read_only": true,
            "mutation_performed": false,
            "accepted": false
        })),
        ControlPlaneTool::MemoryExplain | ControlPlaneTool::MemoryHistory => {
            let claim_id = input.claim_id.as_deref().ok_or_else(|| {
                refusal(
                    "invalid_payload",
                    "memory_request.claim_id is required for explain/history",
                )
            })?;
            Ok(json!({
                "base_revision_id": projection.base_revision_id,
                "claim_id": claim_id,
                "item": projection.items.iter().find(|item| item.claim_id == claim_id),
                "omissions": projection.omissions.iter().filter(|item| item.claim_id == claim_id).collect::<Vec<_>>(),
                "contested": projection.contested_claim_ids.iter().any(|id| id == claim_id),
                "projection_content_hash": projection.projection_content_hash,
                "read_only": true,
                "mutation_performed": false,
                "accepted": false
            }))
        }
        ControlPlaneTool::MemorySources => {
            let claim_id = input.claim_id.as_deref().ok_or_else(|| {
                refusal(
                    "invalid_payload",
                    "memory_request.claim_id is required for sources",
                )
            })?;
            let item = projection
                .items
                .iter()
                .find(|item| item.claim_id == claim_id);
            let source_records = item
                .is_some()
                .then(|| source_records_for_claim(&replay.case_space, claim_id))
                .unwrap_or_default();
            Ok(json!({
                "base_revision_id": projection.base_revision_id,
                "claim_id": claim_id,
                "source_refs": item.map(|item| item.source_refs.clone()).unwrap_or_default(),
                "source_records": source_records,
                "omissions": projection.omissions.iter().filter(|item| item.claim_id == claim_id).collect::<Vec<_>>(),
                "projection_content_hash": projection.projection_content_hash,
                "read_only": true,
                "mutation_performed": false,
                "accepted": false
            }))
        }
        _ => unreachable!("memory read helper called for proposal tool"),
    }
}

fn memory_proposal_tool(
    delegate: &OperationalDelegate,
    request: &ControlPlaneRequest,
) -> Result<Value, ControlPlaneRefusal> {
    let input: MemoryProposalInput = payload_value(&request.payload, "memory_proposal")?;
    let replay = replay_exact_memory_case(delegate, request, &input.case_space_id)?;
    if input.claim.scope.case_space_id.as_deref() != Some(input.case_space_id.as_str()) {
        return Err(refusal(
            "memory_scope_mismatch",
            "memory proposal claim scope must name the replayed CaseSpace",
        ));
    }
    let artifact_bytes = read_confined_artifact(&delegate.artifact_path, &input.artifact_path)?;
    let mut findings = validate_memory_policy(&input.policy);
    findings.extend(validate_memory_claim(&input.claim, Some(&input.policy)));
    findings.extend(validate_memory_proposal(
        &input.source_record,
        &input.claim,
        &artifact_bytes,
    ));
    if request.tool == ControlPlaneTool::MemoryProposeProcedure
        && input.claim.memory_kind != MemoryKind::Procedure
    {
        return Err(refusal(
            "memory_kind_mismatch",
            "memory_propose_procedure requires memory_kind procedure",
        ));
    }
    if !findings.is_empty() {
        return Err(findings_refusal("memory_proposal_refused", &findings));
    }
    let proposal = build_claim_proposal(
        &input.source_record,
        &input.claim,
        &artifact_bytes,
        &replay.case_space.space_id,
    )
    .map_err(|findings| findings_refusal("memory_proposal_refused", &findings))?;

    let relation_proposal = match request.tool {
        ControlPlaneTool::MemoryProposeSupersession | ControlPlaneTool::MemoryProposeRetraction => {
            let target = input.target_claim_id.as_deref().ok_or_else(|| {
                refusal(
                    "invalid_payload",
                    "memory_proposal.target_claim_id is required for relation proposals",
                )
            })?;
            if !replay.case_space.case_cells.iter().any(|cell| {
                cell.id.as_str() == target && cell.metadata.contains_key("memory_claim")
            }) {
                return Err(refusal(
                    "unknown_memory_target",
                    "target claim is absent from the replayed CaseSpace",
                ));
            }
            let (relation_type, relation_type_str) =
                if request.tool == ControlPlaneTool::MemoryProposeSupersession {
                    (MemoryRelationKind::Supersedes, "supersedes")
                } else {
                    (MemoryRelationKind::Retracts, "retracts")
                };
            let material = serde_json::to_vec(&(
                relation_type_str,
                input.claim.claim_id.as_str(),
                target,
                request.base_revision_id.as_deref(),
            ))
            .expect("memory relation material serializes");
            let digest = format!("{:x}", Sha256::digest(&material));
            serde_json::to_value(MemoryRelationProposal {
                schema: MEMORY_RELATION_PROPOSAL_SCHEMA.to_owned(),
                relation_id: format!("memory-relation:{digest}"),
                relation_type,
                relation_strength: RelationStrength::Hard,
                from_id: input.claim.claim_id.clone(),
                to_id: target.to_owned(),
                review_status: "unreviewed".to_owned(),
                accepted: false,
            })
            .expect("typed memory relation proposal serializes")
        }
        ControlPlaneTool::MemoryProposeClaim | ControlPlaneTool::MemoryProposeProcedure => {
            Value::Null
        }
        _ => unreachable!("memory proposal helper called for read tool"),
    };
    Ok(json!({
        "base_revision_id": replay.current_revision_id,
        "claim_proposal": proposal,
        "relation_proposal": relation_proposal,
        "review_status": "unreviewed",
        "accepted": false,
        "mutation_performed": false
    }))
}

fn replay_exact_memory_case(
    delegate: &OperationalDelegate,
    request: &ControlPlaneRequest,
    case_space_id: &str,
) -> Result<casegraphen::native_store::NativeCaseSpaceReplay, ControlPlaneRefusal> {
    let expected_revision = request.base_revision_id.as_deref().ok_or_else(|| {
        refusal(
            "explicit_revision_required",
            "Memory Plane tools require the client-observed revision",
        )
    })?;
    let case_space_id = safe_id(case_space_id)?;
    let replay = NativeCaseStore::new(delegate.store_path.clone())
        .replay_current_case_space(&case_space_id)
        .map_err(store_refusal)?;
    if replay.current_revision_id.as_str() != expected_revision {
        return Err(ControlPlaneRefusal::stale(
            expected_revision,
            replay.current_revision_id.to_string(),
        ));
    }
    Ok(replay)
}

impl ResourceDelegate for OperationalDelegate {
    fn read_resource(&mut self, uri: &str) -> Result<Value, ControlPlaneRefusal> {
        let path = uri
            .strip_prefix("casegraphen://")
            .ok_or_else(|| refusal("unsupported_resource_uri", "expected casegraphen URI"))?;
        let parts = path.split('/').collect::<Vec<_>>();
        match parts.as_slice() {
            ["spaces", case_space_id, "status"]
            | ["spaces", case_space_id, "frontier"]
            | ["spaces", case_space_id, "reviews"] => {
                let id = safe_id(case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&id)
                    .map_err(store_refusal)?;
                let evaluation = evaluate_native_case(&replay.case_space)
                    .map_err(|error| refusal("case_evaluation_refused", &format!("{error:?}")))?;
                match parts[2] {
                    "status" => Ok(json!({
                        "case_space_id": replay.case_space_id,
                        "current_revision_id": replay.current_revision_id,
                        "evaluation": evaluation,
                    })),
                    "frontier" => Ok(json!({
                        "case_space_id": replay.case_space_id,
                        "current_revision_id": replay.current_revision_id,
                        "readiness": evaluation.readiness,
                    })),
                    _ => Ok(json!({
                        "case_space_id": replay.case_space_id,
                        "current_revision_id": replay.current_revision_id,
                        "review_gaps": evaluation.review_gaps,
                        "reviewed_cells": replay.case_space.case_cells.iter()
                            .filter(|cell| cell.provenance.review_status != higher_graphen_core::ReviewStatus::Unreviewed)
                            .collect::<Vec<_>>(),
                    })),
                }
            }
            ["spaces", case_space_id, "revisions", revision_id] => {
                let case_space_id = safe_id(case_space_id)?;
                safe_component(revision_id)?;
                let record = NativeCaseStore::new(self.store_path.clone())
                    .inspect_case_space(&case_space_id)
                    .map_err(store_refusal)?;
                let revision = record
                    .revisions
                    .into_iter()
                    .find(|revision| revision.revision_id.as_str() == *revision_id)
                    .ok_or_else(|| {
                        refusal(
                            "unknown_revision",
                            "revision is not in the case-space history",
                        )
                    })?;
                serde_json::to_value(revision)
                    .map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ["spaces", case_space_id, "halts"] => read_external_projection(
                &self.artifact_path.join("halts"),
                case_space_id,
                "case_space_id",
            ),
            ["runs", run_id] => {
                read_external_projection(&self.artifact_path.join("runs"), run_id, "run_id")
            }
            ["topologies", topology_id] => read_external_projection(
                &self.artifact_path.join("topologies"),
                topology_id,
                "topology_id",
            ),
            _ => Err(refusal(
                "unsupported_resource_uri",
                "resource URI does not match the operational host catalog",
            )),
        }
    }
}

fn topology_from_payload(
    payload: &Value,
) -> Result<casegraphen::execution_topology::ExecutionTopology, ControlPlaneRefusal> {
    parse_execution_topology(required_string(payload, "topology_json")?).map_err(|findings| {
        refusal(
            "invalid_execution_topology",
            &serde_json::to_string(&findings).expect("findings serialize"),
        )
    })
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, ControlPlaneRefusal> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        refusal(
            "invalid_payload",
            &format!("payload.{field} must be a string"),
        )
    })
}

fn payload_value<T: for<'de> Deserialize<'de>>(
    payload: &Value,
    field: &str,
) -> Result<T, ControlPlaneRefusal> {
    serde_json::from_value(
        payload
            .get(field)
            .cloned()
            .ok_or_else(|| refusal("invalid_payload", &format!("payload.{field} is required")))?,
    )
    .map_err(|error| refusal("invalid_payload", &format!("payload.{field}: {error}")))
}

fn require_matching_revision(
    request: &ControlPlaneRequest,
    embedded_revision: &str,
) -> Result<(), ControlPlaneRefusal> {
    match request.base_revision_id.as_deref() {
        Some(revision) if revision == embedded_revision => Ok(()),
        Some(revision) => Err(refusal(
            "revision_binding_mismatch",
            &format!(
                "control-plane base revision {revision:?} differs from embedded revision {embedded_revision:?}"
            ),
        )),
        None => Err(refusal(
            "explicit_revision_required",
            "the client-observed base revision is required",
        )),
    }
}

fn findings_refusal(code: &str, findings: &impl serde::Serialize) -> ControlPlaneRefusal {
    refusal(
        code,
        &serde_json::to_string(findings)
            .unwrap_or_else(|_| "finding serialization failed".to_owned()),
    )
}

fn safe_component(value: &str) -> Result<&str, ControlPlaneRefusal> {
    if value.is_empty()
        || value.contains('/')
        || value == "."
        || value == ".."
        || value.contains('\0')
    {
        Err(refusal(
            "invalid_resource_id",
            "resource identifiers must be one non-traversing path segment",
        ))
    } else {
        Ok(value)
    }
}

fn safe_id(value: &str) -> Result<Id, ControlPlaneRefusal> {
    safe_component(value)?;
    Id::new(value.to_owned()).map_err(|error| refusal("invalid_resource_id", &error.to_string()))
}

/// Contract for [`ResourceProjection`], the `halts`/`runs`/`topologies`
/// `resources/read` shape (#122, ADR 0036).
pub const RESOURCE_PROJECTION_SCHEMA: &str =
    "casegraphen.experimental.control_plane.resource_projection.v0";

/// The `halts`/`runs`/`topologies` `resources/read` shape, constructed at
/// its one site (`read_external_projection`, below). `accepted` is pinned
/// `const: false` and `required` in its schema, #117's pattern: this
/// projection reflects content an external runtime wrote to the artifact
/// directory, and only the canonical review morphism accepts anything.
/// `schema` self-identifies the record because `resources/read` wraps
/// content as an opaque JSON string in `contents[].text` with no envelope of
/// its own (ADR 0034's response envelope covers `tools/call` only) — a
/// consumer parsing that string needs some way to know which contract
/// governs what it just read, and this is that way.
#[derive(Serialize)]
struct ResourceProjection {
    schema: String,
    projection: Value,
    content_hash: String,
    accepted: bool,
}

fn read_external_projection(
    directory: &Path,
    id: &str,
    identity_field: &str,
) -> Result<Value, ControlPlaneRefusal> {
    safe_component(id)?;
    let path = directory.join(format!("{id}.json"));
    let bytes = fs::read(&path).map_err(io_refusal)?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| refusal("invalid_projection", &error.to_string()))?;
    if value.get(identity_field).and_then(Value::as_str) != Some(id) {
        return Err(refusal(
            "projection_identity_mismatch",
            "projection content does not name the requested identity",
        ));
    }
    let projection = ResourceProjection {
        schema: RESOURCE_PROJECTION_SCHEMA.to_owned(),
        content_hash: format!("sha256:{:x}", Sha256::digest(&bytes)),
        projection: value,
        accepted: false,
    };
    serde_json::to_value(projection)
        .map_err(|error| refusal("serialization_failure", &error.to_string()))
}

fn safe_relative_path(value: &str) -> Result<PathBuf, ControlPlaneRefusal> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(refusal(
            "invalid_bundle_path",
            "compiler artifact path must be a normal relative path",
        ));
    }
    Ok(path.to_path_buf())
}

/// Reads only a regular, non-symlink file whose resolved path remains beneath
/// the configured artifact root. The bytes, rather than caller-supplied
/// digests or proof fields, are passed to the canonical lineage constructors.
fn read_confined_artifact(
    artifact_root: &Path,
    value: &str,
) -> Result<Vec<u8>, ControlPlaneRefusal> {
    let relative = safe_relative_path(value)?;
    let canonical_root = fs::canonicalize(artifact_root).map_err(io_refusal)?;
    let candidate = canonical_root.join(relative);
    let metadata = fs::symlink_metadata(&candidate).map_err(io_refusal)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(refusal(
            "invalid_lineage_artifact_file",
            "lineage artifact must be a regular non-symlink file",
        ));
    }
    let canonical_candidate = fs::canonicalize(&candidate).map_err(io_refusal)?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(refusal(
            "lineage_artifact_outside_root",
            "lineage artifact must resolve beneath the configured artifact root",
        ));
    }
    fs::read(canonical_candidate).map_err(io_refusal)
}

fn persist_bundle(
    artifact_root: &Path,
    bundle: DeploymentBundle,
    reviewed_authority: bool,
) -> Result<Value, ControlPlaneRefusal> {
    let bundle_directory = artifact_root
        .join("bundles")
        .join(bundle.manifest_content_hash.replace(':', "-"));
    fs::create_dir_all(&bundle_directory).map_err(io_refusal)?;
    for artifact in &bundle.artifacts {
        let relative = safe_relative_path(&artifact.path)?;
        let output = bundle_directory.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(io_refusal)?;
        }
        write_exact_content(&output, &artifact.bytes)?;
    }
    write_exact_content(
        &bundle_directory.join("manifest.json"),
        &bundle.manifest_bytes,
    )?;
    Ok(json!({
        "manifest": bundle.manifest,
        "manifest_content_hash": bundle.manifest_content_hash,
        "bundle_directory": bundle_directory,
        "deployment_authority": if reviewed_authority { "reviewed" } else { "proposal_only" },
        "generated_plan_review_status": "unreviewed",
        "accepted_runtime_output": false,
        "accepted": false
    }))
}

fn load_verified_bundle(
    artifact_root: &Path,
    manifest_content_hash: &str,
) -> Result<VerifiedDeploymentBundle, ControlPlaneRefusal> {
    if manifest_content_hash.len() != 64
        || !manifest_content_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(refusal(
            "invalid_deployment_bundle_hash",
            "deployment bundle hash must be a lowercase SHA-256 hex digest",
        ));
    }
    let directory = artifact_root.join("bundles").join(manifest_content_hash);
    let manifest_bytes = fs::read(directory.join("manifest.json")).map_err(io_refusal)?;
    let manifest: BundleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| refusal("invalid_deployment_bundle_manifest", &error.to_string()))?;
    let mut artifacts = Vec::with_capacity(manifest.artifacts.len());
    for entry in &manifest.artifacts {
        let relative = safe_relative_path(&entry.path)?;
        let bytes = fs::read(directory.join(relative)).map_err(io_refusal)?;
        artifacts.push(BundleArtifact {
            path: entry.path.clone(),
            content_hash: entry.content_hash.clone(),
            bytes,
        });
    }
    verify_deployment_bundle(
        DeploymentBundle {
            artifacts,
            manifest,
            manifest_bytes,
            manifest_content_hash: manifest_content_hash.to_owned(),
        },
        manifest_content_hash,
    )
    .map_err(|finding| findings_refusal("deployment_bundle_integrity_failure", &finding))
}

fn write_exact_content(path: &Path, bytes: &[u8]) -> Result<(), ControlPlaneRefusal> {
    match fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(io_refusal)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::read(path).map_err(io_refusal)? == bytes {
                Ok(())
            } else {
                Err(refusal(
                    "content_address_collision",
                    "existing bundle path has different bytes",
                ))
            }
        }
        Err(error) => Err(io_refusal(error)),
    }
}

fn store_refusal(error: casegraphen::native_store::NativeStoreError) -> ControlPlaneRefusal {
    refusal("case_store_refusal", &error.to_string())
}

fn io_refusal(error: io::Error) -> ControlPlaneRefusal {
    refusal("host_io_error", &error.to_string())
}

fn allocator_refusal(
    error: casegraphen::resource_allocator::ResourceAllocatorError,
) -> ControlPlaneRefusal {
    refusal("resource_allocator_refused", &error.to_string())
}

fn refusal(code: &str, detail: &str) -> ControlPlaneRefusal {
    ControlPlaneRefusal {
        code: code.to_owned(),
        detail: detail.to_owned(),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "inspect_host_state_and_retry_explicitly".to_owned(),
    }
}

/// Refusal for the five mutation tools this host release never implements
/// (#166). Deliberately not built with the generic `refusal()` above:
/// `inspect_host_state_and_retry_explicitly` is honest advice for the
/// transient/malformed-input refusals `refusal()` covers elsewhere in this
/// file, but these five can never succeed on this host release — mutation
/// is permanently delegated to the CaseGraphen CLI, not merely stalled.
/// Suggesting a retry costs more than the tokens: an agent that believes it
/// spends a round trip on a loop the host already knows is futile, and
/// nothing in the generic wording said the outcome was permanent. This
/// names the CLI command that actually performs the operation instead of
/// suggesting the caller retry the same refusal.
///
/// `suggested_next_operation` stays a populated string rather than becoming
/// optional: nothing in this codebase branches on its value (it is prose
/// for the calling agent, not a machine-actionable directive — no schema
/// pins an enum of legal values, and no client here parses it), so there is
/// no established need to widen the wire contract to make it absent. What
/// was missing was truthful wording, not a different shape.
fn unsupported_operational_host_tool_refusal(tool: ControlPlaneTool) -> ControlPlaneRefusal {
    let (operation, cli_command) = match tool {
        ControlPlaneTool::ApplyEvidencePacket => {
            ("evidence-packet application", "casegraphen packet apply")
        }
        ControlPlaneTool::ReviewAccept => ("review acceptance", "casegraphen review accept"),
        ControlPlaneTool::ReviewReject => ("review rejection", "casegraphen review reject"),
        ControlPlaneTool::Resume => ("dispatch resume", "casegraphen packet resume"),
        ControlPlaneTool::SupersedeDispatch => (
            "superseding a stalled dispatch trace",
            "casegraphen run/operate --supersede-trace",
        ),
        _ => unreachable!(
            "called only for the five host-release-unsupported mutation tools; see invoke()'s match arm"
        ),
    };
    ControlPlaneRefusal {
        code: "unsupported_operational_host_tool".to_owned(),
        detail: format!(
            "{operation} is not implemented by this host release. This is permanent for this \
             release, not a transient condition worth retrying: the operation is delegated to \
             the existing CaseGraphen CLI owner. Run `{cli_command}` instead."
        ),
        supplied_base_revision_id: None,
        current_revision_id: None,
        suggested_next_operation: "operate_via_casegraphen_cli".to_owned(),
    }
}

/// The embedded `cli_usage.txt`'s `casegraphen-mcp-host` line — the same
/// source the `casegraphen` binary's `--help` reads from, so this host's
/// usage can never drift from what that text documents.
const USAGE: &str = include_str!("../cli_usage.txt");

fn parse_configuration() -> Result<Option<HostConfiguration>, String> {
    let mut args = env::args().skip(1);
    if args.len() == 1 && args.next().as_deref() == Some("--health-check") {
        println!(
            "{}",
            json!({"status":"ok","schedules_agents":false,"calls_models":false,"automatic_retry":false})
        );
        return Ok(None);
    }
    let mut args = env::args().skip(1);
    if args.len() == 1 && args.next().as_deref() == Some("--help") {
        let usage = USAGE
            .lines()
            .find(|line| line.trim_start().starts_with("casegraphen-mcp-host "))
            .expect("cli_usage.txt documents casegraphen-mcp-host");
        println!("{}", usage.trim_start());
        return Ok(None);
    }
    let mut state_path = None;
    let mut store_path = None;
    let mut artifact_path = None;
    let mut resource_journal_path = None;
    let mut resource_configuration_path = None;
    let mut resource_retention_policy_path = None;
    let mut resource_checkpoint_interval = None;
    let mut token_env = None;
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--state" => state_path = Some(PathBuf::from(value)),
            "--store" => store_path = Some(PathBuf::from(value)),
            "--artifacts" => artifact_path = Some(PathBuf::from(value)),
            "--resource-journal" => resource_journal_path = Some(PathBuf::from(value)),
            "--resource-capacities" => resource_configuration_path = Some(PathBuf::from(value)),
            "--resource-retention-policy" => {
                resource_retention_policy_path = Some(PathBuf::from(value))
            }
            "--resource-checkpoint-interval" => {
                let interval = value.parse::<u64>().map_err(|_| {
                    "--resource-checkpoint-interval must be a positive integer".to_owned()
                })?;
                if interval == 0 {
                    return Err("--resource-checkpoint-interval must be positive".to_owned());
                }
                resource_checkpoint_interval = Some(interval);
            }
            "--auth-token-env" => token_env = Some(value),
            _ => return Err(format!("unsupported argument {flag}")),
        }
    }
    let token_env = token_env.ok_or("--auth-token-env is required")?;
    let authorization_token = env::var(&token_env)
        .map_err(|_| format!("authorization token environment variable {token_env} is missing"))?;
    let artifact_path = artifact_path.ok_or("--artifacts is required")?;
    if resource_retention_policy_path.is_some() != resource_checkpoint_interval.is_some() {
        return Err("--resource-retention-policy and --resource-checkpoint-interval must be supplied together".to_owned());
    }
    Ok(Some(HostConfiguration {
        state_path: state_path.ok_or("--state is required")?,
        store_path: store_path.ok_or("--store is required")?,
        resource_journal_path: resource_journal_path
            .unwrap_or_else(|| artifact_path.join("resource-allocator-journal")),
        resource_configuration_path,
        resource_retention_policy_path,
        resource_checkpoint_interval,
        artifact_path,
        authorization_token,
    }))
}

fn main() -> io::Result<()> {
    let Some(configuration) = parse_configuration()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
    else {
        return Ok(());
    };
    fs::create_dir_all(&configuration.artifact_path)?;
    let resource_configuration = match &configuration.resource_configuration_path {
        Some(path) => serde_json::from_slice::<ResourceAllocatorConfiguration>(&fs::read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        None => ResourceAllocatorConfiguration {
            schema: casegraphen::resource_allocator::RESOURCE_ALLOCATOR_CONFIGURATION_SCHEMA
                .to_owned(),
            schema_version: 0,
            capacities: Vec::new(),
        },
    };
    let allocator =
        AtomicResourceAllocator::new(configuration.resource_journal_path, resource_configuration)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let resource_retention_policy = match &configuration.resource_retention_policy_path {
        Some(path) => {
            let policy =
                serde_json::from_slice::<ResourceAllocatorRetentionPolicy>(&fs::read(path)?)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            validate_resource_allocator_retention_policy(&policy)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
            Some(policy)
        }
        None => None,
    };
    let delegate = OperationalDelegate {
        store_path: configuration.store_path,
        artifact_path: configuration.artifact_path,
        allocator,
        resource_retention_policy,
        resource_checkpoint_interval: configuration.resource_checkpoint_interval,
    };
    let mut server = McpStdioServer::new_durable_authenticated(
        delegate,
        configuration.state_path,
        configuration.authorization_token,
    )?;
    serve_stdio(&mut server, io::stdin().lock(), io::stdout().lock())
}

/// Issue #118: proves every external MCP input type still round-trips
/// through, and validates against, its shipped `schemas/experimental/mcp.*`
/// contract. These types live in this binary crate, not the `casegraphen`
/// library, so they cannot be reached from `tests/experimental_schema_conformance.rs`
/// the way every other Rust-owned experimental contract is proven; this
/// module is the equivalent gate for this crate target. The shared
/// inventory/reference/gate checks (does every type here have a registered
/// contract at all) still live in `scripts/experimental-schema-conformance.py
/// --check`, run from `tests/experimental_schema_conformance.rs`.
#[cfg(test)]
mod mcp_input_contract_tests {
    use super::*;
    use std::{path::Path, process::Command};

    fn roundtrip<T: serde::de::DeserializeOwned + Serialize>(example_file: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("schemas/experimental")
            .join(example_file);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let typed: T = serde_json::from_str(&source).unwrap_or_else(|error| {
            panic!("deserialize {} into Rust owner: {error}", path.display())
        });
        serde_json::to_value(&typed).expect("Rust owner serializes")
    }

    #[test]
    fn every_external_mcp_input_type_round_trips_against_its_shipped_schema() {
        let instances = vec![
            json!({"schema_id": PROPOSAL_COMPILER_INPUT_SCHEMA, "instance": roundtrip::<ProposalCompilerInput>("mcp.proposal_compiler_input.v0.example.json")}),
            json!({"schema_id": REVIEWED_COMPILER_INPUT_SCHEMA, "instance": roundtrip::<ReviewedCompilerInput>("mcp.reviewed_compiler_input.v0.example.json")}),
            json!({"schema_id": RESOURCE_RESERVATION_INPUT_SCHEMA, "instance": roundtrip::<ResourceReservationInput>("mcp.resource_reservation_input.v0.example.json")}),
            json!({"schema_id": RESOURCE_DISPOSITION_INPUT_SCHEMA, "instance": roundtrip::<ResourceDispositionInput>("mcp.resource_disposition_input.v0.example.json")}),
            json!({"schema_id": RESOURCE_RECONCILIATION_INPUT_SCHEMA, "instance": roundtrip::<ResourceReconciliationInput>("mcp.resource_reconciliation_input.v0.example.json")}),
            json!({"schema_id": EXPANSION_EVALUATION_INPUT_SCHEMA, "instance": roundtrip::<ExpansionEvaluationInput>("mcp.expansion_evaluation_input.v0.example.json")}),
            json!({"schema_id": STREAMING_RUN_INPUT_SCHEMA, "instance": roundtrip::<StreamingRunInput>("mcp.streaming_run_input.v0.example.json")}),
            json!({"schema_id": VERIFICATION_LINEAGE_INPUT_SCHEMA, "instance": roundtrip::<VerificationLineageInput>("mcp.verification_lineage_input.v0.example.json")}),
            json!({"schema_id": MEMORY_READ_INPUT_SCHEMA, "instance": roundtrip::<MemoryReadInput>("mcp.memory_read_input.v0.example.json")}),
            json!({"schema_id": MEMORY_PROPOSAL_INPUT_SCHEMA, "instance": roundtrip::<MemoryProposalInput>("mcp.memory_proposal_input.v0.example.json")}),
        ];
        let bundle = std::env::temp_dir().join(format!(
            "casegraphen-mcp-host-schema-instances-{}.json",
            std::process::id()
        ));
        fs::write(&bundle, serde_json::to_vec(&instances).unwrap()).expect("write instance bundle");
        let status = Command::new("python3")
            .arg("scripts/experimental-schema-conformance.py")
            .arg("--instances")
            .arg(&bundle)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("validate Rust serialization against JSON Schema");
        let _ = fs::remove_file(&bundle);
        assert!(
            status.success(),
            "Rust serialization did not match shipped JSON Schema"
        );
    }
}
