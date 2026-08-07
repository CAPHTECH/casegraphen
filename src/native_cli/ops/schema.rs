use super::{report, NativeCliError};
use crate::schema_catalog::catalog;
use serde_json::{json, Value};

/// `casegraphen schema list`: every embedded schema/example, its declared
/// id (when it has one), and whether it is stable or experimental. Never
/// fails — the catalog is compiled in, not read from disk at run time.
pub(in crate::native_cli) fn schema_list() -> Value {
    let schemas: Vec<Value> = catalog()
        .iter()
        .map(|entry| {
            json!({
                "file": entry.file,
                "stability": entry.stability.as_str(),
                "id": entry.id,
            })
        })
        .collect();
    report("casegraphen schema list", json!({ "schemas": schemas }))
}

/// `casegraphen schema get`: the raw content of exactly one embedded file,
/// selected by a schema's own `$id` (`id`) or by its exact filename
/// (`file`). The parser (`native_cli/parser.rs::parse_schema`) guarantees
/// exactly one of the two is `Some`.
///
/// `--id` matches only `*.schema.json` entries, never an example: an
/// example's own `schema` field intentionally repeats its owning schema's
/// `$id` (that is how `schema_catalog.rs::declared_id` finds it at all), so
/// looking `--id` up across both kinds would make the match ambiguous
/// between a schema and its own example — `--file` is how an example gets
/// selected instead.
pub(in crate::native_cli) fn schema_get(
    id: Option<&str>,
    file: Option<&str>,
) -> Result<Value, NativeCliError> {
    let entry = match (id, file) {
        (Some(id), None) => catalog()
            .iter()
            .find(|entry| entry.file.ends_with(".schema.json") && entry.id.as_deref() == Some(id))
            .ok_or_else(|| NativeCliError::usage(format!("unknown schema id {id:?}")))?,
        (None, Some(file)) => catalog()
            .iter()
            .find(|entry| entry.file == file)
            .ok_or_else(|| NativeCliError::usage(format!("unknown schema file {file:?}")))?,
        _ => {
            return Err(NativeCliError::usage(
                "schema get requires exactly one of --id or --file",
            ))
        }
    };
    let content: Value = serde_json::from_str(entry.content)?;
    Ok(report(
        "casegraphen schema get",
        json!({
            "file": entry.file,
            "stability": entry.stability.as_str(),
            "id": entry.id,
            "content": content,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_reports_both_stability_tiers() {
        let value = schema_list();
        let schemas = value["result"]["schemas"]
            .as_array()
            .expect("schemas array");
        assert!(schemas
            .iter()
            .any(|entry| entry["stability"] == json!("stable")));
        assert!(schemas
            .iter()
            .any(|entry| entry["stability"] == json!("experimental")));
    }

    #[test]
    fn get_by_id_returns_the_named_schema() {
        let value = schema_get(Some("highergraphen.case.operation_gate_profiles.v1"), None)
            .expect("known id resolves");
        assert_eq!(
            value["result"]["file"],
            json!("operation-gate-profiles.schema.json")
        );
        assert_eq!(value["result"]["stability"], json!("stable"));
        assert_eq!(
            value["result"]["content"]["$id"],
            json!("highergraphen.case.operation_gate_profiles.v1")
        );
    }

    #[test]
    fn get_by_file_returns_the_named_content() {
        let value =
            schema_get(None, Some("runtime.node_report.schema.json")).expect("known file resolves");
        assert_eq!(value["result"]["stability"], json!("experimental"));
        assert_eq!(
            value["result"]["id"],
            json!("casegraphen.experimental.runtime.node_report.v0")
        );
    }

    #[test]
    fn unknown_id_is_refused() {
        let error = schema_get(Some("highergraphen.case.does_not_exist.v1"), None)
            .expect_err("unknown id refused");
        assert_eq!(error.error_code(), "usage");
    }

    #[test]
    fn unknown_file_is_refused() {
        let error =
            schema_get(None, Some("does-not-exist.schema.json")).expect_err("unknown file refused");
        assert_eq!(error.error_code(), "usage");
    }

    #[test]
    fn neither_id_nor_file_is_refused() {
        let error = schema_get(None, None).expect_err("neither selector refused");
        assert_eq!(error.error_code(), "usage");
    }
}
