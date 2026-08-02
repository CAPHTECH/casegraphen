use super::*;
use crate::native_model::{
    CaseMorphism, CaseMorphismType, MorphismLogEntry, Projection, ProjectionAudience, ReviewAction,
    Revision, NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION,
    NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
};
use arbtest::arbitrary::Arbitrary;
use higher_graphen_core::{Provenance, SourceKind, SourceRef};
use serde_json::{json, Map, Value};

const NATIVE_EXAMPLE: &str =
    include_str!("../../schemas/casegraphen/native.case.space.example.json");

#[test]
fn native_example_evaluates_with_domain_findings() {
    let space: CaseSpace = serde_json::from_str(NATIVE_EXAMPLE).expect("native example");
    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:review-native-contract")));
    assert!(evaluation
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:native-schema-json-valid")));
    assert_eq!(evaluation.projection_loss.len(), 1);
}

#[test]
fn hard_dependencies_control_ready_and_frontier() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:ready",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:blocked",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:dep",
        CaseCellType::Work,
        CaseCellLifecycle::Accepted,
    ));
    space.case_relations.push(relation(
        "relation:blocked-depends-on-ready-dep",
        CaseRelationType::DependsOn,
        "work:blocked",
        "work:dep",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:ready")));
    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:blocked")));
    assert!(evaluation.frontier_cell_ids.contains(&id("work:ready")));
    assert!(evaluation.frontier_cell_ids.contains(&id("work:blocked")));
}

#[test]
fn soft_dependencies_do_not_block_native_readiness() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:soft-dependent",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:soft-dependency",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut soft = relation(
        "relation:soft-dependent-depends-on-soft-dependency",
        CaseRelationType::DependsOn,
        "work:soft-dependent",
        "work:soft-dependency",
    );
    soft.relation_strength = RelationStrength::Soft;
    space.case_relations.push(soft);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:soft-dependent")));
    assert!(!evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:soft-dependent")));
}

#[test]
fn completed_or_superseded_targets_are_removed_from_frontier() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:completed-target",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "event:completion",
        CaseCellType::Event,
        CaseCellLifecycle::Accepted,
    ));
    space.case_relations.push(relation(
        "relation:event-completes-work",
        CaseRelationType::Completes,
        "event:completion",
        "work:completed-target",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .ready_cell_ids
        .contains(&id("work:completed-target")));
    assert!(!evaluation
        .frontier_cell_ids
        .contains(&id("work:completed-target")));
}

/// Issue #28: a cell whose only blocking obstruction is `ExternalWait`
/// belongs in `waiting_cell_ids`.
#[test]
fn waiting_cell_ids_includes_a_cell_blocked_only_by_an_external_wait() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:waits-only",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "event:unresolved",
        CaseCellType::Event,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:waits-only-waits-for-event",
        CaseRelationType::WaitsFor,
        "work:waits-only",
        "event:unresolved",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:waits-only")));
    assert!(evaluation
        .readiness
        .waiting_cell_ids
        .contains(&id("work:waits-only")));
}

