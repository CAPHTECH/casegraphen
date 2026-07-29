use super::super::{path_helpers::path_segment, NativeCliError};
use crate::native_model::{CaseMorphism, CaseSpace, NATIVE_CASE_SPACE_SCHEMA_VERSION};
use higher_graphen_core::{Confidence, Id, Provenance, ReviewStatus, SourceKind, SourceRef};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const PROPOSAL_SCHEMA: &str = "highergraphen.case.native_cli.morphism_proposal.v1";
const PROPOSAL_DIR: &str = "native_morphism_proposals";

pub(super) fn proposal_value(case_space_id: &Id, morphism: &CaseMorphism) -> Value {
    json!({
        "schema": PROPOSAL_SCHEMA,
        "schema_version": 1,
        "case_space_id": case_space_id,
        "morphism": morphism
    })
}

pub(super) fn read_proposal(
    store: &Path,
    case_space_id: &Id,
    morphism_id: &Id,
) -> Result<CaseMorphism, NativeCliError> {
    let path = proposal_path(store, case_space_id, morphism_id)?;
    let value = read_json(&path)?;
    if value["schema"] != json!(PROPOSAL_SCHEMA) {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported proposal schema",
            path.display()
        )));
    }
    if value["schema_version"] != json!(NATIVE_CASE_SPACE_SCHEMA_VERSION) {
        return Err(NativeCliError::invalid(format!(
            "{}: unsupported proposal schema version",
            path.display()
        )));
    }
    if value["case_space_id"] != json!(case_space_id) {
        return Err(NativeCliError::invalid(format!(
            "{}: proposal belongs to a different case space",
            path.display()
        )));
    }
    let morphism: CaseMorphism = serde_json::from_value(value["morphism"].clone())?;
    if morphism.morphism_id != *morphism_id {
        return Err(NativeCliError::invalid(format!(
            "{}: proposal morphism id mismatch",
            path.display()
        )));
    }
    Ok(morphism)
}

pub(super) fn proposal_path(
    store: &Path,
    case_space_id: &Id,
    morphism_id: &Id,
) -> Result<PathBuf, NativeCliError> {
    Ok(store
        .join(PROPOSAL_DIR)
        .join(path_segment(case_space_id))
        .join(format!("{}.case_morphism.json", path_segment(morphism_id))))
}

pub(super) fn read_case_space(path: &Path) -> Result<CaseSpace, NativeCliError> {
    serde_json::from_value(read_json(path)?).map_err(NativeCliError::from)
}

pub(super) fn read_morphism(path: &Path) -> Result<CaseMorphism, NativeCliError> {
    serde_json::from_value(read_json(path)?).map_err(NativeCliError::from)
}

pub(super) fn read_json(path: &Path) -> Result<Value, NativeCliError> {
    let text = fs::read_to_string(path).map_err(|source| NativeCliError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text).map_err(NativeCliError::from)
}

pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), NativeCliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| NativeCliError::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{text}\n")).map_err(|source| NativeCliError::Io {
        path: path.to_owned(),
        source,
    })
}

pub(super) fn known_ids(case_space: &CaseSpace) -> Vec<Id> {
    case_space
        .case_cells
        .iter()
        .map(|cell| cell.id.clone())
        .chain(
            case_space
                .case_relations
                .iter()
                .map(|relation| relation.id.clone()),
        )
        .chain(
            case_space
                .projections
                .iter()
                .map(|projection| projection.projection_id.clone()),
        )
        .chain(
            case_space
                .morphism_log
                .iter()
                .flat_map(|entry| [entry.entry_id.clone(), entry.morphism_id.clone()]),
        )
        .chain([case_space.revision.revision_id.clone()])
        .collect()
}

pub(super) fn case_space_checksum(case_space: &CaseSpace) -> Result<String, NativeCliError> {
    crate::native_hash::case_space_checksum(case_space).map_err(NativeCliError::from)
}

pub(super) fn provenance(kind: SourceKind, review_status: ReviewStatus) -> Provenance {
    Provenance::new(
        SourceRef::new(kind),
        Confidence::new(1.0).expect("valid confidence"),
    )
    .with_review_status(review_status)
}

pub(super) fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}
