use super::{
    io::{case_space_checksum, read_case_space},
    new_case_space, path_segment, report, retarget_latest_revision, source_boundary_value,
    workflow_lift::materialize_workflow_graph,
    write_genesis_materialization, NativeCliError,
};
use crate::{
    native_model::CaseSpace, native_store::NativeCaseStore, workflow_model::WorkflowCaseGraph,
};
use higher_graphen_core::Id;
use serde_json::{json, Map, Value};
use std::path::Path;

pub(in crate::native_cli) fn case_new(
    store: &Path,
    case_space_id: &Id,
    space_id: &Id,
    title: &str,
    revision_id: &Id,
) -> Result<Value, NativeCliError> {
    let case_space = new_case_space(case_space_id, space_id, title, revision_id)?;
    let record = NativeCaseStore::new(store.to_path_buf()).import_case_space(&case_space)?;
    Ok(report(
        "casegraphen space new",
        json!({ "record": record, "case_space": case_space }),
    ))
}

pub(in crate::native_cli) fn case_import(
    store: &Path,
    input: &Path,
    revision_id: &Id,
) -> Result<Value, NativeCliError> {
    let mut case_space = read_case_space(input)?;
    retarget_latest_revision(&mut case_space, revision_id)?;
    let record = NativeCaseStore::new(store.to_path_buf()).import_case_space(&case_space)?;
    Ok(report(
        "casegraphen lift native",
        json!({ "record": record, "case_space": case_space }),
    ))
}

pub(in crate::native_cli) fn lift_structured_source(
    store: &Path,
    input: &Path,
    revision_id: &Id,
    adapter: &str,
) -> Result<Value, NativeCliError> {
    // Read once. Two reads of the same path let a concurrent writer make the
    // recorded source identity describe one document while the materialized
    // cells come from another, with nothing in the audit record to show it.
    let bytes = std::fs::read(input).map_err(|source| NativeCliError::Io {
        path: input.to_path_buf(),
        source,
    })?;
    let lift = read_lift_input(&bytes, adapter)?;
    let case_space_id = Id::new(format!("case_space:{}", path_segment(&lift.source_id)))?;
    let mut case_space = new_case_space(
        &case_space_id,
        &lift.space_id,
        &format!("Lifted {}", lift.source_schema),
        revision_id,
    )?;
    let mut information_loss = vec![json!({
        "source_schema": lift.source_schema,
        "input": input.display().to_string(),
        "note": "The first lift adapter records source identity and boundary metadata; full cell/relation materialization is handled by later morphism reducers."
    })];
    // The workflow adapter materializes (ADR 0003): the graph's items,
    // relations, and evidence replace the synthetic root cell, and the
    // genesis payload below is rebuilt from them so the lifted space
    // replays, rebuilds, and validates like any other.
    if adapter == "workflow" {
        let graph = parse_workflow_graph(&bytes)?;
        let materialized = materialize_workflow_graph(&graph)?;
        case_space.case_cells = materialized.cells;
        case_space.case_relations = materialized.relations;
        information_loss = vec![json!({
            "source_schema": lift.source_schema,
            "input": input.display().to_string(),
            "note": "Work items, workflow relations, and evidence records were materialized as case cells and relations; the unmapped families are declared below."
        })];
        information_loss.extend(materialized.information_loss);
    }
    let source_boundary = source_boundary_value(
        Id::new(format!("source_boundary:{}", path_segment(&case_space_id)))?,
        std::slice::from_ref(&lift.lift_source_id),
        &[adapter],
        "Structured source records are accepted as bounded lift input; generated records require review before they satisfy hard requirements.",
        "Lift adapters preserve source identifiers and declare unsupported source fields as information loss.",
        information_loss,
    );
    let input_content_hash = crate::native_hash::sha256_hex(&bytes);
    let annotate_root_cell = adapter != "workflow";
    annotate_lift_metadata(
        &mut case_space,
        &lift,
        &input_content_hash,
        adapter,
        input,
        source_boundary,
        annotate_root_cell,
    );
    refresh_lift_checksums(&mut case_space)?;
    let record = NativeCaseStore::new(store.to_path_buf()).import_case_space(&case_space)?;
    Ok(report(
        &format!("casegraphen lift {adapter}"),
        json!({
            "record": record,
            "case_space": case_space,
            "lift": {
                "adapter": adapter,
                "source_schema": lift.source_schema,
                "input": input.display().to_string()
            }
        }),
    ))
}

