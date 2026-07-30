#![allow(missing_docs)]

use casegraphen::{
    native_eval::evaluate_native_case,
    native_model::{CaseCell, CaseCellLifecycle, CaseCellType, CaseSpace, ProjectionAudience},
    native_review::{check_operation_gate, NativeOperationGate},
};
use higher_graphen_core::{Id, ReviewStatus};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const CAPABILITY_ID: &str = "capability:plan-review";
const EXPECTED_OPERATION: &str = "plan-review";
const SNAPSHOT_MUTATION_COUNT: usize = 8;

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

type GateMutation = fn(&mut CaseSpace, &mut NativeOperationGate);

struct GateMutator {
    name: &'static str,
    apply: GateMutation,
}

const GATE_MUTATORS: [GateMutator; 10] = [
    GateMutator {
        name: "operation differs from expected operation",
        apply: break_expected_operation,
    },
    GateMutator {
        name: "operation scope differs from case space",
        apply: break_operation_scope,
    },
    GateMutator {
        name: "audience is neither audit nor system",
        apply: break_audience,
    },
    GateMutator {
        name: "capability ids are empty",
        apply: break_non_empty_capabilities,
    },
    GateMutator {
        name: "capability id does not resolve",
        apply: break_capability_resolution,
    },
    GateMutator {
        name: "capability cell has the wrong type",
        apply: break_capability_type,
    },
    GateMutator {
        name: "capability cell has an inactive lifecycle",
        apply: break_capability_lifecycle,
    },
    GateMutator {
        name: "capability cell is not accepted",
        apply: break_capability_review_status,
    },
    GateMutator {
        name: "capability cell does not grant the actor",
        apply: break_actor_grant,
    },
    GateMutator {
        name: "source boundary differs from the declaration",
        apply: break_source_boundary,
    },
];

type SnapshotMutation = fn(&mut CaseSpace);

struct SnapshotMutator {
    name: &'static str,
    apply: SnapshotMutation,
}

const SNAPSHOT_MUTATORS: [SnapshotMutator; SNAPSHOT_MUTATION_COUNT] = [
    SnapshotMutator {
        name: "cell id collides with revision id",
        apply: collide_cell_with_revision,
    },
    SnapshotMutator {
        name: "cell id collides with genesis entry id",
        apply: collide_cell_with_genesis_entry,
    },
    SnapshotMutator {
        name: "cell id collides with genesis morphism id",
        apply: collide_cell_with_genesis_morphism,
    },
    SnapshotMutator {
        name: "cell id is duplicated",
        apply: duplicate_cell_id,
    },
    SnapshotMutator {
        name: "cell id collides with relation id",
        apply: collide_cell_with_relation,
    },
    SnapshotMutator {
        name: "cell belongs to a different space",
        apply: change_cell_space,
    },
    SnapshotMutator {
        name: "cell title is empty",
        apply: empty_cell_title,
    },
    SnapshotMutator {
        name: "cell source ids are empty",
        apply: empty_cell_sources,
    },
];

#[test]
fn operation_gate_refuses_every_non_empty_subset_of_failed_conditions() {
    let (case_space, gate) = known_good_gate();

    check_operation_gate(&case_space, &gate, EXPECTED_OPERATION)
        .expect("anti-vacuity: the unmutated operation gate must be accepted");

    for mutator in &GATE_MUTATORS {
        let mut mutated_space = case_space.clone();
        let mut mutated_gate = gate.clone();
        (mutator.apply)(&mut mutated_space, &mut mutated_gate);
        let result = check_operation_gate(&mutated_space, &mutated_gate, EXPECTED_OPERATION);
        assert!(
            result.is_err(),
            "anti-vacuity: single mutator {:?} did not break its condition",
            mutator.name
        );
    }

    let subset_count = 1_u16 << GATE_MUTATORS.len();
    for subset in 0..subset_count {
        let mut mutated_space = case_space.clone();
        let mut mutated_gate = gate.clone();
        let mut mutation_names = Vec::new();
        for (index, mutator) in GATE_MUTATORS.iter().enumerate() {
            if subset & (1_u16 << index) != 0 {
                (mutator.apply)(&mut mutated_space, &mut mutated_gate);
                mutation_names.push(mutator.name);
            }
        }

        let result = check_operation_gate(&mutated_space, &mutated_gate, EXPECTED_OPERATION);
        assert_eq!(
            result.is_ok(),
            subset == 0,
            "operation-gate subset {subset:#05x} misbehaved: {mutation_names:?}; result: {result:?}"
        );
    }
}

