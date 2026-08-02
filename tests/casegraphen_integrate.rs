#![allow(missing_docs)]

use serde_json::Value;
use std::{fs, path::Path};

fn read(relative: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)).unwrap()
}

#[test]
fn skill_delegates_rules_and_stops_at_review() {
    let skill = read("skills/casegraphen-integrate/SKILL.md");
    for required in [
        "GenericJsonlReconciler",
        "runtime_protocol::reconcile_runtime_reports",
        "base_revision_id",
        "incomplete_runtime_reports",
        "needs_review",
        "unreviewed",
        "accepted` remains false",
        "Never infer retry lineage from line order",
        "Never turn an ingest report into accepted evidence",
    ] {
        assert!(
            skill.contains(required),
            "missing integration boundary: {required}"
        );
    }
    assert!(!skill.contains("evidence attach"));
    assert!(!skill.contains("morphism apply"));
}

#[test]
fn shipped_jsonl_envelope_schema_is_strict() {
    let schema: Value = serde_json::from_str(&read(
        "schemas/experimental/runtime.integration.jsonl-record.v0.schema.json",
    ))
    .unwrap();
    assert_eq!(
        schema["$id"],
        "casegraphen.experimental.runtime.integration.jsonl_record.v0"
    );
    assert_eq!(schema["oneOf"][0]["additionalProperties"], false);
    assert_eq!(schema["oneOf"][1]["additionalProperties"], false);
    assert_eq!(schema["oneOf"][2]["additionalProperties"], false);
    assert_eq!(
        schema["oneOf"][2]["properties"]["kind"]["const"],
        "resource_allocation"
    );
}
