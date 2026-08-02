use super::*;
use crate::exec::records::{
    ExecutionObstruction, EXECUTION_RECORD_SCHEMA_VERSION, EXECUTION_TRACE_SCHEMA,
};
use crate::exec::{AllowedTransitionClass, ExecutionStep, EXECUTION_PLAN_SCHEMA};
use crate::native_eval::evaluate_native_case;
use crate::native_model::{
    CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType, CaseRelation,
    CaseRelationType, CaseSpace, MorphismLogEntry, RelationStrength, NATIVE_CASE_SPACE_SCHEMA,
    NATIVE_CASE_SPACE_SCHEMA_VERSION, NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
};
use crate::native_review::NativeOperationGate;
use arbtest::arbitrary::Arbitrary;
use higher_graphen_core::{Confidence, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{Map, Value};

fn id(value: &str) -> Id {
    Id::new(value.to_owned()).expect("test id")
}

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence")
}

fn provenance(kind: SourceKind, review_status: ReviewStatus) -> Provenance {
    Provenance::new(SourceRef::new(kind), confidence(1.0)).with_review_status(review_status)
}

fn cell(id_value: &str, cell_type: CaseCellType, lifecycle: CaseCellLifecycle) -> CaseCell {
    CaseCell {
        id: id(id_value),
        cell_type,
        space_id: id("space:halt-fixture"),
        title: id_value.to_owned(),
        summary: None,
        lifecycle,
        source_ids: vec![id("source:test")],
        structure_ids: Vec::new(),
        provenance: provenance(SourceKind::Human, ReviewStatus::Reviewed),
        metadata: Map::new(),
    }
}

fn relation(
    id_value: &str,
    relation_type: CaseRelationType,
    from_id: &str,
    to_id: &str,
) -> CaseRelation {
    CaseRelation {
        id: id(id_value),
        relation_type,
        relation_strength: RelationStrength::Hard,
        from_id: id(from_id),
        to_id: id(to_id),
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        provenance: provenance(SourceKind::Human, ReviewStatus::Reviewed),
        metadata: Map::new(),
    }
}

fn source_boundary_metadata() -> Value {
    serde_json::json!({
        "id": "source_boundary:halt-fixture",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["native.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture makes no inferred claims",
        "information_loss": []
    })
}

fn fixture_space() -> CaseSpace {
    let source_boundary = source_boundary_metadata();
    let revision = crate::native_model::Revision {
        revision_id: id("revision:halt-fixture-v1"),
        case_space_id: id("case_space:halt-fixture"),
        applied_entry_ids: vec![id("morphism_log_entry:genesis")],
        applied_morphism_ids: vec![id("morphism:create-fixture")],
        checksum: "sha256:fixture".to_owned(),
        parent_revision_id: None,
        created_at: "2026-04-26T00:00:00Z".to_owned(),
        source_ids: vec![id("source:test")],
        metadata: Map::new(),
    };
    let mut morphism_metadata = Map::new();
    morphism_metadata.insert(
        "lift_semantics".to_owned(),
        serde_json::json!("fixture_to_case_space"),
    );
    morphism_metadata.insert(
        "source_boundary_id".to_owned(),
        serde_json::json!("source_boundary:halt-fixture"),
    );
    morphism_metadata.insert("source_boundary".to_owned(), source_boundary.clone());
    let morphism = CaseMorphism {
        morphism_id: id("morphism:create-fixture"),
        morphism_type: CaseMorphismType::Create,
        source_revision_id: None,
        target_revision_id: revision.revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![id("source:test")],
        metadata: morphism_metadata,
    };
    let mut metadata = Map::new();
    metadata.insert("source_boundary".to_owned(), source_boundary);
    CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: id("case_space:halt-fixture"),
        space_id: id("space:halt-fixture"),
        case_cells: Vec::new(),
        case_relations: Vec::new(),
        morphism_log: vec![MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: 1,
            case_space_id: id("case_space:halt-fixture"),
            sequence: 1,
            entry_id: id("morphism_log_entry:genesis"),
            morphism_id: id("morphism:create-fixture"),
            source_revision_id: None,
            target_revision_id: revision.revision_id.clone(),
            morphism,
            actor_id: id("actor:test"),
            recorded_at: "2026-04-26T00:00:00Z".to_owned(),
            provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
            source_ids: vec![id("source:test")],
            previous_entry_hash: None,
            replay_checksum: "sha256:fixture".to_owned(),
        }],
        projections: Vec::new(),
        revision,
        close_policy_id: Some(id("close_policy:native-default")),
        metadata,
    }
}