/// Issue #28's all-or-nothing rule: a cell that is waiting on an external
/// event *and* missing evidence is not purely waiting, and must not appear
/// in `waiting_cell_ids` — it surfaces through `needs_evidence` instead.
#[test]
fn waiting_cell_ids_excludes_a_cell_also_missing_evidence() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:mixed-blockers",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "event:mixed-wait",
        CaseCellType::Event,
        CaseCellLifecycle::Active,
    ));
    let mut missing_evidence = cell(
        "evidence:mixed-requirement",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    missing_evidence.source_ids.clear();
    space.case_cells.push(missing_evidence);
    space.case_relations.push(relation(
        "relation:mixed-waits-for-event",
        CaseRelationType::WaitsFor,
        "work:mixed-blockers",
        "event:mixed-wait",
    ));
    space.case_relations.push(relation(
        "relation:mixed-requires-evidence",
        CaseRelationType::RequiresEvidence,
        "work:mixed-blockers",
        "evidence:mixed-requirement",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:mixed-blockers")));
    assert!(!evaluation
        .readiness
        .waiting_cell_ids
        .contains(&id("work:mixed-blockers")));
}

/// The defect #28 fixes: the old derivation matched `"external-wait"` as a
/// substring of the *rendered obstruction id*, not the typed
/// `NativeObstructionType`. Here the cell has an already-satisfied wait
/// (so `wait_ids` is non-empty but no `ExternalWait` obstruction is
/// produced) and a *separate*, genuinely blocking `MissingEvidence`
/// obstruction whose witness id happens to contain the literal text
/// `external-wait` — which sanitizes straight into the generated
/// obstruction id. The old substring predicate misread that id and
/// classified this cell as waiting even though it has no `ExternalWait`
/// obstruction at all.
#[test]
fn waiting_cell_ids_is_not_fooled_by_an_obstruction_id_containing_external_wait_text() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:decoy",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    // Already accepted, so `wait_satisfied` is true and this wait never
    // produces a blocking `ExternalWait` obstruction — but it still
    // populates `wait_ids`, which is exactly what the old predicate keyed
    // its `!wait_ids.is_empty()` half on.
    space.case_cells.push(cell(
        "event:satisfied-wait",
        CaseCellType::Event,
        CaseCellLifecycle::Accepted,
    ));
    let mut missing_evidence = cell(
        "evidence:external-wait-decoy",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    missing_evidence.source_ids.clear();
    space.case_cells.push(missing_evidence);
    space.case_relations.push(relation(
        "relation:decoy-waits-for-satisfied-event",
        CaseRelationType::WaitsFor,
        "work:decoy",
        "event:satisfied-wait",
    ));
    space.case_relations.push(relation(
        "relation:decoy-requires-evidence",
        CaseRelationType::RequiresEvidence,
        "work:decoy",
        "evidence:external-wait-decoy",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    let decoy_obstructions = evaluation
        .obstructions
        .iter()
        .filter(|obstruction| obstruction.affected_ids.contains(&id("work:decoy")))
        .collect::<Vec<_>>();
    assert!(
        decoy_obstructions
            .iter()
            .any(|obstruction| obstruction.id.as_str().contains("external-wait")),
        "fixture must reproduce an obstruction id containing the literal text \
         external-wait: {decoy_obstructions:?}"
    );
    assert!(!decoy_obstructions
        .iter()
        .any(|obstruction| obstruction.obstruction_type == NativeObstructionType::ExternalWait));

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:decoy")));
    assert!(!evaluation
        .readiness
        .waiting_cell_ids
        .contains(&id("work:decoy")));
}

/// Issue #28's property, generalized: `waiting_cell_ids` must not depend on
/// the *text* of any id. The same structure (cell count, whether each cell
/// waits, and each cell's obstruction types/blocking flags) is rendered
/// twice — once with cell ids, wait target ids, and obstruction witness ids
/// that contain the literal text `external-wait` (the exact text
/// `obstruction_type_stem(ExternalWait)` renders to), and once with
/// structurally identical but neutral ids. Renaming a structure must not
/// change which of its cells are classified as waiting: mapping each
/// rendering's `waiting_cell_ids` back to cell index must produce the same
/// set. This catches any future dependence on id text, not only the one
/// shape `waiting_cell_ids_is_not_fooled_by_an_obstruction_id_containing_external_wait_text`
/// above pins down concretely.
#[test]
fn waiting_cell_ids_is_invariant_under_renaming_that_preserves_structure() {
    struct CellSpec {
        has_wait: bool,
        obstructions: Vec<(NativeObstructionType, bool)>,
    }

    fn render(specs: &[CellSpec], poisoned: bool) -> (Vec<CellEvaluation>, Vec<Id>) {
        let mut results = Vec::new();
        let mut cell_ids = Vec::new();
        for (cell_index, spec) in specs.iter().enumerate() {
            let cell_id = if poisoned {
                id(&format!("work:external-wait-{cell_index}"))
            } else {
                id(&format!("work:cell-{cell_index}"))
            };
            let wait_ids = if spec.has_wait {
                vec![if poisoned {
                    id(&format!("event:external-wait-{cell_index}"))
                } else {
                    id(&format!("event:wait-{cell_index}"))
                }]
            } else {
                Vec::new()
            };
            let obstructions = spec
                .obstructions
                .iter()
                .enumerate()
                .map(|(witness_index, (obstruction_type, blocking))| {
                    let witness_id = if poisoned {
                        id(&format!(
                            "witness:external-wait-{cell_index}-{witness_index}"
                        ))
                    } else {
                        id(&format!("witness:{cell_index}-{witness_index}"))
                    };
                    let mut built = obstruction(
                        *obstruction_type,
                        &cell_id,
                        &witness_id,
                        "constraint:property-test",
                        "generated".to_owned(),
                        Severity::Medium,
                        "resolve",
                    );
                    built.blocking = *blocking;
                    built
                })
                .collect::<Vec<_>>();
            cell_ids.push(cell_id.clone());
            results.push(CellEvaluation {
                cell_id,
                lifecycle: CaseCellLifecycle::Active,
                hard_dependency_ids: Vec::new(),
                wait_ids,
                evidence_requirement_ids: Vec::new(),
                proof_requirement_ids: Vec::new(),
                obstructions,
                rule_results: Vec::new(),
            });
        }
        (results, cell_ids)
    }

    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let cell_count = u.int_in_range(1_usize..=4)?;
            let mut specs = Vec::new();
            for _ in 0..cell_count {
                let has_wait = bool::arbitrary(u)?;
                let obstruction_count = u.int_in_range(0_usize..=3)?;
                let mut obstructions = Vec::new();
                for _ in 0..obstruction_count {
                    let obstruction_type = *u.choose(&[
                        NativeObstructionType::UnresolvedDependency,
                        NativeObstructionType::ExternalWait,
                        NativeObstructionType::MissingEvidence,
                        NativeObstructionType::MissingProof,
                        NativeObstructionType::Contradiction,
                        NativeObstructionType::ReviewRequired,
                    ])?;
                    let blocking = bool::arbitrary(u)?;
                    obstructions.push((obstruction_type, blocking));
                }
                specs.push(CellSpec {
                    has_wait,
                    obstructions,
                });
            }

            let (poisoned_results, poisoned_ids) = render(&specs, true);
            let (neutral_results, neutral_ids) = render(&specs, false);

            let poisoned_waiting =
                readiness_result(&fixture_space(), &poisoned_results).waiting_cell_ids;
            let neutral_waiting =
                readiness_result(&fixture_space(), &neutral_results).waiting_cell_ids;

            // Compare by cell index, not by id text, so the renaming itself
            // cannot be mistaken for a difference.
            let index_of = |cell_ids: &[Id], target: &Id| -> usize {
                cell_ids
                    .iter()
                    .position(|candidate| candidate == target)
                    .expect("waiting id must be one of the generated cells")
            };
            let poisoned_waiting_indices = poisoned_waiting
                .iter()
                .map(|waiting_id| index_of(&poisoned_ids, waiting_id))
                .collect::<BTreeSet<_>>();
            let neutral_waiting_indices = neutral_waiting
                .iter()
                .map(|waiting_id| index_of(&neutral_ids, waiting_id))
                .collect::<BTreeSet<_>>();

            assert_eq!(poisoned_waiting_indices, neutral_waiting_indices);
            Ok(())
        },
    );
}

#[test]
fn missing_evidence_and_proof_block_readiness() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "proof:obligation",
        CaseCellType::Proof,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:needs-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs",
        "evidence:missing",
    ));
    space.case_relations.push(relation(
        "relation:needs-proof",
        CaseRelationType::RequiresProof,
        "work:needs",
        "proof:obligation",
    ));
    refresh_morphism(&mut space);

    let err = evaluate_native_case(&space).expect_err("dangling evidence is malformed");
    assert!(err
        .violations
        .iter()
        .any(|violation| violation.code == NativeEvalViolationCode::DanglingReference));

    let mut missing_evidence = cell(
        "evidence:missing",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    missing_evidence.source_ids.clear();
    space.case_cells.push(missing_evidence);
    refresh_morphism(&mut space);
    let evaluation = evaluate_native_case(&space).expect("evaluation");
    assert!(evaluation
        .obstructions
        .iter()
        .any(|obstruction| obstruction.obstruction_type == NativeObstructionType::MissingEvidence));
    assert!(evaluation
        .obstructions
        .iter()
        .any(|obstruction| obstruction.obstruction_type == NativeObstructionType::MissingProof));
}

