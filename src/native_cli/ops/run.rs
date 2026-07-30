use super::{
    append_validated_morphism,
    binding::binding_path,
    case_reason,
    io::{provenance, timestamp, write_json},
    plan::{plan_path, read_stored_plan, verified_plan_review_status},
    report, require_current_revision, NativeCliError, NativeReasonSection, NativeRunStepOptions,
};
use crate::{
    exec::{
        binding::{
            resolve_worker_binding_identity, validate_worker_binding, WorkerBinding, WorkerKind,
        },
        records::{
            ExecutionDispatchState, ExecutionInformationLoss, ExecutionObstruction, ExecutionTrace,
            WorkerOutput, WorkerOutputName, WorkerReport, EXECUTION_RECORD_SCHEMA_VERSION,
            EXECUTION_TRACE_SCHEMA, WORKER_REPORT_SCHEMA, WORKER_REPORT_TRUST_BOUNDARY,
        },
        transition_permitted,
        worker::{execute_worker, WorkerContext},
        ExecutionPlan, ExecutionStep,
    },
    native_eval::evaluate_native_case,
    native_model::{
        apply_morphism, CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType,
        CaseRelation, CaseRelationType, CaseSpace, MorphismLogEntry, MorphismPayload,
        RelationStrength,
    },
    native_review::{check_operation_gate, NativeOperationGate},
    native_store::NativeCaseStore,
};
use higher_graphen_core::{Id, ReviewStatus, Severity, SourceKind};
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
        actor_id: options.actor_id.clone(),
        operation: "dispatch".to_owned(),
        operation_scope_id: options.gate_options.operation_scope_id.clone(),
        audience: options.gate_options.audience,
        capability_ids: options.gate_options.capability_ids.clone(),
        source_boundary_id: options.gate_options.source_boundary_id.clone(),
    };
    let evaluation = evaluate_native_case(&replay.case_space)?;
    let traces = read_execution_traces(store, &replay.case_space)?;
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
    let trace_identity = reserve_trace_identity(store, &plan, step, &traces)?;
    let trace_started_at = timestamp();
    let expected_binding_hash = expected_binding_hash(&plan, &step.worker_binding_id);
    let mut trace_guard = TraceGuard::start(
        store,
        options.case_space_id,
        options.actor_id,
        &replay.case_space,
        &plan,
        step,
        options.base_revision_id,
        &trace_identity,
        expected_binding_hash
            .clone()
            .unwrap_or_else(empty_content_hash),
        &gate,
        &trace_started_at,
    )?;

    if let Err(error) = check_operation_gate(&replay.case_space, &gate, "dispatch") {
        let trace = trace_guard.finish(
            &replay.case_space,
            ExecutionDispatchState::Failed,
            "operation_gate_rejected",
            vec![ExecutionObstruction {
                obstruction_type: "operation_gate_rejected".to_owned(),
                summary: error.to_string(),
                witness_ids: vec![gate.actor_id.clone()],
                blocking: true,
            }],
        )?;
        return Ok(run_report(
            "no_dispatchable_step",
            Some(trace),
            None,
            selection.step_reasons,
        ));
    }

    let binding_path = binding_path(store, &step.worker_binding_id);
    let verified_binding = match read_verified_worker_binding_snapshot(
        &binding_path,
        &step.worker_binding_id,
        expected_binding_hash.as_deref(),
    )? {
        BindingSnapshot::Verified(verified) => verified,
        snapshot @ (BindingSnapshot::Missing | BindingSnapshot::HashMismatch { .. }) => {
            let (obstruction_type, actual_binding_hash) = match snapshot {
                BindingSnapshot::Missing => ("binding_not_registered", None),
                BindingSnapshot::HashMismatch { actual_hash } => {
                    ("binding_hash_mismatch", Some(actual_hash))
                }
                BindingSnapshot::Verified(_) => unreachable!("matched verified binding"),
            };
            let trace_binding_hash = actual_binding_hash
                .or_else(|| expected_binding_hash.clone())
                .unwrap_or_else(empty_content_hash);
            trace_guard.trace.binding_content_hash = trace_binding_hash;
            let trace = trace_guard.finish(
                &replay.case_space,
                ExecutionDispatchState::Failed,
                obstruction_type,
                vec![ExecutionObstruction {
                    obstruction_type: obstruction_type.to_owned(),
                    summary: format!(
                        "worker binding {} content hash does not match the hash accepted with plan {}",
                        step.worker_binding_id, plan.plan_id
                    ),
                    witness_ids: vec![step.worker_binding_id.clone()],
                    blocking: true,
                }],
            )?;
            return Ok(run_report(
                "no_dispatchable_step",
                Some(trace),
                None,
                selection.step_reasons,
            ));
        }
    };
    let VerifiedWorkerBinding {
        mut binding,
        content_hash: binding_content_hash,
    } = *verified_binding;
    trace_guard.trace.binding_content_hash = binding_content_hash.clone();
    let missing_binding_capability_ids = binding
        .capability_ids
        .iter()
        .filter(|capability_id| !gate.capability_ids.contains(capability_id))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_binding_capability_ids.is_empty() {
        let trace = trace_guard.finish(
            &replay.case_space,
            ExecutionDispatchState::Failed,
            "operation_gate_rejected",
            vec![ExecutionObstruction {
                obstruction_type: "operation_gate_rejected".to_owned(),
                summary: format!(
                    "dispatch operation gate does not cover worker binding {} capability_ids: {}",
                    binding.binding_id,
                    missing_binding_capability_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                witness_ids: missing_binding_capability_ids,
                blocking: true,
            }],
        )?;
        return Ok(run_report(
            "no_dispatchable_step",
            Some(trace),
            None,
            selection.step_reasons,
        ));
    }
    let resolved_identity = match resolve_worker_binding_identity(&binding) {
        Ok(identity)
            if identity.resolved_command_path == binding.resolved_command_path
                && identity.resolved_working_directory == binding.resolved_working_directory
                && identity.command_content_hash == binding.command_content_hash =>
        {
            identity
        }
        Ok(identity) => {
            let trace = trace_guard.finish(
                &replay.case_space,
                ExecutionDispatchState::Failed,
                "binding_identity_mismatch",
                vec![ExecutionObstruction {
                    obstruction_type: "binding_identity_mismatch".to_owned(),
                    summary: format!(
                        "worker binding {} resolved identity no longer matches registration \
                         (command {}, working directory {}, command hash {})",
                        binding.binding_id,
                        identity.resolved_command_path,
                        identity.resolved_working_directory,
                        identity.command_content_hash
                    ),
                    witness_ids: vec![binding.binding_id.clone()],
                    blocking: true,
                }],
            )?;
            return Ok(run_report(
                "no_dispatchable_step",
                Some(trace),
                None,
                selection.step_reasons,
            ));
        }
        Err(error) => {
            let trace = trace_guard.finish(
                &replay.case_space,
                ExecutionDispatchState::Failed,
                "binding_identity_mismatch",
                vec![ExecutionObstruction {
                    obstruction_type: "binding_identity_mismatch".to_owned(),
                    summary: format!(
                        "worker binding {} identity could not be re-verified: {error}",
                        binding.binding_id
                    ),
                    witness_ids: vec![binding.binding_id.clone()],
                    blocking: true,
                }],
            )?;
            return Ok(run_report(
                "no_dispatchable_step",
                Some(trace),
                None,
                selection.step_reasons,
            ));
        }
    };
    binding.command = resolved_identity.resolved_command_path;
    binding.working_directory = resolved_identity.resolved_working_directory;
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
    trace_guard
        .trace
        .metadata
        .insert("worker_invoked".to_owned(), Value::Bool(true));
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

    let mut obstructions = Vec::new();
    let worker_succeeded = invocation.exit_status == Some(0) && !invocation.timed_out;
    let relation_requirement_ids = if worker_succeeded {
        existing_requirement_ids(&replay.case_space, step)
    } else {
        Vec::new()
    };
    let evidence_morphism = evidence_morphism(
        &replay.case_space,
        &plan,
        step,
        &trace_identity,
        &worker_report,
        &relation_requirement_ids,
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
    trace_guard.trace.appended_entry_ids = appended_entry_ids.clone();
    trace_guard.trace.result_revision_id = result_revision_id.clone();
    let mut transition_applied = false;
    let post_evidence = store_api.replay_current_case_space(options.case_space_id)?;
    let post_evaluation = evaluate_native_case(&post_evidence.case_space)?;
    let unsatisfied_success_evidence_requirement_ids = run_scoped_unsatisfied_requirement_ids(
        &post_evidence.case_space,
        &step.success_evidence_requirement_ids,
        &evidence_cell_id,
        &trace_identity.trace_id,
    );
    let status;

    if worker_succeeded {
        let transition = transition_morphism(
            &post_evidence.case_space,
            &plan,
            step,
            &trace_identity,
            &evidence_cell_id,
        )?;
        let mut candidate = post_evidence.case_space.clone();
        apply_morphism(&mut candidate, &transition)
            .map_err(|error| NativeCliError::invalid(error.to_string()))?;
        let candidate_evaluation = evaluate_native_case(&candidate)?;
        let new_hard_obstruction_ids =
            new_hard_obstruction_ids(&post_evaluation, &candidate_evaluation);
        if !unsatisfied_success_evidence_requirement_ids.is_empty()
            || !new_hard_obstruction_ids.is_empty()
        {
            write_json(
                &trace_identity.run_directory.join("proposed.morphism.json"),
                &serde_json::to_value(&transition)?,
            )?;
            if !unsatisfied_success_evidence_requirement_ids.is_empty() {
                obstructions.push(ExecutionObstruction {
                    obstruction_type: "success_conditions_unsatisfied".to_owned(),
                    summary: format!(
                        "step {} success evidence requirements remain unsatisfied after worker execution",
                        step.step_id
                    ),
                    witness_ids: unsatisfied_success_evidence_requirement_ids.clone(),
                    blocking: true,
                });
            }
            if !new_hard_obstruction_ids.is_empty() {
                obstructions.push(ExecutionObstruction {
                    obstruction_type: "invariant_regression".to_owned(),
                    summary: format!(
                        "step {} introduced new high or critical blocking obstructions",
                        step.step_id
                    ),
                    witness_ids: new_hard_obstruction_ids,
                    blocking: true,
                });
            }
            status = "transition_not_authorized";
        } else if step
            .allowed_transition_classes
            .iter()
            .any(|allowed| transition_permitted(allowed, &transition, &post_evidence.case_space))
        {
            let mut transition = transition;
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
            trace_guard.trace.appended_entry_ids = appended_entry_ids.clone();
            trace_guard.trace.result_revision_id = result_revision_id.clone();
            trace_guard.trace.transition_applied = true;
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
                blocking: true,
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
            blocking: true,
        });
        status = "step_failed";
    }

    trace_guard.trace = ExecutionTrace {
        schema: EXECUTION_TRACE_SCHEMA.to_owned(),
        schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
        trace_id: trace_identity.trace_id.clone(),
        plan_id: plan.plan_id.clone(),
        step_id: step.step_id.clone(),
        case_space_id: options.case_space_id.clone(),
        base_revision_id: options.base_revision_id.clone(),
        result_revision_id,
        work_cell_id: step.work_cell_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_content_hash,
        operation_gate: gate.clone(),
        worker_report_id: worker_report.report_id.clone(),
        appended_entry_ids,
        dispatch_state: ExecutionDispatchState::Started,
        transition_applied,
        unsatisfied_success_evidence_requirement_ids,
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
    let final_replay = store_api.replay_current_case_space(options.case_space_id)?;
    let dispatch_state = if status == "step_executed" {
        ExecutionDispatchState::Completed
    } else {
        ExecutionDispatchState::Failed
    };
    let trace = trace_guard.finish(&final_replay.case_space, dispatch_state, status, Vec::new())?;
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
    let derived_status = verified_plan_review_status(plan, case_space)?;
    if derived_status != ReviewStatus::Accepted {
        return Err(NativeCliError::invalid(format!(
            "execution plan {} is not accepted by its latest plan review (derived status: {:?})",
            plan.plan_id, derived_status
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
        let prior_started = traces.iter().any(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == step.step_id
                && trace.dispatch_state == ExecutionDispatchState::Started
        });
        let prior_applied = traces.iter().any(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == step.step_id
                && trace.transition_applied
        });
        let prior_failed = traces.iter().any(|trace| {
            trace.plan_id == plan.plan_id
                && trace.step_id == step.step_id
                && trace.dispatch_state == ExecutionDispatchState::Failed
        });
        if prior_started {
            reasons.push("dispatch_in_progress");
            obstructions.push(ExecutionObstruction {
                obstruction_type: "dispatch_in_progress".to_owned(),
                summary: format!("step {} already has a dispatch in progress", step.step_id),
                witness_ids: vec![step.step_id.clone()],
                blocking: true,
            });
        }
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
                blocking: true,
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

fn read_execution_traces(
    store: &Path,
    case_space: &CaseSpace,
) -> Result<Vec<ExecutionTrace>, NativeCliError> {
    verify_recorded_trace_anchors(store, case_space)?;
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
            let trace: ExecutionTrace = serde_json::from_slice(&bytes).map_err(|error| {
                NativeCliError::invalid(format!(
                    "execution trace {} could not be read: {error}",
                    path.display()
                ))
            })?;
            let expected_run_directory_name = path_segment(&trace.trace_id);
            if path.parent().and_then(Path::file_name).and_then(|name| name.to_str())
                != Some(expected_run_directory_name.as_str())
            {
                return Err(NativeCliError::invalid(format!(
                    "execution trace {} is stored under a run directory that does not match its trace id",
                    trace.trace_id
                )));
            }
            if let Some(recorded_hash) = recorded_trace_hash(case_space, &trace.trace_id) {
                let actual_hash = crate::native_hash::sha256_hex(&bytes);
                if actual_hash != recorded_hash {
                    return Err(NativeCliError::invalid(format!(
                        "execution trace {} at {} does not match its morphism-log content hash; \
                         the trace may have been rewritten",
                        trace.trace_id,
                        path.display()
                    )));
                }
            }
            Ok(trace)
        })
        .collect()
}

fn verify_recorded_trace_anchors(
    store: &Path,
    case_space: &CaseSpace,
) -> Result<(), NativeCliError> {
    for entry in &case_space.morphism_log {
        if entry.morphism.morphism_type
            != CaseMorphismType::Custom("execution_trace_anchor".to_owned())
            || entry.morphism.review_status != ReviewStatus::Accepted
        {
            continue;
        }
        let Some(trace_id) = entry
            .morphism
            .metadata
            .get("trace_id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(recorded_hash) = entry
            .morphism
            .metadata
            .get("trace_content_hash")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(relative_path) = entry
            .morphism
            .metadata
            .get("trace_path")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let trace_path = store.join(relative_path);
        let bytes = fs::read(&trace_path).map_err(|error| {
            NativeCliError::invalid(format!(
                "execution trace {trace_id} at {} cannot be verified against its morphism-log content hash: {error}",
                trace_path.display()
            ))
        })?;
        if crate::native_hash::sha256_hex(&bytes) != recorded_hash {
            return Err(NativeCliError::invalid(format!(
                "execution trace {trace_id} at {} does not match its morphism-log content hash; \
                 the trace may have been rewritten",
                trace_path.display()
            )));
        }
    }
    Ok(())
}

fn recorded_trace_hash<'a>(case_space: &'a CaseSpace, trace_id: &Id) -> Option<&'a str> {
    case_space.morphism_log.iter().rev().find_map(|entry| {
        if entry.morphism.morphism_type
            != CaseMorphismType::Custom("execution_trace_anchor".to_owned())
            || entry.morphism.review_status != ReviewStatus::Accepted
        {
            return None;
        }
        let metadata = &entry.morphism.metadata;
        (metadata.get("trace_id").and_then(Value::as_str) == Some(trace_id.as_str()))
            .then(|| metadata.get("trace_content_hash").and_then(Value::as_str))
            .flatten()
    })
}

#[derive(Debug)]
struct TraceIdentity {
    trace_id: Id,
    worker_report_id: Id,
    run_directory: PathBuf,
}

fn reserve_trace_identity(
    store: &Path,
    plan: &ExecutionPlan,
    step: &ExecutionStep,
    traces: &[ExecutionTrace],
) -> Result<TraceIdentity, NativeCliError> {
    let mut attempt = traces
        .iter()
        .filter(|trace| trace.plan_id == plan.plan_id && trace.step_id == step.step_id)
        .count()
        + 1;
    let run_root = store.join(RUN_DIRECTORY);
    fs::create_dir_all(&run_root).map_err(|source| NativeCliError::Io {
        path: run_root.clone(),
        source,
    })?;
    loop {
        let trace_id = Id::new(format!(
            "execution_trace:{}:{}:{attempt}",
            path_segment(&plan.plan_id),
            path_segment(&step.step_id)
        ))?;
        let worker_report_id = Id::new(format!("worker_report:{}", path_segment(&trace_id)))?;
        let run_directory = run_root.join(path_segment(&trace_id));
        match fs::create_dir(&run_directory) {
            Ok(()) => {
                return Ok(TraceIdentity {
                    trace_id,
                    worker_report_id,
                    run_directory,
                })
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                let trace_path = run_directory.join("execution.trace.json");
                match fs::read(&trace_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<ExecutionTrace>(&bytes).ok())
                {
                    Some(trace) if trace.dispatch_state != ExecutionDispatchState::Started => {
                        attempt += 1;
                    }
                    Some(trace) => {
                        return Err(NativeCliError::invalid(format!(
                            "execution trace {} already has a dispatch in progress",
                            trace.trace_id
                        )));
                    }
                    None => {
                        return Err(NativeCliError::invalid(format!(
                            "run directory {} is already reserved by a dispatch in progress",
                            run_directory.display()
                        )));
                    }
                }
            }
            Err(source) => {
                return Err(NativeCliError::Io {
                    path: run_directory,
                    source,
                })
            }
        }
    }
}

fn expected_binding_hash(plan: &ExecutionPlan, binding_id: &Id) -> Option<String> {
    plan.metadata
        .get("worker_binding_hashes")
        .and_then(Value::as_object)
        .and_then(|hashes| hashes.get(binding_id.as_str()))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

struct VerifiedWorkerBinding {
    binding: WorkerBinding,
    content_hash: String,
}

enum BindingSnapshot {
    Missing,
    HashMismatch { actual_hash: String },
    Verified(Box<VerifiedWorkerBinding>),
}

fn read_verified_worker_binding_snapshot(
    path: &Path,
    binding_id: &Id,
    expected_hash: Option<&str>,
) -> Result<BindingSnapshot, NativeCliError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BindingSnapshot::Missing)
        }
        Err(source) => {
            return Err(NativeCliError::Io {
                path: path.to_owned(),
                source,
            })
        }
    };
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let actual_hash = match &parsed {
        Ok(value) => crate::native_hash::sha256_hex(serde_json::to_string(value)?.as_bytes()),
        Err(_) => crate::native_hash::sha256_hex(&bytes),
    };
    if expected_hash != Some(actual_hash.as_str()) {
        return Ok(BindingSnapshot::HashMismatch { actual_hash });
    }

    let binding: WorkerBinding = serde_json::from_value(parsed?)?;
    validate_worker_binding(&binding)
        .map_err(|error| NativeCliError::invalid(format!("{}: {error}", path.display())))?;
    if binding.binding_id != *binding_id {
        return Err(NativeCliError::invalid(format!(
            "{}: worker binding id {} does not match requested {binding_id}",
            path.display(),
            binding.binding_id
        )));
    }

    Ok(BindingSnapshot::Verified(Box::new(VerifiedWorkerBinding {
        binding,
        content_hash: actual_hash,
    })))
}