fn refresh_morphism(space: &mut CaseSpace) {
    space.morphism_log[0].morphism.added_ids = space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .collect();
}

fn single_step_plan(work_cell_id: &str) -> ExecutionPlan {
    ExecutionPlan {
        schema: EXECUTION_PLAN_SCHEMA.to_owned(),
        schema_version: 1,
        plan_id: id("plan:halt-fixture"),
        case_space_id: id("case_space:halt-fixture"),
        base_revision_id: id("revision:halt-fixture-v1"),
        steps: vec![ExecutionStep {
            step_id: id("step:only"),
            work_cell_id: id(work_cell_id),
            worker_binding_id: id("worker_binding:halt-fixture"),
            success_evidence_requirement_ids: vec![id("evidence:success")],
            allowed_transition_classes: vec![AllowedTransitionClass {
                morphism_type: CaseMorphismType::Update,
                target_cell_types: Vec::new(),
                to_lifecycles: Vec::new(),
            }],
        }],
        provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
        review_status: ReviewStatus::Accepted,
        metadata: Map::new(),
    }
}

fn failed_trace(step_id: &str, obstruction_type: &str, finished_at: &str) -> ExecutionTrace {
    ExecutionTrace {
        schema: EXECUTION_TRACE_SCHEMA.to_owned(),
        schema_version: EXECUTION_RECORD_SCHEMA_VERSION,
        trace_id: id(&format!(
            "execution_trace:halt-fixture:{step_id}:{finished_at}"
        )),
        plan_id: id("plan:halt-fixture"),
        step_id: id(step_id),
        case_space_id: id("case_space:halt-fixture"),
        base_revision_id: id("revision:halt-fixture-v1"),
        result_revision_id: None,
        work_cell_id: id("work:target"),
        binding_id: id("worker_binding:halt-fixture"),
        binding_content_hash: String::new(),
        operation_gate: NativeOperationGate {
            actor_id: id("actor:test"),
            operation: "dispatch".to_owned(),
            operation_scope_id: id("case_space:halt-fixture"),
            audience: crate::native_model::ProjectionAudience::Audit,
            capability_ids: Vec::new(),
            source_boundary_id: id("source_boundary:halt-fixture"),
        },
        worker_report_id: id("worker_report:halt-fixture"),
        worker_report_content_hash: String::new(),
        stdout_content_hash: String::new(),
        stderr_content_hash: String::new(),
        appended_entry_ids: Vec::new(),
        dispatch_state: ExecutionDispatchState::Failed,
        transition_applied: false,
        unsatisfied_success_evidence_requirement_ids: Vec::new(),
        obstructions: vec![ExecutionObstruction {
            obstruction_type: obstruction_type.to_owned(),
            summary: "test".to_owned(),
            witness_ids: Vec::new(),
            blocking: true,
        }],
        information_loss: Vec::new(),
        started_at: finished_at.to_owned(),
        finished_at: finished_at.to_owned(),
        metadata: Map::new(),
    }
}

#[test]
fn dispatchable_is_always_progress_unless_budget_is_exhausted() {
    let space = fixture_space();
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:nonexistent");

    assert_eq!(
        derive_halt(
            true,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        None
    );
    assert_eq!(
        derive_halt(
            true,
            true,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::RoundBudgetExhausted)
    );
}

#[test]
fn not_dispatchable_with_nothing_blocking_is_nothing_eligible() {
    let space = fixture_space();
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:nonexistent");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );
    // Budget exhaustion is meaningless when nothing is dispatchable: the
    // spec's own `halt()` only fires `RoundBudgetExhausted` when `has(Dispatchable)`.
    assert_eq!(
        derive_halt(
            false,
            true,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );
}

#[test]
fn missing_evidence_on_the_plans_own_work_cell_halts_on_needs_evidence() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "evidence:required",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:requires-evidence",
        CaseRelationType::RequiresEvidence,
        "work:target",
        "evidence:required",
    ));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsEvidence)
    );
}

#[test]
fn external_wait_on_the_plans_own_work_cell_halts_on_needs_external() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "event:awaited",
        CaseCellType::Event,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:waits-for",
        CaseRelationType::WaitsFor,
        "work:target",
        "event:awaited",
    ));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsExternal)
    );
}

