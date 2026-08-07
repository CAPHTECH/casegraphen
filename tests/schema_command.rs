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

/// `schema get`'s content for `id`, parsed. Panics on refusal — every call
/// site here expects a known selector to resolve.
fn get_content(selector: &str, value: &str) -> Value {
    let output = run(&["schema", "get", selector, value, "--format", "json"]);
    assert!(
        output.status.success(),
        "schema get {selector} {value}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("get JSON");
    report["result"]["content"].clone()
}

/// Runs a bare `python3 -m jsonschema` — no `--base-uri`, no test-only
/// resolution helper (`tests/command.rs`'s `assert_jsonschema_valid` adds
/// one, but that is for the *source files on disk*; a consumer served
/// through the binary has neither the helper nor the files) — against
/// `schema` and `instance`, both already-parsed `Value`s from `schema get`.
/// This is exactly the recipe `references/mutating.md` documents.
fn validate_with_bare_jsonschema(
    schema: &Value,
    instance: &Value,
    tag: &str,
) -> std::process::Output {
    let directory = std::env::temp_dir().join(format!(
        "casegraphen-schema-command-{tag}-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create temp directory");
    let schema_path = directory.join("schema.json");
    let instance_path = directory.join("instance.json");
    fs::write(
        &schema_path,
        serde_json::to_vec(schema).expect("serialize schema"),
    )
    .expect("write schema");
    fs::write(
        &instance_path,
        serde_json::to_vec(instance).expect("serialize instance"),
    )
    .expect("write instance");

    let validation = Command::new("python3")
        .args([
            "-m",
            "jsonschema",
            schema_path.to_str().expect("schema path"),
            "--instance",
            instance_path.to_str().expect("instance path"),
        ])
        .output()
        .expect("run python jsonschema validator");
    fs::remove_dir_all(&directory).ok();
    validation
}

/// Issue #147's distribution consequence: `native.morphism-propose-input`
/// reuses `case_morphism`'s property definitions from
/// `native.case.space.schema.json` by reference instead of duplicating
/// them, but a consumer has only `schema get` — never a checkout to resolve
/// a relative cross-file `$ref` against. This proves the served schema is
/// self-contained: a schema with a cross-file `$ref` that only validated
/// through `tests/command.rs`'s `--base-uri` helper would still fail here.
#[test]
fn get_by_id_serves_a_schema_that_validates_standalone_with_no_base_uri() {
    let schema = get_content("--id", "highergraphen.case.morphism_propose_input.v1");
    let example = get_content("--file", "native.morphism-propose-input.example.json");

    let validation = validate_with_bare_jsonschema(&schema, &example, "standalone");
    assert!(
        validation.status.success(),
        "served schema should validate standalone (no --base-uri)\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&validation.stdout),
        String::from_utf8_lossy(&validation.stderr)
    );
}