#[test]
fn every_imported_snapshot_remains_readable_by_the_evaluator() {
    let baseline = fixture_case_space();
    evaluate_native_case(&baseline)
        .expect("anti-vacuity: the unmutated snapshot must evaluate successfully");

    let directory = TestDirectory::new();
    assert!(
        exercise_import_boundary(directory.path(), 0, "unmutated baseline", &baseline),
        "anti-vacuity: the unmutated snapshot must import successfully"
    );

    let mut executed = 0_usize;
    let mut refused = 0_usize;
    let mut imported_mutations = Vec::new();
    for (index, mutator) in SNAPSHOT_MUTATORS.iter().enumerate() {
        let mut mutated = baseline.clone();
        (mutator.apply)(&mut mutated);
        executed += 1;
        let label = format!("single {}: {}", index, mutator.name);
        if !exercise_import_boundary(directory.path(), executed, &label, &mutated) {
            refused += 1;
        } else {
            imported_mutations.push(label);
        }
    }

    for (first, first_mutator) in SNAPSHOT_MUTATORS.iter().enumerate() {
        for (second, second_mutator) in SNAPSHOT_MUTATORS.iter().enumerate().skip(first + 1) {
            let mut mutated = baseline.clone();
            (first_mutator.apply)(&mut mutated);
            (second_mutator.apply)(&mut mutated);
            executed += 1;
            let label = format!(
                "pair ({first}, {second}): {} + {}",
                first_mutator.name, second_mutator.name
            );
            if !exercise_import_boundary(directory.path(), executed, &label, &mutated) {
                refused += 1;
            } else {
                imported_mutations.push(label);
            }
        }
    }

    assert_eq!(
        executed, 36,
        "expected 8 single mutations and all 28 mutation pairs"
    );
    assert!(
        refused > 0,
        "anti-vacuity: at least one invalid snapshot must be refused"
    );
    eprintln!(
        "property 2 exercised {executed} mutated snapshots: {refused} refused, {} imported and readable: {imported_mutations:?}",
        imported_mutations.len()
    );
}

fn known_good_gate() -> (CaseSpace, NativeOperationGate) {
    let case_space = fixture_case_space();
    let capability = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == id(CAPABILITY_ID))
        .expect("fixture capability");
    assert_eq!(
        capability.cell_type,
        CaseCellType::Custom("capability".to_owned())
    );
    assert!(matches!(
        capability.lifecycle,
        CaseCellLifecycle::Active | CaseCellLifecycle::Accepted
    ));
    assert_eq!(capability.provenance.review_status, ReviewStatus::Accepted);
    let actor_id = capability.metadata["actor_ids"]
        .as_array()
        .and_then(|actor_ids| actor_ids.first())
        .and_then(Value::as_str)
        .map(id)
        .expect("fixture capability grants an actor");
    let source_boundary_id = case_space.metadata["source_boundary"]["id"]
        .as_str()
        .map(id)
        .expect("fixture declares a source boundary id");
    let gate = NativeOperationGate {
        actor_id,
        operation: EXPECTED_OPERATION.to_owned(),
        operation_scope_id: case_space.case_space_id.clone(),
        audience: ProjectionAudience::Audit,
        capability_ids: vec![capability.id.clone()],
        source_boundary_id,
    };
    (case_space, gate)
}

fn break_expected_operation(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.operation = "different-operation".to_owned();
}

fn break_operation_scope(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.operation_scope_id = id("case_space:different");
}

fn break_audience(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.audience = ProjectionAudience::HumanReview;
}

fn break_non_empty_capabilities(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.capability_ids.clear();
}

fn break_capability_resolution(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.capability_ids = vec![id("capability:missing")];
}

fn break_capability_type(case_space: &mut CaseSpace, _: &mut NativeOperationGate) {
    capability_cell(case_space).cell_type = CaseCellType::Goal;
}

fn break_capability_lifecycle(case_space: &mut CaseSpace, _: &mut NativeOperationGate) {
    capability_cell(case_space).lifecycle = CaseCellLifecycle::Retired;
}

fn break_capability_review_status(case_space: &mut CaseSpace, _: &mut NativeOperationGate) {
    capability_cell(case_space).provenance.review_status = ReviewStatus::Reviewed;
}

fn break_actor_grant(case_space: &mut CaseSpace, _: &mut NativeOperationGate) {
    capability_cell(case_space)
        .metadata
        .insert("actor_ids".to_owned(), json!(["actor:not-granted"]));
}

fn break_source_boundary(_: &mut CaseSpace, gate: &mut NativeOperationGate) {
    gate.source_boundary_id = id("source_boundary:different");
}