/// A `satisfies_evidence_requirement` relation only mints coverage when the
/// morphism that carries it says so (`sections::canonical_evidence_coverage`
/// reads `metadata.payload.added_relations`, not the graph edge itself). This
/// mirrors what a real `evidence attach --satisfies` records for genesis, and
/// is shared by every test below that needs one holder's requirement
/// discharged through coverage of a *different* cell (issue #34's exact
/// mechanism) rather than through the requirement's own trust.
fn record_coverage(space: &mut CaseSpace, covering_id: &str, target_id: &str) {
    let coverage = relation(
        &format!("relation:{covering_id}-satisfies-{target_id}"),
        CaseRelationType::SatisfiesEvidenceRequirement,
        covering_id,
        target_id,
    );
    space.case_relations.push(coverage.clone());
    let mut added_relations = space.morphism_log[0]
        .morphism
        .metadata
        .get("payload")
        .and_then(|payload| payload.get("added_relations"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    added_relations
        .as_array_mut()
        .expect("added_relations is an array")
        .push(serde_json::to_value(&coverage).expect("relation serializes"));
    space.morphism_log[0].morphism.metadata.insert(
        "payload".to_owned(),
        json!({ "added_relations": added_relations }),
    );
}

fn source_backed_evidence(id_value: &str) -> CaseCell {
    let mut evidence = cell(id_value, CaseCellType::Evidence, CaseCellLifecycle::Active);
    evidence
        .metadata
        .insert("evidence_boundary".to_owned(), json!("source_backed"));
    evidence
}

/// Issue #34's exact reproduction, at the evaluator level:
/// `work:w1` --hard `requires_evidence`--> `evidence:a` --hard
/// `requires_evidence`--> `evidence:x`; `work:w2` --hard
/// `requires_evidence`--> `evidence:x` directly. Trusted source-backed
/// `evidence:y` covers `evidence:a` only, never `evidence:x`. The shipped
/// union marked `evidence:x` satisfied via holder `evidence:a` even though
/// `work:w2`'s own requirement of it was never covered.
/// `docs/specs/requirement-satisfaction.fsl`'s `satisfied_for_all()` is the
/// fix: `compute_satisfied_requirement_ids` now requires every holder.
#[test]
fn compute_satisfied_requirement_ids_requires_every_holder() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:w1",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:w2",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence_a = cell(
        "evidence:a",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence_a.provenance.review_status = ReviewStatus::Unreviewed;
    space.case_cells.push(evidence_a);
    let mut evidence_x = cell(
        "evidence:x",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence_x.provenance.review_status = ReviewStatus::Unreviewed;
    space.case_cells.push(evidence_x);
    space.case_cells.push(source_backed_evidence("evidence:y"));

    space.case_relations.push(relation(
        "relation:w1-requires-a",
        CaseRelationType::RequiresEvidence,
        "work:w1",
        "evidence:a",
    ));
    space.case_relations.push(relation(
        "relation:a-requires-x",
        CaseRelationType::RequiresEvidence,
        "evidence:a",
        "evidence:x",
    ));
    space.case_relations.push(relation(
        "relation:w2-requires-x",
        CaseRelationType::RequiresEvidence,
        "work:w2",
        "evidence:x",
    ));
    record_coverage(&mut space, "evidence:y", "evidence:a");
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    // work:w2's own requirement of evidence:x is genuinely unsatisfied —
    // the readiness layer was never wrong about this.
    assert!(
        evaluation.obstructions.iter().any(|obstruction| {
            obstruction.obstruction_type == NativeObstructionType::MissingEvidence
                && obstruction.witness_ids == vec![id("evidence:x")]
                && obstruction.affected_ids == vec![id("work:w2")]
        }),
        "obstructions: {:?}",
        evaluation.obstructions
    );

    // With every-holder scoping, evidence:x's own gap must not read
    // satisfied just because a different holder (evidence:a) is covered.
    let x_gap = evaluation
        .review_gaps
        .iter()
        .find(|gap| gap.target_id == id("evidence:x"))
        .expect("evidence:x review gap");
    assert!(
        !x_gap.requirement_satisfied,
        "evidence:x gap: {x_gap:?}, full evaluation obstructions: {:?}",
        evaluation.obstructions
    );
}

/// Reproduces the invariant-duplication-auditor's Finding 1: before the fix,
/// `compute_satisfied_requirement_ids` ranged over *all* cells while
/// obstruction production (`evaluate_cells`, via `readiness_subject`) ranges
/// only over readiness subjects. Under the pre-#34 union that asymmetry could
/// only add satisfaction; after #34's every-holder rule it could only
/// subtract it — a second holder leaving the readiness-subject set (by
/// retiring or resolving, the ordinary way work finishes) silently dropped
/// out of the holder set `compute_satisfied_requirement_ids` counted too, so
/// a requirement started reading satisfied with nothing in the case space
/// explaining why the obstruction that used to gate it disappeared: zero
/// obstructions anywhere, yet the close check still failed and assurance
/// still read `review_required`, naming no reason. The fix (filtering
/// `compute_satisfied_requirement_ids`'s holder scan through
/// `readiness_subject`, matching `evaluate_cells`) makes the two ranges agree
/// again.
///
/// `work:w1` and `work:w2` both hard-`requires_evidence` -> `evidence:x`;
/// source-backed `evidence:y` covers `work:w1` only (never `evidence:x`
/// itself, never `work:w2`). With `work:w2` `Active`, its own requirement of
/// `evidence:x` is genuinely unsatisfied and must still gate the axis. With
/// `work:w2` `Retired` or `Resolved`, it leaves the readiness-subject set and
/// `evidence:x` is satisfied for every remaining holder (`work:w1`, via
/// coverage of `work:w1`) — closable on merit, not silently.
#[test]
fn second_holder_leaving_the_readiness_subject_set_changes_whether_the_shared_requirement_reads_satisfied(
) {
    for (
        second_holder_lifecycle,
        expect_missing_evidence,
        expect_satisfied,
        expect_gaps_closed,
        expect_assurance,
    ) in [
        (
            CaseCellLifecycle::Active,
            1,
            false,
            false,
            NativeAssurance::ReviewRequired,
        ),
        (
            CaseCellLifecycle::Retired,
            0,
            true,
            true,
            NativeAssurance::Unreviewed,
        ),
        (
            CaseCellLifecycle::Resolved,
            0,
            true,
            true,
            NativeAssurance::Unreviewed,
        ),
    ] {
        let mut space = fixture_space();
        space.case_cells.push(cell(
            "work:w1",
            CaseCellType::Work,
            CaseCellLifecycle::Active,
        ));
        space
            .case_cells
            .push(cell("work:w2", CaseCellType::Work, second_holder_lifecycle));
        let mut evidence_x = cell(
            "evidence:x",
            CaseCellType::Evidence,
            CaseCellLifecycle::Active,
        );
        evidence_x.provenance.review_status = ReviewStatus::Unreviewed;
        space.case_cells.push(evidence_x);
        space.case_cells.push(source_backed_evidence("evidence:y"));

        space.case_relations.push(relation(
            "relation:w1-requires-x",
            CaseRelationType::RequiresEvidence,
            "work:w1",
            "evidence:x",
        ));
        space.case_relations.push(relation(
            "relation:w2-requires-x",
            CaseRelationType::RequiresEvidence,
            "work:w2",
            "evidence:x",
        ));
        record_coverage(&mut space, "evidence:y", "work:w1");
        refresh_morphism(&mut space);

        let evaluation = evaluate_native_case(&space).expect("evaluation");

        let missing_evidence_obstructions = evaluation
            .obstructions
            .iter()
            .filter(|obstruction| {
                obstruction.obstruction_type == NativeObstructionType::MissingEvidence
                    && obstruction.witness_ids == vec![id("evidence:x")]
            })
            .count();
        assert_eq!(
            missing_evidence_obstructions, expect_missing_evidence,
            "second_holder_lifecycle={second_holder_lifecycle:?}: obstructions={:?}",
            evaluation.obstructions
        );

        let x_satisfied = NativeEvaluationContext::new(&space)
            .satisfied_requirement_ids
            .contains("evidence:x");
        assert_eq!(
            x_satisfied, expect_satisfied,
            "second_holder_lifecycle={second_holder_lifecycle:?}"
        );

        let gaps_closed = evaluation
            .close_check
            .invariant_results
            .iter()
            .find(|result| result.invariant_id == id("close:native-review-gaps-closed"))
            .expect("review-gaps-closed invariant present")
            .passed;
        assert_eq!(
            gaps_closed, expect_gaps_closed,
            "second_holder_lifecycle={second_holder_lifecycle:?}"
        );

        assert_eq!(
            evaluation.assurance, expect_assurance,
            "second_holder_lifecycle={second_holder_lifecycle:?}"
        );
    }
}

/// `docs/specs/requirement-satisfaction.fsl`'s `INV-EVID-002`: a requirement
/// nothing holds a hard `requires_evidence` edge into is not satisfied, even
/// though it is itself trusted. An unguarded `forall` over holders would be
/// vacuously true here — there being no holder is exactly what the guard
/// exists to catch.
#[test]
fn a_requirement_nobody_holds_is_not_satisfied() {
    let mut space = fixture_space();
    space
        .case_cells
        .push(source_backed_evidence("evidence:unheld"));
    refresh_morphism(&mut space);

    let context = NativeEvaluationContext::new(&space);

    assert!(!context
        .satisfied_requirement_ids
        .contains("evidence:unheld"));
}

/// `docs/specs/requirement-satisfaction.fsl`'s `INV-EVID-003`: with exactly
/// one holder, the old union and the new every-holder rule are the same
/// answer, in both directions — this is the compatibility claim that keeps
/// every existing single-holder fixture unmoved by #34.
#[test]
fn a_single_holder_requirement_reads_identically_satisfied_or_not() {
    let mut satisfied = fixture_space();
    satisfied.case_cells.push(cell(
        "work:single-holder-satisfied",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    satisfied
        .case_cells
        .push(source_backed_evidence("evidence:single-satisfied"));
    satisfied.case_relations.push(relation(
        "relation:single-holder-requires-satisfied",
        CaseRelationType::RequiresEvidence,
        "work:single-holder-satisfied",
        "evidence:single-satisfied",
    ));
    refresh_morphism(&mut satisfied);
    let satisfied_context = NativeEvaluationContext::new(&satisfied);
    assert!(satisfied_context
        .satisfied_requirement_ids
        .contains("evidence:single-satisfied"));

    let mut unsatisfied = fixture_space();
    unsatisfied.case_cells.push(cell(
        "work:single-holder-unsatisfied",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut requirement = cell(
        "evidence:single-unsatisfied",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    requirement.source_ids.clear();
    unsatisfied.case_cells.push(requirement);
    unsatisfied.case_relations.push(relation(
        "relation:single-holder-requires-unsatisfied",
        CaseRelationType::RequiresEvidence,
        "work:single-holder-unsatisfied",
        "evidence:single-unsatisfied",
    ));
    refresh_morphism(&mut unsatisfied);
    let unsatisfied_context = NativeEvaluationContext::new(&unsatisfied);
    assert!(!unsatisfied_context
        .satisfied_requirement_ids
        .contains("evidence:single-unsatisfied"));
}

/// `docs/specs/requirement-satisfaction.fsl`'s `INV-EVID-001`
/// (`HolderSoundness`) and `INV-EVID-004` (`NoUnexplainedUnsatisfied`), the
/// two properties every consumer of `requirement_satisfied` assumes without
/// checking, generated over holders that mix readiness subjects and
/// non-subjects. The original generator drew only `CaseCellType::Work`
/// holders at `CaseCellLifecycle::Active`, so every generated holder was a
/// readiness subject and the shape the invariant-duplication audit's Finding
/// 1 exploited — a non-subject holder's unsatisfied edge silently changing
/// the requirement's reading — could never be drawn. Holders here also vary
/// by cell type (`Evidence`/`Review`/`Projection` are non-subjects by type;
/// `Work`/`Goal` are subjects) and by lifecycle (`Resolved`/`Retired`/
/// `Rejected`/`Superseded`/`Accepted` are non-subject lifecycles;
/// `Active`/`Proposed`/`Waiting` are subject lifecycles), independently of
/// whether the requirement is discharged for that holder.
#[test]
fn satisfied_requirement_ids_never_names_a_requirement_with_a_blocking_missing_evidence_obstruction(
) {
    const HOLDER_CELL_TYPES: [CaseCellType; 5] = [
        CaseCellType::Work,
        CaseCellType::Goal,
        CaseCellType::Evidence,
        CaseCellType::Review,
        CaseCellType::Projection,
    ];
    const HOLDER_LIFECYCLES: [CaseCellLifecycle; 8] = [
        CaseCellLifecycle::Active,
        CaseCellLifecycle::Proposed,
        CaseCellLifecycle::Waiting,
        CaseCellLifecycle::Resolved,
        CaseCellLifecycle::Retired,
        CaseCellLifecycle::Rejected,
        CaseCellLifecycle::Superseded,
        CaseCellLifecycle::Accepted,
    ];

    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let mut space = fixture_space();
            let requirement_id = "evidence:requirement";
            let mut requirement = cell(
                requirement_id,
                CaseCellType::Evidence,
                CaseCellLifecycle::Active,
            );
            requirement.source_ids.clear();
            space.case_cells.push(requirement);

            let holder_count = u.int_in_range(1_usize..=4)?;
            // Tracks the FSL's `counts(h)` guard on `INV-EVID-004`: whether
            // any generated holder is a readiness subject at all. Every
            // holder here holds the requirement's edge unconditionally, so
            // `counts(h) == is_subject(h)` for this generator.
            let mut any_subject_holder = false;
            for holder_index in 0..holder_count {
                let holder_id = format!("holder-{holder_index}");
                let holder_type = HOLDER_CELL_TYPES
                    [u.int_in_range(0_usize..=HOLDER_CELL_TYPES.len() - 1)?]
                .clone();
                let holder_lifecycle =
                    HOLDER_LIFECYCLES[u.int_in_range(0_usize..=HOLDER_LIFECYCLES.len() - 1)?];
                let holder_cell = cell(&holder_id, holder_type, holder_lifecycle);
                any_subject_holder |= readiness_subject(&holder_cell);
                space.case_cells.push(holder_cell);
                space.case_relations.push(relation(
                    &format!("relation:{holder_id}-requires-requirement"),
                    CaseRelationType::RequiresEvidence,
                    &holder_id,
                    requirement_id,
                ));
                // 0 = uncovered, 1 = covered via the requirement's own trust
                // (satisfies every holder alike), 2 = covered via a holder-
                // specific coverage claim (satisfies only this holder).
                match u.int_in_range(0_u8..=2)? {
                    1 => {
                        let covering_id = format!("evidence:covers-requirement-{holder_index}");
                        space.case_cells.push(source_backed_evidence(&covering_id));
                        record_coverage(&mut space, &covering_id, requirement_id);
                    }
                    2 => {
                        let covering_id = format!("evidence:covers-holder-{holder_index}");
                        space.case_cells.push(source_backed_evidence(&covering_id));
                        record_coverage(&mut space, &covering_id, &holder_id);
                    }
                    _ => {}
                }
            }
            refresh_morphism(&mut space);

            let evaluation = evaluate_native_case(&space).expect("evaluation");

            let satisfied = evaluation
                .review_gaps
                .iter()
                .any(|gap| gap.target_id == id(requirement_id) && gap.requirement_satisfied)
                || NativeEvaluationContext::new(&space)
                    .satisfied_requirement_ids
                    .contains(requirement_id);
            let has_blocking_missing_evidence = evaluation.obstructions.iter().any(|obstruction| {
                obstruction.obstruction_type == NativeObstructionType::MissingEvidence
                    && obstruction.blocking
                    && obstruction.witness_ids == vec![id(requirement_id)]
            });

            // INV-EVID-001 (HolderSoundness): a satisfied requirement leaves
            // no holder blocked on it.
            assert!(
                !(satisfied && has_blocking_missing_evidence),
                "requirement marked satisfied while a holder was still blocked on it: \
                 obstructions={:?}, review_gaps={:?}",
                evaluation.obstructions,
                evaluation.review_gaps
            );

            // INV-EVID-004 (NoUnexplainedUnsatisfied), the direction Finding
            // 1's defect broke: if some readiness-subject holder holds this
            // requirement and it still reads unsatisfied, some
            // readiness-subject holder must be blocked on it — an
            // obstruction exists explaining why. Without the fix, a
            // non-subject holder's unsatisfied edge could make the left side
            // true while nothing on the right explained it.
            assert!(
                !any_subject_holder || satisfied || has_blocking_missing_evidence,
                "requirement read unsatisfied with a readiness-subject holder present, but no \
                 blocking obstruction explains it: obstructions={:?}, review_gaps={:?}",
                evaluation.obstructions,
                evaluation.review_gaps
            );
            Ok(())
        },
    );
}

#[test]
fn inferred_evidence_does_not_satisfy_requirement() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:ai-guess",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-ai-guess",
        CaseRelationType::RequiresEvidence,
        "work:needs-evidence",
        "evidence:ai-guess",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-evidence")));
    assert!(evaluation
        .evidence_findings
        .unreviewed_inference_ids
        .contains(&id("evidence:ai-guess")));
}

#[test]
fn canonical_review_accept_flips_inferred_evidence_findings_and_assurance() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:log-accepted-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let before = evaluate_native_case(&space).expect("evaluation before review");
    assert!(before
        .evidence_findings
        .unreviewed_inference_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(!before
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(before.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedInference
            && gap.target_id == id("evidence:log-accepted-inference")
    }));
    assert_ne!(before.assurance, NativeAssurance::Accepted);

    // The stored `provenance.review_status` never moves — review morphisms
    // have empty `updated_ids` by design — so only the log carries the
    // acceptance. This mirrors how `native_review::accept_review_morphism`
    // builds the canonical review the CLI's `review accept` appends.
    let review = crate::native_review::accept_review_morphism(
        &space,
        crate::native_review::NativeReviewRequest {
            target_kind: crate::native_review::NativeReviewTargetKind::Evidence,
            target_id: id("evidence:log-accepted-inference"),
            action: ReviewAction::Accept,
            reviewer_id: id("reviewer:native-eval-test"),
            reviewed_at: "2026-04-26T00:30:00Z".to_owned(),
            reason: "Reviewed during the evidence-findings divergence test.".to_owned(),
            evidence_ids: vec![id("evidence:log-accepted-inference")],
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:log-accepted-review"),
        },
    )
    .expect("canonical accept");
    append_review_entry(&mut space, review, "entry:log-accepted-review");
    assert!(
        space
            .case_cells
            .iter()
            .any(|cell| cell.id == id("evidence:log-accepted-inference")
                && cell.provenance.review_status == ReviewStatus::Unreviewed),
        "the stored provenance must stay unreviewed; only the log carries the acceptance"
    );

    let after = evaluate_native_case(&space).expect("evaluation after review");
    assert!(
        !after
            .evidence_findings
            .unreviewed_inference_ids
            .contains(&id("evidence:log-accepted-inference")),
        "an accepted-by-review inferred claim must stop reading as unreviewed"
    );
    assert!(after
        .evidence_findings
        .accepted_evidence_ids
        .contains(&id("evidence:log-accepted-inference")));
    assert!(!after.review_gaps.iter().any(|gap| {
        gap.gap_type == NativeReviewGapType::UnreviewedInference
            && gap.target_id == id("evidence:log-accepted-inference")
    }));
    assert_eq!(after.assurance, NativeAssurance::Accepted);
}

#[test]
fn canonical_review_reject_produces_rejected_evidence_boundary_violation() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:log-rejected-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let review = crate::native_review::reject_review_morphism(
        &space,
        crate::native_review::NativeReviewRequest {
            target_kind: crate::native_review::NativeReviewTargetKind::Evidence,
            target_id: id("evidence:log-rejected-inference"),
            action: ReviewAction::Reject,
            reviewer_id: id("reviewer:native-eval-test"),
            reviewed_at: "2026-04-26T00:30:00Z".to_owned(),
            reason: "Reviewed and rejected during the evidence-findings divergence test."
                .to_owned(),
            evidence_ids: vec![id("evidence:log-rejected-inference")],
            source_ids: vec![id("source:test")],
            target_revision_id: id("revision:log-rejected-review"),
        },
    )
    .expect("canonical reject");
    append_review_entry(&mut space, review, "entry:log-rejected-review");

    let after = evaluate_native_case(&space).expect("evaluation after rejection");
    assert!(after
        .evidence_findings
        .boundary_violations
        .iter()
        .any(|violation| violation.violation_type
            == NativeEvidenceBoundaryViolationType::RejectedEvidenceUsed
            && violation.evidence_id == id("evidence:log-rejected-inference")));
    assert_eq!(after.assurance, NativeAssurance::Rejected);
}

#[test]
fn evidence_without_explicit_boundary_does_not_satisfy_hard_requirement() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-explicit-boundary",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:document-without-boundary",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Document, ReviewStatus::Accepted);
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-explicit-boundary",
        CaseRelationType::RequiresEvidence,
        "work:needs-explicit-boundary",
        "evidence:document-without-boundary",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(evaluation
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-explicit-boundary")));
    assert!(evaluation
        .evidence_findings
        .inference_record_ids
        .contains(&id("evidence:document-without-boundary")));
}

#[test]
fn review_promoted_evidence_requires_accepted_review() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-promoted-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:pending-promotion",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Human, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("review_promoted".to_owned()),
    );
    space.case_cells.push(evidence);
    space.case_relations.push(relation(
        "relation:needs-promoted-evidence",
        CaseRelationType::RequiresEvidence,
        "work:needs-promoted-evidence",
        "evidence:pending-promotion",
    ));
    refresh_morphism(&mut space);

    let pending = evaluate_native_case(&space).expect("pending evaluation");
    assert!(pending
        .readiness
        .blocked_cell_ids
        .contains(&id("work:needs-promoted-evidence")));

    let promoted = space
        .case_cells
        .iter_mut()
        .find(|cell| cell.id == id("evidence:pending-promotion"))
        .expect("promoted evidence");
    promoted.provenance = provenance(SourceKind::Human, ReviewStatus::Accepted);
    let accepted = evaluate_native_case(&space).expect("accepted evaluation");
    assert!(accepted
        .readiness
        .ready_cell_ids
        .contains(&id("work:needs-promoted-evidence")));
}