fn empty_content_hash() -> String {
    crate::native_hash::sha256_hex(&[])
}

struct TraceGuard {
    store: PathBuf,
    actor_id: Id,
    run_directory: PathBuf,
    trace: ExecutionTrace,
    finished: bool,
}

impl TraceGuard {
    #[allow(clippy::too_many_arguments)]
    fn start(
        store: &Path,
        case_space_id: &Id,
        actor_id: &Id,
        case_space: &CaseSpace,
        plan: &ExecutionPlan,
        step: &ExecutionStep,
        base_revision_id: &Id,
        identity: &TraceIdentity,
        binding_content_hash: String,
        operation_gate: &NativeOperationGate,
        started_at: &str,
    ) -> Result<Self, NativeCliError> {
        let trace = ExecutionTrace {
            schema: EXECUTION_TRACE_SCHEMA.to_owned(),
            schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
            trace_id: identity.trace_id.clone(),
            plan_id: plan.plan_id.clone(),
            step_id: step.step_id.clone(),
            case_space_id: case_space_id.clone(),
            base_revision_id: base_revision_id.clone(),
            result_revision_id: None,
            work_cell_id: step.work_cell_id.clone(),
            binding_id: step.worker_binding_id.clone(),
            binding_content_hash,
            operation_gate: operation_gate.clone(),
            worker_report_id: identity.worker_report_id.clone(),
            appended_entry_ids: Vec::new(),
            dispatch_state: ExecutionDispatchState::Started,
            transition_applied: false,
            unsatisfied_success_evidence_requirement_ids: Vec::new(),
            obstructions: Vec::new(),
            information_loss: Vec::new(),
            started_at: started_at.to_owned(),
            finished_at: started_at.to_owned(),
            metadata: Map::from_iter([
                (
                    "dispatch_status".to_owned(),
                    Value::String("started".to_owned()),
                ),
                ("worker_invoked".to_owned(), Value::Bool(false)),
                (
                    "reserved_base_revision_id".to_owned(),
                    json!(case_space.revision.revision_id),
                ),
            ]),
        };
        write_trace(&identity.run_directory, &trace)?;
        Ok(Self {
            store: store.to_path_buf(),
            actor_id: actor_id.clone(),
            run_directory: identity.run_directory.clone(),
            trace,
            finished: false,
        })
    }