fn capability_cell(case_space: &mut CaseSpace) -> &mut CaseCell {
    cell_mut(case_space, CAPABILITY_ID)
}

fn collide_cell_with_revision(case_space: &mut CaseSpace) {
    let revision_id = case_space.revision.revision_id.clone();
    cell_mut(case_space, "goal:native-case-contract").id = revision_id;
}

fn collide_cell_with_genesis_entry(case_space: &mut CaseSpace) {
    let entry_id = case_space.morphism_log[0].entry_id.clone();
    cell_mut(case_space, "case:native-contract-example").id = entry_id;
}

fn collide_cell_with_genesis_morphism(case_space: &mut CaseSpace) {
    let morphism_id = case_space.morphism_log[0].morphism_id.clone();
    cell_mut(case_space, "work:review-native-contract").id = morphism_id;
}

fn duplicate_cell_id(case_space: &mut CaseSpace) {
    let duplicate_id = case_space
        .case_cells
        .iter()
        .find(|cell| cell.id == id("capability:casegraphen-cli:close-check"))
        .expect("fixture duplicate target")
        .id
        .clone();
    cell_mut(case_space, "evidence:native-schema-json-valid").id = duplicate_id;
}

fn collide_cell_with_relation(case_space: &mut CaseSpace) {
    let relation_id = case_space.case_relations[0].id.clone();
    cell_mut(case_space, "review:native-contract-acceptance").id = relation_id;
}

fn change_cell_space(case_space: &mut CaseSpace) {
    cell_mut(case_space, CAPABILITY_ID).space_id = id("space:different");
}

fn empty_cell_title(case_space: &mut CaseSpace) {
    cell_mut(case_space, "capability:dispatch").title.clear();
}

fn empty_cell_sources(case_space: &mut CaseSpace) {
    cell_mut(case_space, "capability:durable-mutation")
        .source_ids
        .clear();
}

fn cell_mut<'a>(case_space: &'a mut CaseSpace, cell_id: &str) -> &'a mut CaseCell {
    case_space
        .case_cells
        .iter_mut()
        .find(|cell| cell.id == id(cell_id))
        .unwrap_or_else(|| panic!("fixture cell {cell_id}"))
}

fn exercise_import_boundary(
    root: &Path,
    case_number: usize,
    label: &str,
    case_space: &CaseSpace,
) -> bool {
    let input = root.join(format!("snapshot-{case_number}.json"));
    let store = root.join(format!("store-{case_number}"));
    let serialized = serde_json::to_vec(case_space).expect("serialize mutated snapshot");
    fs::write(&input, serialized).expect("write mutated snapshot");

    let lift = run_lift_native(&store, &input, case_space.revision.revision_id.as_str());
    if !lift.status.success() {
        return false;
    }

    let report: Value = serde_json::from_slice(&lift.stdout)
        .unwrap_or_else(|error| panic!("{label}: parse lift report: {error}"));
    let imported: CaseSpace = serde_json::from_value(report["result"]["case_space"].clone())
        .unwrap_or_else(|error| panic!("{label}: parse imported case space: {error}"));
    evaluate_native_case(&imported)
        .unwrap_or_else(|error| panic!("{label}: imported snapshot did not evaluate: {error:?}"));

    for operation in ["reason", "frontier", "validate"] {
        let output = run_space_command(&store, imported.case_space_id.as_str(), operation);
        assert!(
            output.status.success(),
            "{label}: `space {operation}` failed after import\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    true
}

fn run_lift_native(store: &Path, input: &Path, revision_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .arg("lift")
        .arg("native")
        .arg("--store")
        .arg(store)
        .arg("--input")
        .arg(input)
        .arg("--revision-id")
        .arg(revision_id)
        .arg("--format")
        .arg("json")
        .output()
        .expect("run `casegraphen lift native`")
}

fn run_space_command(store: &Path, case_space_id: &str, operation: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .arg("space")
        .arg(operation)
        .arg("--store")
        .arg(store)
        .arg("--case-space-id")
        .arg(case_space_id)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap_or_else(|error| panic!("run `casegraphen space {operation}`: {error}"))
}

fn fixture_case_space() -> CaseSpace {
    serde_json::from_str(include_str!(
        "../schemas/casegraphen/native.case.space.example.json"
    ))
    .expect("native case-space fixture")
}

fn id(value: &str) -> Id {
    Id::new(value).expect("valid test id")
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "casegraphen-trust-invariants-{}-{nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create trust-invariant temp directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