struct LiftInput {
    source_schema: String,
    source_id: Id,
    space_id: Id,
    lift_source_id: Id,
}

fn read_lift_input(bytes: &[u8], adapter: &str) -> Result<LiftInput, NativeCliError> {
    let value: Value = serde_json::from_slice(bytes)?;
    let object = value
        .as_object()
        .ok_or_else(|| NativeCliError::invalid("lift input must be a JSON object"))?;
    let source_schema = object
        .get("schema")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let source_id = source_id_for_lift(adapter, object)?;
    let space_id = object
        .get("space_id")
        .and_then(Value::as_str)
        .ok_or_else(|| NativeCliError::invalid("lift input must contain space_id"))?;
    let lift_source_id = Id::new(format!("source:{}", path_segment(&source_id)))?;
    Ok(LiftInput {
        source_schema,
        source_id,
        space_id: Id::new(space_id.to_owned())?,
        lift_source_id,
    })
}

fn annotate_lift_metadata(
    case_space: &mut CaseSpace,
    lift: &LiftInput,
    input_content_hash: &str,
    adapter: &str,
    input: &Path,
    source_boundary: Value,
    annotate_root_cell: bool,
) {
    case_space
        .metadata
        .insert("source_boundary".to_owned(), source_boundary.clone());
    case_space.metadata.insert(
        "lift".to_owned(),
        json!({
            "adapter": adapter,
            "source_schema": lift.source_schema,
            "source_id": lift.source_id,
            "input": input.display().to_string(),
            "input_content_hash": input_content_hash
        }),
    );
    if let Some(entry) = case_space.morphism_log.first_mut() {
        entry.source_ids = vec![lift.lift_source_id.clone()];
        entry.morphism.source_ids = vec![lift.lift_source_id.clone()];
        entry
            .morphism
            .metadata
            .insert("lift_semantics".to_owned(), json!(adapter));
        entry
            .morphism
            .metadata
            .insert("source_boundary".to_owned(), source_boundary);
        entry
            .morphism
            .metadata
            .insert("source_schema".to_owned(), json!(lift.source_schema));
        entry
            .morphism
            .metadata
            .insert("input".to_owned(), json!(input.display().to_string()));
        entry
            .morphism
            .metadata
            .insert("input_content_hash".to_owned(), json!(input_content_hash));
    }
    // A materializing lift's cells carry their own workflow-declared sources;
    // only the shallow adapters stamp the synthetic root cell.
    if annotate_root_cell {
        if let Some(cell) = case_space.case_cells.first_mut() {
            cell.source_ids = vec![lift.lift_source_id.clone()];
            cell.metadata
                .insert("lifted_from".to_owned(), json!(lift.source_id));
            cell.metadata
                .insert("source_schema".to_owned(), json!(lift.source_schema));
        }
    }
    case_space.revision.source_ids = vec![lift.lift_source_id.clone()];
}

fn refresh_lift_checksums(case_space: &mut CaseSpace) -> Result<(), NativeCliError> {
    write_genesis_materialization(case_space)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    case_space.revision.checksum.clear();
    if let Some(entry) = case_space.morphism_log.first_mut() {
        entry.replay_checksum.clear();
    }
    let checksum = case_space_checksum(case_space)?;
    case_space.revision.checksum = checksum.clone();
    if let Some(entry) = case_space.morphism_log.first_mut() {
        entry.replay_checksum = checksum;
    }
    Ok(())
}

fn parse_workflow_graph(bytes: &[u8]) -> Result<WorkflowCaseGraph, NativeCliError> {
    let graph: WorkflowCaseGraph = serde_json::from_slice(bytes)?;
    crate::workflow_model::validate_workflow_graph(&graph)
        .map_err(|error| NativeCliError::invalid(error.to_string()))?;
    Ok(graph)
}

fn source_id_for_lift(adapter: &str, object: &Map<String, Value>) -> Result<Id, NativeCliError> {
    let field = match adapter {
        "workflow" => "workflow_graph_id",
        "case-graph" => "case_graph_id",
        "native" => "case_space_id",
        _ => "id",
    };
    let raw = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| NativeCliError::invalid(format!("lift input must contain {field}")))?;
    Ok(Id::new(raw.to_owned())?)
}