#[test]
fn caller_declared_native_review_promotion_is_not_acceptable_without_review() {
    let mut space = fixture_space();
    let mut evidence = cell(
        "evidence:caller-promoted",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    evidence.provenance = provenance(SourceKind::Human, ReviewStatus::Unreviewed);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("review_promoted".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);
    let evidence = space
        .case_cells
        .iter()
        .find(|cell| cell.id == id("evidence:caller-promoted"))
        .expect("caller-promoted evidence");

    assert!(!crate::evidence_trust::evidence_is_acceptable(
        evidence_trust_input(&space, evidence)
    ));
}

#[test]
fn projection_loss_and_evolution_summaries_are_reported() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:kept",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.projections.push(Projection {
        projection_id: id("projection:lossy"),
        audience: ProjectionAudience::AiAgent,
        revision_id: space.revision.revision_id.clone(),
        represented_cell_ids: Vec::new(),
        represented_relation_ids: Vec::new(),
        omitted_cell_ids: vec![id("work:kept")],
        omitted_relation_ids: Vec::new(),
        information_loss: vec![crate::native_model::ProjectionLoss {
            description: "AI projection hides work cell.".to_owned(),
            represented_ids: Vec::new(),
            omitted_ids: vec![id("work:kept")],
        }],
        allowed_operations: Vec::new(),
        source_ids: Vec::new(),
        warnings: vec![crate::native_model::ProjectionWarning::InformationLoss],
        metadata: Map::new(),
    });
    refresh_morphism(&mut space);
    space.morphism_log[0].morphism.violated_invariant_ids = vec![id("invariant:loss-disclosed")];

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.projection_loss.len(), 1);
    assert_eq!(evaluation.evolution.invariant_breaks.len(), 1);
}