/// An unresolved hard dependency must never be reported as `NeedsReview`.
/// This test asserted exactly that until it was disproved end to end:
/// `review accept` against the dependency appends a real, gated `waiver`
/// morphism and advances the revision, while `complete_cell` — the predicate
/// the obstruction actually tests — still reads the dependency's own
/// unchanged `lifecycle: active` / `review_status: reviewed`, so the
/// obstruction survives verbatim. A halt naming an operation that cannot
/// discharge it is the deadlock-wearing-a-vocabulary-word that
/// `REQ-OPERATE-009` forbids, so the old assertion was encoding the defect.
///
/// With no other obstruction anywhere, nothing in the ledger names an
/// operation for a merely-incomplete dependency, and `NothingEligible` is the
/// honest answer.
#[test]
fn an_unresolved_dependency_alone_is_not_reported_as_a_review_that_would_clear_it() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:upstream",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:depends-on",
        CaseRelationType::DependsOn,
        "work:target",
        "work:upstream",
    ));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );
}

/// The masking half of the same defect: an `UnresolvedDependency` is a
/// pointer, not a cause. When the dependency is itself blocked by something
/// clearable, the halt must name *that* — here the unsatisfied requirement
/// `evidence attach --satisfies` takes — instead of stopping at the pointer
/// and reporting a review of the blocked cell.
#[test]
fn a_dependency_blocked_by_missing_evidence_halts_on_the_upstream_requirement() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:upstream",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "goal:required-evidence",
        CaseCellType::Goal,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:depends-on",
        CaseRelationType::DependsOn,
        "work:target",
        "work:upstream",
    ));
    space.case_relations.push(relation(
        "relation:upstream-requires-evidence",
        CaseRelationType::RequiresEvidence,
        "work:upstream",
        "goal:required-evidence",
    ));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsEvidence)
    );
    let report = build_halt_report(
        Halt::NeedsEvidence,
        std::path::Path::new("/tmp/store"),
        &id("case_space:halt-fixture"),
        &plan,
        &id("revision:halt-fixture-v1"),
        &evaluation,
        &[],
        &BTreeSet::new(),
        &BTreeSet::new(),
    );
    // The requirement `evidence attach --satisfies` takes, reached through
    // the dependency — not the blocked cell the plan happens to name.
    assert_eq!(report.target_ids, vec![id("goal:required-evidence")]);
}

#[test]
fn review_required_on_the_plans_own_work_cell_halts_on_needs_review() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "review:required",
        CaseCellType::Review,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:accepts",
        CaseRelationType::Accepts,
        "work:target",
        "review:required",
    ));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsReview)
    );
}

/// The vacuous-self-satisfaction trap
/// (`skills/casegraphen-operate/references/authoring.md`): a review gap
/// marked `requirement_satisfied` must never be the reason `needs_review`
/// fires, because no review can clear an obligation the tool itself already
/// considers discharged. This is not reachable through `evaluate_native_case`
/// alone without a much larger fixture (the mark is set by
/// `sections::review_gaps` from a satisfied hard `requires_evidence` edge on
/// an `UnreviewedInference` gap), so it is exercised directly against
/// `derive_halt` with a hand-built evaluation.
#[test]
fn a_satisfied_requirement_placeholder_gap_does_not_halt_on_needs_review() {
    let space = fixture_space();
    let mut evaluation = evaluate_native_case(&space).expect("evaluation");
    evaluation
        .review_gaps
        .push(crate::native_eval::NativeReviewGap {
            id: id("review_gap:placeholder"),
            target_id: id("evidence:placeholder"),
            gap_type: crate::native_eval::NativeReviewGapType::UnreviewedInference,
            explanation: "test".to_owned(),
            requirement_satisfied: true,
        });
    let plan = single_step_plan("work:nonexistent");

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );

    // An unsatisfied gap of the same shape, by contrast, does halt.
    evaluation
        .review_gaps
        .last_mut()
        .unwrap()
        .requirement_satisfied = false;
    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsReview)
    );
}

