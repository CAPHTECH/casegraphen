#![allow(missing_docs)]

use casegraphen::native_eval::{evaluate_native_case, validate_native_case_space};
use casegraphen::native_model::{
    CaseCell, CaseCellLifecycle, CaseCellType, CaseMorphism, CaseMorphismType, CaseRelation,
    CaseRelationType, CaseSpace, MorphismLogEntry, RelationStrength, Revision,
    NATIVE_CASE_SPACE_SCHEMA, NATIVE_CASE_SPACE_SCHEMA_VERSION, NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
};
use higher_graphen_core::{Confidence, Id, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{json, Map};
use std::hint::black_box;
use std::time::{Duration, Instant};

const SAMPLE_RUNS: usize = 3;
const MIN_SAMPLE_TIME: Duration = Duration::from_millis(10);

#[test]
fn native_evaluation_scales_subquadratically() {
    let small = synthetic_path_space(500);
    let large = synthetic_path_space(2_000);

    validate_native_case_space(&small).expect("N=500 synthetic space should be valid");
    validate_native_case_space(&large).expect("N=2000 synthetic space should be valid");

    let probe = best_of_three(&small, 1);
    let mut repetitions = if probe < Duration::from_millis(2) {
        repetitions_for_stable_sample(probe)
    } else {
        1
    };
    let mut small_time = best_of_three(&small, repetitions);
    if repetitions == 1 && small_time < Duration::from_millis(2) {
        repetitions = repetitions_for_stable_sample(small_time);
        small_time = best_of_three(&small, repetitions);
    }
    let large_time = best_of_three(&large, repetitions);
    let ratio = large_time.as_secs_f64() / small_time.as_secs_f64();

    eprintln!(
        "native evaluation scale: N=500 {small_time:?}, N=2000 {large_time:?}, ratio {ratio:.2}x (best of {SAMPLE_RUNS}, {repetitions} evaluation(s) per sample)"
    );

    assert!(
        ratio < 8.0,
        "expected sub-quadratic scaling across a 4x input increase, got {ratio:.2}x"
    );
    assert!(
        large_time < Duration::from_secs(2),
        "N=2000 evaluation took {large_time:?}, exceeding the 2s sanity ceiling"
    );
}

fn best_of_three(space: &CaseSpace, repetitions: usize) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..SAMPLE_RUNS {
        let started = Instant::now();
        for _ in 0..repetitions {
            black_box(evaluate_native_case(black_box(space)).expect("synthetic evaluation"));
        }
        best = best.min(started.elapsed());
    }
    best.div_f64(repetitions as f64)
}

fn repetitions_for_stable_sample(single_run: Duration) -> usize {
    let run_nanos = single_run.as_nanos().max(1);
    let target_nanos = MIN_SAMPLE_TIME.as_nanos();
    target_nanos.div_ceil(run_nanos).clamp(3, 100) as usize
}

fn synthetic_path_space(cell_count: usize) -> CaseSpace {
    let case_space_id = id("case_space:evaluation-scale");
    let space_id = id("space:evaluation-scale");
    let revision_id = id("revision:evaluation-scale-v1");
    let entry_id = id("morphism_log_entry:evaluation-scale-genesis");
    let morphism_id = id("morphism:evaluation-scale-genesis");
    let source_id = id("source:evaluation-scale-generator");
    let source_boundary = json!({
        "id": "source_boundary:evaluation-scale",
        "included_sources": [source_id.as_str()],
        "excluded_sources": [],
        "adapters": ["native.synthetic-path.v1"],
        "accepted_fact_policy": "generated path cells and relations are accepted test input",
        "inference_policy": "the scale fixture contains no inferred claims",
        "information_loss": []
    });

    let case_cells = (0..cell_count)
        .map(|index| CaseCell {
            id: entity_id(index),
            cell_type: CaseCellType::Custom("entity".to_owned()),
            space_id: space_id.clone(),
            title: format!("Synthetic entity {index}"),
            summary: None,
            lifecycle: CaseCellLifecycle::Active,
            source_ids: vec![source_id.clone()],
            structure_ids: Vec::new(),
            provenance: provenance(),
            metadata: Map::new(),
        })
        .collect();
    let case_relations = (0..cell_count.saturating_sub(1))
        .map(|index| CaseRelation {
            id: id(&format!("relation:mentions:{index}")),
            relation_type: CaseRelationType::Custom("mentions".to_owned()),
            relation_strength: RelationStrength::Diagnostic,
            from_id: entity_id(index),
            to_id: entity_id(index + 1),
            evidence_ids: Vec::new(),
            source_ids: vec![source_id.clone()],
            provenance: provenance(),
            metadata: Map::new(),
        })
        .collect();

    let revision = Revision {
        revision_id: revision_id.clone(),
        case_space_id: case_space_id.clone(),
        applied_entry_ids: vec![entry_id.clone()],
        applied_morphism_ids: vec![morphism_id.clone()],
        checksum: "sha256:evaluation-scale-placeholder".to_owned(),
        parent_revision_id: None,
        created_at: "2026-07-30T00:00:00Z".to_owned(),
        source_ids: vec![source_id.clone()],
        metadata: Map::new(),
    };
    let mut morphism_metadata = Map::new();
    morphism_metadata.insert(
        "lift_semantics".to_owned(),
        json!("synthetic_path_to_case_space"),
    );
    morphism_metadata.insert(
        "source_boundary_id".to_owned(),
        json!("source_boundary:evaluation-scale"),
    );
    morphism_metadata.insert("source_boundary".to_owned(), source_boundary.clone());
    let morphism = CaseMorphism {
        morphism_id: morphism_id.clone(),
        morphism_type: CaseMorphismType::Create,
        source_revision_id: None,
        target_revision_id: revision_id.clone(),
        added_ids: Vec::new(),
        updated_ids: Vec::new(),
        retired_ids: Vec::new(),
        preserved_ids: Vec::new(),
        violated_invariant_ids: Vec::new(),
        review_status: ReviewStatus::Accepted,
        evidence_ids: Vec::new(),
        source_ids: vec![source_id.clone()],
        metadata: morphism_metadata,
    };
    let mut metadata = Map::new();
    metadata.insert("source_boundary".to_owned(), source_boundary);

    CaseSpace {
        schema: NATIVE_CASE_SPACE_SCHEMA.to_owned(),
        schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
        case_space_id: case_space_id.clone(),
        space_id,
        case_cells,
        case_relations,
        morphism_log: vec![MorphismLogEntry {
            schema: NATIVE_MORPHISM_LOG_ENTRY_SCHEMA.to_owned(),
            schema_version: NATIVE_CASE_SPACE_SCHEMA_VERSION,
            case_space_id,
            sequence: 1,
            entry_id,
            morphism_id,
            source_revision_id: None,
            target_revision_id: revision_id,
            morphism,
            actor_id: id("actor:evaluation-scale-generator"),
            recorded_at: "2026-07-30T00:00:00Z".to_owned(),
            provenance: provenance(),
            source_ids: vec![source_id],
            previous_entry_hash: None,
            replay_checksum: revision.checksum.clone(),
        }],
        projections: Vec::new(),
        revision,
        close_policy_id: None,
        metadata,
    }
}

fn entity_id(index: usize) -> Id {
    id(&format!("entity:{index}"))
}

fn id(value: &str) -> Id {
    Id::new(value).expect("synthetic id should be valid")
}

fn provenance() -> Provenance {
    Provenance::new(
        SourceRef::new(SourceKind::Human),
        Confidence::new(1.0).expect("valid confidence"),
    )
    .with_review_status(ReviewStatus::Accepted)
}
