use super::{
    append_validated_morphism,
    binding::{binding_path, read_registered_worker_binding},
    case_reason,
    io::{provenance, timestamp, write_json},
    plan::{plan_path, read_stored_plan},
    report, require_current_revision, NativeCliError, NativeReasonSection, NativeRunStepOptions,
};
use crate::{
    exec::{
        accepted_plan_content_hash_matches,
        binding::{worker_binding_content_hash, WorkerKind},
        records::{
            ExecutionInformationLoss, ExecutionObstruction, ExecutionTrace, WorkerOutput,
            WorkerOutputName, WorkerReport, EXECUTION_RECORD_SCHEMA_VERSION,
            EXECUTION_TRACE_SCHEMA, WORKER_REPORT_SCHEMA, WORKER_REPORT_TRUST_BOUNDARY,
        },
        transition_permitted,
        worker::{execute_worker, WorkerContext},
        ExecutionPlan, ExecutionStep,
    },
    native_eval::evaluate_native_case,
    native_model::{
        CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType, CaseRelation,
        CaseRelationType, CaseSpace, MorphismLogEntry, MorphismPayload, RelationStrength,
    },
    native_review::{check_operation_gate, NativeOperationGate},
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus, SourceKind};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::super::path_helpers::path_segment;

const RUN_DIRECTORY: &str = "runs";