#[test]
fn projection_revision_and_loss_references_are_validated() {
    let mut space = fixture_space();
    space.projections.push(Projection {
        projection_id: id("projection:stale"),
        audience: ProjectionAudience::AiAgent,
        revision_id: id("revision:missing"),
        represented_cell_ids: Vec::new(),
        represented_relation_ids: Vec::new(),
        omitted_cell_ids: Vec::new(),
        omitted_relation_ids: Vec::new(),
        information_loss: vec![crate::native_model::ProjectionLoss {
            description: "Stale projection references missing loss ids.".to_owned(),
            represented_ids: Vec::new(),
            omitted_ids: vec![id("work:missing")],
        }],
        allowed_operations: Vec::new(),
        source_ids: Vec::new(),
        warnings: Vec::new(),
        metadata: Map::new(),
    });
    refresh_morphism(&mut space);

    let err = evaluate_native_case(&space).expect_err("invalid projection references");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::DanglingReference
            && violation.field == "projection.revision_id"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::DanglingReference
            && violation.field == "projection.information_loss.ids"
    }));
}

#[test]
fn close_check_is_blocked_by_obstructions_and_review_gaps() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:needs-evidence",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut placeholder = cell(
        "evidence:placeholder",
        CaseCellType::Evidence,
        CaseCellLifecycle::Proposed,
    );
    placeholder.source_ids.clear();
    space.case_cells.push(placeholder);
    space.case_relations.push(relation(
        "relation:needs-placeholder",
        CaseRelationType::RequiresEvidence,
        "work:needs-evidence",
        "evidence:placeholder",
    ));
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert!(!evaluation.close_check.closable);
    assert!(evaluation
        .close_check
        .invariant_results
        .iter()
        .any(|result| !result.passed));
}