/// A plan step's work cell that exists, is active, and carries no other
/// obstruction — the shape `select_steps` would actually mark eligible if
/// nothing had failed. Every retry/plan-review test below builds its plan
/// against a cell like this rather than `single_step_plan("work:nonexistent")`
/// (a cell `select_steps` would report `work_cell_missing` for, permanently,
/// regardless of any trace): a hand-built `solely_retry_blocked_step_ids`
/// asserting a step no real `select_steps` run could ever call
/// "solely blocked by a failed trace" would test nothing but this module's
/// own arithmetic, not the contract with `select_steps` finding 2 exists to
/// enforce.
fn selectable_work_cell(id_value: &str) -> CaseCell {
    cell(id_value, CaseCellType::Work, CaseCellLifecycle::Active)
}

#[test]
fn a_worker_execution_failure_halts_on_needs_retry_decision() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");
    let traces = vec![failed_trace(
        "step:only",
        "worker_execution_failed",
        "unix:100",
    )];
    let solely_retry_blocked = BTreeSet::from([id("step:only")]);

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &traces,
            &solely_retry_blocked,
            &BTreeSet::new()
        ),
        Some(Halt::NeedsRetryDecision)
    );
}

/// Finding 2's actual fix: a failed step is a `needs_retry_decision`
/// candidate only when `select_steps` says the failed trace is its *sole*
/// blocking reason. A step whose work cell left the frontier for a permanent
/// reason (resolved by hand, or by a sibling step) has a trace that will
/// never become retryable — `solely_retry_blocked_step_ids` empty, exactly
/// as `select_steps` would report it, must not still surface
/// `NeedsRetryDecision` naming a `--retry-step` that reselects nothing.
#[test]
fn a_failed_step_no_longer_solely_blocked_by_that_failure_does_not_halt_on_needs_retry_decision() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");
    let traces = vec![failed_trace(
        "step:only",
        "worker_execution_failed",
        "unix:100",
    )];

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &traces,
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );
}

#[test]
fn a_transition_outside_the_accepted_plan_halts_on_needs_plan_review() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");
    let traces = vec![failed_trace(
        "step:only",
        "transition_not_authorized",
        "unix:100",
    )];

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &traces,
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsPlanReview)
    );
}

/// Only the most recently *finished* trace decides a step's halt. An older
/// failure a later, differently-classified failure has superseded must not
/// keep naming a halt that no longer describes the step's actual state.
#[test]
fn only_the_latest_finished_trace_per_step_decides_its_halt() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");
    let traces = vec![
        failed_trace("step:only", "worker_execution_failed", "unix:100"),
        failed_trace("step:only", "transition_not_authorized", "unix:200"),
    ];

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &traces,
            &BTreeSet::new(),
            &BTreeSet::new()
        ),
        Some(Halt::NeedsPlanReview)
    );
}

/// REQ-OPERATE-009/010's real-code analogue: a halt names something that
/// actually clears it. Retrying a step whose latest trace succeeds removes
/// `NeedsRetryDecision`; a step with no unresolved failure at all
/// contributes nothing.
#[test]
fn needs_retry_decision_has_an_exit() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:target");
    let failed = vec![failed_trace(
        "step:only",
        "worker_execution_failed",
        "unix:100",
    )];
    let solely_retry_blocked = BTreeSet::from([id("step:only")]);
    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &failed,
            &solely_retry_blocked,
            &BTreeSet::new()
        ),
        Some(Halt::NeedsRetryDecision)
    );

    let mut succeeded = failed_trace("step:only", "worker_execution_failed", "unix:100");
    succeeded.dispatch_state = ExecutionDispatchState::Completed;
    succeeded.transition_applied = true;
    succeeded.obstructions.clear();
    let mut later_success = succeeded.clone();
    later_success.finished_at = "unix:200".to_owned();
    later_success.started_at = "unix:200".to_owned();
    let cleared = vec![failed[0].clone(), later_success];

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &cleared,
            &solely_retry_blocked,
            &BTreeSet::new()
        ),
        Some(Halt::NothingEligible)
    );
}