pub(in crate::native_cli) fn run_step(
    store: &Path,
    options: NativeRunStepOptions<'_>,
) -> Result<Value, NativeCliError> {
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let replay = store_api.replay_current_case_space(options.case_space_id)?;
    require_current_revision(&replay.current_revision_id, options.base_revision_id)?;

    let plan = read_stored_plan(&plan_path(store, options.plan_id), options.plan_id)?;
    verify_accepted_plan(&plan, &replay.case_space)?;

    let gate = NativeOperationGate {
        actor_id: options.gate_options.actor_id.clone(),
        operation: "dispatch".to_owned(),
        operation_scope_id: options.gate_options.operation_scope_id.clone(),
        audience: options.gate_options.audience,
        capability_ids: options.gate_options.capability_ids.clone(),
        source_boundary_id: options.gate_options.source_boundary_id.clone(),
    };
    if let Err(error) = check_operation_gate(&replay.case_space, &gate, "dispatch") {
        return Ok(no_dispatchable_report(
            vec![ExecutionObstruction {
                obstruction_type: "operation_gate_rejected".to_owned(),
                summary: error.to_string(),
                witness_ids: vec![gate.actor_id],
            }],
            Vec::new(),
        ));
    }

    let evaluation = evaluate_native_case(&replay.case_space)?;
    let traces = read_execution_traces(store)?;
    let selection = select_step(
        &plan,
        &replay.case_space,
        &evaluation.frontier_cell_ids,
        &traces,
        options.retry_step_id,
    );
    let Some(step_index) = selection.step_index else {
        return Ok(no_dispatchable_report(
            selection.obstructions,
            selection.step_reasons,
        ));
    };
    let step = &plan.steps[step_index];
    let trace_identity = trace_identity(store, &plan, step, &traces)?;
    let trace_started_at = timestamp();

    let expected_binding_hash = expected_binding_hash(&plan, &step.worker_binding_id);
    let binding_path = binding_path(store, &step.worker_binding_id);
    let actual_binding_hash = binding_file_content_hash(&binding_path)?;
    let trace_binding_hash = actual_binding_hash
        .clone()
        .or_else(|| expected_binding_hash.clone())
        .unwrap_or_else(empty_content_hash);
    let binding_hash_matches = matches!(
        (&actual_binding_hash, &expected_binding_hash),
        (Some(actual), Some(expected)) if actual == expected
    );
    if !binding_hash_matches {
        let summary = format!(
            "worker binding {} content hash does not match the hash accepted with plan {}",
            step.worker_binding_id, plan.plan_id
        );
        let obstruction_type = if actual_binding_hash.is_none() {
            "binding_not_registered"
        } else {
            "binding_hash_mismatch"
        };
        let trace = write_pre_dispatch_trace(PreDispatchTraceInput {
            case_space: &replay.case_space,
            plan: &plan,
            step,
            base_revision_id: options.base_revision_id,
            identity: &trace_identity,
            binding_content_hash: trace_binding_hash,
            started_at: trace_started_at,
            obstruction: ExecutionObstruction {
                obstruction_type: obstruction_type.to_owned(),
                summary,
                witness_ids: vec![step.worker_binding_id.clone()],
            },
        })?;
        return Ok(run_report(
            "no_dispatchable_step",
            Some(trace),
            None,
            selection.step_reasons,
        ));
    }

    let binding = read_registered_worker_binding(store, &step.worker_binding_id)?;
    let binding_content_hash = worker_binding_content_hash(&binding)?;
    if binding.worker_kind == WorkerKind::Shell
        && !options
            .enabled_worker_kinds
            .iter()
            .any(|kind| kind == "shell")
    {
        return Err(NativeCliError::invalid(
            "shell worker kind is disabled by default; pass --enable-worker shell",
        ));
    }

    fs::create_dir_all(&trace_identity.run_directory).map_err(|source| NativeCliError::Io {
        path: trace_identity.run_directory.clone(),
        source,
    })?;
    let input_report_path = trace_identity.run_directory.join("input.report.json");
    let input_report = case_reason(store, options.case_space_id, NativeReasonSection::Reason)?;
    write_json(&input_report_path, &input_report)?;

    let invocation = execute_worker(
        &binding,
        &WorkerContext {
            run_directory: trace_identity.run_directory.clone(),
            input_report_path: input_report_path.clone(),
            case_space_id: options.case_space_id.clone(),
            plan_id: plan.plan_id.clone(),
            step_id: step.step_id.clone(),
            work_cell_id: step.work_cell_id.clone(),
        },
    )?;
    write_bytes(
        &trace_identity.run_directory.join("stdout"),
        &invocation.stdout,
    )?;
    write_bytes(
        &trace_identity.run_directory.join("stderr"),
        &invocation.stderr,
    )?;
    let worker_report = worker_report(
        &plan,
        step,
        &trace_identity,
        &binding_content_hash,
        &input_report_path,
        &invocation,
    );
    write_json(
        &trace_identity.run_directory.join("worker.report.json"),
        &serde_json::to_value(&worker_report)?,
    )?;

    let mut obstructions = missing_evidence_requirement_obstructions(&replay.case_space, step);
    let existing_requirement_ids = existing_requirement_ids(&replay.case_space, step);
    let evidence_morphism = evidence_morphism(
        &replay.case_space,
        &plan,
        step,
        &trace_identity,
        &worker_report,
        &existing_requirement_ids,
    )?;
    let evidence_report = append_validated_morphism(
        &store_api,
        &replay.case_space,
        evidence_morphism,
        Some(options.actor_id.clone()),
        "casegraphen run --step evidence attach",
    )?;
    let evidence_entry = report_entry(&evidence_report)?;
    let evidence_cell_id = evidence_entry
        .morphism
        .evidence_ids
        .first()
        .cloned()
        .ok_or_else(|| NativeCliError::invalid("worker evidence morphism has no evidence id"))?;
    let mut appended_entry_ids = vec![evidence_entry.entry_id.clone()];
    let mut result_revision_id = Some(evidence_entry.target_revision_id.clone());
    let mut transition_applied = false;
    let status;

    if invocation.exit_status == Some(0) && !invocation.timed_out {
        let post_evidence = store_api.replay_current_case_space(options.case_space_id)?;
        let mut transition = transition_morphism(
            &post_evidence.case_space,
            &plan,
            step,
            &trace_identity,
            &evidence_cell_id,
        )?;
        let permitted = step
            .allowed_transition_classes
            .iter()
            .any(|allowed| transition_permitted(allowed, &transition, &post_evidence.case_space));
        if permitted {
            transition.review_status = ReviewStatus::Accepted;
            let transition_report = append_validated_morphism(
                &store_api,
                &post_evidence.case_space,
                transition,
                Some(options.actor_id.clone()),
                "casegraphen run --step transition",
            )?;
            let transition_entry = report_entry(&transition_report)?;
            appended_entry_ids.push(transition_entry.entry_id);
            result_revision_id = Some(transition_entry.target_revision_id);
            transition_applied = true;
            status = "step_executed";
        } else {
            write_json(
                &trace_identity.run_directory.join("proposed.morphism.json"),
                &serde_json::to_value(&transition)?,
            )?;
            obstructions.push(ExecutionObstruction {
                obstruction_type: "transition_not_authorized".to_owned(),
                summary: format!(
                    "accepted plan {} does not authorize the proposed transition for step {}",
                    plan.plan_id, step.step_id
                ),
                witness_ids: vec![plan.plan_id.clone(), step.step_id.clone()],
            });
            status = "transition_not_authorized";
        }
    } else {
        obstructions.push(ExecutionObstruction {
            obstruction_type: "worker_execution_failed".to_owned(),
            summary: format!(
                "worker {} exited with {:?}; timed_out={}",
                binding.binding_id, invocation.exit_status, invocation.timed_out
            ),
            witness_ids: vec![evidence_cell_id],
        });
        status = "step_failed";
    }

    let trace = ExecutionTrace {
        schema: EXECUTION_TRACE_SCHEMA.to_owned(),
        schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
        trace_id: trace_identity.trace_id,
        plan_id: plan.plan_id,
        step_id: step.step_id.clone(),
        case_space_id: options.case_space_id.clone(),
        base_revision_id: options.base_revision_id.clone(),
        result_revision_id,
        work_cell_id: step.work_cell_id.clone(),
        binding_id: binding.binding_id,
        binding_content_hash,
        worker_report_id: worker_report.report_id.clone(),
        appended_entry_ids,
        transition_applied,
        obstructions,
        information_loss: vec![ExecutionInformationLoss {
            description:
                "The worker received a derived reason report rather than the raw case space."
                    .to_owned(),
            represented_ids: vec![options.base_revision_id.clone()],
            omitted_ids: vec![options.case_space_id.clone()],
        }],
        started_at: trace_started_at,
        finished_at: timestamp(),
        metadata: Map::from_iter([("worker_invoked".to_owned(), Value::Bool(true))]),
    };
    write_trace(&trace_identity.run_directory, &trace)?;
    Ok(run_report(
        status,
        Some(trace),
        Some(&worker_report),
        selection.step_reasons,
    ))
}