#[test]
fn invalid_morphism_is_structured_error() {
    let mut space = fixture_space();
    space.morphism_log[0].morphism_id = id("morphism:outer");
    space.morphism_log[0].morphism.morphism_id = id("morphism:inner");

    let err = evaluate_native_case(&space).expect_err("invalid morphism");

    assert!(err
        .violations
        .iter()
        .any(|violation| violation.code == NativeEvalViolationCode::InvalidMorphism));
}

#[test]
fn empty_morphism_log_is_structured_error() {
    let mut space = fixture_space();
    space.morphism_log.clear();

    let err = evaluate_native_case(&space).expect_err("empty morphism log");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "morphism_log"
    }));
}

#[test]
fn invalid_log_continuity_and_entry_version_are_structured_errors() {
    let mut space = fixture_space();
    space.morphism_log[0].schema_version = 2;
    space.morphism_log[0].source_revision_id = Some(id("revision:unexpected-parent"));
    space.morphism_log[0].morphism.source_revision_id = Some(id("revision:unexpected-parent"));

    let err = evaluate_native_case(&space).expect_err("invalid log contract");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::UnsupportedSchemaVersion
            && violation.field == "schema_version"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "source_revision_id"
    }));
}

#[test]
fn materialized_revision_must_match_latest_log_checksum_and_case_space() {
    let mut space = fixture_space();
    space.revision.case_space_id = id("case_space:other");
    space.revision.checksum = "sha256:stale".to_owned();

    let err = evaluate_native_case(&space).expect_err("invalid revision materialization");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "revision.case_space_id"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::InvalidMorphismLog
            && violation.field == "revision.checksum"
    }));
}

#[test]
fn case_space_source_boundary_is_required() {
    let mut space = fixture_space();
    space.metadata.remove("source_boundary");

    let err = evaluate_native_case(&space).expect_err("missing source boundary");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "metadata.source_boundary"
    }));
}

