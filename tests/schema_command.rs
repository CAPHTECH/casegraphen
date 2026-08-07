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

/// Issue #153: the only genesis example `schema get` served was
/// `native.case.space.example.json` at 37,637 bytes (~9,400 tokens) — an
/// operator either copied that or wrote one from the 2,085-token schema.
/// `mini-genesis.case.space.json`
/// (`docs/guides/entry-ladder/mini-genesis.case.space.json`) is the minimal
/// governed genesis #123 already built and `entry_ladder_conformance.rs`
/// already tests — 3,692 bytes, ~920 tokens — but `schema get --file
/// mini-genesis.case.space.json` answered `unknown schema file`. This
/// proves it is now reachable, matches the on-disk file exactly (same file,
/// not a copy — `schema_catalog::EXTRA` embeds it from its real path), and
/// is not merely present but usable: `lift native` accepts it unmodified,
/// the same way the issue's own dogfooding did.
#[test]
fn get_by_file_returns_the_minimal_genesis_and_it_still_lifts() {
    let output = run(&[
        "schema",
        "get",
        "--file",
        "mini-genesis.case.space.json",
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("get JSON");
    assert_eq!(report["result"]["id"], Value::Null);

    let on_disk: Value = serde_json::from_str(
        &fs::read_to_string(root().join("docs/guides/entry-ladder/mini-genesis.case.space.json"))
            .expect("read mini-genesis from disk"),
    )
    .expect("on-disk mini-genesis JSON");
    assert_eq!(report["result"]["content"], on_disk);

    let directory = std::env::temp_dir().join(format!(
        "casegraphen-schema-command-mini-genesis-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create temp directory");
    let input_path = directory.join("served-mini-genesis.json");
    fs::write(
        &input_path,
        serde_json::to_vec(&report["result"]["content"]).expect("serialize served content"),
    )
    .expect("write served content");

    let lift = run(&[
        "lift",
        "native",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input_path.to_str().expect("input path"),
        "--revision-id",
        "revision:mini-genesis",
        "--format",
        "json",
    ]);
    assert!(
        lift.status.success(),
        "the served content should lift the same way the file on disk does: {}",
        String::from_utf8_lossy(&lift.stderr)
    );

    fs::remove_dir_all(&directory).ok();
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

/// `morphism.metadata` stays a fully open object — other reserved keys
/// (`operation_gate`, `native_review_schema_version`, ...) must remain
/// admissible — but `metadata.payload`, when present, is now closed to the
/// `MorphismPayload` shape the CLI actually parses. Before this, an invented
/// key such as `retired_cells` validated clean against the schema and was
/// only caught at propose time by the CLI's own refusal; the schema
/// documented nothing about it because `metadata` was `{"type": "object"}`
/// end to end.
#[test]
fn served_schema_closes_metadata_payload_but_not_metadata_itself() {
    let schema = get_content("--id", "highergraphen.case.morphism_propose_input.v1");
    let mut example = get_content("--file", "native.morphism-propose-input.example.json");

    // An invented payload key is refused.
    let mut invented_payload_key = example.clone();
    invented_payload_key["metadata"]["payload"]["retired_cells"] = serde_json::json!(["work:x"]);
    let rejected = validate_with_bare_jsonschema(&schema, &invented_payload_key, "bad-payload-key");
    assert!(
        !rejected.status.success(),
        "metadata.payload.retired_cells should fail schema validation \
         (retired_cells has never been a MorphismPayload field — retirement is retired_ids)"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("retired_cells"),
        "the refusal should name the invented key: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    // A reserved metadata key that is not `payload` stays admissible —
    // `metadata` itself must not have become closed.
    example["metadata"]["operation_gate"] = serde_json::json!({"unrelated": "to payload"});
    let still_open = validate_with_bare_jsonschema(&schema, &example, "open-metadata-key");
    assert!(
        still_open.status.success(),
        "an unrelated metadata key should still validate\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&still_open.stdout),
        String::from_utf8_lossy(&still_open.stderr)
    );
}