fn verify_accepted_plan(
    plan: &ExecutionPlan,
    case_space: &CaseSpace,
) -> Result<(), NativeCliError> {
    if plan.review_status != ReviewStatus::Accepted {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} is not accepted",
            plan.plan_id
        )));
    }
    let recorded_hash = case_space
        .morphism_log
        .iter()
        .rev()
        .find_map(|entry| {
            let metadata = &entry.morphism.metadata;
            (metadata.get("target_kind").and_then(Value::as_str) == Some("plan")
                && metadata.get("target_id").and_then(Value::as_str) == Some(plan.plan_id.as_str())
                && metadata.get("action").and_then(Value::as_str) == Some("accept"))
            .then(|| metadata.get("plan_content_hash").and_then(Value::as_str))
            .flatten()
        })
        .ok_or_else(|| {
            NativeCliError::invalid(format!(
                "accepted plan {} has no acceptance morphism with a plan_content_hash",
                plan.plan_id
            ))
        })?;
    if !accepted_plan_content_hash_matches(plan, recorded_hash)? {
        return Err(NativeCliError::invalid(format!(
            "accepted plan {} checksum mismatch: stored plan content no longer matches {recorded_hash}",
            plan.plan_id
        )));
    }
    Ok(())
}

struct StepSelection {
    step_index: Option<usize>,
    step_reasons: Vec<Value>,
    obstructions: Vec<ExecutionObstruction>,
}

