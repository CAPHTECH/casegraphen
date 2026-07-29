use crate::model::{ProjectionDefinition, PROJECTION_SCHEMA};
use crate::workflow_eval::{validate_workflow_graph, WorkflowValidationError};
use crate::workflow_model::{
    WorkflowCaseGraph, WORKFLOW_GRAPH_SCHEMA, WORKFLOW_GRAPH_SCHEMA_VERSION,
};
use serde::de::DeserializeOwned;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug)]
pub enum StoreError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Contract {
        path: PathBuf,
        reason: String,
    },
    Validation {
        path: PathBuf,
        source: WorkflowValidationError,
    },
}

pub fn read_projection(path: &Path) -> StoreResult<ProjectionDefinition> {
    let projection: ProjectionDefinition = read_json(path)?;
    require_schema(path, &projection.schema, PROJECTION_SCHEMA)?;
    Ok(projection)
}

pub fn read_workflow_graph(path: &Path) -> StoreResult<WorkflowCaseGraph> {
    let graph: WorkflowCaseGraph = read_json(path)?;
    require_schema(path, &graph.schema, WORKFLOW_GRAPH_SCHEMA)?;
    require_schema_version(path, graph.schema_version, WORKFLOW_GRAPH_SCHEMA_VERSION)?;
    validate_workflow_graph(&graph).map_err(|source| StoreError::Validation {
        path: path.to_owned(),
        source,
    })?;
    Ok(graph)
}

pub fn write_report(path: &Path, report: &impl serde::Serialize) -> StoreResult<()> {
    write_json(path, report)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> StoreResult<T> {
    let text = fs::read_to_string(path).map_err(|source| StoreError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| StoreError::Json {
        path: path.to_owned(),
        source,
    })
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> StoreResult<()> {
    let text = serde_json::to_string_pretty(value).map_err(|source| StoreError::Json {
        path: path.to_owned(),
        source,
    })?;
    fs::write(path, format!("{text}\n")).map_err(|source| StoreError::Io {
        path: path.to_owned(),
        source,
    })
}

fn require_schema(path: &Path, actual: &str, expected: &str) -> StoreResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(StoreError::Contract {
        path: path.to_owned(),
        reason: format!("unsupported schema {actual:?}; expected {expected:?}"),
    })
}

fn require_schema_version(path: &Path, actual: u32, expected: u32) -> StoreResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(StoreError::Contract {
        path: path.to_owned(),
        reason: format!("unsupported schema version {actual}; expected {expected}"),
    })
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Json { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Contract { path, reason } => write!(formatter, "{}: {reason}", path.display()),
            Self::Validation { path, source } => {
                write!(formatter, "{}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for StoreError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn read_workflow_graph_rejects_unsupported_schema() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time since epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("casegraphen-workflow-store-test-{nanos}"));
        fs::create_dir_all(&root).expect("create temp store");
        let path = root.join("bad.workflow.graph.json");
        let mut value: Value = serde_json::from_str(include_str!(
            "../schemas/casegraphen/workflow.graph.example.json"
        ))
        .expect("workflow graph example");
        value["schema"] = json!("highergraphen.case.workflow.graph.v0");
        write_json(&path, &value).expect("write workflow graph");

        let error = read_workflow_graph(&path).expect_err("unsupported schema");
        assert!(error.to_string().contains("unsupported schema"));

        fs::remove_dir_all(root).expect("remove temp store");
    }
}