#[test]
fn genesis_morphism_must_preserve_lift_boundary() {
    let mut space = fixture_space();
    space.morphism_log[0].morphism.metadata.clear();

    let err = evaluate_native_case(&space).expect_err("missing lift boundary");

    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "morphism.metadata.lift_semantics"
    }));
    assert!(err.violations.iter().any(|violation| {
        violation.code == NativeEvalViolationCode::EmptyRequiredField
            && violation.field == "morphism.metadata.source_boundary"
    }));
}

#[test]
fn blocked_work_and_review_gap_are_both_reported() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:blocked",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_cells.push(cell(
        "work:dependency",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    space.case_relations.push(relation(
        "relation:blocked-depends-on-dependency",
        CaseRelationType::DependsOn,
        "work:blocked",
        "work:dependency",
    ));
    let mut inference = cell(
        "evidence:unreviewed-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    inference.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    inference.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(inference);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Blocked);
    assert_eq!(evaluation.assurance, NativeAssurance::ReviewRequired);
}

#[test]
fn resolved_space_reports_complete_progress_and_pending_assurance() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:done",
        CaseCellType::Work,
        CaseCellLifecycle::Resolved,
    ));
    let mut inference = cell(
        "evidence:unreviewed-inference",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    inference.provenance = provenance(SourceKind::Ai, ReviewStatus::Unreviewed);
    inference.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("inferred".to_owned()),
    );
    space.case_cells.push(inference);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Complete);
    assert_eq!(evaluation.assurance, NativeAssurance::ReviewRequired);
}

#[test]
fn rejected_evidence_drives_assurance_to_rejected() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:open",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut rejected = cell(
        "evidence:rejected",
        CaseCellType::Evidence,
        CaseCellLifecycle::Active,
    );
    rejected.provenance = provenance(SourceKind::Human, ReviewStatus::Rejected);
    rejected.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("rejected".to_owned()),
    );
    space.case_cells.push(rejected);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.assurance, NativeAssurance::Rejected);
}

#[test]
fn clean_accepted_evidence_reports_accepted_assurance() {
    let mut space = fixture_space();
    space.case_cells.push(cell(
        "work:open",
        CaseCellType::Work,
        CaseCellLifecycle::Active,
    ));
    let mut evidence = cell(
        "evidence:accepted",
        CaseCellType::Evidence,
        CaseCellLifecycle::Accepted,
    );
    evidence.provenance = provenance(SourceKind::Document, ReviewStatus::Accepted);
    evidence.metadata.insert(
        "evidence_boundary".to_owned(),
        Value::String("source_backed".to_owned()),
    );
    space.case_cells.push(evidence);
    refresh_morphism(&mut space);

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Active);
    assert_eq!(evaluation.assurance, NativeAssurance::Accepted);
}

#[test]
fn empty_space_is_active_and_unreviewed() {
    let space = fixture_space();

    let evaluation = evaluate_native_case(&space).expect("evaluation");

    assert_eq!(evaluation.progress, NativeProgress::Active);
    assert_eq!(evaluation.assurance, NativeAssurance::Unreviewed);
}

#[test]
fn assurance_axis_ignores_a_gap_marked_requirement_satisfied() {
    // The requirement-placeholder pattern
    // (`skills/casegraphen-operate/references/authoring.md`): an evidence cell
    // that exists only so a hard `requires_evidence` edge has something to
    // point at. It never becomes reviewed itself, so its own
    // `UnreviewedInference` gap must stop driving the axis once
    // `sections::review_gaps` has marked it `requirement_satisfied` at
    // production time.
    let gap = NativeReviewGap {
        id: id("review_gap:placeholder-inference"),
        target_id: id("evidence:placeholder"),
        gap_type: NativeReviewGapType::UnreviewedInference,
        explanation: "placeholder".to_owned(),
        requirement_satisfied: true,
    };
    let evidence_findings = NativeEvidenceFindings {
        accepted_evidence_ids: vec![id("evidence:claim")],
        source_backed_evidence_ids: Vec::new(),
        inference_record_ids: vec![id("evidence:placeholder")],
        unreviewed_inference_ids: vec![id("evidence:placeholder")],
        promoted_evidence_ids: Vec::new(),
        boundary_violations: Vec::new(),
        findings: Vec::new(),
    };

    let assurance = assurance_axis(&[], &evidence_findings, &[gap]);

    assert_eq!(assurance, NativeAssurance::Accepted);
}

#[test]
fn assurance_axis_still_requires_review_for_an_unmarked_gap() {
    let gap = NativeReviewGap {
        id: id("review_gap:placeholder-inference"),
        target_id: id("evidence:placeholder"),
        gap_type: NativeReviewGapType::UnreviewedInference,
        explanation: "placeholder".to_owned(),
        requirement_satisfied: false,
    };
    let evidence_findings = NativeEvidenceFindings {
        accepted_evidence_ids: vec![id("evidence:claim")],
        source_backed_evidence_ids: Vec::new(),
        inference_record_ids: vec![id("evidence:placeholder")],
        unreviewed_inference_ids: vec![id("evidence:placeholder")],
        promoted_evidence_ids: Vec::new(),
        boundary_violations: Vec::new(),
        findings: Vec::new(),
    };

    let assurance = assurance_axis(&[], &evidence_findings, &[gap]);

    assert_eq!(assurance, NativeAssurance::ReviewRequired);
}

#[test]
fn review_gaps_marks_requirement_satisfied_only_for_unreviewed_inference_gaps() {
    // The type restriction lives entirely here, in the one place that
    // produces gaps — `assurance_axis` and `close_check_skeleton` both just
    // read whatever this function wrote, with no type check of their own.
    // Both `evidence:placeholder` and `completion_candidate:shared-id` are
    // in the satisfied set, but only the `UnreviewedInference` gap may read
    // `requirement_satisfied: true` from it.
    let space = fixture_space();
    let evidence_findings = NativeEvidenceFindings {
        accepted_evidence_ids: Vec::new(),
        source_backed_evidence_ids: Vec::new(),
        inference_record_ids: vec![id("evidence:placeholder")],
        unreviewed_inference_ids: vec![id("evidence:placeholder")],
        promoted_evidence_ids: Vec::new(),
        boundary_violations: Vec::new(),
        findings: Vec::new(),
    };
    let completion_candidate = NativeCompletionCandidate {
        id: id("completion_candidate:shared-id"),
        candidate_type: NativeCompletionCandidateType::NativeCompletionCell,
        target_ids: Vec::new(),
        suggested_structure: Value::Null,
        inferred_from: Vec::new(),
        rationale: "generated".to_owned(),
        confidence: confidence(0.5),
        review_status: ReviewStatus::Unreviewed,
        provenance: generated_provenance("test", 0.5),
    };
    let satisfied_requirement_ids: BTreeSet<String> = [
        "evidence:placeholder".to_owned(),
        "completion_candidate:shared-id".to_owned(),
    ]
    .into();

    let gaps = review_gaps(
        &space,
        &evidence_findings,
        std::slice::from_ref(&completion_candidate),
        &satisfied_requirement_ids,
    );

    let inference_gap = gaps
        .iter()
        .find(|gap| gap.gap_type == NativeReviewGapType::UnreviewedInference)
        .expect("inference gap present");
    assert!(inference_gap.requirement_satisfied);
    let completion_gap = gaps
        .iter()
        .find(|gap| gap.gap_type == NativeReviewGapType::UnreviewedCompletion)
        .expect("completion gap present");
    assert!(!completion_gap.requirement_satisfied);
}