    fn finish(
        mut self,
        case_space: &CaseSpace,
        dispatch_state: ExecutionDispatchState,
        status: &str,
        mut obstructions: Vec<ExecutionObstruction>,
    ) -> Result<ExecutionTrace, NativeCliError> {
        self.trace.dispatch_state = dispatch_state;
        self.trace.finished_at = timestamp();
        self.trace.obstructions.append(&mut obstructions);
        self.trace.metadata.insert(
            "dispatch_status".to_owned(),
            Value::String(status.to_owned()),
        );
        self.finished = true;
        write_and_anchor_trace(
            &self.store,
            &self.actor_id,
            case_space,
            &self.run_directory,
            self.trace.clone(),
        )
    }
}

impl Drop for TraceGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.trace.dispatch_state = ExecutionDispatchState::Failed;
        self.trace.transition_applied = false;
        self.trace.finished_at = timestamp();
        self.trace.metadata.insert(
            "dispatch_status".to_owned(),
            Value::String("dispatch_failed".to_owned()),
        );
        self.trace.obstructions.push(ExecutionObstruction {
            obstruction_type: "dispatch_failed".to_owned(),
            summary: format!(
                "dispatch {} failed after its run directory was reserved",
                self.trace.trace_id
            ),
            witness_ids: vec![self.trace.trace_id.clone()],
            blocking: true,
        });
        let store_api = NativeCaseStore::new(self.store.clone());
        let anchored = store_api
            .replay_current_case_space(&self.trace.case_space_id)
            .ok()
            .and_then(|replay| {
                write_and_anchor_trace(
                    &self.store,
                    &self.actor_id,
                    &replay.case_space,
                    &self.run_directory,
                    self.trace.clone(),
                )
                .ok()
            });
        if anchored.is_none() {
            let _ = write_trace(&self.run_directory, &self.trace);
        }
    }
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
        descendants_may_survive: invocation.descendants_may_survive,
        outputs: vec![
            WorkerOutput {
                name: WorkerOutputName::Stdout,
                content_hash: invocation.stdout_sha256.clone(),
                byte_len: invocation.stdout_byte_len,
                retained_byte_len: u64::try_from(invocation.stdout.len()).unwrap_or(u64::MAX),
                truncated: invocation.stdout_truncated,
                incomplete: invocation.stdout_incomplete,
            },
            WorkerOutput {
                name: WorkerOutputName::Stderr,
                content_hash: invocation.stderr_sha256.clone(),
                byte_len: invocation.stderr_byte_len,
                retained_byte_len: u64::try_from(invocation.stderr.len()).unwrap_or(u64::MAX),
                truncated: invocation.stderr_truncated,
                incomplete: invocation.stderr_incomplete,
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
    let evidence_cell_ids = case_space
        .case_cells
        .iter()
        .filter(|cell| cell.cell_type == CaseCellType::Evidence)
        .map(|cell| &cell.id)
        .collect::<BTreeSet<_>>();
    step.success_evidence_requirement_ids
        .iter()
        .filter(|id| evidence_cell_ids.contains(id))
        .cloned()
        .collect()
}

fn run_scoped_unsatisfied_requirement_ids(
    case_space: &CaseSpace,
    requirement_ids: &[Id],
    evidence_cell_id: &Id,
    trace_id: &Id,
) -> Vec<Id> {
    let evidence_from_this_run = case_space.case_cells.iter().any(|cell| {
        cell.id == *evidence_cell_id
            && cell.cell_type == CaseCellType::Evidence
            && cell.metadata.get("trace_id").and_then(Value::as_str) == Some(trace_id.as_str())
    });
    requirement_ids
        .iter()
        .filter(|requirement_id| {
            !evidence_from_this_run
                || !case_space.case_relations.iter().any(|relation| {
                    relation.relation_type == CaseRelationType::SatisfiesEvidenceRequirement
                        && relation.from_id == *evidence_cell_id
                        && relation.to_id == **requirement_id
                        && relation.evidence_ids.contains(evidence_cell_id)
                })
        })
        .cloned()
        .collect()
}

fn new_hard_obstruction_ids(
    before: &crate::native_eval::NativeCaseEvaluation,
    after: &crate::native_eval::NativeCaseEvaluation,
) -> Vec<Id> {
    let before_ids = before
        .obstructions
        .iter()
        .filter(|obstruction| {
            obstruction.blocking
                && matches!(obstruction.severity, Severity::High | Severity::Critical)
        })
        .map(|obstruction| &obstruction.id)
        .collect::<BTreeSet<_>>();
    after
        .obstructions
        .iter()
        .filter(|obstruction| {
            obstruction.blocking
                && matches!(obstruction.severity, Severity::High | Severity::Critical)
                && !before_ids.contains(&obstruction.id)
        })
        .map(|obstruction| obstruction.id.clone())
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
            ("trace_id".to_owned(), json!(identity.trace_id)),
            (
                "evidence_boundary".to_owned(),
                Value::String("worker_output".to_owned()),
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
                relation_strength: RelationStrength::Diagnostic,
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
        review_status: ReviewStatus::Unreviewed,
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

fn write_and_anchor_trace(
    store: &Path,
    actor_id: &Id,
    case_space: &CaseSpace,
    run_directory: &Path,
    mut trace: ExecutionTrace,
) -> Result<ExecutionTrace, NativeCliError> {
    let morphism_id = Id::new(format!(
        "morphism:execution-trace-anchor:{}",
        path_segment(&trace.trace_id)
    ))?;
    let anchor_entry_id = Id::new(format!(
        "morphism_log_entry:{}:{}",
        path_segment(&morphism_id),
        case_space.morphism_log.len() + 1
    ))?;
    let target_revision_id = Id::new(format!(
        "revision:execution-trace-anchor:{}",
        path_segment(&trace.trace_id)
    ))?;
    trace.result_revision_id = Some(target_revision_id.clone());
    trace.appended_entry_ids.push(anchor_entry_id.clone());
    write_trace(run_directory, &trace)?;
    let trace_path = run_directory.join("execution.trace.json");
    let trace_bytes = fs::read(&trace_path).map_err(|source| NativeCliError::Io {
        path: trace_path.clone(),
        source,
    })?;
    let trace_content_hash = crate::native_hash::sha256_hex(&trace_bytes);
    let relative_trace_path = trace_path
        .strip_prefix(store)
        .unwrap_or(&trace_path)
        .display()
        .to_string();
    let anchor = CaseMorphism {
        morphism_id,
        morphism_type: CaseMorphismType::Custom("execution_trace_anchor".to_owned()),
        source_revision_id: Some(case_space.revision.revision_id.clone()),
        target_revision_id: target_revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![trace.trace_id.clone()],
        metadata: Map::from_iter([
            ("trace_id".to_owned(), json!(trace.trace_id)),
            (
                "trace_content_hash".to_owned(),
                Value::String(trace_content_hash),
            ),
            ("trace_path".to_owned(), Value::String(relative_trace_path)),
        ]),
    };
    let store_api = NativeCaseStore::new(store.to_path_buf());
    let report = append_validated_morphism(
        &store_api,
        case_space,
        anchor,
        Some(actor_id.clone()),
        "casegraphen run --step trace anchor",
    )?;
    let entry = report_entry(&report)?;
    if entry.entry_id != anchor_entry_id || entry.target_revision_id != target_revision_id {
        return Err(NativeCliError::invalid(format!(
            "execution trace {} anchor entry identity did not match the trace",
            trace.trace_id
        )));
    }
    Ok(trace)
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
            "descendants_may_survive": report.descendants_may_survive,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::binding::worker_binding_content_hash;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn verified_binding_snapshot_is_unchanged_after_the_file_is_swapped() {
        let directory = std::env::temp_dir().join(format!(
            "casegraphen-binding-snapshot-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("create snapshot test directory");
        let path = directory.join("worker.binding.json");
        let accepted: WorkerBinding = serde_json::from_str(include_str!(
            "../../../schemas/casegraphen/worker.binding.example.json"
        ))
        .expect("accepted worker binding");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&accepted).expect("serialize accepted binding"),
        )
        .expect("write accepted binding");
        let expected_hash = worker_binding_content_hash(&accepted).expect("accepted binding hash");

        let snapshot = read_verified_worker_binding_snapshot(
            &path,
            &accepted.binding_id,
            Some(&expected_hash),
        )
        .expect("read and verify binding exactly once");

        let mut malicious = accepted.clone();
        malicious.args = vec!["-c".to_owned(), "printf 'malicious'".to_owned()];
        fs::write(
            &path,
            serde_json::to_vec_pretty(&malicious).expect("serialize malicious binding"),
        )
        .expect("swap binding after verification");

        let BindingSnapshot::Verified(verified) = snapshot else {
            panic!("accepted binding should produce a verified snapshot");
        };
        assert_eq!(verified.binding, accepted);
        assert_eq!(verified.content_hash, expected_hash);
        assert_ne!(
            serde_json::from_slice::<WorkerBinding>(
                &fs::read(&path).expect("read swapped binding for test assertion")
            )
            .expect("parse swapped binding"),
            verified.binding
        );
        fs::remove_dir_all(directory).expect("remove snapshot test directory");
    }

    #[test]
    fn concurrent_run_directory_reservation_is_rejected() {
        let directory = std::env::temp_dir().join(format!(
            "casegraphen-run-reservation-{}-{}",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let plan: ExecutionPlan = serde_json::from_str(include_str!(
            "../../../schemas/casegraphen/execution.plan.example.json"
        ))
        .expect("execution plan example");
        let step = &plan.steps[0];

        let first =
            reserve_trace_identity(&directory, &plan, step, &[]).expect("first reservation");
        let error = reserve_trace_identity(&directory, &plan, step, &[])
            .expect_err("concurrent reservation must fail");

        assert!(first.run_directory.is_dir());
        assert!(error.to_string().contains("dispatch in progress"));
        fs::remove_dir_all(directory).expect("remove reservation test directory");
    }

    #[test]
    fn preexisting_evidence_does_not_satisfy_a_run_scoped_success_requirement() {
        let case_space: CaseSpace = serde_json::from_str(include_str!(
            "../../../schemas/casegraphen/native.case.space.example.json"
        ))
        .expect("native case space example");
        let evidence_id = Id::new("evidence:native-schema-json-valid").expect("evidence id");
        let trace_id = Id::new("execution_trace:run-scoped-test").expect("trace id");

        let unsatisfied = run_scoped_unsatisfied_requirement_ids(
            &case_space,
            std::slice::from_ref(&evidence_id),
            &evidence_id,
            &trace_id,
        );

        assert_eq!(unsatisfied, vec![evidence_id]);
    }
}
