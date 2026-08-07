#![allow(missing_docs)]

//! Issue #111: a consumer with only the installed binary must be able to
//! obtain every schema and example a skill instructs them to author
//! against, without cloning this repository. These tests drive the real
//! `casegraphen` binary (never the library directly) because that is what a
//! consumer actually has.

use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("run `casegraphen {}`: {error}", args.join(" ")))
}

#[test]
fn list_reports_the_stable_and_experimental_tiers() {
    let output = run(&["schema", "list", "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("list JSON");
    let schemas = report["result"]["schemas"]
        .as_array()
        .expect("schemas array");

    let gate_profiles = schemas
        .iter()
        .find(|entry| entry["file"] == "operation-gate-profiles.schema.json")
        .expect("operation-gate-profiles.schema.json listed");
    assert_eq!(gate_profiles["stability"], "stable");
    assert_eq!(
        gate_profiles["id"],
        "highergraphen.case.operation_gate_profiles.v1"
    );

    let node_report = schemas
        .iter()
        .find(|entry| entry["file"] == "runtime.node_report.schema.json")
        .expect("runtime.node_report.schema.json listed");
    assert_eq!(node_report["stability"], "experimental");
}

/// The sharp case from issue #111: `skills/casegraphen-operate/SKILL.md`
/// names `highergraphen.case.operation_gate_profiles.v1` and instructs the
/// reader to author a gate profile document against it, without shipping
/// the schema. This proves the binary alone now supplies it, and that what
/// it supplies is semantically identical to the schema this crate ships —
/// not a copy that could drift from it. The comparison is on parsed
/// `serde_json::Value`s, not raw bytes: `schema get` re-serializes the
/// embedded content into the report envelope, so byte-for-byte equality
/// with the source file was never the property to prove here.
#[test]
fn get_by_id_returns_the_gate_profiles_schema_matching_the_source_file() {
    let output = run(&[
        "schema",
        "get",
        "--id",
        "highergraphen.case.operation_gate_profiles.v1",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("get JSON");
    assert_eq!(
        report["result"]["file"],
        "operation-gate-profiles.schema.json"
    );
    assert_eq!(report["result"]["stability"], "stable");

    let on_disk: Value = serde_json::from_str(
        &fs::read_to_string(root().join("schemas/casegraphen/operation-gate-profiles.schema.json"))
            .expect("read schema from disk"),
    )
    .expect("on-disk schema JSON");
    assert_eq!(report["result"]["content"], on_disk);
}

#[test]
fn get_by_file_returns_an_experimental_schema() {
    let output = run(&[
        "schema",
        "get",
        "--file",
        "runtime.node_report.schema.json",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("get JSON");
    assert_eq!(report["result"]["stability"], "experimental");
    assert_eq!(
        report["result"]["id"],
        "casegraphen.experimental.runtime.node_report.v0"
    );
}

/// An example fixture is reachable by filename even where it declares no
/// `$id` of its own to be looked up by (`--id` only matches `*.schema.json`
/// entries).
#[test]
fn get_by_file_returns_an_example_fixture() {
    let output = run(&[
        "schema",
        "get",
        "--file",
        "operation-gate-profiles.example.json",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("get JSON");
    assert_eq!(
        report["result"]["content"]["profiles"][0]["name"],
        "native-audit"
    );
}

#[test]
fn get_without_a_selector_is_refused() {
    let output = run(&["schema", "get", "--format", "json"]);
    assert!(!output.status.success());
    let refusal: Value = serde_json::from_slice(&output.stderr).expect("refusal JSON");
    assert_eq!(refusal["error_code"], "usage");
}

#[test]
fn get_with_an_unknown_id_is_refused() {
    let output = run(&[
        "schema",
        "get",
        "--id",
        "highergraphen.case.does_not_exist.v1",
        "--format",
        "json",
    ]);
    assert!(!output.status.success());
    let refusal: Value = serde_json::from_slice(&output.stderr).expect("refusal JSON");
    assert_eq!(refusal["error_code"], "usage");
}
