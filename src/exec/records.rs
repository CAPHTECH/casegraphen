use crate::native_review::NativeOperationGate;
use higher_graphen_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const WORKER_REPORT_SCHEMA: &str = "highergraphen.case.workflow.worker_report.v1";
pub const EXECUTION_TRACE_SCHEMA: &str = "highergraphen.case.workflow.execution_trace.v1";
pub const EXECUTION_RECORD_SCHEMA_VERSION: u32 = 1;
pub const WORKER_REPORT_TRUST_BOUNDARY: &str =
    "local_process_output_untrusted_until_validated_and_reviewed";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutputName {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerOutput {
    pub name: WorkerOutputName,
    pub content_hash: String,
    pub byte_len: u64,
    pub retained_byte_len: u64,
    pub truncated: bool,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReport {
    pub schema: String,
    pub schema_version: u32,
    pub report_id: Id,
    pub binding_id: Id,
    pub binding_content_hash: String,
    pub work_cell_id: Id,
    pub plan_id: Id,
    pub step_id: Id,
    pub exit_status: Option<i32>,
    pub timed_out: bool,
    pub descendants_may_survive: bool,
    pub outputs: Vec<WorkerOutput>,
    pub trust_boundary: String,
    pub started_at: String,
    pub finished_at: String,
    pub metadata: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionObstruction {
    pub obstruction_type: String,
    pub summary: String,
    pub witness_ids: Vec<Id>,
    pub blocking: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionInformationLoss {
    pub description: String,
    pub represented_ids: Vec<Id>,
    pub omitted_ids: Vec<Id>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionTrace {
    pub schema: String,
    pub schema_version: u32,
    pub trace_id: Id,
    pub plan_id: Id,
    pub step_id: Id,
    pub case_space_id: Id,
    pub base_revision_id: Id,
    pub result_revision_id: Option<Id>,
    pub work_cell_id: Id,
    pub binding_id: Id,
    pub binding_content_hash: String,
    pub operation_gate: NativeOperationGate,
    pub worker_report_id: Id,
    pub appended_entry_ids: Vec<Id>,
    pub transition_applied: bool,
    pub unsatisfied_success_evidence_requirement_ids: Vec<Id>,
    pub obstructions: Vec<ExecutionObstruction>,
    pub information_loss: Vec<ExecutionInformationLoss>,
    pub started_at: String,
    pub finished_at: String,
    pub metadata: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_record_examples_round_trip() {
        let report: WorkerReport = serde_json::from_str(include_str!(
            "../../schemas/casegraphen/worker.report.example.json"
        ))
        .expect("worker report example");
        let trace: ExecutionTrace = serde_json::from_str(include_str!(
            "../../schemas/casegraphen/execution.trace.example.json"
        ))
        .expect("execution trace example");

        assert_eq!(report.schema, WORKER_REPORT_SCHEMA);
        assert_eq!(trace.schema, EXECUTION_TRACE_SCHEMA);
        assert_eq!(report.trust_boundary, WORKER_REPORT_TRUST_BOUNDARY);
        assert_eq!(report.schema_version, EXECUTION_RECORD_SCHEMA_VERSION);
        assert_eq!(trace.schema_version, EXECUTION_RECORD_SCHEMA_VERSION);
    }
}
