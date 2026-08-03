//! Durable, authenticated external host for the CaseGraphen MCP boundary.

use casegraphen::{
    control_plane::{
        ControlPlaneRefusal, ControlPlaneRequest, ControlPlaneTool, DecisionDelegate,
        ResourceDelegate,
    },
    dynamic_expansion::{ExpansionCandidate, ExpansionController, ExpansionPolicy},
    execution_topology::{execution_topology_content_hash, parse_execution_topology},
    graph_compiler::{
        compile_execution_topology, CompilationMode, CompilationTarget, CompilerRequest,
        NodePlanMapping,
    },
    graph_lint::lint_execution_topology,
    graph_simulation::{simulate_execution_topology, GraphSimulationRequest},
    mcp_stdio::{serve_stdio, McpStdioServer},
    native_eval::evaluate_native_case,
    native_store::NativeCaseStore,
    resource_allocator::{AtomicResourceAllocator, ResourceAllocatorConfiguration},
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
};
use higher_graphen_core::Id;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

struct HostConfiguration {
    state_path: PathBuf,
    store_path: PathBuf,
    artifact_path: PathBuf,
    resource_journal_path: PathBuf,
    resource_configuration_path: Option<PathBuf>,
    authorization_token: String,
}

struct OperationalDelegate {
    store_path: PathBuf,
    artifact_path: PathBuf,
    allocator: AtomicResourceAllocator,
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceReservationInput {
    declaration: ResourceDeclaration,
    reservation: ResourceReservation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceDispositionInput {
    assertion: ReservationDispositionAssertion,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceReconciliationInput {
    declaration: ResourceDeclaration,
    reservation: ResourceReservation,
    allocations: Vec<RuntimeResourceAllocation>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpansionEvaluationInput {
    policy: ExpansionPolicy,
    attempt_id: String,
    rounds: Vec<ExpansionRoundStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpansionRoundStep {
    candidates: Vec<ExpansionCandidate>,
    accounted_round_cost: f64,
    accounted_round_latency_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamingRunInput {
    case_space_id: String,
    expectation: RuntimeGraphExpectation,
    #[serde(default)]
    events: Vec<RuntimeStreamEvent>,
    #[serde(default)]
    terminal_reports: Vec<RuntimeNodeReport>,
    #[serde(default)]
    observed_artifact_ids: Vec<String>,
    #[serde(default)]
    resource_expectations: Vec<RuntimeResourceExpectation>,
    runtime_jsonl: String,
    run_closed: bool,
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
                match fs::OpenOptions::new().create_new(true).write(true).open(&path) {
                    Ok(mut file) => {
                        use std::io::Write;
                        file.write_all(record.as_bytes()).map_err(io_refusal)?;
                        file.sync_all().map_err(io_refusal)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let existing = fs::read(&path).map_err(io_refusal)?;
                        if existing != record.as_bytes() {
                            return Err(refusal("artifact_hash_collision", "stored runtime record bytes disagree"));
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
                    request.payload.get("compiler_request").cloned()
                        .ok_or_else(|| refusal("invalid_payload", "payload.compiler_request is required"))?
                ).map_err(|error| refusal("invalid_compiler_request", &error.to_string()))?;
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
                let bundle = compile_execution_topology(&topology, &compiler_request)
                    .map_err(|report| refusal(
                        "compilation_refused",
                        &serde_json::to_string(&report).expect("compiler report serializes"),
                    ))?;
                let bundle_directory = self.artifact_path.join("bundles")
                    .join(bundle.manifest_content_hash.replace(':', "-"));
                fs::create_dir_all(&bundle_directory).map_err(io_refusal)?;
                for artifact in &bundle.artifacts {
                    let relative = safe_relative_path(&artifact.path)?;
                    let output = bundle_directory.join(relative);
                    if let Some(parent) = output.parent() { fs::create_dir_all(parent).map_err(io_refusal)?; }
                    write_exact_content(&output, &artifact.bytes)?;
                }
                write_exact_content(&bundle_directory.join("manifest.json"), &bundle.manifest_bytes)?;
                Ok(json!({
                    "manifest": bundle.manifest,
                    "manifest_content_hash": bundle.manifest_content_hash,
                    "bundle_directory": bundle_directory,
                    "review_status": "unreviewed",
                    "accepted": false
                }))
            }
            ControlPlaneTool::ReconcileRun => {
                let topology = topology_from_payload(&request.payload)?;
                let jsonl = required_string(&request.payload, "runtime_jsonl")?;
                let base_revision_id = request
                    .base_revision_id
                    .as_deref()
                    .ok_or_else(|| refusal("explicit_revision_required", "reconcile_run requires the client-observed revision"))?;
                let mut reconciler = GenericJsonlReconciler::new();
                reconciler.ingest_jsonl(jsonl);
                let report = if let Some(bundle) = request.payload.get("resource_expectation_bundle") {
                    let bundle: ResourceExpectationBundle = serde_json::from_value(bundle.clone())
                        .map_err(|error| refusal("invalid_resource_expectation_bundle", &error.to_string()))?;
                    let expectations = bundle.validate(&topology, base_revision_id)
                        .map_err(|findings| findings_refusal("resource_expectation_bundle_refused", &findings))?;
                    for entry in &bundle.expectations {
                        if !self.allocator.contains_exact_reservation(&entry.declaration, &entry.reservation)
                            .map_err(allocator_refusal)?
                        {
                            return Err(refusal("noncanonical_resource_reservation", "resource expectation does not name an exact allocator journal reservation"));
                        }
                        for assertion in &entry.disposition_evidence {
                            if !self.allocator.contains_disposition(assertion).map_err(allocator_refusal)? {
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
            ControlPlaneTool::ReserveResources => {
                let topology = topology_from_payload(&request.payload)?;
                let input: ResourceReservationInput = payload_value(&request.payload, "resource_request")?;
                let base_revision_id = request.base_revision_id.as_deref().ok_or_else(||
                    refusal("explicit_revision_required", "resource allocation requires a base revision")
                )?;
                let outcome = self.allocator.reserve(
                    &topology,
                    base_revision_id,
                    input.declaration,
                    input.reservation,
                    &request.idempotency_key,
                ).map_err(allocator_refusal)?;
                Ok(json!({
                    "topology_content_hash": execution_topology_content_hash(&topology).expect("typed topology hashes"),
                    "base_revision_id": request.base_revision_id,
                    "allocator_event": outcome.event,
                    "allocator_generation": outcome.snapshot.generation,
                    "active_reservations": outcome.snapshot.active_reservations,
                    "replayed": outcome.replayed,
                    "accepted_runtime_output": false
                }))
            }
            ControlPlaneTool::ReleaseResources => {
                let input: ResourceDispositionInput = payload_value(&request.payload, "resource_disposition")?;
                let base_revision_id = request.base_revision_id.as_deref().ok_or_else(||
                    refusal("explicit_revision_required", "resource disposition requires a base revision")
                )?;
                let outcome = self.allocator.disposition(
                    base_revision_id,
                    input.assertion,
                    &request.idempotency_key,
                ).map_err(allocator_refusal)?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "allocator_event": outcome.event,
                    "allocator_generation": outcome.snapshot.generation,
                    "active_reservations": outcome.snapshot.active_reservations,
                    "replayed": outcome.replayed,
                    "accepted_runtime_output": false
                }))
            }
            ControlPlaneTool::ReconcileResources => {
                let input: ResourceReconciliationInput = payload_value(&request.payload, "resource_reconciliation")?;
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
                let simulation: GraphSimulationRequest = payload_value(&request.payload, "simulation_request")?;
                let report = simulate_execution_topology(&topology, &simulation)
                    .map_err(|findings| findings_refusal("simulation_refused", &findings))?;
                serde_json::to_value(report)
                    .map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ControlPlaneTool::EvaluateExpansionRound => {
                let topology = topology_from_payload(&request.payload)?;
                let input: ExpansionEvaluationInput = payload_value(&request.payload, "expansion_round")?;
                if input.rounds.is_empty() {
                    return Err(refusal(
                        "expansion_refused",
                        "expansion_round.rounds must contain at least one bounded round",
                    ));
                }
                let mut controller = ExpansionController::new(input.policy, &topology)
                    .map_err(|findings| findings_refusal("expansion_refused", &findings))?;
                controller.begin_attempt(&input.attempt_id, &topology)
                    .map_err(|finding| findings_refusal("expansion_refused", &[finding]))?;
                let mut results = Vec::new();
                for round in input.rounds {
                    let result = controller.process_round(
                        &input.attempt_id,
                        round.candidates,
                        round.accounted_round_cost,
                        round.accounted_round_latency_ms,
                    ).map_err(|finding| findings_refusal("expansion_refused", &[finding]))?;
                    let terminal = !matches!(
                        result.halt,
                        casegraphen::dynamic_expansion::ExpansionHalt::Continue
                    );
                    results.push(result);
                    if terminal {
                        break;
                    }
                }
                controller.finish_attempt(&input.attempt_id)
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
                    refusal("explicit_revision_required", "streaming reconciliation requires an exact case revision")
                })?;
                let case_space_id = safe_id(&input.case_space_id)?;
                let replay = NativeCaseStore::new(self.store_path.clone())
                    .replay_current_case_space(&case_space_id).map_err(store_refusal)?;
                if replay.current_revision_id.as_str() != expected_revision {
                    return Err(ControlPlaneRefusal::stale(expected_revision, replay.current_revision_id.to_string()));
                }
                let acceptance = derive_streaming_acceptance(&replay.case_space, &topology)
                    .map_err(|finding| findings_refusal("streaming_acceptance_refused", &[finding]))?;
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
                ).map_err(|findings| findings_refusal("streaming_resource_refused", &findings))?;
                serde_json::to_value(reconcile_stream(StreamingReconciliationInput {
                    topology: &topology,
                    expectation: &input.expectation,
                    events: &input.events,
                    terminal_reports: &input.terminal_reports,
                    observed_artifact_ids: &input.observed_artifact_ids,
                    expected_case_revision_id: expected_revision,
                    resource_permits: Some(&permits),
                    acceptance: Some(&acceptance),
                    run_closed: input.run_closed,
                })).map_err(|error| refusal("serialization_failure", &error.to_string()))
            }
            ControlPlaneTool::ProposeTopologyRedesign => {
                let old = topology_from_payload(&request.payload)?;
                let proposed_json = required_string(&request.payload, "proposed_topology_json")?;
                let proposed = parse_execution_topology(proposed_json)
                    .map_err(|findings| findings_refusal("invalid_proposed_topology", &findings))?;
                let input: RedesignProposalInput = payload_value(&request.payload, "redesign_request")?;
                let proposal = propose_redesign(&old, &proposed, input)
                    .map_err(|findings| findings_refusal("redesign_refused", &findings))?;
                Ok(json!({
                    "base_revision_id": request.base_revision_id,
                    "proposal": proposal,
                    "accepted": false,
                    "review_status": "unreviewed"
                }))
            }
            _ => Err(refusal(
                "unsupported_operational_host_tool",
                "this host release supports topology proposal/lint, content-addressed runtime attachment, and canonical run reconciliation; mutation tools remain delegated to the existing CaseGraphen CLI owner",
            )),
        }
    }
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
    Ok(json!({
        "projection": value,
        "content_hash": format!("sha256:{:x}", Sha256::digest(&bytes)),
        "accepted": false
    }))
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

fn parse_configuration() -> Result<Option<HostConfiguration>, String> {
    let mut args = env::args().skip(1);
    if args.len() == 1 && args.next().as_deref() == Some("--health-check") {
        println!(
            "{}",
            json!({"status":"ok","schedules_agents":false,"calls_models":false,"automatic_retry":false})
        );
        return Ok(None);
    }
    let mut state_path = None;
    let mut store_path = None;
    let mut artifact_path = None;
    let mut resource_journal_path = None;
    let mut resource_configuration_path = None;
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
            "--auth-token-env" => token_env = Some(value),
            _ => return Err(format!("unsupported argument {flag}")),
        }
    }
    let token_env = token_env.ok_or("--auth-token-env is required")?;
    let authorization_token = env::var(&token_env)
        .map_err(|_| format!("authorization token environment variable {token_env} is missing"))?;
    let artifact_path = artifact_path.ok_or("--artifacts is required")?;
    Ok(Some(HostConfiguration {
        state_path: state_path.ok_or("--state is required")?,
        store_path: store_path.ok_or("--store is required")?,
        resource_journal_path: resource_journal_path
            .unwrap_or_else(|| artifact_path.join("resource-allocator-journal")),
        resource_configuration_path,
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
    let delegate = OperationalDelegate {
        store_path: configuration.store_path,
        artifact_path: configuration.artifact_path,
        allocator,
    };
    let mut server = McpStdioServer::new_durable_authenticated(
        delegate,
        configuration.state_path,
        configuration.authorization_token,
    )?;
    serve_stdio(&mut server, io::stdin().lock(), io::stdout().lock())
}
