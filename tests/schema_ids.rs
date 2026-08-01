//! Verifies that Rust input/record schema IDs have shipped JSON Schema identities.

use casegraphen::{
    exec::{
        binding::WORKER_BINDING_SCHEMA,
        records::{EXECUTION_TRACE_SCHEMA, WORKER_REPORT_SCHEMA},
        EXECUTION_PLAN_SCHEMA,
    },
    github_issue_snapshot::GITHUB_ISSUE_SNAPSHOT_SCHEMA,
    native_cli::OPERATION_GATE_PROFILES_SCHEMA,
    native_model::{NATIVE_CASE_SPACE_SCHEMA, NATIVE_MORPHISM_LOG_ENTRY_SCHEMA},
    workflow_model::WORKFLOW_GRAPH_SCHEMA,
};
use serde_json::Value;
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn every_input_and_record_schema_constant_has_a_shipped_schema_id() {
    assert_eq!(
        EXECUTION_TRACE_SCHEMA,
        "highergraphen.case.workflow.execution_trace.v2"
    );
    let schema_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/casegraphen");
    let shipped_ids = fs::read_dir(&schema_directory)
        .expect("read schema directory")
        .map(|entry| entry.expect("read schema directory entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".schema.json"))
        })
        .map(|path| {
            let text = fs::read_to_string(&path).expect("read schema file");
            let schema: Value = serde_json::from_str(&text).expect("parse schema file");
            schema["$id"]
                .as_str()
                .unwrap_or_else(|| panic!("{} must declare a string $id", path.display()))
                .to_owned()
        })
        .collect::<BTreeSet<_>>();

    let input_and_record_ids = [
        WORKFLOW_GRAPH_SCHEMA,
        GITHUB_ISSUE_SNAPSHOT_SCHEMA,
        NATIVE_CASE_SPACE_SCHEMA,
        NATIVE_MORPHISM_LOG_ENTRY_SCHEMA,
        EXECUTION_PLAN_SCHEMA,
        WORKER_BINDING_SCHEMA,
        WORKER_REPORT_SCHEMA,
        EXECUTION_TRACE_SCHEMA,
        OPERATION_GATE_PROFILES_SCHEMA,
    ];

    // Report IDs are intentionally excluded: workflow reasoning and native/shared
    // operation reports are validated through report envelopes and schema aliases,
    // rather than being independent INPUT or RECORD contracts.
    for schema_id in input_and_record_ids {
        assert!(
            shipped_ids.contains(schema_id),
            "schema constant {schema_id:?} has no matching schemas/casegraphen/*.schema.json $id"
        );
    }
}
