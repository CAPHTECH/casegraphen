use serde_json::{json, Value};

const REPORT_SCHEMA: &str = "highergraphen.case.native_cli.report.v1";
const REPORT_TYPE: &str = "native_cli_operation";
const REPORT_VERSION: u32 = 1;

/// Closed vocabulary for the top-level `result.status` emitted by native CLI
/// operations. Commands with a different shape (for example `operate`, whose
/// stop reason is `result.halt`) omit the field. The Skill capability generator
/// reads this definition instead of maintaining a documentation-only copy.
pub(crate) const OPERATION_STATUSES: &[&str] = &[
    "completed",
    "no_dispatchable_step",
    "paused_for_review",
    "round_executed",
    "step_executed",
    "step_failed",
    "transition_not_authorized",
];

pub(super) fn report(command: &str, result: Value) -> Value {
    debug_assert!(
        result
            .get("status")
            .and_then(Value::as_str)
            .map_or(true, |status| OPERATION_STATUSES.contains(&status)),
        "native CLI report used an undeclared operation status"
    );
    json!({
        "schema": REPORT_SCHEMA,
        "report_type": REPORT_TYPE,
        "report_version": REPORT_VERSION,
        "metadata": {
            "command": command,
            "tool_package": "casegraphen",
            "core_packages": [
                "higher-graphen-core"
            ]
        },
        "input": {
            "command": command
        },
        "result": result,
        "projection": {
            "human_review": {
                "summary": "Native CaseGraphen CLI operation completed."
            },
            "ai_view": {
                "operation": command,
                "native_boundary": "CaseSpace plus MorphismLog state is replayed before derived reports are emitted."
            },
            "audit_trace": {
                "source_ids": [],
                "information_loss": [
                    "Native CLI operation reports include the operation result but not a full command-line argv transcript."
                ]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reports_name_the_actual_tool_package() {
        let value = report("casegraphen space inspect", json!({}));

        assert_eq!(value["metadata"]["tool_package"], json!("casegraphen"));
    }
}
