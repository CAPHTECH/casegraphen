use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;

pub const WORKER_BINDING_SCHEMA: &str = "highergraphen.case.workflow.worker_binding.v1";
pub const WORKER_BINDING_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    Shell,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBinding {
    pub schema: String,
    pub schema_version: u32,
    pub binding_id: Id,
    pub worker_kind: WorkerKind,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: String,
    pub env_allowlist: Vec<String>,
    pub timeout_ms: u64,
    pub capability_ids: Vec<Id>,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerBindingValidationError {
    message: String,
}

impl WorkerBindingValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WorkerBindingValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkerBindingValidationError {}

pub fn validate_worker_binding(
    binding: &WorkerBinding,
) -> Result<(), WorkerBindingValidationError> {
    if binding.schema != WORKER_BINDING_SCHEMA {
        return Err(WorkerBindingValidationError::new(format!(
            "unsupported worker binding schema {:?}; expected {WORKER_BINDING_SCHEMA:?}",
            binding.schema
        )));
    }
    if binding.schema_version != WORKER_BINDING_SCHEMA_VERSION {
        return Err(WorkerBindingValidationError::new(format!(
            "unsupported worker binding schema version {}; expected {WORKER_BINDING_SCHEMA_VERSION}",
            binding.schema_version
        )));
    }
    if !Id::is_valid_value(binding.binding_id.as_str()) {
        return Err(WorkerBindingValidationError::new(
            "binding_id is not a well-formed id",
        ));
    }
    if binding.command.trim().is_empty() {
        return Err(WorkerBindingValidationError::new(
            "worker binding command must not be empty",
        ));
    }
    if binding.working_directory.trim().is_empty() {
        return Err(WorkerBindingValidationError::new(
            "worker binding working_directory must not be empty",
        ));
    }
    if binding.timeout_ms == 0 {
        return Err(WorkerBindingValidationError::new(
            "worker binding timeout_ms must be at least 1",
        ));
    }
    if binding.capability_ids.is_empty() {
        return Err(WorkerBindingValidationError::new(
            "worker binding capability_ids must not be empty",
        ));
    }
    for (index, capability_id) in binding.capability_ids.iter().enumerate() {
        if !Id::is_valid_value(capability_id.as_str()) {
            return Err(WorkerBindingValidationError::new(format!(
                "capability_ids[{index}] is not a well-formed id"
            )));
        }
    }
    Ok(())
}

pub fn worker_binding_content_hash(binding: &WorkerBinding) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_string(&serde_json::to_value(binding)?)?;
    Ok(crate::native_hash::sha256_hex(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKER_BINDING_EXAMPLE: &str =
        include_str!("../../schemas/casegraphen/worker.binding.example.json");

    #[test]
    fn worker_binding_example_validates_and_round_trips() {
        let binding: WorkerBinding =
            serde_json::from_str(WORKER_BINDING_EXAMPLE).expect("worker binding example");
        validate_worker_binding(&binding).expect("valid worker binding");
        let round_trip: WorkerBinding =
            serde_json::from_value(serde_json::to_value(&binding).expect("serialize binding"))
                .expect("deserialize binding");

        assert_eq!(round_trip, binding);
        assert_eq!(
            worker_binding_content_hash(&binding).expect("hash").len(),
            64
        );
    }
}