fn select_step(
    plan: &ExecutionPlan,
    case_space: &CaseSpace,
    frontier_cell_ids: &[Id],
    traces: &[ExecutionTrace],
    retry_step_id: Option<&Id>,
) -> StepSelection {
    let frontier = frontier_cell_ids.iter().collect::<BTreeSet<_>>();
    let mut selected = None;
    let mut step_reasons = Vec::new();
    let mut obstructions = Vec::new();
    for (index, step) in plan.steps.iter().enumerate() {
        let mut reasons = Vec::new();
        let prior_applied = traces.iter().any(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == step.step_id
                && trace.transition_applied
        });
        let prior_failed = traces.iter().any(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == step.step_id
                && !trace.transition_applied
        });
        if prior_applied {
            reasons.push("already_executed");
        }
        if prior_failed && retry_step_id != Some(&step.step_id) {
            reasons.push("prior_failed_trace_requires_retry");
            obstructions.push(ExecutionObstruction {
                obstruction_type: "retry_required".to_owned(),
                summary: format!(
                    "step {} has a failed execution trace; pass --retry-step {} to retry it",
                    step.step_id, step.step_id
                ),
                witness_ids: vec![step.step_id.clone()],
            });
        }
        if !frontier.contains(&step.work_cell_id) {
            reasons.push("work_cell_not_on_frontier");
        }
        match case_space
            .case_cells
            .iter()
            .find(|cell| cell.id == step.work_cell_id)
        {
            Some(cell) if cell.lifecycle != CaseCellLifecycle::Active => {
                reasons.push("work_cell_lifecycle_not_active");
            }
            Some(_) => {}
            None => reasons.push("work_cell_missing"),
        }
        let eligible = reasons.is_empty();
        if selected.is_none() && eligible {
            selected = Some(index);
        }
        step_reasons.push(json!({
            "step_id": step.step_id,
            "work_cell_id": step.work_cell_id,
            "eligible": eligible,
            "reasons": reasons,
        }));
    }
    StepSelection {
        step_index: selected,
        step_reasons,
        obstructions,
    }
}

fn read_execution_traces(store: &Path) -> Result<Vec<ExecutionTrace>, NativeCliError> {
    let root = store.join(RUN_DIRECTORY);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(NativeCliError::Io { path: root, source }),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| NativeCliError::Io {
            path: root.clone(),
            source,
        })?;
        let path = entry.path().join("execution.trace.json");
        if path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).map_err(|source| NativeCliError::Io {
                path: path.clone(),
                source,
            })?;
            serde_json::from_slice(&bytes).map_err(NativeCliError::from)
        })
        .collect()
}

struct TraceIdentity {
    trace_id: Id,
    worker_report_id: Id,
    run_directory: PathBuf,
}

fn trace_identity(
    store: &Path,
    plan: &ExecutionPlan,
    step: &ExecutionStep,
    traces: &[ExecutionTrace],
) -> Result<TraceIdentity, NativeCliError> {
    let attempt = traces
        .iter()
        .filter(|trace| trace.plan_id == plan.plan_id && trace.step_id == step.step_id)
        .count()
        + 1;
    let trace_id = Id::new(format!(
        "execution_trace:{}:{}:{attempt}",
        path_segment(&plan.plan_id),
        path_segment(&step.step_id)
    ))?;
    let worker_report_id = Id::new(format!("worker_report:{}", path_segment(&trace_id)))?;
    let run_directory = store.join(RUN_DIRECTORY).join(path_segment(&trace_id));
    Ok(TraceIdentity {
        trace_id,
        worker_report_id,
        run_directory,
    })
}

fn expected_binding_hash(plan: &ExecutionPlan, binding_id: &Id) -> Option<String> {
    plan.metadata
        .get("worker_binding_hashes")
        .and_then(Value::as_object)
        .and_then(|hashes| hashes.get(binding_id.as_str()))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn binding_file_content_hash(path: &Path) -> Result<Option<String>, NativeCliError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(NativeCliError::Io {
                path: path.to_owned(),
                source,
            })
        }
    };
    let canonical = match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => serde_json::to_string(&value)?.into_bytes(),
        Err(_) => bytes,
    };
    Ok(Some(crate::native_hash::sha256_hex(&canonical)))
}