#[test]
fn assurance_axis_matches_the_worst_wins_truth_table() {
    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let rejected_in_use = bool::arbitrary(u)?;
            let unresolved_gap_present = bool::arbitrary(u)?;
            let missing_source_violation_present = bool::arbitrary(u)?;
            let review_required_obstruction_present = bool::arbitrary(u)?;
            let accepted_evidence_present = bool::arbitrary(u)?;

            let mut boundary_violations = Vec::new();
            if rejected_in_use {
                boundary_violations.push(NativeEvidenceBoundaryViolation {
                    id: id("violation:rejected"),
                    evidence_id: id("evidence:x"),
                    violation_type: NativeEvidenceBoundaryViolationType::RejectedEvidenceUsed,
                    explanation: "generated".to_owned(),
                    severity: Severity::High,
                });
            }
            if missing_source_violation_present {
                boundary_violations.push(NativeEvidenceBoundaryViolation {
                    id: id("violation:missing-source"),
                    evidence_id: id("evidence:y"),
                    violation_type: NativeEvidenceBoundaryViolationType::MissingSource,
                    explanation: "generated".to_owned(),
                    severity: Severity::High,
                });
            }
            let evidence_findings = NativeEvidenceFindings {
                accepted_evidence_ids: if accepted_evidence_present {
                    vec![id("evidence:accepted")]
                } else {
                    Vec::new()
                },
                source_backed_evidence_ids: Vec::new(),
                inference_record_ids: Vec::new(),
                unreviewed_inference_ids: Vec::new(),
                promoted_evidence_ids: Vec::new(),
                boundary_violations,
                findings: Vec::new(),
            };
            let review_gaps = if unresolved_gap_present {
                vec![NativeReviewGap {
                    id: id("review_gap:x"),
                    target_id: id("evidence:x"),
                    gap_type: NativeReviewGapType::UnreviewedInference,
                    explanation: "generated".to_owned(),
                    requirement_satisfied: false,
                }]
            } else {
                Vec::new()
            };
            let obstructions = if review_required_obstruction_present {
                vec![obstruction(
                    NativeObstructionType::ReviewRequired,
                    &id("work:x"),
                    &id("work:x"),
                    "constraint:test",
                    "generated".to_owned(),
                    Severity::Medium,
                    "resolve",
                )]
            } else {
                Vec::new()
            };

            let assurance = assurance_axis(&obstructions, &evidence_findings, &review_gaps);

            let expected = if rejected_in_use {
                NativeAssurance::Rejected
            } else if unresolved_gap_present
                || missing_source_violation_present
                || review_required_obstruction_present
            {
                NativeAssurance::ReviewRequired
            } else if accepted_evidence_present {
                NativeAssurance::Accepted
            } else {
                NativeAssurance::Unreviewed
            };
            assert_eq!(assurance, expected);

            Ok(())
        },
    );
}

fn fixture_space() -> CaseSpace {
    let source_boundary = source_boundary_metadata();
    let revision = Revision {
        revision_id: id("revision:native-fixture-v1"),
        case_space_id: id("case_space:native-fixture"),
        applied_entry_ids: vec![id("morphism_log_entry:genesis")],
        applied_morphism_ids: vec![id("morphism:create-fixture")],
        checksum: "sha256:fixture".to_owned(),
        parent_revision_id: None,
        created_at: "2026-04-26T00:00:00Z".to_owned(),
        source_ids: vec![id("source:test")],
        metadata: Map::new(),
    };
    let mut morphism_metadata = Map::new();
    morphism_metadata.insert("lift_semantics".to_owned(), json!("fixture_to_case_space"));
    morphism_metadata.insert(
        "source_boundary_id".to_owned(),
        json!("source_boundary:native-fixture"),
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
        case_space_id: id("case_space:native-fixture"),
        space_id: id("space:native-fixture"),
        case_cells: Vec::new(),
        case_relations: Vec::new(),
        morphism_log: vec![MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: 1,
            case_space_id: id("case_space:native-fixture"),
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

fn source_boundary_metadata() -> Value {
    json!({
        "id": "source_boundary:native-fixture",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["native.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture makes no inferred claims",
        "information_loss": []
    })
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

/// Appends a canonical review morphism the way an appended log entry actually
/// looks, so evaluation sees the same structural log
/// `evidence_findings`/`latest_evidence_review_statuses` must read, not a
/// synthetic shortcut.
fn append_review_entry(space: &mut CaseSpace, morphism: CaseMorphism, entry_id: &str) {
    let previous_revision_id = space.revision.revision_id.clone();
    let target_revision_id = morphism.target_revision_id.clone();
    let previous_entry_hash = space
        .morphism_log
        .last()
        .map(crate::native_hash::morphism_log_entry_hash)
        .transpose()
        .expect("previous entry hash");
    space.morphism_log.push(MorphismLogEntry {
        schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
        schema_version: 1,
        case_space_id: space.case_space_id.clone(),
        sequence: space.morphism_log.len() as u64 + 1,
        entry_id: id(entry_id),
        morphism_id: morphism.morphism_id.clone(),
        source_revision_id: Some(previous_revision_id.clone()),
        target_revision_id: target_revision_id.clone(),
        morphism,
        actor_id: id("actor:reviewer"),
        recorded_at: "2026-04-26T00:30:00Z".to_owned(),
        provenance: provenance(SourceKind::Human, ReviewStatus::Accepted),
        source_ids: vec![id("source:test")],
        previous_entry_hash,
        replay_checksum: "fixture-review".to_owned(),
    });
    space.revision.revision_id = target_revision_id;
    space.revision.parent_revision_id = Some(previous_revision_id);
    space.revision.checksum = "fixture-review".to_owned();
    for projection in &mut space.projections {
        projection.revision_id = space.revision.revision_id.clone();
    }
}

fn cell(id_value: &str, cell_type: CaseCellType, lifecycle: CaseCellLifecycle) -> CaseCell {
    CaseCell {
        id: id(id_value),
        cell_type,
        space_id: id("space:native-fixture"),
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

fn provenance(kind: SourceKind, review_status: ReviewStatus) -> Provenance {
    Provenance::new(SourceRef::new(kind), confidence(1.0)).with_review_status(review_status)
}