/// Priority: `NeedsRetryDecision` outranks `NeedsPlanReview` when different
/// steps of the same plan need each — matching `docs/specs/operate-halt.fsl`'s
/// `def halt()` order exactly.
#[test]
fn needs_retry_decision_outranks_needs_plan_review_across_steps() {
    let mut space = fixture_space();
    space.case_cells.push(selectable_work_cell("work:target"));
    space.case_cells.push(selectable_work_cell("work:second"));
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let mut plan = single_step_plan("work:target");
    plan.steps.push(ExecutionStep {
        step_id: id("step:second"),
        work_cell_id: id("work:second"),
        worker_binding_id: id("worker_binding:halt-fixture"),
        success_evidence_requirement_ids: vec![id("evidence:success")],
        allowed_transition_classes: vec![AllowedTransitionClass {
            morphism_type: CaseMorphismType::Update,
            target_cell_types: Vec::new(),
            to_lifecycles: Vec::new(),
        }],
    });
    let traces = vec![
        failed_trace("step:only", "transition_not_authorized", "unix:100"),
        failed_trace("step:second", "worker_execution_failed", "unix:100"),
    ];
    let solely_retry_blocked = BTreeSet::from([id("step:second")]);

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &traces,
            &solely_retry_blocked,
            &BTreeSet::new()
        ),
        Some(Halt::NeedsRetryDecision)
    );
}

/// `INV-OPERATE-003`/`INV-OPERATE-004` for the real implementation: the loop
/// is stopped iff the derivation names a halt, and a dispatchable step
/// bounded by budget is exactly the one case that still stops.
#[test]
fn derive_halt_is_stopped_iff_named_property() {
    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let dispatchable = bool::arbitrary(u)?;
            let budget_exhausted = bool::arbitrary(u)?;
            let include_review_gap = bool::arbitrary(u)?;

            let space = fixture_space();
            let mut evaluation = evaluate_native_case(&space).expect("evaluation");
            if include_review_gap {
                evaluation
                    .review_gaps
                    .push(crate::native_eval::NativeReviewGap {
                        id: id("review_gap:property"),
                        target_id: id("evidence:property"),
                        gap_type: crate::native_eval::NativeReviewGapType::UnreviewedInference,
                        explanation: "test".to_owned(),
                        requirement_satisfied: false,
                    });
            }
            let plan = single_step_plan("work:nonexistent");

            let halt = derive_halt(
                dispatchable,
                budget_exhausted,
                &evaluation,
                &plan,
                &[],
                &BTreeSet::new(),
                &BTreeSet::new(),
            );

            assert_eq!(halt.is_none(), dispatchable && !budget_exhausted);
            Ok(())
        },
    );
}

/// `MODEL-OPERATE-010`: another process's started dispatch is a named stop,
/// not `NothingEligible`. `in_flight_step_ids` is exactly
/// `select_steps`'s own `dispatch_in_progress` reason — see
/// `src/native_cli/ops/run.rs::in_flight_step_ids`, which reads it the same
/// way `solely_retry_blocked_step_ids` reads
/// `prior_failed_trace_requires_retry` — never re-derived from the trace
/// list here.
#[test]
fn an_in_flight_dispatch_halts_on_dispatch_in_progress_instead_of_nothing_eligible() {
    let space = fixture_space();
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:nonexistent");
    let in_flight = BTreeSet::from([id("step:only")]);

    assert_eq!(
        derive_halt(
            false,
            false,
            &evaluation,
            &plan,
            &[],
            &BTreeSet::new(),
            &in_flight,
        ),
        Some(Halt::DispatchInProgress)
    );
}

/// `REQ-OPERATE-014`'s real-code analogue: `DispatchInProgress` names the
/// started *trace*, not the step — `--supersede-trace` (ADR 0014) takes a
/// trace id, and the step id alone would not tell an operator which of
/// several attempts to assert dead.
#[test]
fn dispatch_in_progress_names_the_started_trace_as_its_exit() {
    let space = fixture_space();
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    let plan = single_step_plan("work:nonexistent");
    let in_flight = BTreeSet::from([id("step:only")]);
    let mut started = failed_trace("step:only", "n/a", "unix:100");
    started.dispatch_state = ExecutionDispatchState::Started;
    started.trace_id = id("execution_trace:halt-fixture:step:only:started");
    let traces = vec![started];

    let report = build_halt_report(
        Halt::DispatchInProgress,
        std::path::Path::new("/tmp/store"),
        &id("case_space:halt-fixture"),
        &plan,
        &id("revision:halt-fixture-v1"),
        &evaluation,
        &traces,
        &BTreeSet::new(),
        &in_flight,
    );
    assert_eq!(
        report.target_ids,
        vec![id("execution_trace:halt-fixture:step:only:started")]
    );
    assert_eq!(report.next_operations.len(), 1);
    assert_eq!(
        report.next_operations[0].arguments.get("supersede_trace"),
        Some(&"execution_trace:halt-fixture:step:only:started".to_owned())
    );
}