fn empty_content_hash() -> String {
    crate::native_hash::sha256_hex(&[])
}

struct PreDispatchTraceInput<'a> {
    case_space: &'a CaseSpace,
    plan: &'a ExecutionPlan,
    step: &'a ExecutionStep,
    base_revision_id: &'a Id,
    identity: &'a TraceIdentity,
    binding_content_hash: String,
    started_at: String,
    obstruction: ExecutionObstruction,
}

fn write_pre_dispatch_trace(
    input: PreDispatchTraceInput<'_>,
) -> Result<ExecutionTrace, NativeCliError> {
    fs::create_dir_all(&input.identity.run_directory).map_err(|source| NativeCliError::Io {
        path: input.identity.run_directory.clone(),
        source,
    })?;
    let trace = ExecutionTrace {
        schema: EXECUTION_TRACE_SCHEMA.to_owned(),
        schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
        trace_id: input.identity.trace_id.clone(),
        plan_id: input.plan.plan_id.clone(),
        step_id: input.step.step_id.clone(),
        case_space_id: input.case_space.case_space_id.clone(),
        base_revision_id: input.base_revision_id.clone(),
        result_revision_id: None,
        work_cell_id: input.step.work_cell_id.clone(),
        binding_id: input.step.worker_binding_id.clone(),
        binding_content_hash: input.binding_content_hash,
        worker_report_id: input.identity.worker_report_id.clone(),
        appended_entry_ids: Vec::new(),
        transition_applied: false,
        obstructions: vec![input.obstruction],
        information_loss: Vec::new(),
        started_at: input.started_at,
        finished_at: timestamp(),
        metadata: Map::from_iter([("worker_invoked".to_owned(), Value::Bool(false))]),
    };
    write_trace(&input.identity.run_directory, &trace)?;
    Ok(trace)
}

fn worker_report(
    plan: &ExecutionPlan,
    step: &ExecutionStep,
    identity: &TraceIdentity,
    binding_content_hash: &str,
    input_report_path: &Path,
    invocation: &crate::exec::worker::WorkerInvocation,
) -> WorkerReport {
    WorkerReport {
        schema: WORKER_REPORT_SCHEMA.to_owned(),
        schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
        report_id: identity.worker_report_id.clone(),
        binding_id: step.worker_binding_id.clone(),
        binding_content_hash: binding_content_hash.to_owned(),
        work_cell_id: step.work_cell_id.clone(),
        plan_id: plan.plan_id.clone(),
        step_id: step.step_id.clone(),
        exit_status: invocation.exit_status,
        timed_out: invocation.timed_out,
        outputs: vec![
            WorkerOutput {
                name: WorkerOutputName::Stdout,
                content_hash: invocation.stdout_sha256.clone(),
                byte_len: u64::try_from(invocation.stdout.len()).unwrap_or(u64::MAX),
                truncated: invocation.stdout_truncated,
            },
            WorkerOutput {
                name: WorkerOutputName::Stderr,
                content_hash: invocation.stderr_sha256.clone(),
                byte_len: u64::try_from(invocation.stderr.len()).unwrap_or(u64::MAX),
                truncated: invocation.stderr_truncated,
            },
        ],
        trust_boundary: WORKER_REPORT_TRUST_BOUNDARY.to_owned(),
        started_at: invocation.started_at.clone(),
        finished_at: invocation.finished_at.clone(),
        metadata: Map::from_iter([(
            "input_report_path".to_owned(),
            Value::String(input_report_path.display().to_string()),
        )]),
    }
}

fn existing_requirement_ids(case_space: &CaseSpace, step: &ExecutionStep) -> Vec<Id> {
    let cell_ids = case_space
        .case_cells
        .iter()
        .map(|cell| &cell.id)
        .collect::<BTreeSet<_>>();
    step.success_evidence_requirement_ids
        .iter()
        .filter(|id| cell_ids.contains(id))
        .cloned()
        .collect()
}

fn missing_evidence_requirement_obstructions(
    case_space: &CaseSpace,
    step: &ExecutionStep,
) -> Vec<ExecutionObstruction> {
    let cell_ids = case_space
        .case_cells
        .iter()
        .map(|cell| &cell.id)
        .collect::<BTreeSet<_>>();
    step.success_evidence_requirement_ids
        .iter()
        .filter(|id| !cell_ids.contains(id))
        .map(|id| ExecutionObstruction {
            obstruction_type: "missing_evidence_requirement_cell".to_owned(),
            summary: format!("success evidence requirement {id} is not a materialized case cell"),
            witness_ids: vec![id.clone()],
        })
        .collect()
}

fn evidence_morphism(
    case_space: &CaseSpace,
    plan: &ExecutionPlan,
    step: &ExecutionStep,
    identity: &TraceIdentity,
    worker_report: &WorkerReport,
    requirement_ids: &[Id],
) -> Result<CaseMorphism, NativeCliError> {
    let evidence_id = Id::new(format!(
        "evidence:worker-output:{}",
        path_segment(&identity.trace_id)
    ))?;
    let stdout = worker_report
        .outputs
        .iter()
        .find(|output| output.name == WorkerOutputName::Stdout)
        .ok_or_else(|| NativeCliError::invalid("worker report has no stdout output"))?;
    let evidence_provenance = provenance(
        SourceKind::Custom("tool_captured_artifact".to_owned()),
        ReviewStatus::Unreviewed,
    );
    let evidence_cell = CaseCell {
        id: evidence_id.clone(),
        cell_type: CaseCellType::Evidence,
        space_id: case_space.space_id.clone(),
        title: format!("Worker stdout for {}", step.step_id),
        summary: Some(
            "Locally captured worker process output; untrusted until reviewed.".to_owned(),
        ),
        lifecycle: CaseCellLifecycle::Active,
        source_ids: vec![worker_report.report_id.clone()],
        structure_ids: Vec::new(),
        provenance: evidence_provenance.clone(),
        metadata: Map::from_iter([
            (
                "content_hash".to_owned(),
                Value::String(stdout.content_hash.clone()),
            ),
            (
                "worker_report_id".to_owned(),
                json!(worker_report.report_id),
            ),
            ("exit_status".to_owned(), json!(worker_report.exit_status)),
        ]),
    };
    let relations = requirement_ids
        .iter()
        .enumerate()
        .map(|(index, requirement_id)| {
            Ok(CaseRelation {
                id: Id::new(format!(
                    "relation:worker-evidence:{}:{}",
                    path_segment(&identity.trace_id),
                    index + 1
                ))?,
                relation_type: CaseRelationType::SatisfiesEvidenceRequirement,
                relation_strength: RelationStrength::Hard,
                from_id: evidence_id.clone(),
                to_id: requirement_id.clone(),
                evidence_ids: vec![evidence_id.clone()],
                source_ids: vec![worker_report.report_id.clone()],
                provenance: evidence_provenance.clone(),
                metadata: Map::new(),
            })
        })
        .collect::<Result<Vec<_>, NativeCliError>>()?;
    let mut added_ids = vec![evidence_id.clone()];
    added_ids.extend(relations.iter().map(|relation| relation.id.clone()));
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            added_cells: vec![evidence_cell],
            added_relations: relations,
            ..MorphismPayload::default()
        })?,
    );
    metadata.insert("plan_id".to_owned(), json!(plan.plan_id));
    metadata.insert("step_id".to_owned(), json!(step.step_id));
    metadata.insert("trace_id".to_owned(), json!(identity.trace_id));
    Ok(CaseMorphism {
        morphism_id: Id::new(format!(
            "morphism:worker-evidence:{}",
            path_segment(&identity.trace_id)
        ))?,
        morphism_type: CaseMorphismType::EvidenceAttach,
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: Id::new(format!(
            "revision:worker-evidence:{}",
            path_segment(&identity.trace_id)
        ))?,
        added_ids,
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: requirement_ids.to_vec(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: vec![evidence_id],
        source_ids: vec![worker_report.report_id.clone()],
        metadata,
    })
}

fn transition_morphism(
    case_space: &CaseSpace,
    plan: &ExecutionPlan,
    step: &ExecutionStep,
    identity: &TraceIdentity,
    evidence_cell_id: &Id,
) -> Result<CaseMorphism, NativeCliError> {
    let mut updated_cell = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == step.work_cell_id)
        .cloned()
        .ok_or_else(|| {
            NativeCliError::invalid(format!("unknown work cell {}", step.work_cell_id))
        })?;
    if updated_cell.lifecycle != CaseCellLifecycle::Active {
        return Err(NativeCliError::invalid(format!(
            "work cell {} must remain active until its transition is applied",
            step.work_cell_id
        )));
    }
    updated_cell.lifecycle = CaseCellLifecycle::Resolved;
    let mut metadata = Map::new();
    metadata.insert(
        "payload".to_owned(),
        serde_json::to_value(MorphismPayload {
            updated_cells: vec![updated_cell],
            ..MorphismPayload::default()
        })?,
    );
    metadata.insert("plan_id".to_owned(), json!(plan.plan_id));
    metadata.insert("step_id".to_owned(), json!(step.step_id));
    metadata.insert("trace_id".to_owned(), json!(identity.trace_id));
    metadata.insert(
        "authorization_source".to_owned(),
        Value::String("accepted_execution_plan".to_owned()),
    );
    Ok(CaseMorphism {
        morphism_id: Id::new(format!(
            "morphism:worker-transition:{}",
            path_segment(&identity.trace_id)
        ))?,
        morphism_type: CaseMorphismType::Update,
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: Id::new(format!(
            "revision:worker-transition:{}",
            path_segment(&identity.trace_id)
        ))?,
        added_ids: Vec::new(),
        updated_ids: vec![step.work_cell_id.clone()],
        retired_ids: Vec::new(),
        preserved_ids: vec![evidence_cell_id.clone()],
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Unreviewed,
        evidence_ids: vec![evidence_cell_id.clone()],
        source_ids: vec![evidence_cell_id.clone()],
        metadata,
    })
}

fn report_entry(report: &Value) -> Result<MorphismLogEntry, NativeCliError> {
    serde_json::from_value(report["result"]["entry"].clone()).map_err(NativeCliError::from)
}

fn write_trace(run_directory: &Path, trace: &ExecutionTrace) -> Result<(), NativeCliError> {
    write_json(
        &run_directory.join("execution.trace.json"),
        &serde_json::to_value(trace)?,
    )
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), NativeCliError> {
    fs::write(path, bytes).map_err(|source| NativeCliError::Io {
        path: path.to_owned(),
        source,
    })
}

fn no_dispatchable_report(
    obstructions: Vec<ExecutionObstruction>,
    step_reasons: Vec<Value>,
) -> Value {
    report(
        "casegraphen run --step",
        json!({
            "status": "no_dispatchable_step",
            "trace": null,
            "worker_report_summary": null,
            "appended_entry_ids": [],
            "obstructions": obstructions,
            "step_reasons": step_reasons,
        }),
    )
}

fn run_report(
    status: &str,
    trace: Option<ExecutionTrace>,
    worker_report: Option<&WorkerReport>,
    step_reasons: Vec<Value>,
) -> Value {
    let appended_entry_ids = trace
        .as_ref()
        .map(|trace| trace.appended_entry_ids.clone())
        .unwrap_or_default();
    let worker_report_summary = worker_report.map(|report| {
        json!({
            "report_id": report.report_id,
            "exit_status": report.exit_status,
            "timed_out": report.timed_out,
            "outputs": report.outputs,
            "trust_boundary": report.trust_boundary,
        })
    });
    report(
        "casegraphen run --step",
        json!({
            "status": status,
            "trace": trace,
            "worker_report_summary": worker_report_summary,
            "appended_entry_ids": appended_entry_ids,
            "step_reasons": step_reasons,
        }),
    )
}
