#![allow(missing_docs)]

use arbtest::arbitrary::Arbitrary;
use casegraphen::{
    exec::AllowedTransitionClass,
    execution_topology::{execution_topology_content_hash, ExecutionTopology},
    graph_compiler::{
        compile_execution_topology, reviewed_compilation_mode, CompilationTarget, CompilerRequest,
        NodePlanMapping,
    },
    native_model::{CaseCellLifecycle, CaseCellType, CaseMorphismType},
    native_store::NativeCaseStore,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn version_command_reports_package_version() {
    // This binary is built from this checkout's own git worktree by cargo's
    // test harness, so build.rs always finds a repository here: the commit
    // suffix is present, not merely optional. A packaged/registry build
    // (exercised separately in the packaging gate) is the case where it is
    // absent and the plain version prints instead.
    let head = git_short_head();
    // Recomputes the same definition build.rs uses (dirty means tracked
    // files differ from HEAD; an untracked file does not count) rather than
    // deriving it independently — this only proves build.rs stays in sync
    // with itself, not that the definition is the right one.
    let dirty = !std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .expect("git status")
        .stdout
        .is_empty();
    let expected = format!(
        "casegraphen {} ({}{})",
        env!("CARGO_PKG_VERSION"),
        head,
        if dirty { "-dirty" } else { "" }
    );

    for args in [["version"], ["--version"], ["-V"]] {
        let output = run_cli(&args);

        assert!(output.status.success(), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output).trim_end(), expected);
        assert!(stderr(&output).is_empty());
    }
}

fn git_short_head() -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("git rev-parse");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("utf8 git output")
        .trim()
        .to_string()
}

#[test]
fn native_case_commands_create_import_list_inspect_history_and_replay() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let created = run_cli(&[
        "space",
        "new",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        "case_space:native-cli-created",
        "--space-id",
        "space:native-cli",
        "--title",
        "Native CLI created case",
        "--revision-id",
        "revision:native-cli-created",
        "--format",
        "json",
    ]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert_eq!(
        stdout_json(&created)["result"]["record"]["case_space_id"],
        json!("case_space:native-cli-created")
    );

    let imported = import_native_case_space(&directory, "revision:native-cli-imported");
    assert_eq!(
        stdout_json(&imported)["metadata"]["command"],
        json!("casegraphen lift native")
    );

    let list = run_cli(&[
        "space",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--format",
        "json",
    ]);
    assert!(list.status.success(), "stderr: {}", stderr(&list));
    assert_eq!(
        stdout_json(&list)["result"]["case_spaces"]
            .as_array()
            .expect("case spaces")
            .len(),
        2
    );

    let inspect = run_native_case_store_command(&directory, "inspect");
    assert_eq!(
        stdout_json(&inspect)["result"]["record"]["current_revision_id"],
        json!("revision:native-cli-imported")
    );

    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"][0]["entry_id"],
        json!("morphism_log_entry:genesis-native-contract")
    );

    let replay = run_native_case_store_command(&directory, "replay");
    let replay_json = stdout_json(&replay);
    assert_eq!(
        replay_json["result"]["replay"]["case_space"]["case_space_id"],
        json!("case_space:native-case-management-contract")
    );
    assert!(replay_json["result"]["replay"]["case_space"]["projections"]
        .as_array()
        .expect("projections")
        .iter()
        .all(|projection| projection["revision_id"] == json!("revision:native-cli-imported")));

    // ADR 0011: the log is reported once, inside the folded case space. A
    // caller wanting only the log runs `space history`, which is the command
    // this assertion pairs with above.
    assert!(
        replay_json["result"]["replay"].get("history").is_none(),
        "space replay must not echo the morphism log beside case_space.morphism_log"
    );
    assert_eq!(
        replay_json["result"]["replay"]["case_space"]["morphism_log"]
            .as_array()
            .expect("replayed morphism log")
            .len(),
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len()
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_new_report_and_gate_refusal_declare_the_space_has_no_capability_cells() {
    // #124: `space new` mints a genesis with only a root cell, and capability
    // cells enter only at lift/import (ADR 0003 §4) — there is no post-genesis
    // path that adds one. So a space made this way can never satisfy an
    // operation gate, and both ends of that experience must say so: the
    // creation report at the moment the space is made, and the eventual gate
    // refusal in terms that point at the space rather than the supplied id.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:no-capability-cells";
    let source_boundary_id = format!("source_boundary:{case_space_id}");

    let created = run_cli(&[
        "space",
        "new",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--space-id",
        "space:no-capability-cells",
        "--title",
        "No capability cells",
        "--revision-id",
        "revision:no-capability-cells-genesis",
        "--format",
        "json",
    ]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let created_json = stdout_json(&created);
    let capability_gate = &created_json["result"]["capability_gate"];
    assert_eq!(capability_gate["capability_cell_count"], json!(0));
    assert_eq!(capability_gate["durable_mutation_possible"], json!(false));
    assert!(
        capability_gate["note"]
            .as_str()
            .expect("capability_gate note")
            .contains("no capability cells"),
        "space new must declare, at creation, that it made a space with no capability cells: {capability_gate}"
    );

    let gated = run_cli(&[
        "cell",
        "transition",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--base-revision-id",
        "revision:no-capability-cells-genesis",
        "--cell-id",
        "case:native-root",
        "--to",
        "active",
        "--actor-id",
        "actor:anyone",
        "--capability-id",
        "capability:anything",
        "--operation-scope-id",
        case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        &source_boundary_id,
        "--format",
        "json",
    ]);
    assert!(!gated.status.success());
    let gated_json = stderr_json(&gated);
    assert_eq!(gated_json["error_code"], json!("gate_violation"));
    let message = gated_json["message"].as_str().expect("refusal message");
    assert!(
        message.contains("this case space has no capability cells at all"),
        "refusal must name the space's own property, not just the supplied id: {message}"
    );
    assert!(
        message.contains("lift native"),
        "refusal must name the remedy: {message}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn gate_refusal_for_a_wrong_capability_id_stays_id_scoped_when_the_space_has_capabilities() {
    // The other arm of #124's boundary: when a space *does* have capability
    // cells and the caller simply named one that is not among them, the
    // original id-scoped message is correct and must not gain the
    // no-capability-cells wording — that would misdescribe a space that could
    // in fact satisfy the gate with a different id.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-wrong-capability-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let gated = run_cli(&[
        "cell",
        "transition",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:native-wrong-capability-base",
        "--cell-id",
        "goal:native-case-contract",
        "--to",
        "resolved",
        "--reason",
        "The goal is complete",
        "--actor-id",
        "actor:native-transition-cli",
        "--capability-id",
        "capability:not-present",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!gated.status.success());
    let gated_json = stderr_json(&gated);
    assert_eq!(gated_json["error_code"], json!("gate_violation"));
    let message = gated_json["message"].as_str().expect("refusal message");
    assert!(
        message.contains(
            "capability capability:not-present does not resolve to an existing case cell"
        ),
        "stderr: {message}"
    );
    assert!(
        !message.contains("no capability cells at all"),
        "a space that has capability cells must not get the no-capability-cells wording: {message}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_rebuild_recovers_a_deleted_nearest_snapshot() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-rebuild");
    let imported_json = stdout_json(&imported);
    let relative_snapshot = imported_json["result"]["record"]["nearest_snapshot_path"]
        .as_str()
        .expect("nearest snapshot path");
    let snapshot_path = directory.join(relative_snapshot);
    fs::remove_file(&snapshot_path).expect("delete nearest snapshot");

    let rebuild = run_cli(&[
        "space",
        "rebuild",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);

    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));
    let rebuild_json = stdout_json(&rebuild);
    assert_eq!(
        rebuild_json["metadata"]["command"],
        json!("casegraphen space rebuild")
    );
    assert_eq!(
        rebuild_json["result"]["rebuild"]["revisions"][0]["snapshot_status"],
        json!("rebuilt")
    );
    assert!(snapshot_path.exists());
    let validation = run_native_case_store_command(&directory, "validate");
    assert_eq!(
        stdout_json(&validation)["result"]["validation"]["valid"],
        json!(true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_rebuild_adopts_a_missing_head_only_after_full_verification() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-adopt-head");
    let imported_json = stdout_json(&imported);
    let log_path = imported_native_log_path(&directory, &imported_json);
    let head_path = log_path.with_file_name("morphism_log.head.json");
    fs::remove_file(&head_path).expect("remove morphism log head");

    for operation in ["replay", "validate", "rebuild"] {
        let refused = run_cli(&[
            "space",
            operation,
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--format",
            "json",
        ]);
        assert!(
            !refused.status.success(),
            "{operation} must refuse a missing head"
        );
        assert!(stderr(&refused).contains("morphism log head is required"));
    }

    let adopted = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);

    assert!(adopted.status.success(), "stderr: {}", stderr(&adopted));
    assert_eq!(
        stdout_json(&adopted)["result"]["rebuild"]["head_adopted"],
        json!(true)
    );
    assert!(head_path.is_file());
    for operation in ["replay", "validate", "reason"] {
        let restored = run_native_case_store_command(&directory, operation);
        assert!(
            restored.status.success(),
            "{operation} must work after adoption"
        );
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_rebuild_refuses_to_adopt_a_tampered_log_and_leaves_head_missing() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-adopt-tampered");
    let imported_json = stdout_json(&imported);
    let log_path = imported_native_log_path(&directory, &imported_json);
    let head_path = log_path.with_file_name("morphism_log.head.json");
    let snapshot_path = directory.join(
        imported_json["result"]["record"]["nearest_snapshot_path"]
            .as_str()
            .expect("nearest snapshot path"),
    );
    fs::remove_file(&head_path).expect("remove morphism log head");
    fs::remove_file(&snapshot_path).expect("remove snapshot so adoption must verify the fold");

    let mut entry: Value = serde_json::from_str(
        fs::read_to_string(&log_path)
            .expect("read morphism log")
            .trim_end(),
    )
    .expect("parse morphism log entry");
    entry["morphism"]["metadata"]["payload"]["added_cells"][0]["title"] =
        json!("Tampered before adoption");
    fs::write(
        &log_path,
        format!(
            "{}\n",
            serde_json::to_string(&entry).expect("serialize tampered log entry")
        ),
    )
    .expect("write tampered morphism log");

    let refused = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);

    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("disagrees with folded log"));
    // A store integrity mismatch (issue #22): the on-disk log disagrees
    // with what folding it produces, distinct from every other refusal
    // code — the correct response is to stop and investigate, not retry.
    assert_eq!(
        stderr_json(&refused)["error_code"],
        json!("store_integrity")
    );
    assert!(
        !head_path.exists(),
        "refused adoption must not write a head"
    );
    assert!(
        !snapshot_path.exists(),
        "refused adoption must not rebuild snapshots before the fold verifies"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_rebuild_adoption_refuses_a_disagreeing_existing_head_without_overwriting_it() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-adopt-stale-head");
    let imported_json = stdout_json(&imported);
    let log_path = imported_native_log_path(&directory, &imported_json);
    let head_path = log_path.with_file_name("morphism_log.head.json");
    let original_head = fs::read(&head_path).expect("read original morphism log head");
    let mut entry: Value = serde_json::from_str(
        fs::read_to_string(&log_path)
            .expect("read morphism log")
            .trim_end(),
    )
    .expect("parse morphism log entry");
    entry["actor_id"] = json!("actor:tampered-before-adoption");
    fs::write(
        &log_path,
        format!(
            "{}\n",
            serde_json::to_string(&entry).expect("serialize tampered log entry")
        ),
    )
    .expect("write tampered morphism log");

    let refused = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);

    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("morphism log head is stale or disagrees"));
    assert_eq!(
        stderr_json(&refused)["error_code"],
        json!("store_integrity")
    );
    assert_eq!(
        fs::read(&head_path).expect("read refused morphism log head"),
        original_head
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_rebuild_adoption_is_a_no_op_for_a_healthy_modern_head() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-adopt-no-op");
    let imported_json = stdout_json(&imported);
    let log_path = imported_native_log_path(&directory, &imported_json);
    let head_path = log_path.with_file_name("morphism_log.head.json");
    let original_head = fs::read(&head_path).expect("read original morphism log head");

    let rebuild = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);

    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));
    assert_eq!(
        stdout_json(&rebuild)["result"]["rebuild"]["head_adopted"],
        json!(false)
    );
    assert_eq!(
        fs::read(&head_path).expect("read unchanged morphism log head"),
        original_head
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn canonical_higher_order_commands_route_to_native_reports() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let created = run_cli(&[
        "space",
        "new",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        "case_space:canonical-cli-created",
        "--space-id",
        "space:canonical-cli",
        "--title",
        "Canonical CLI created space",
        "--revision-id",
        "revision:canonical-cli-created",
        "--format",
        "json",
    ]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert_eq!(
        stdout_json(&created)["metadata"]["command"],
        json!("casegraphen space new")
    );

    import_native_case_space(&directory, "revision:native-cli-imported");

    let obstruction = run_cli(&[
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        obstruction.status.success(),
        "stderr: {}",
        stderr(&obstruction)
    );
    assert_eq!(
        stdout_json(&obstruction)["metadata"]["command"],
        json!("casegraphen obstruction list")
    );

    let completion = run_cli(&[
        "completion",
        "candidates",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        completion.status.success(),
        "stderr: {}",
        stderr(&completion)
    );
    assert_eq!(
        stdout_json(&completion)["metadata"]["command"],
        json!("casegraphen completion candidates")
    );

    let invariant = run_cli(&[
        "invariant",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(invariant.status.success(), "stderr: {}", stderr(&invariant));
    assert_eq!(
        stdout_json(&invariant)["metadata"]["command"],
        json!("casegraphen invariant check")
    );
    assert_eq!(
        stdout_json(&invariant)["result"]["validation"]["valid"],
        json!(true)
    );
    assert!(stdout_json(&invariant)["result"]["evaluation"]
        .as_object()
        .expect("invariant evaluation")
        .contains_key("evidence_findings"));
    assert!(stdout_json(&invariant)["result"]["evidence_findings"]
        .as_object()
        .expect("invariant evidence findings")
        .contains_key("unreviewed_inference_ids"));
    assert!(stdout_json(&invariant)["result"]["projection_loss"]
        .as_array()
        .is_some());

    let projection = run_cli(&[
        "projection",
        "apply",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--projection",
        projection_fixture().to_str().expect("projection path"),
        "--format",
        "json",
    ]);
    assert!(
        projection.status.success(),
        "stderr: {}",
        stderr(&projection)
    );
    assert_eq!(
        stdout_json(&projection)["metadata"]["command"],
        json!("casegraphen projection apply")
    );
    assert_eq!(
        stdout_json(&projection)["result"]["projection_request"]["projection_id"],
        json!("projection:ai-review")
    );

    let equivalence = run_cli(&[
        "equivalence",
        "check",
        "--left-store",
        directory.to_str().expect("temp path"),
        "--left-case-space-id",
        native_case_space_id(),
        "--right-store",
        directory.to_str().expect("temp path"),
        "--right-case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        equivalence.status.success(),
        "stderr: {}",
        stderr(&equivalence)
    );
    assert_eq!(
        stdout_json(&equivalence)["metadata"]["command"],
        json!("casegraphen equivalence check")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn generated_native_cli_report_validates_against_schema() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:native-cli-imported");
    assert!(imported.status.success(), "stderr: {}", stderr(&imported));

    let report_path = directory.join("native-cli-import.report.json");
    fs::write(&report_path, stdout(&imported)).expect("write native CLI report");
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/native-cli.report.schema.json"),
        &report_path,
    );

    let topology_report_path = directory.join("native-cli-topology.report.json");
    let topology = run_cli(&[
        "space",
        "topology",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
        "--higher-order",
        "--max-dimension",
        "1",
        "--output",
        topology_report_path.to_str().expect("report path"),
    ]);
    assert!(topology.status.success(), "stderr: {}", stderr(&topology));
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/native-cli.report.schema.json"),
        &topology_report_path,
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_reasoning_commands_emit_domain_reports_and_output_file() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let reason = run_native_case_store_command(&directory, "reason");
    assert!(reason.status.success(), "stderr: {}", stderr(&reason));
    let reason_json = stdout_json(&reason);
    assert_eq!(
        reason_json["result"]["evaluation"]["progress"],
        json!("active")
    );
    assert_eq!(
        reason_json["result"]["evaluation"]["assurance"],
        json!("review_required")
    );
    assert!(reason_json["result"]["evaluation"]["completion_candidates"]
        .as_array()
        .expect("completion candidates")
        .is_empty());

    let frontier_output = directory.join("native.frontier.report.json");
    let frontier = run_cli(&[
        "space",
        "frontier",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
        "--output",
        frontier_output.to_str().expect("frontier output path"),
    ]);
    assert!(frontier.status.success(), "stderr: {}", stderr(&frontier));
    assert!(stdout(&frontier).is_empty());
    assert!(json_file(frontier_output)["result"]["frontier_cell_ids"]
        .as_array()
        .expect("frontier ids")
        .contains(&json!("goal:native-case-contract")));

    let close_check = run_cli(&[
        "invariant",
        "close-check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:native-cli-imported",
        "--validation-evidence-id",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
    ]);
    assert!(
        close_check.status.success(),
        "stderr: {}",
        stderr(&close_check)
    );
    assert_eq!(
        stdout_json(&close_check)["result"]["close_check"]["case_space_id"],
        json!(native_case_space_id())
    );
    assert!(
        stdout_json(&close_check)["result"]["close_check"]["invariant_results"]
            .as_array()
            .expect("close invariants")
            .iter()
            .any(|invariant| invariant["invariant_id"]
                == json!("close:native-projection-loss-declared")
                && invariant["passed"] == json!(false))
    );
    assert!(
        stdout_json(&close_check)["result"]["core_extensions"]["derivations"]
            .as_array()
            .expect("close-check core derivations")
            .iter()
            .any(|derivation| derivation["conclusion"]
                == stdout_json(&close_check)["result"]["close_check"]["check_id"])
    );
    assert_eq!(
        stdout_json(&close_check)["result"]["core_extension_blocked"],
        json!(false)
    );

    for command in ["obstructions", "completions", "evidence", "project"] {
        let output = match command {
            "obstructions" => run_native_store_command(&directory, "obstruction", "list"),
            "completions" => run_native_store_command(&directory, "completion", "candidates"),
            "evidence" | "project" => run_native_case_store_command(&directory, command),
            _ => unreachable!("test command set is fixed"),
        };
        assert!(
            output.status.success(),
            "{command} stderr: {}",
            stderr(&output)
        );
        let expected = match command {
            "obstructions" => "casegraphen obstruction list",
            "completions" => "casegraphen completion candidates",
            "evidence" => "casegraphen space evidence",
            "project" => "casegraphen space project",
            _ => unreachable!("test command set is fixed"),
        };
        assert_eq!(stdout_json(&output)["metadata"]["command"], json!(expected));
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn space_reason_text_renders_the_evaluation_without_changing_json_or_exit_semantics() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let graph_id = "workflow_graph:issue3-text";
    let graph = workflow_attack_graph(graph_id, Vec::new());
    let lifted = lift_workflow_graph(&directory, &graph, "issue3-text");
    assert!(lifted.status.success(), "stderr: {}", stderr(&lifted));
    let case_space_id = format!("case_space:{graph_id}");

    let json_report = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]);
    assert!(
        json_report.status.success(),
        "stderr: {}",
        stderr(&json_report)
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&json_report.stdout)),
        "a9f86abdf9ce4303889e895c828f12b08a3b5f135285aa381518cfbd52703699",
        "the JSON bytes changed from the two-axis-status baseline — expected here: \
         NativeReviewGap gained `requirement_satisfied` (issue #20's gap-marking fix)"
    );
    let evaluation = &stdout_json(&json_report)["result"]["evaluation"];
    let frontier_ids = evaluation["frontier_cell_ids"]
        .as_array()
        .expect("frontier ids");
    assert!(!frontier_ids.is_empty());
    let blocking_obstruction = evaluation["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .find(|obstruction| obstruction["blocking"] == json!(true))
        .expect("blocking obstruction");

    let text_report = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "text",
    ]);
    assert!(
        text_report.status.success(),
        "stderr: {}",
        stderr(&text_report)
    );
    let text = stdout(&text_report);
    for frontier_id in frontier_ids {
        assert!(
            text.contains(frontier_id.as_str().expect("frontier id string")),
            "text omitted frontier id {frontier_id}"
        );
    }
    assert!(text.contains(
        blocking_obstruction["explanation"]
            .as_str()
            .expect("obstruction explanation")
    ));
    for witness_id in blocking_obstruction["witness_ids"]
        .as_array()
        .expect("witness ids")
    {
        assert!(
            text.contains(witness_id.as_str().expect("witness id string")),
            "text omitted obstruction witness {witness_id}"
        );
    }

    for format in ["json", "text"] {
        let strict = run_cli(&[
            "space",
            "reason",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            &case_space_id,
            "--strict",
            "--format",
            format,
        ]);
        assert_eq!(
            strict.status.code(),
            Some(2),
            "strict {format} stderr: {}",
            stderr(&strict)
        );
        if format == "text" {
            assert_eq!(strict.stdout, text_report.stdout);
        }
    }

    let refused = run_cli(&[
        "space",
        "inspect",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "text",
    ]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(stderr(&refused).contains("--format json is required"));

    let output_path = directory.join("reason.txt");
    let written = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "text",
        "--output",
        output_path.to_str().expect("output path"),
    ]);
    assert!(written.status.success(), "stderr: {}", stderr(&written));
    assert!(written.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(output_path).expect("read text output"),
        text
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// `--since-revision` is an assertion (ADR 0008's `--base-revision-id`
/// discipline): a revision this case space's history actually reached
/// produces exactly the log slice recorded after it, and a revision it never
/// reached is refused rather than resolved to "nearest".
#[test]
fn space_reason_text_since_revision_lists_the_log_slice_recorded_after_it() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let base_revision = "revision:native-cli-imported";
    import_native_case_space(&directory, base_revision);
    let case_space_id = native_case_space_id();

    let transitioned = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            case_space_id,
            "--base-revision-id",
            base_revision,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "resolved",
            "--reason",
            "since-revision fixture transition",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(
        transitioned.status.success(),
        "stderr: {}",
        stderr(&transitioned)
    );
    let transitioned_json = stdout_json(&transitioned);
    let next_revision = transitioned_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("transitioned revision")
        .to_owned();

    let since = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
        "--since-revision",
        base_revision,
    ]);
    assert!(since.status.success(), "stderr: {}", stderr(&since));
    let since_text = stdout(&since);
    assert!(since_text.contains("\nChanged since:"));
    assert!(since_text.contains(&next_revision));
    assert!(since_text.contains("goal:native-case-contract"));

    // Asking from the space's own current revision must report no changes,
    // not an error — the slice after the tip of history is empty.
    let since_tip = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
        "--since-revision",
        &next_revision,
    ]);
    assert!(since_tip.status.success(), "stderr: {}", stderr(&since_tip));
    assert!(stdout(&since_tip).contains("\nChanged since:\n  (none)"));

    // A revision this space's history never reached is refused, not resolved
    // to the nearest one.
    let unknown = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
        "--since-revision",
        "revision:never-reached",
    ]);
    // Refusals on a `--format text` command render as the same prose the
    // report itself would have used, not the JSON refusal envelope
    // (`cli.rs::refusal_text` renders in the command's own resolved
    // format) — so this refusal is read off stderr as text, not JSON.
    assert!(!unknown.status.success());
    assert!(stderr(&unknown)
        .contains("--since-revision revision:never-reached is not a revision recorded"));

    // `--since-revision` only means something for the text rendering it
    // feeds; combined with `--format json` it must be refused rather than
    // silently ignored. This refusal is a parse-time usage error, rendered
    // in the format `scan_requested_format` reads off the raw argv (`json`
    // here), so it does come back as the structured envelope.
    let wrong_format = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
        "--since-revision",
        base_revision,
    ]);
    assert!(!wrong_format.status.success());
    assert_eq!(stderr_json(&wrong_format)["error_code"], json!("usage"));
    assert!(stderr_json(&wrong_format)["message"]
        .as_str()
        .expect("usage message")
        .contains("--since-revision is only valid on space reason --format text"));

    // `--since-revision` is extracted from argv only for `space reason`
    // (`parser.rs::extract_since_revision`, called from `parse_space`'s
    // "reason" arm alone) — every other reason-family operation, and
    // `space history`, never scan for the token at all, so it is refused as
    // a plain unrecognized argument rather than through a second copy of
    // the "only valid on space reason" message.
    let frontier_refused = run_cli(&[
        "space",
        "frontier",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
        "--since-revision",
        base_revision,
    ]);
    assert!(!frontier_refused.status.success());
    assert_eq!(stderr_json(&frontier_refused)["error_code"], json!("usage"));
    assert!(stderr_json(&frontier_refused)["message"]
        .as_str()
        .expect("usage message")
        .contains("unsupported native argument \"--since-revision\""));

    // `--format text` here, so this refusal renders as prose, not the JSON
    // envelope (same `scan_requested_format`-driven rule as `unknown` above).
    let history_refused = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
        "--since-revision",
        base_revision,
    ]);
    assert!(!history_refused.status.success());
    assert!(stderr(&history_refused).contains("unsupported native argument \"--since-revision\""));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_case_topology_emits_domain_report() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let output = run_cli(&[
        "space",
        "topology",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["metadata"]["command"],
        json!("casegraphen space topology")
    );
    assert_eq!(
        value["result"]["topology"]["topology"]["homology"]["coefficient_field"],
        json!("z2")
    );
    assert!(!value["result"]["topology"]["source_mapping"]["nodes"]
        .as_array()
        .expect("topology nodes")
        .is_empty());
    assert!(value["result"]["topology"].get("higher_order").is_none());

    let higher_order = run_cli(&[
        "space",
        "topology",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
        "--higher-order",
        "--max-dimension",
        "1",
    ]);
    assert!(
        higher_order.status.success(),
        "stderr: {}",
        stderr(&higher_order)
    );
    let higher_order_json = stdout_json(&higher_order);
    assert_eq!(
        higher_order_json["result"]["topology"]["higher_order"]["options"]["max_dimension"],
        json!(1)
    );
    assert!(
        !higher_order_json["result"]["topology"]["higher_order"]["persistence"]["intervals"]
            .as_array()
            .expect("native higher-order intervals")
            .is_empty()
    );
    assert_eq!(
        higher_order_json["result"]["topology"]["higher_order"]["filtration_source"],
        json!("native_morphism_log")
    );
    assert!(
        higher_order_json["result"]["topology"]["higher_order"]["stage_sources"]
            .as_array()
            .expect("native stage sources")
            .iter()
            .any(|stage| stage["source_type"] == json!("native_morphism_log_entry"))
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_case_topology_diff_compares_store_replays() {
    let left_directory = unique_temp_dir();
    let right_directory = unique_temp_dir();
    fs::create_dir_all(&left_directory).expect("create left temp directory");
    fs::create_dir_all(&right_directory).expect("create right temp directory");
    import_native_case_space(&left_directory, "revision:native-cli-left");
    import_native_case_space(&right_directory, "revision:native-cli-right");

    let output = run_cli(&[
        "space",
        "topology",
        "diff",
        "--left-store",
        left_directory.to_str().expect("left temp path"),
        "--left-case-space-id",
        native_case_space_id(),
        "--right-store",
        right_directory.to_str().expect("right temp path"),
        "--right-case-space-id",
        native_case_space_id(),
        "--format",
        "json",
        "--higher-order",
        "--max-dimension",
        "1",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["metadata"]["command"],
        json!("casegraphen space topology diff")
    );
    assert_eq!(
        value["result"]["topology_diff"]["right_space_id"],
        json!("space:higher-graphen-casegraphen")
    );
    assert!(value["result"]["topology_diff"]
        .get("higher_order")
        .is_some());

    fs::remove_dir_all(left_directory).expect("remove left temp directory");
    fs::remove_dir_all(right_directory).expect("remove right temp directory");
}

#[test]
fn native_close_check_uses_metadata_core_extensions_as_close_gate() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let native_path = directory.join("native.with-core-extension.json");
    let mut native_case = json_file(native_case_fixture());
    native_case["metadata"]["higher_graphen_extensions"] =
        invalid_core_extensions(native_case_space_id());
    fs::write(
        &native_path,
        serde_json::to_string_pretty(&native_case).expect("serialize native case"),
    )
    .expect("write native case");
    import_native_case_space_from_input(&directory, &native_path, "revision:native-cli-imported");

    let close_check = run_cli(&[
        "invariant",
        "close-check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:native-cli-imported",
        "--validation-evidence-id",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
    ]);

    assert!(
        close_check.status.success(),
        "stderr: {}",
        stderr(&close_check)
    );
    let value = stdout_json(&close_check);
    assert_eq!(value["result"]["core_extension_blocked"], json!(true));
    assert_eq!(value["result"]["close_check"]["closeable"], json!(false));
    assert_eq!(
        value["result"]["core_extensions"]["validation"]["blocked_count"],
        json!(1)
    );
    assert_eq!(
        value["result"]["close_check"]["operation_gate"]["actor_id"],
        json!("actor:casegraphen-cli")
    );
    assert_eq!(
        value["result"]["close_check"]["operation_gate"]["audience"],
        json!("audit")
    );
    assert_eq!(
        value["result"]["close_check"]["operation_gate"]["source_boundary_id"],
        json!("source_boundary:native-case-management-contract")
    );
    let temporal_checks = value["result"]["mathematical_diagnostics"]["temporal_checks"]
        .as_array()
        .expect("temporal checks");
    assert!(temporal_checks.iter().any(|check| check["id"]
        == json!(
            "temporal:no-dead-end-except-current-revision:case_space:native-case-management-contract"
        )));
    assert!(temporal_checks.iter().any(|check| check["id"]
        == json!(
            "temporal:validation-evidence-eventual:case_space:native-case-management-contract"
        )));
}

#[test]
fn native_morphism_propose_check_apply_and_reject_flow() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let apply_morphism_path = directory.join("apply.case_morphism.json");
    write_native_metadata_morphism(
        &apply_morphism_path,
        "morphism:native-cli-apply",
        "revision:native-cli-imported",
        "revision:native-cli-applied",
    );

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        apply_morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    assert_eq!(
        stdout_json(&propose)["result"]["morphism"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        stdout_json(&propose)["result"]["proposal_status"],
        json!("checked")
    );

    let check = run_cli(&[
        "morphism",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-cli-apply",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    assert_eq!(stdout_json(&check)["result"]["applicable"], json!(true));
    assert_eq!(
        stdout_json(&check)["result"]["morphism"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        stdout_json(&check)["result"]["core_extensions"]["scenarios"][0]["scenario_kind"],
        json!("planned")
    );
    assert!(
        stdout_json(&check)["result"]["core_extensions"]["schema_morphisms"]
            .as_array()
            .expect("morphism core schema morphisms")
            .iter()
            .any(|schema_morphism| schema_morphism["verification"]["checks"]
                .as_array()
                .expect("schema morphism checks")
                .contains(&json!("morphism:native-cli-apply")))
    );

    let apply_args = [
        "morphism",
        "apply",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-cli-apply",
        "--base-revision-id",
        "revision:native-cli-imported",
        "--reviewer-id",
        "reviewer:native-cli",
        "--reason",
        "Accept metadata-only CLI morphism",
        "--format",
        "json",
    ];
    let ungated_apply = run_cli(&apply_args);
    assert!(!ungated_apply.status.success());
    assert!(stderr(&ungated_apply).contains("--actor-id <id> is required for morphism apply"));

    let apply = run_cli_with_mutation_gate(&apply_args, "actor:native-mutation-cli");
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let apply_json = stdout_json(&apply);
    assert_eq!(
        apply_json["result"]["record"]["current_revision_id"],
        json!("revision:native-cli-applied")
    );
    assert_eq!(
        apply_json["result"]["entry"]["morphism"]["metadata"]["operation_gate"]["operation"],
        json!("morphism-apply")
    );
    assert_eq!(
        apply_json["result"]["entry"]["actor_id"],
        json!("actor:native-mutation-cli")
    );

    let reject_morphism_path = directory.join("reject.case_morphism.json");
    write_native_metadata_morphism(
        &reject_morphism_path,
        "morphism:native-cli-reject",
        "revision:native-cli-applied",
        "revision:native-cli-reject-candidate",
    );
    let propose_reject = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        reject_morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(
        propose_reject.status.success(),
        "stderr: {}",
        stderr(&propose_reject)
    );

    let reject_args = [
        "morphism",
        "reject",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-cli-reject",
        "--reviewer-id",
        "reviewer:native-cli",
        "--reason",
        "Reject native CLI proposal",
        "--revision-id",
        "revision:native-cli-rejected",
        "--format",
        "json",
    ];
    let ungated_reject = run_cli(&reject_args);
    assert!(!ungated_reject.status.success());
    assert!(stderr(&ungated_reject).contains("--actor-id <id> is required for morphism reject"));

    let reject = run_cli_with_mutation_gate(&reject_args, "actor:native-mutation-cli");
    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    let reject_json = stdout_json(&reject);
    assert_eq!(
        reject_json["result"]["entry"]["morphism"]["metadata"]["outcome_review_status"],
        json!("rejected")
    );
    assert_eq!(
        reject_json["result"]["record"]["current_revision_id"],
        json!("revision:native-cli-rejected")
    );
    assert_eq!(
        reject_json["result"]["entry"]["morphism"]["metadata"]["operation_gate"]["operation"],
        json!("morphism-reject")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn morphism_propose_accepts_the_shipped_example_and_derives_added_ids() {
    // The shipped `native.morphism-propose-input.example.json` targets
    // `native_case_fixture()` at exactly this revision, so a caller can feed
    // it straight through `morphism propose` with zero edits (issue #147:
    // the shipped example must be directly copyable). It also omits
    // `added_ids` and the other now-optional arrays, so this doubles as the
    // integration test for deriving `added_ids` from `metadata.payload` and
    // defaulting the rest to `[]`.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-contract-v1");

    let example_path = repo_path("schemas/casegraphen/native.morphism-propose-input.example.json");
    let example_before: Value = serde_json::from_str(
        &fs::read_to_string(&example_path).expect("read the shipped propose-input example"),
    )
    .expect("shipped example parses");
    for omitted in [
        "added_ids",
        "updated_ids",
        "retired_ids",
        "preserved_ids",
        "violated_invariant_ids",
        "evidence_ids",
        "source_ids",
    ] {
        assert!(
            example_before.get(omitted).is_none(),
            "shipped example should omit {omitted} to demonstrate it is optional on input"
        );
    }

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        example_path.to_str().expect("example path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let morphism = stdout_json(&propose)["result"]["morphism"].clone();
    assert_eq!(morphism["added_ids"], json!(["work:example-added-cell"]));
    for defaulted in [
        "updated_ids",
        "retired_ids",
        "preserved_ids",
        "violated_invariant_ids",
        "evidence_ids",
        "source_ids",
    ] {
        assert_eq!(
            morphism[defaulted],
            json!([]),
            "{defaulted} should default to [] when omitted from input"
        );
    }

    let apply = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:add-example-cell",
            "--base-revision-id",
            "revision:native-contract-v1",
            "--reviewer-id",
            "reviewer:native-cli",
            "--reason",
            "accept the shipped morphism-propose-input example",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let applied = stdout_json(&apply);
    assert_eq!(
        applied["result"]["record"]["current_revision_id"],
        json!("revision:native-contract-v2")
    );
    let applied_morphism = &applied["result"]["entry"]["morphism"];
    // The stored record still carries every array field the case-space
    // contract requires — defaulting on input never changes what gets
    // written (native.case.space.schema.json's `case_morphism` still
    // requires all of them).
    assert_eq!(
        applied_morphism["added_ids"],
        json!(["work:example-added-cell"])
    );
    for still_present in [
        "updated_ids",
        "retired_ids",
        "preserved_ids",
        "violated_invariant_ids",
        "evidence_ids",
        "source_ids",
    ] {
        assert_eq!(applied_morphism[still_present], json!([]));
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn morphism_propose_still_refuses_a_declared_added_ids_that_disagrees_with_the_payload() {
    // Deriving `added_ids` when it is omitted must not weaken the existing
    // cross-check: an author who declares a non-empty, wrong `added_ids`
    // still gets refused exactly as before (native_model.rs's
    // `require_matching_ids`). Derivation only fills a gap; it never
    // overrides a declared value.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-contract-v1");

    let example_path = repo_path("schemas/casegraphen/native.morphism-propose-input.example.json");
    let mut morphism: Value = serde_json::from_str(
        &fs::read_to_string(&example_path).expect("read the shipped propose-input example"),
    )
    .expect("shipped example parses");
    morphism["added_ids"] = json!(["bogus:mismatch"]);
    let mismatch_path = directory.join("mismatch.case_morphism.json");
    fs::write(
        &mismatch_path,
        serde_json::to_vec_pretty(&morphism).expect("serialize the mismatched morphism"),
    )
    .expect("write the mismatched morphism");

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        mismatch_path.to_str().expect("mismatch path"),
        "--format",
        "json",
    ]);
    assert!(!propose.status.success());
    assert!(stderr(&propose).contains("added_ids [bogus:mismatch] do not match"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn generic_morphism_refuses_capability_self_grant() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let native_path = directory.join("native.owner-capability.json");
    let mut native_case = json_file(native_case_fixture());
    let capability = native_case["case_cells"]
        .as_array_mut()
        .expect("native cells")
        .iter_mut()
        .find(|cell| cell["id"] == json!("capability:durable-mutation"))
        .expect("durable mutation capability");
    capability["metadata"]["actor_ids"] = json!(["actor:owner"]);
    let mut self_granted_capability = capability.clone();
    self_granted_capability["metadata"]["actor_ids"] = json!(["actor:owner", "actor:attacker"]);
    fs::write(
        &native_path,
        serde_json::to_string_pretty(&native_case).expect("serialize native case"),
    )
    .expect("write native case");
    import_native_case_space_from_input(
        &directory,
        &native_path,
        "revision:capability-self-grant-base",
    );

    let morphism_path = directory.join("capability-self-grant.case_morphism.json");
    let morphism = json!({
        "morphism_id": "morphism:capability-self-grant",
        "morphism_type": "update",
        "source_revision_id": "revision:capability-self-grant-base",
        "target_revision_id": "revision:capability-self-granted",
        "added_ids": [],
        "updated_ids": ["capability:durable-mutation"],
        "retired_ids": [],
        "preserved_ids": [],
        "violated_invariant_ids": [],
        "review_status": "unreviewed",
        "evidence_ids": [],
        "source_ids": ["source:attacker"],
        "metadata": {
            "payload": {
                "updated_cells": [self_granted_capability]
            }
        }
    });
    fs::write(
        &morphism_path,
        serde_json::to_string_pretty(&morphism).expect("serialize self-grant morphism"),
    )
    .expect("write self-grant morphism");

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);

    assert!(!propose.status.success());
    assert!(stderr(&propose).contains(
        "cannot update capability cell capability:durable-mutation: custom:capability cells are \
         administered only at lift/import time inside the declared source boundary"
    ));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_capability_authorizes_only_the_operations_it_names() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:capability-scope-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let transition = |capability: &str| {
        run_cli(&[
            "cell",
            "transition",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:capability-scope-base",
            "--cell-id",
            "work:review-native-contract",
            "--to",
            "waiting",
            "--reason",
            "capability scope test",
            "--actor-id",
            "actor:native-transition-cli",
            "--capability-id",
            capability,
            "--operation-scope-id",
            native_case_space_id(),
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:native-case-management-contract",
            "--format",
            "json",
        ])
    };

    // `capability:plan-review` grants its actors plan review and nothing else.
    // Before this, holding any capability admitted every gated operation, so the
    // roles a genesis separates were four labels for one power.
    let wrong_capability = transition("capability:plan-review");
    assert!(!wrong_capability.status.success());
    assert!(
        stderr(&wrong_capability).contains(
            "does not authorize operation cell-transition; metadata.operations must list it"
        ),
        "stderr: {}",
        stderr(&wrong_capability)
    );
    // A gate violation is its own error_code (issue #22): the correct
    // response is "a different actor or capability is required", not "fix
    // this call's shape and retry with the same identity" — a different
    // kind of answer from a plain usage or business-rule refusal.
    let wrong_capability_refusal = stderr_json(&wrong_capability);
    assert_eq!(
        wrong_capability_refusal["error_code"],
        json!("gate_violation")
    );
    assert!(
        wrong_capability_refusal["data"]["witness_ids"]
            .as_array()
            .expect("witness_ids is an array")
            .contains(&json!("capability:plan-review")),
        "witness_ids: {}",
        wrong_capability_refusal["data"]["witness_ids"]
    );

    let right_capability = transition("capability:durable-mutation");
    assert!(
        right_capability.status.success(),
        "stderr: {}",
        stderr(&right_capability)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// A case space carrying two capabilities separated on purpose: the
/// implementer may attach evidence and transition cells, the reviewer may
/// only review. Packet tests use this so the actor-seam assertion (ADR 0015)
/// is checking a real capability boundary, not a single actor wearing two hats.
fn packet_test_fixture() -> Value {
    let mut fixture = json_file(native_case_fixture());
    let space_id = fixture["space_id"].clone();
    let capability = |id: &str, actor_id: &str, operations: &[&str]| {
        json!({
            "id": id,
            "cell_type": "custom:capability",
            "space_id": space_id,
            "title": format!("Packet test capability {id}"),
            "lifecycle": "accepted",
            "source_ids": ["source:native-design-doc"],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "document", "title": "Packet test fixture"},
                "confidence": 1.0,
                "review_status": "accepted"
            },
            "metadata": {
                "actor_ids": [actor_id],
                "operations": operations
            }
        })
    };
    fixture["case_cells"]
        .as_array_mut()
        .expect("case cells array")
        .extend([
            capability(
                "capability:packet-implementer",
                "actor:packet-implementer",
                &["evidence-attach", "cell-transition"],
            ),
            capability(
                "capability:packet-reviewer",
                "actor:packet-reviewer",
                &["review"],
            ),
        ]);
    fixture
}

fn import_packet_test_case_space(directory: &Path, revision_id: &str) -> Output {
    let fixture_path = directory.join("packet-test-fixture.case.space.json");
    write_json_value(&fixture_path, &packet_test_fixture());
    import_native_case_space_from_input(directory, &fixture_path, revision_id)
}

fn packet_value(claim_id: &str) -> Value {
    json!({
        "schema": "highergraphen.case.evidence_packet.v1",
        "schema_version": 1,
        "case_space_id": native_case_space_id(),
        "target": {"cell_id": "work:review-native-contract", "transition_to": "active"},
        "claim": {
            "id": claim_id,
            "cell_type": "evidence",
            "space_id": "space:higher-graphen-casegraphen",
            "title": "Packet claim",
            "lifecycle": "active",
            "source_ids": ["source:packet-test"],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "log", "title": "CI"},
                "confidence": 0.8,
                "review_status": "unreviewed"
            },
            "metadata": {}
        },
        "artifacts": ["build.log"],
        "satisfies": ["evidence:native-schema-json-valid"],
        "completion": {"reason": "Packet test completion reason"}
    })
}

#[test]
fn packet_apply_pauses_for_review_then_resume_transitions_after_acceptance() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-happy-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet_path = directory.join("happy.evidence.packet.json");
    write_json_value(&packet_path, &packet_value("evidence:packet-happy"));
    let artifact_path = directory.join("build.log");
    fs::write(&artifact_path, b"packet artifact bytes\n").expect("write artifact");
    let packet_str = packet_path.to_str().expect("packet path").to_owned();

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-happy-base",
        "--packet",
        &packet_str,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["result"]["status"], json!("paused_for_review"));
    assert_eq!(
        apply_json["result"]["claim_cell_id"],
        json!("evidence:packet-happy")
    );
    assert_eq!(
        apply_json["result"]["artifact_cell_ids"]
            .as_array()
            .expect("artifact cell ids")
            .len(),
        1
    );
    // `packet apply`'s pause is a producer of the shared ADR 0016 halt
    // object, and only of it now: `completed_through`/`next_operations`
    // used to be duplicated at the top level and inside `halt` — two
    // sources for one fact, free to drift. `needs_review` is exactly what
    // the pause means.
    assert_eq!(apply_json["result"]["halt"]["halt"], json!("needs_review"));
    let completed_through = apply_json["result"]["halt"]["completed_through"]
        .as_str()
        .expect("halt.completed_through")
        .to_owned();
    assert_eq!(
        apply_json["result"]["record"]["current_revision_id"],
        json!(completed_through)
    );
    assert_eq!(
        apply_json["result"]["record"]["revision_count"],
        json!(2),
        "packet apply must append exactly one revision on top of genesis"
    );
    assert_eq!(
        apply_json["result"]["halt"]["target_ids"],
        json!(["evidence:packet-happy"])
    );
    // Structured, not shell text: a packet-controlled `claim.id` must not be
    // able to inject flags into a string an operator is told to paste.
    let next_operations = apply_json["result"]["halt"]["next_operations"]
        .as_array()
        .expect("halt.next_operations");
    assert_eq!(next_operations.len(), 2);
    assert_eq!(next_operations[0]["command"], json!("review accept"));
    assert_eq!(
        next_operations[0]["arguments"]["target_id"],
        json!("evidence:packet-happy")
    );
    assert_eq!(
        next_operations[0]["arguments"]["base_revision_id"],
        json!(completed_through)
    );
    assert_eq!(next_operations[1]["command"], json!("packet resume"));
    assert_eq!(
        next_operations[1]["arguments"]["completed_through"],
        json!(completed_through)
    );
    // `halts` is the same ranked list every other stoppable command reports;
    // `packet apply`'s pause is always exactly this one `needs_review`.
    assert_eq!(
        apply_json["result"]["halts"],
        json!([apply_json["result"]["halt"]])
    );

    // Gate seam, confirmed the hard way — but read this refusal carefully
    // (issue #40): this fixture's implementer actor holds only
    // `capability:packet-implementer`, which does not list the `review`
    // operation, so this is the ordinary capability check refusing —
    // `does not authorize operation review` — not an identity check. Nothing
    // in the crate compares the reviewing actor to the actor that produced
    // the claim being reviewed; an actor whose own capability separately
    // listed `review` would accept its own claim unrefused. What this test
    // proves is `review accept`'s ordinary gate resolution making the pause
    // meaningful for *this* fixture's actor, not self-review prevention in
    // general — see `docs/security/worker-execution-policy.md`'s residual
    // risk on self-review and `docs/specs/operate-halt.fsl`'s
    // `INV-OPERATE-002`.
    let self_review = run_cli(&[
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:packet-happy",
        "--reviewer-id",
        "reviewer:should-not-work",
        "--reason",
        "attempting self-review under the implementer gate",
        "--base-revision-id",
        &completed_through,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!self_review.status.success());
    assert!(
        stderr(&self_review).contains("does not authorize operation review"),
        "stderr: {}",
        stderr(&self_review)
    );

    // Resume before any review exists is refused, naming `review accept`.
    let resume_before_review = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &completed_through,
        "--packet",
        &packet_str,
        "--completed-through",
        &completed_through,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume_before_review.status.success());
    assert!(
        stderr(&resume_before_review).contains("review accept"),
        "stderr: {}",
        stderr(&resume_before_review)
    );

    let review = run_cli(&[
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:packet-happy",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "reviewed under the independent reviewer capability",
        "--base-revision-id",
        &completed_through,
        "--evidence-id",
        "evidence:packet-happy",
        "--actor-id",
        "actor:packet-reviewer",
        "--capability-id",
        "capability:packet-reviewer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(review.status.success(), "stderr: {}", stderr(&review));
    let review_revision = stdout_json(&review)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("review revision")
        .to_owned();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &review_revision,
        "--packet",
        &packet_str,
        "--completed-through",
        &completed_through,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(resume.status.success(), "stderr: {}", stderr(&resume));
    let resume_json = stdout_json(&resume);
    assert_eq!(resume_json["result"]["status"], json!("completed"));
    assert_eq!(
        resume_json["result"]["entry"]["morphism"]["metadata"]["payload"]["updated_cells"][0]
            ["lifecycle"],
        json!("active")
    );
    assert_eq!(
        resume_json["result"]["entry"]["morphism"]["metadata"]["operation_gate"]["operation"],
        json!("cell-transition")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_refuses_a_claim_id_naming_an_unattached_accepted_evidence_cell() {
    // Reproduction of the audit's HIGH finding against ADR 0015: a packet
    // whose `claim.id` names an already-accepted EVIDENCE cell that no
    // packet ever attached (a genesis-authored evidence cell, trusted by the
    // source boundary with no review morphism anywhere in the log) must not
    // sail through resume. Unlike the earlier non-evidence-cell reproduction,
    // this cell genuinely is `cell_type: evidence` with accepted provenance
    // -- the fix must refuse on "this claim was never attached by an
    // EvidenceAttach morphism", not merely on cell type.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-unattached-claim-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet = packet_value("evidence:native-schema-json-valid");
    let packet_path = directory.join("unattached-claim.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let history_before = run_native_case_store_command(&directory, "history");
    let entries_before = stdout_json(&history_before)["result"]["entries"]
        .as_array()
        .expect("history entries before")
        .len();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-unattached-claim-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--completed-through",
        "revision:packet-unattached-claim-base",
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("was not added by the EvidenceAttach morphism"),
        "stderr: {}",
        stderr(&resume)
    );

    let history_after = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history_after)["result"]["entries"]
            .as_array()
            .expect("history entries after")
            .len(),
        entries_before,
        "a refused resume must append nothing"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_refuses_a_claim_attached_by_a_different_packets_apply() {
    // A claim genuinely attached and genuinely accepted -- just not by the
    // apply `--completed-through` names. Reusing a foreign attach's accepted
    // claim to authorize a different packet's transition must refuse, even
    // though the claim really is accepted evidence somewhere in the log.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-cross-attach-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet_a_path = directory.join("packet-a.evidence.packet.json");
    write_json_value(&packet_a_path, &packet_value("evidence:packet-a-claim"));
    let packet_b_path = directory.join("packet-b.evidence.packet.json");
    write_json_value(&packet_b_path, &packet_value("evidence:packet-b-claim"));
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");

    let apply_a = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-cross-attach-base",
        "--packet",
        packet_a_path.to_str().expect("packet a path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(apply_a.status.success(), "stderr: {}", stderr(&apply_a));
    let apply_a_json = stdout_json(&apply_a);
    let revision_after_a = apply_a_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("revision after a")
        .to_owned();

    let apply_b = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &revision_after_a,
        "--packet",
        packet_b_path.to_str().expect("packet b path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(apply_b.status.success(), "stderr: {}", stderr(&apply_b));
    let completed_through_b = stdout_json(&apply_b)["result"]["halt"]["completed_through"]
        .as_str()
        .expect("halt.completed_through b")
        .to_owned();

    // Claim A is honestly reviewed and accepted.
    let review = run_cli(&[
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:packet-a-claim",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "claim A really was reviewed",
        "--base-revision-id",
        &completed_through_b,
        "--evidence-id",
        "evidence:packet-a-claim",
        "--actor-id",
        "actor:packet-reviewer",
        "--capability-id",
        "capability:packet-reviewer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(review.status.success(), "stderr: {}", stderr(&review));
    let review_revision = stdout_json(&review)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("review revision")
        .to_owned();

    // Claim A's now-accepted id, paired with B's own completed-through
    // revision -- the revision where claim A was genuinely attached and
    // accepted is deliberately NOT what is named here. B's own claim was
    // never reviewed at all, so if this pairing were accepted, A's honest
    // review would have authorized B's transition.
    let mut forged = packet_value("evidence:packet-a-claim");
    forged["target"] = packet_value("evidence:packet-b-claim")["target"].clone();
    forged["completion"] = packet_value("evidence:packet-b-claim")["completion"].clone();
    let forged_path = directory.join("forged.evidence.packet.json");
    write_json_value(&forged_path, &forged);

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &review_revision,
        "--packet",
        forged_path.to_str().expect("forged packet path"),
        "--completed-through",
        &completed_through_b,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("was not added by the EvidenceAttach morphism"),
        "stderr: {}",
        stderr(&resume)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_validates_its_gate_before_reading_the_packet_file() {
    // `evidence attach` documents authorizing before touching inputs so an
    // actor holding no capability cannot distinguish a missing file from an
    // unknown one through the refusal text; `packet resume` must follow the
    // same ordering. A nonexistent packet path plus a capability id that
    // does not resolve to any cell: if the gate ran second, the refusal
    // would be about the missing file.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-gate-order-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-gate-order-base",
        "--packet",
        "definitely-does-not-exist.evidence.packet.json",
        "--completed-through",
        "revision:packet-gate-order-base",
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:does-not-exist",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("does not resolve to an existing case cell"),
        "the gate must be validated before the packet file is ever read: stderr: {}",
        stderr(&resume)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_refuses_a_completed_through_revision_absent_from_history() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-stale-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet_path = directory.join("stale.evidence.packet.json");
    write_json_value(&packet_path, &packet_value("evidence:packet-stale"));
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");
    let packet_str = packet_path.to_str().expect("packet path").to_owned();

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-stale-base",
        "--packet",
        &packet_str,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let current_revision = stdout_json(&apply)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("current revision")
        .to_owned();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &current_revision,
        "--packet",
        &packet_str,
        "--completed-through",
        "revision:never-appended",
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("is not in this case space's history"),
        "stderr: {}",
        stderr(&resume)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_refuses_after_the_claim_is_rejected() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-rejected-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet_path = directory.join("rejected.evidence.packet.json");
    write_json_value(&packet_path, &packet_value("evidence:packet-rejected"));
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");
    let packet_str = packet_path.to_str().expect("packet path").to_owned();

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-rejected-base",
        "--packet",
        &packet_str,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let completed_through = stdout_json(&apply)["result"]["halt"]["completed_through"]
        .as_str()
        .expect("halt.completed_through")
        .to_owned();

    let reject = run_cli(&[
        "review",
        "reject",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:packet-rejected",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "the claim does not hold up",
        "--base-revision-id",
        &completed_through,
        "--actor-id",
        "actor:packet-reviewer",
        "--capability-id",
        "capability:packet-reviewer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    let reject_revision = stdout_json(&reject)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("reject revision")
        .to_owned();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &reject_revision,
        "--packet",
        &packet_str,
        "--completed-through",
        &completed_through,
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("is not accepted"),
        "stderr: {}",
        stderr(&resume)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_resume_refuses_a_claim_id_naming_an_accepted_non_evidence_cell() {
    // Reproduction of an audit finding: a packet whose `claim.id` names an
    // already-accepted, non-evidence cell (a genesis goal cell) must not sail
    // through resume just because that id happens to resolve to something
    // with `provenance.review_status: accepted` in the case space. Nothing
    // here ever ran `packet apply` or `evidence attach` — the attack is that
    // resume looked up `claim.id` and read whatever cell it found.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-non-evidence-claim-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet = packet_value("goal:native-case-contract");
    let packet_path = directory.join("non-evidence-claim.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let history_before = run_native_case_store_command(&directory, "history");
    let entries_before = stdout_json(&history_before)["result"]["entries"]
        .as_array()
        .expect("history entries before")
        .len();

    let resume = run_cli(&[
        "packet",
        "resume",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-non-evidence-claim-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--completed-through",
        "revision:packet-non-evidence-claim-base",
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!resume.status.success());
    assert!(
        stderr(&resume).contains("is not an evidence cell"),
        "stderr: {}",
        stderr(&resume)
    );

    let history_after = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history_after)["result"]["entries"]
            .as_array()
            .expect("history entries after")
            .len(),
        entries_before,
        "a refused resume must append nothing"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_refuses_a_case_space_id_mismatch_before_any_mutation() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-mismatch-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let mut packet = packet_value("evidence:packet-mismatch");
    packet["case_space_id"] = json!("case_space:not-this-one");
    let packet_path = directory.join("mismatch.evidence.packet.json");
    write_json_value(&packet_path, &packet);
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-mismatch-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!apply.status.success());
    assert!(
        stderr(&apply).contains("does not match --case-space-id"),
        "stderr: {}",
        stderr(&apply)
    );

    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1,
        "a mismatched packet must append nothing"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_report_validates_against_the_native_cli_report_schema() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-schema-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let packet_path = directory.join("schema.evidence.packet.json");
    write_json_value(&packet_path, &packet_value("evidence:packet-schema"));
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");
    let output_path = directory.join("packet-apply.report.json");

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-schema-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/native-cli.report.schema.json"),
        &output_path,
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// `packet apply` with `store`/`case_space_id`/`base_revision_id` fixed to the
/// packet-test fixture, varying only the packet file and its artifacts. Used
/// by the confinement tests below so each one states only what differs: the
/// packet path and what its `artifacts:` list names.
fn apply_packet_test_fixture(store: &str, packet_path: &Path) -> Output {
    run_cli(&[
        "packet",
        "apply",
        "--store",
        store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-confinement-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ])
}

#[test]
fn packet_apply_refuses_an_artifact_naming_a_dot_dot_escape_from_the_packet_directory() {
    let directory = unique_temp_dir();
    let packet_directory = directory.join("packet");
    fs::create_dir_all(&packet_directory).expect("create packet directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    // The packet lives in its own subdirectory; the artifact it names walks
    // back out of it with `..` to a file that is a sibling of that directory,
    // not inside it.
    fs::write(directory.join("outside.log"), b"outside bytes\n").expect("write outside file");
    let mut packet = packet_value("evidence:packet-dotdot-escape");
    packet["artifacts"] = json!(["../outside.log"]);
    let packet_path = packet_directory.join("dotdot.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!apply.status.success());
    assert!(
        stderr(&apply).contains("does not resolve inside the packet's directory"),
        "stderr: {}",
        stderr(&apply)
    );

    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1,
        "a refused packet artifact must append nothing"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_refuses_an_artifact_naming_an_absolute_path() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    // An absolute path replaces the packet-relative join entirely
    // (`Path::join`), so this is exactly the reproduction the fix closes: an
    // artifact entry that names a path anywhere on the filesystem.
    let outside_directory = unique_temp_dir();
    fs::create_dir_all(&outside_directory).expect("create outside directory");
    let absolute_target = outside_directory.join("absolute-target.log");
    fs::write(&absolute_target, b"absolute bytes\n").expect("write absolute target");
    let mut packet = packet_value("evidence:packet-absolute-escape");
    packet["artifacts"] = json!([absolute_target.to_str().expect("absolute path")]);
    let packet_path = directory.join("absolute.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!apply.status.success());
    assert!(
        stderr(&apply).contains("does not resolve inside the packet's directory"),
        "stderr: {}",
        stderr(&apply)
    );

    fs::remove_dir_all(&directory).expect("remove temp directory");
    fs::remove_dir_all(&outside_directory).expect("remove outside directory");
}

#[test]
#[cfg(unix)]
fn packet_apply_refuses_an_artifact_symlink_pointing_outside_the_packet_directory() {
    let directory = unique_temp_dir();
    let packet_directory = directory.join("packet");
    fs::create_dir_all(&packet_directory).expect("create packet directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let outside_target = directory.join("outside-target.log");
    fs::write(&outside_target, b"outside bytes via symlink\n").expect("write outside target");
    std::os::unix::fs::symlink(&outside_target, packet_directory.join("escape.log"))
        .expect("create escaping symlink");
    let mut packet = packet_value("evidence:packet-symlink-escape");
    packet["artifacts"] = json!(["escape.log"]);
    let packet_path = packet_directory.join("symlink.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!apply.status.success());
    assert!(
        stderr(&apply).contains("does not resolve inside the packet's directory"),
        "stderr: {}",
        stderr(&apply)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_accepts_a_plain_artifact_beside_it_and_records_the_canonical_uri() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let artifact_path = directory.join("build.log");
    fs::write(&artifact_path, b"packet artifact bytes\n").expect("write artifact");
    let packet_path = directory.join("plain.evidence.packet.json");
    write_json_value(
        &packet_path,
        &packet_value("evidence:packet-plain-artifact"),
    );

    let apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let added_cells = stdout_json(&apply)["result"]["entry"]["morphism"]["metadata"]["payload"]
        ["added_cells"]
        .as_array()
        .expect("added cells")
        .clone();
    let artifact_cell = added_cells
        .iter()
        .find(|cell| cell["cell_type"] == json!("custom:artifact"))
        .expect("artifact cell present in the payload");
    let canonical_artifact_path =
        fs::canonicalize(&artifact_path).expect("canonicalize artifact path");
    assert_eq!(
        artifact_cell["metadata"]["artifact_uri"],
        json!(canonical_artifact_path
            .to_str()
            .expect("canonical artifact path"))
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_accepts_a_bare_relative_packet_filename_from_its_own_directory() {
    // `Path::new("packet.json").parent()` is `Some("")`, not `None` — the
    // regression this closes. `packet apply --packet packet.json` run from
    // the packet's own directory is the most natural invocation there is,
    // and every other confinement test here passes an absolute or
    // temp-rooted path, which is exactly why the regression went unnoticed.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    fs::write(directory.join("build.log"), b"packet artifact bytes\n").expect("write artifact");
    write_json_value(
        &directory.join("relative.evidence.packet.json"),
        &packet_value("evidence:packet-relative-filename"),
    );

    let apply = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .current_dir(&directory)
        .args([
            "packet",
            "apply",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:packet-confinement-base",
            "--packet",
            "relative.evidence.packet.json",
            "--actor-id",
            "actor:packet-implementer",
            "--capability-id",
            "capability:packet-implementer",
            "--operation-scope-id",
            native_case_space_id(),
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:native-case-management-contract",
            "--format",
            "json",
        ])
        .output()
        .expect("run casegraphen CLI");
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
#[cfg(unix)]
fn packet_apply_resolves_artifacts_beside_a_symlinked_packet_file() {
    // The join base for `artifacts:` entries and the confinement root must be
    // the same directory answer. A symlinked packet *file* is the case that
    // distinguishes "canonicalize `packet_path.parent()`" from "canonicalize
    // `packet_path` and take its parent": the packet's real neighbour lives
    // next to the file the symlink resolves to, not next to the symlink.
    let directory = unique_temp_dir();
    let real_directory = directory.join("real");
    let link_directory = directory.join("linkdir");
    fs::create_dir_all(&real_directory).expect("create real directory");
    fs::create_dir_all(&link_directory).expect("create link directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    fs::write(
        real_directory.join("data.txt"),
        b"the packet's real neighbour\n",
    )
    .expect("write the packet's real neighbour");
    let mut packet = packet_value("evidence:packet-symlinked-file");
    packet["artifacts"] = json!(["data.txt"]);
    let real_packet_path = real_directory.join("packet.json");
    write_json_value(&real_packet_path, &packet);
    let linked_packet_path = link_directory.join("link.json");
    std::os::unix::fs::symlink(&real_packet_path, &linked_packet_path)
        .expect("create symlinked packet file");

    let apply = apply_packet_test_fixture(&store, &linked_packet_path);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let added_cells = stdout_json(&apply)["result"]["entry"]["morphism"]["metadata"]["payload"]
        ["added_cells"]
        .as_array()
        .expect("added cells")
        .clone();
    let artifact_cell = added_cells
        .iter()
        .find(|cell| cell["cell_type"] == json!("custom:artifact"))
        .expect("the packet's real neighbour was resolved as an artifact");
    let canonical_data_path =
        fs::canonicalize(real_directory.join("data.txt")).expect("canonicalize data path");
    assert_eq!(
        artifact_cell["metadata"]["artifact_uri"],
        json!(canonical_data_path.to_str().expect("canonical data path"))
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn packet_apply_refuses_an_existing_and_a_missing_absolute_artifact_identically() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let outside_directory = unique_temp_dir();
    fs::create_dir_all(&outside_directory).expect("create outside directory");
    let absolute_target = outside_directory.join("oracle-target.log");
    let mut packet = packet_value("evidence:packet-oracle-probe");
    packet["artifacts"] = json!([absolute_target.to_str().expect("absolute path")]);
    let packet_path = directory.join("oracle-probe.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    // The same packet, byte for byte, applied twice against the same
    // unadvanced base revision (a refused apply appends nothing). Only the
    // filesystem changes between the two calls: first the named absolute
    // path does not exist at all, then it exists but is outside the packet
    // directory. The observable outcome must not let those two states be
    // told apart — comparing stderr alone is not enough: a future change
    // that moves the refusal payload elsewhere (stdout, an exit-code-only
    // signal) could leave two empty, trivially-equal stderr strings while
    // the actual signal — did the operation succeed — still leaked which
    // state held. Compare exit code, stdout, and stderr, and confirm neither
    // call mutated the store.
    let missing_apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!missing_apply.status.success());

    fs::write(&absolute_target, b"oracle target bytes\n").expect("write absolute target");
    let existing_apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!existing_apply.status.success());

    assert_eq!(missing_apply.status.code(), existing_apply.status.code());
    assert_eq!(stdout(&missing_apply), stdout(&existing_apply));
    assert_eq!(
        stderr(&missing_apply),
        stderr(&existing_apply),
        "a nonexistent absolute artifact path and the identical, now-existing path outside the \
         packet directory must refuse with a byte-identical observable outcome — otherwise a \
         packet can probe whether an arbitrary filesystem path exists"
    );
    assert_eq!(
        stdout_json(&run_native_case_store_command(&directory, "history"))["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1,
        "neither refused apply may have mutated the store"
    );

    fs::remove_dir_all(&directory).expect("remove temp directory");
    fs::remove_dir_all(&outside_directory).expect("remove outside directory");
}

#[test]
fn packet_apply_refuses_a_climb_and_return_artifact_identically_whether_the_probed_directory_exists(
) {
    // The exploit issue #21 defect 2 reopened: an `artifacts:` entry that
    // starts at an arbitrary absolute directory, climbs with enough `..` to
    // reach real `/` (any depth — `..` at `/` is a no-op, and enough climbs
    // is always safe even though too few silently is not, since a symlinked
    // ancestor like macOS's `/var` -> `/private/var` makes the real resolved
    // depth deeper than the string's own component count), and descends back
    // through the packet's own real directory to a genuinely in-root file.
    // Before the lexical pre-check, this canonicalized and dispatched
    // successfully whenever the climbed-through directory happened to exist
    // — an existence oracle over the operator's filesystem, and a durable
    // mutation on a hit.
    //
    // One packet, byte for byte, applied twice against the same unadvanced
    // base revision (a refused apply appends nothing): the artifact entry
    // never changes, only whether the directory it climbs through exists on
    // disk between the two calls.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-confinement-base");
    let store = directory.to_str().expect("temp path").to_owned();
    fs::write(directory.join("build.log"), b"packet artifact bytes\n").expect("write artifact");
    let canonical_directory = fs::canonicalize(&directory).expect("canonicalize packet directory");
    let root_suffix = canonical_directory
        .strip_prefix("/")
        .expect("packet directory is absolute")
        .to_str()
        .expect("packet directory is UTF-8");
    let climb = "../".repeat(64);

    let probe_directory = unique_temp_dir();
    assert!(
        !probe_directory.exists(),
        "probe must not exist yet for this test to mean anything"
    );
    let artifact_entry = format!(
        "{}/{climb}{root_suffix}/build.log",
        probe_directory.to_str().expect("probe dir is UTF-8")
    );

    // The crafted string is a genuine working exploit shape, not a typo:
    // canonicalizing it directly (bypassing the CLI) fails while the probed
    // directory is absent...
    assert!(fs::canonicalize(&artifact_entry).is_err());

    let mut packet = packet_value("evidence:packet-climb-probe");
    packet["artifacts"] = json!([artifact_entry]);
    let packet_path = directory.join("climb.evidence.packet.json");
    write_json_value(&packet_path, &packet);

    let missing_apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!missing_apply.status.success());

    // ...and, once the probed directory is created, resolves to the real
    // in-root file — proving the oracle is genuinely there for the code to
    // close, not merely hypothesized.
    fs::create_dir_all(&probe_directory).expect("create the probed directory");
    assert_eq!(
        fs::canonicalize(&artifact_entry).expect("the exploit string must now resolve"),
        canonical_directory.join("build.log")
    );

    let existing_apply = apply_packet_test_fixture(&store, &packet_path);
    assert!(!existing_apply.status.success());

    assert_eq!(missing_apply.status.code(), existing_apply.status.code());
    assert_eq!(stdout(&missing_apply), stdout(&existing_apply));
    assert_eq!(
        stderr(&missing_apply),
        stderr(&existing_apply),
        "whether the climbed-through probe directory exists must not be observable in the \
         refusal"
    );
    assert_eq!(
        stdout_json(&run_native_case_store_command(&directory, "history"))["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1,
        "neither apply may have mutated the store — a hit on the existing probe directory \
         previously dispatched successfully and attached a real artifact"
    );

    fs::remove_dir_all(&directory).expect("remove temp directory");
    fs::remove_dir_all(&probe_directory).expect("remove probe directory");
}

#[test]
fn packet_strict_parse_refuses_an_unknown_field() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_packet_test_case_space(&directory, "revision:packet-strict-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let mut packet = packet_value("evidence:packet-strict");
    packet["target"]["bogus_field"] = json!("x");
    let packet_path = directory.join("strict.evidence.packet.json");
    write_json_value(&packet_path, &packet);
    fs::write(directory.join("build.log"), b"artifact\n").expect("write artifact");

    let apply = run_cli(&[
        "packet",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:packet-strict-base",
        "--packet",
        packet_path.to_str().expect("packet path"),
        "--actor-id",
        "actor:packet-implementer",
        "--capability-id",
        "capability:packet-implementer",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!apply.status.success());
    let message = stderr(&apply);
    assert!(
        message.contains("target.bogus_field"),
        "refusal did not locate the field: {message}"
    );
    assert!(
        message.contains("unknown field"),
        "refusal did not keep serde's reason: {message}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn the_store_refuses_a_morphism_whose_result_it_could_not_load_back() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:loadable-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let morphism_path = directory.join("loadable.case_morphism.json");

    // `retire` removes a relation outright, and the genesis projections still
    // name this one. The loader has always refused a case space with a dangling
    // reference; the writer did not check, so this used to be written and then
    // every read path failed — including `morphism propose`, which left no way
    // to repair it through the CLI, while `space rebuild` still reported
    // success because the fold checks checksums rather than this contract.
    fs::write(
        &morphism_path,
        serde_json::to_string_pretty(&json!({
            "morphism_id": "morphism:retire-referenced-relation",
            "morphism_type": "retire",
            "source_revision_id": "revision:loadable-base",
            "target_revision_id": "revision:retired-referenced-relation",
            "added_ids": [], "updated_ids": [],
            "retired_ids": ["relation:work-waits-for-review"],
            "preserved_ids": [], "evidence_ids": [],
            "source_ids": ["source:native-cli"],
            "violated_invariant_ids": [], "review_status": "unreviewed",
            "metadata": {}
        }))
        .expect("serialize morphism"),
    )
    .expect("write morphism");
    let proposed = run_cli(&[
        "morphism",
        "propose",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(proposed.status.success(), "stderr: {}", stderr(&proposed));

    let applied = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:retire-referenced-relation",
            "--base-revision-id",
            "revision:loadable-base",
            "--reviewer-id",
            "reviewer:tidy",
            "--reason",
            "tidy up",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(
        !applied.status.success(),
        "the store accepted a state it cannot load"
    );
    assert!(
        stderr(&applied).contains("unknown referenced id relation:work-waits-for-review"),
        "stderr: {}",
        stderr(&applied)
    );

    // Nothing was written, so every read path still works.
    for operation in ["validate", "replay", "inspect"] {
        let read = run_cli(&[
            "space",
            operation,
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--format",
            "json",
        ]);
        assert!(
            read.status.success(),
            "space {operation} stderr: {}",
            stderr(&read)
        );
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn the_store_refuses_an_attached_cell_the_evaluator_could_not_read_back() {
    // The writer used to check only the store's own reference rule, which is
    // narrower than the loader's. `evidence attach` never inspects space_id or
    // title, so either one wrote a store where every derived command failed
    // permanently while `space validate` reported valid: true and
    // `space rebuild` reported success — the two commands the policy names as
    // audit and recovery, with no CLI repair path.
    for (case, cell) in [
        (
            "mismatched space",
            json!({
                "id": "evidence:wrong-space", "cell_type": "evidence", "lifecycle": "active",
                "space_id": "space:somewhere-else", "title": "Wrong space",
                "source_ids": ["source:native-cli"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "doc"}}
            }),
        ),
        (
            "blank title",
            json!({
                "id": "evidence:blank-title", "cell_type": "evidence", "lifecycle": "active",
                "space_id": json_file(native_case_fixture())["space_id"].clone(), "title": "   ",
                "source_ids": ["source:native-cli"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "doc"}}
            }),
        ),
    ] {
        let directory = unique_temp_dir();
        fs::create_dir_all(&directory).expect("create temp directory");
        import_native_case_space(&directory, "revision:evaluable-base");
        let store = directory.to_str().expect("temp path").to_owned();
        let evidence_path = directory.join("unevaluable.evidence.json");
        fs::write(
            &evidence_path,
            serde_json::to_string_pretty(&cell).expect("serialize evidence"),
        )
        .expect("write evidence");

        let attached = run_cli_with_mutation_gate(
            &[
                "evidence",
                "attach",
                "--store",
                &store,
                "--case-space-id",
                native_case_space_id(),
                "--base-revision-id",
                "revision:evaluable-base",
                "--input",
                evidence_path.to_str().expect("evidence path"),
                "--format",
                "json",
            ],
            "actor:native-evidence-cli",
        );
        assert!(
            !attached.status.success(),
            "{case}: the store accepted a state the evaluator cannot read"
        );

        // The read paths that would have broken, not only the ones that kept
        // reporting success on a broken store.
        for command in [
            vec!["obstruction", "list"],
            vec!["space", "frontier"],
            vec!["invariant", "check"],
            vec!["space", "validate"],
        ] {
            let mut args = command.clone();
            args.extend([
                "--store",
                &store,
                "--case-space-id",
                native_case_space_id(),
                "--format",
                "json",
            ]);
            let read = run_cli(&args);
            assert!(
                read.status.success(),
                "{case}: {command:?} stderr: {}",
                stderr(&read)
            );
        }

        fs::remove_dir_all(directory).expect("remove temp directory");
    }
}

#[test]
fn a_relation_update_cannot_change_which_relation_it_is() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:relation-update-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let morphism_path = directory.join("relation-update.case_morphism.json");
    let existing = json_file(native_case_fixture())["case_relations"]
        .as_array()
        .expect("fixture relations")
        .iter()
        .find(|relation| relation["id"] == json!("relation:work-waits-for-review"))
        .expect("fixture relation")
        .clone();

    // Hardening evidence so it cannot be re-pointed at a requirement left the
    // requirement free to be re-pointed at the evidence: the endpoints, the
    // type, and the strength are what make an edge that edge.
    for (field, mutation) in [
        ("to_id", json!("evidence:native-schema-json-valid")),
        ("from_id", json!("evidence:native-schema-json-valid")),
        ("relation_type", json!("verifies")),
        ("relation_strength", json!("soft")),
    ] {
        let mut updated = existing.clone();
        updated[field] = mutation;
        fs::write(
            &morphism_path,
            serde_json::to_string_pretty(&json!({
                "morphism_id": "morphism:relation-update",
                "morphism_type": "update",
                "source_revision_id": "revision:relation-update-base",
                "target_revision_id": "revision:relation-updated",
                "added_ids": [], "updated_ids": ["relation:work-waits-for-review"],
                "retired_ids": [], "preserved_ids": [], "evidence_ids": [],
                "source_ids": ["source:native-cli"],
                "violated_invariant_ids": [], "review_status": "unreviewed",
                "metadata": {"payload": {
                    "added_cells": [], "added_relations": [],
                    "updated_cells": [], "updated_relations": [updated]
                }}
            }))
            .expect("serialize morphism"),
        )
        .expect("write morphism");
        let refused = run_cli(&[
            "morphism",
            "propose",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--input",
            morphism_path.to_str().expect("morphism path"),
            "--format",
            "json",
        ]);
        assert!(!refused.status.success(), "{field} was accepted on update");
        assert!(
            stderr(&refused).contains(&format!("{field} is immutable")),
            "stderr: {}",
            stderr(&refused)
        );
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_hard_evidence_requirement_is_satisfied_only_by_recorded_coverage() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:coverage-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let morphism_path = directory.join("coverage.case_morphism.json");
    let space_id = json_file(native_case_fixture())["space_id"].clone();

    let write_morphism = |value: Value| {
        fs::write(
            &morphism_path,
            serde_json::to_string_pretty(&value).expect("serialize morphism"),
        )
        .expect("write morphism");
        run_cli(&[
            "morphism",
            "propose",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--input",
            morphism_path.to_str().expect("morphism path"),
            "--format",
            "json",
        ])
    };
    let apply = |morphism_id: &str, base: &str| {
        run_cli_with_mutation_gate(
            &[
                "morphism",
                "apply",
                "--store",
                &store,
                "--case-space-id",
                native_case_space_id(),
                "--morphism-id",
                morphism_id,
                "--base-revision-id",
                base,
                "--reviewer-id",
                "reviewer:coverage",
                "--reason",
                "coverage test",
                "--format",
                "json",
            ],
            "actor:native-mutation-cli",
        )
    };
    let blocked = || {
        let listed = run_cli(&[
            "obstruction",
            "list",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--format",
            "json",
        ]);
        assert!(listed.status.success(), "stderr: {}", stderr(&listed));
        stdout_json(&listed)["result"]["obstructions"]
            .as_array()
            .expect("obstructions")
            .iter()
            .any(|obstruction| {
                obstruction["obstruction_type"] == json!("missing_evidence")
                    && obstruction.to_string().contains("work:coverage-target")
            })
    };

    // A work cell with a hard evidence requirement that nothing satisfies yet.
    let setup = json!({
        "morphism_id": "morphism:coverage-setup",
        "morphism_type": "create",
        "source_revision_id": "revision:coverage-base",
        "target_revision_id": "revision:coverage-target",
        "added_ids": ["work:coverage-target", "evidence:coverage-needed", "relation:coverage-requires"],
        "updated_ids": [], "retired_ids": [], "preserved_ids": [],
        "evidence_ids": [], "source_ids": ["source:native-cli"],
        "violated_invariant_ids": [], "review_status": "unreviewed",
        "metadata": {"payload": {
            "added_cells": [
                {"id": "work:coverage-target", "cell_type": "work", "lifecycle": "active",
                 "space_id": space_id, "title": "Work needing evidence",
                 "source_ids": ["source:native-cli"], "structure_ids": [], "metadata": {},
                 "provenance": {"confidence": 0.9, "review_status": "reviewed",
                                "source": {"kind": "human", "title": "t"}}},
                {"id": "evidence:coverage-needed", "cell_type": "evidence", "lifecycle": "proposed",
                 "space_id": space_id, "title": "Required evidence placeholder",
                 "source_ids": ["source:native-cli"], "structure_ids": [],
                 "metadata": {"evidence_boundary": "inferred"},
                 "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                                "source": {"kind": "human", "title": "t"}}}
            ],
            "added_relations": [
                {"id": "relation:coverage-requires", "relation_type": "requires_evidence",
                 "relation_strength": "hard", "from_id": "work:coverage-target",
                 "to_id": "evidence:coverage-needed", "evidence_ids": [],
                 "source_ids": ["source:native-cli"], "metadata": {},
                 "provenance": {"confidence": 1.0, "review_status": "accepted",
                                "source": {"kind": "human", "title": "t"}}}
            ],
            "updated_cells": [], "updated_relations": []
        }}
    });
    assert!(write_morphism(setup).status.success());
    let applied = apply("morphism:coverage-setup", "revision:coverage-base");
    assert!(applied.status.success(), "stderr: {}", stderr(&applied));
    assert!(blocked(), "the requirement should start unsatisfied");

    // A generic morphism points already-trusted evidence at the requirement.
    // The edge is indistinguishable in the graph from the one `evidence attach`
    // mints — same type, same endpoints, same diagnostic strength — so nothing
    // read out of the graph can refuse it. The write is allowed; what it must
    // not do is satisfy the requirement.
    let repoint = json!({
        "morphism_id": "morphism:coverage-repoint",
        "morphism_type": "relate",
        "source_revision_id": "revision:coverage-target",
        "target_revision_id": "revision:coverage-repointed",
        "added_ids": ["relation:coverage-repoint"],
        "updated_ids": [], "retired_ids": [], "preserved_ids": [],
        "evidence_ids": [], "source_ids": ["source:native-cli"],
        "violated_invariant_ids": [], "review_status": "unreviewed",
        "metadata": {"payload": {
            "added_cells": [],
            "added_relations": [
                {"id": "relation:coverage-repoint", "relation_type": "verifies",
                 "relation_strength": "diagnostic", "from_id": "evidence:native-schema-json-valid",
                 "to_id": "evidence:coverage-needed", "evidence_ids": [],
                 "source_ids": ["source:native-cli"], "metadata": {},
                 "provenance": {"confidence": 0.1, "review_status": "unreviewed",
                                "source": {"kind": "human", "title": "t"}}}
            ],
            "updated_cells": [], "updated_relations": []
        }}
    });
    assert!(write_morphism(repoint.clone()).status.success());
    let repointed = apply("morphism:coverage-repoint", "revision:coverage-target");
    assert!(repointed.status.success(), "stderr: {}", stderr(&repointed));
    assert!(
        blocked(),
        "re-pointing trusted evidence through a generic morphism satisfied a hard requirement"
    );

    // Coverage is keyed on the morphism type, and `morphism_type` is a field of
    // a proposal file. Writing `evidence_attach` on the same hand-authored
    // morphism was enough to mint the coverage, so the type is reserved the way
    // the canonical review metadata already was.
    let mut forged = repoint;
    forged["morphism_id"] = json!("morphism:coverage-forged-attach");
    forged["morphism_type"] = json!("evidence_attach");
    forged["added_ids"] = json!(["relation:coverage-forged"]);
    forged["metadata"]["payload"]["added_relations"][0]["id"] = json!("relation:coverage-forged");
    forged["metadata"]["payload"]["added_relations"][0]["relation_type"] =
        json!("satisfies_evidence_requirement");
    let forged_propose = write_morphism(forged);
    assert!(!forged_propose.status.success());
    assert!(
        stderr(&forged_propose).contains("cannot declare morphism_type evidence_attach"),
        "stderr: {}",
        stderr(&forged_propose)
    );

    // The canonical path must still satisfy it: attach records the coverage,
    // review promotes the evidence, and only then is the requirement met.
    let evidence_path = directory.join("coverage-evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&json!({
            "id": "evidence:coverage-real", "cell_type": "evidence", "lifecycle": "active",
            "space_id": space_id, "title": "Attached evidence",
            "source_ids": ["source:native-cli"], "structure_ids": [], "metadata": {},
            "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                           "source": {"kind": "document", "title": "doc"}}
        }))
        .expect("serialize evidence"),
    )
    .expect("write evidence");
    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:coverage-repointed",
            "--input",
            evidence_path.to_str().expect("evidence path"),
            "--satisfies",
            "evidence:coverage-needed",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    assert!(
        blocked(),
        "attached evidence satisfied a hard requirement before it was reviewed"
    );

    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();
    let promoted = run_cli_with_mutation_gate(
        &[
            "review",
            "accept",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:coverage-real",
            "--reviewer-id",
            "reviewer:human",
            "--reason",
            "read the document",
            "--base-revision-id",
            &attached_revision,
            "--evidence-id",
            "evidence:coverage-real",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(promoted.status.success(), "stderr: {}", stderr(&promoted));
    assert!(
        !blocked(),
        "the canonical attach-then-review path no longer satisfies a hard requirement"
    );

    // The trust decision above already reads the log-derived acceptance
    // (that is what cleared the hard obstruction); the findings section must
    // report the same fact rather than the cell's never-updated stored
    // `provenance.review_status`, or the unreviewed-inference finding and its
    // review gap would persist forever and the assurance axis could never
    // reach `accepted` through the CLI review path.
    let reason = run_native_case_store_command(&directory, "reason");
    assert!(reason.status.success(), "stderr: {}", stderr(&reason));
    let evaluation = &stdout_json(&reason)["result"]["evaluation"];
    assert!(
        !evaluation["evidence_findings"]["unreviewed_inference_ids"]
            .as_array()
            .expect("unreviewed inference ids")
            .contains(&json!("evidence:coverage-real")),
        "an accepted-by-review inferred claim must stop reading as unreviewed"
    );
    assert!(evaluation["evidence_findings"]["accepted_evidence_ids"]
        .as_array()
        .expect("accepted evidence ids")
        .contains(&json!("evidence:coverage-real")));
    assert!(
        !evaluation["review_gaps"]
            .as_array()
            .expect("review gaps")
            .iter()
            .any(|gap| gap["gap_type"] == json!("unreviewed_inference")
                && gap["target_id"] == json!("evidence:coverage-real")),
        "the review gap for the now-accepted claim must close"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// A minimal genesis exercising exactly the requirement-placeholder pattern
/// documented in `skills/casegraphen-operate/references/authoring.md`: a work
/// cell with a hard `requires_evidence` edge into an evidence cell that exists
/// only to give that edge something to point at (`lifecycle: proposed`,
/// `review_status: unreviewed`, no declared `evidence_boundary`). Deliberately
/// its own case space rather than an extension of `native_case_fixture()`:
/// that fixture's projections carry a permanent `unreviewed_projection_loss`
/// gap, which would keep assurance at `review_required` regardless of this
/// test and defeat the point of it. `lift native` regenerates the genesis
/// payload and checksums from `case_cells`/`case_relations`/`projections`
/// (`write_genesis_materialization`), so only the top-level records need to be
/// supplied here.
fn assurance_placeholder_fixture() -> Value {
    let space_id = "space:assurance-placeholder-fixture";
    let source_boundary = json!({
        "id": "source_boundary:assurance-placeholder-fixture",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["test.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture makes no inferred claims beyond the declared placeholder",
        "information_loss": []
    });
    json!({
        "schema": "highergraphen.case.space.v1",
        "schema_version": 1,
        "case_space_id": "case_space:assurance-placeholder-fixture",
        "space_id": space_id,
        "case_cells": [
            {
                "id": "work:placeholder-target", "cell_type": "work", "lifecycle": "active",
                "space_id": space_id, "title": "Work needing placeholder evidence",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.9, "review_status": "reviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:placeholder-slot", "cell_type": "evidence", "lifecycle": "proposed",
                "space_id": space_id, "title": "Required evidence placeholder",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "capability:test-mutation", "cell_type": "custom:capability", "lifecycle": "accepted",
                "space_id": space_id, "title": "Authorize test mutations",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {
                    "actor_ids": ["actor:test-mutation-cli"],
                    "operations": ["evidence-attach", "review", "cell-transition"]
                },
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "document", "title": "t"}}
            }
        ],
        "case_relations": [
            {
                "id": "relation:placeholder-requires", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "work:placeholder-target",
                "to_id": "evidence:placeholder-slot", "evidence_ids": [],
                "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            }
        ],
        "morphism_log": [
            {
                "schema": "highergraphen.case.morphism_log_entry.v1", "schema_version": 1,
                "case_space_id": "case_space:assurance-placeholder-fixture", "sequence": 1,
                "entry_id": "morphism_log_entry:genesis", "morphism_id": "morphism:genesis",
                "target_revision_id": "revision:assurance-placeholder-base",
                "morphism": {
                    "morphism_id": "morphism:genesis", "morphism_type": "create",
                    "target_revision_id": "revision:assurance-placeholder-base",
                    "added_ids": [], "updated_ids": [], "retired_ids": [], "preserved_ids": [],
                    "violated_invariant_ids": [], "review_status": "accepted",
                    "evidence_ids": [], "source_ids": ["source:test"],
                    "metadata": {
                        "lift_semantics": "test_fixture_to_case_space",
                        "source_boundary_id": "source_boundary:assurance-placeholder-fixture",
                        "source_boundary": source_boundary
                    }
                },
                "actor_id": "actor:test-author", "recorded_at": "2026-08-01T00:00:00Z",
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}},
                "source_ids": ["source:test"], "replay_checksum": ""
            }
        ],
        "projections": [],
        "revision": {
            "revision_id": "revision:assurance-placeholder-base",
            "case_space_id": "case_space:assurance-placeholder-fixture",
            "applied_entry_ids": ["morphism_log_entry:genesis"],
            "applied_morphism_ids": ["morphism:genesis"],
            "checksum": "", "created_at": "2026-08-01T00:00:00Z",
            "source_ids": ["source:test"], "metadata": {}
        },
        "close_policy_id": null,
        "metadata": {"source_boundary": source_boundary}
    })
}

#[test]
fn assurance_reaches_accepted_only_after_a_requirement_placeholder_is_covered_by_reviewed_evidence()
{
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture_path = directory.join("assurance-placeholder-fixture.case.space.json");
    write_json_value(&fixture_path, &assurance_placeholder_fixture());
    import_native_case_space_from_input(
        &directory,
        &fixture_path,
        "revision:assurance-placeholder-base",
    );
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:assurance-placeholder-fixture";
    let gate_flags = [
        "--actor-id",
        "actor:test-mutation-cli",
        "--capability-id",
        "capability:test-mutation",
        "--operation-scope-id",
        case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:assurance-placeholder-fixture",
    ];
    let reason = || {
        run_cli(&[
            "space",
            "reason",
            "--store",
            &store,
            "--case-space-id",
            case_space_id,
            "--format",
            "json",
        ])
    };

    // Nothing covers the placeholder yet: its own `UnreviewedInference` gap
    // is unresolved, so assurance must not read as accepted.
    let before = reason();
    assert!(before.status.success(), "stderr: {}", stderr(&before));
    let before_evaluation = &stdout_json(&before)["result"]["evaluation"];
    assert_eq!(before_evaluation["assurance"], json!("review_required"));
    let before_placeholder_gap = before_evaluation["review_gaps"]
        .as_array()
        .expect("review gaps")
        .iter()
        .find(|gap| gap["target_id"] == json!("evidence:placeholder-slot"))
        .expect("placeholder gap present before coverage");
    assert_eq!(
        before_placeholder_gap["requirement_satisfied"],
        json!(false),
        "the mark is set at production, in sections::review_gaps, not read from a coverage \
         predicate downstream"
    );

    let claim_path = directory.join("placeholder-claim.evidence.json");
    write_json_value(
        &claim_path,
        &json!({
            "id": "evidence:placeholder-claim", "cell_type": "evidence", "lifecycle": "active",
            "space_id": "space:assurance-placeholder-fixture", "title": "Attached claim",
            "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
            "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                           "source": {"kind": "document", "title": "doc"}}
        }),
    );
    let mut attach_args = vec![
        "evidence",
        "attach",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--base-revision-id",
        "revision:assurance-placeholder-base",
        "--input",
        claim_path.to_str().expect("claim path"),
        "--satisfies",
        "evidence:placeholder-slot",
        "--format",
        "json",
    ];
    attach_args.extend(gate_flags);
    let attached = run_cli(&attach_args);
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();

    // The claim exists but is itself unreviewed, so its coverage of the
    // placeholder is not yet trusted: still not accepted.
    let mid = reason();
    assert!(mid.status.success(), "stderr: {}", stderr(&mid));
    assert_eq!(
        stdout_json(&mid)["result"]["evaluation"]["assurance"],
        json!("review_required")
    );

    let mut review_args = vec![
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--target-id",
        "evidence:placeholder-claim",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "reviewed the attached claim",
        "--base-revision-id",
        attached_revision.as_str(),
        "--evidence-id",
        "evidence:placeholder-claim",
        "--format",
        "json",
    ];
    review_args.extend(gate_flags);
    let reviewed = run_cli(&review_args);
    assert!(reviewed.status.success(), "stderr: {}", stderr(&reviewed));

    // The claim is now trusted and recorded as covering the placeholder, so
    // the placeholder's own `UnreviewedInference` gap must stop driving the
    // axis — but the gap and the inference-separated finding must still be
    // reported, because the placeholder cell itself was never reviewed.
    let after = reason();
    assert!(after.status.success(), "stderr: {}", stderr(&after));
    let after_evaluation = &stdout_json(&after)["result"]["evaluation"];
    assert_eq!(after_evaluation["assurance"], json!("accepted"));
    assert!(
        after_evaluation["evidence_findings"]["unreviewed_inference_ids"]
            .as_array()
            .expect("unreviewed inference ids")
            .contains(&json!("evidence:placeholder-slot")),
        "the placeholder itself was never reviewed and must still surface as an unreviewed \
         inference"
    );
    let after_placeholder_gap = after_evaluation["review_gaps"]
        .as_array()
        .expect("review gaps")
        .iter()
        .find(|gap| {
            gap["gap_type"] == json!("unreviewed_inference")
                && gap["target_id"] == json!("evidence:placeholder-slot")
        })
        .expect(
            "the placeholder's own review gap must remain visible even though it no longer \
                 drives assurance",
        );
    assert_eq!(
        after_placeholder_gap["requirement_satisfied"],
        json!(true),
        "sections::review_gaps must mark the gap once its requirement is satisfied"
    );
    assert!(
        after_evaluation["evidence_findings"]["findings"]
            .as_array()
            .expect("evidence findings")
            .iter()
            .any(
                |finding| finding["finding_type"] == json!("inference_separated")
                    && finding["evidence_ids"] == json!(["evidence:placeholder-slot"])
            ),
        "the inference-separated finding for the placeholder must remain in the report"
    );
    // FIX 2's proof: `assurance: accepted` and a failed
    // `close:native-review-gaps-closed` over the identical gap in the same
    // payload was the self-contradiction this change closed. Both readers
    // now consult the same mark.
    let review_gaps_closed_invariant = after_evaluation["close_check"]["invariant_results"]
        .as_array()
        .expect("close invariant results")
        .iter()
        .find(|invariant| invariant["invariant_id"] == json!("close:native-review-gaps-closed"))
        .expect("close:native-review-gaps-closed invariant present");
    assert_eq!(
        review_gaps_closed_invariant["passed"],
        json!(true),
        "close:native-review-gaps-closed must not fail over the same gap `assurance: accepted` \
         already excluded"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// #24's motivating defect: after attach + review accept, `space reason
/// --format text` showed `Assurance: accepted` alongside "evidence:required
/// is inference and is not accepted evidence." with no visible relationship
/// between the two, reading as a contradiction. The fix renders the review
/// gap's own `requirement_satisfied` next to the finding instead of hiding
/// either fact.
#[test]
fn space_reason_text_shows_a_satisfied_placeholder_gap_without_reading_as_contradictory() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture_path = directory.join("assurance-placeholder-text-fixture.case.space.json");
    write_json_value(&fixture_path, &assurance_placeholder_fixture());
    import_native_case_space_from_input(
        &directory,
        &fixture_path,
        "revision:assurance-placeholder-base",
    );
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:assurance-placeholder-fixture";
    let gate_flags = [
        "--actor-id",
        "actor:test-mutation-cli",
        "--capability-id",
        "capability:test-mutation",
        "--operation-scope-id",
        case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:assurance-placeholder-fixture",
    ];

    let claim_path = directory.join("placeholder-claim-text.evidence.json");
    write_json_value(
        &claim_path,
        &json!({
            "id": "evidence:placeholder-claim", "cell_type": "evidence", "lifecycle": "active",
            "space_id": "space:assurance-placeholder-fixture", "title": "Attached claim",
            "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
            "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                           "source": {"kind": "document", "title": "doc"}}
        }),
    );
    let mut attach_args = vec![
        "evidence",
        "attach",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--base-revision-id",
        "revision:assurance-placeholder-base",
        "--input",
        claim_path.to_str().expect("claim path"),
        "--satisfies",
        "evidence:placeholder-slot",
        "--format",
        "json",
    ];
    attach_args.extend(gate_flags);
    let attached = run_cli(&attach_args);
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();

    let mut review_args = vec![
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--target-id",
        "evidence:placeholder-claim",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "reviewed the attached claim",
        "--base-revision-id",
        attached_revision.as_str(),
        "--evidence-id",
        "evidence:placeholder-claim",
        "--format",
        "json",
    ];
    review_args.extend(gate_flags);
    let reviewed = run_cli(&review_args);
    assert!(reviewed.status.success(), "stderr: {}", stderr(&reviewed));

    let text_report = run_cli(&[
        "space",
        "reason",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
    ]);
    assert!(
        text_report.status.success(),
        "stderr: {}",
        stderr(&text_report)
    );
    let text = stdout(&text_report);
    assert!(text.contains("Assurance: accepted"));
    assert!(text.contains(
        "evidence:placeholder-slot is inference and is not accepted evidence. \
         [review_status=unreviewed] [requirement_satisfied=true]"
    ));
    assert!(
        text.contains("\nReview gaps:"),
        "the new section must be present: {text}"
    );
    assert!(text.contains("[gap_type=unreviewed_inference]"));
    assert!(text.contains("[target=evidence:placeholder-slot]"));
    assert!(
        text.contains("\nWaiting:"),
        "the new section must be present: {text}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// C4 (#24 review): two holders of the same hard evidence requirement, one
/// covered through an unrelated path, one genuinely blocked. `work:w1`
/// requires `evidence:a`; `evidence:a` itself requires `evidence:x`;
/// `work:w2` requires `evidence:x` directly. `evidence:y` (source-backed,
/// trusted) covers `evidence:a` only — never `evidence:x`. That makes
/// `evidence:a` a valid coverage target, so before issue #34,
/// `compute_satisfied_requirement_ids`'s per-holder union marked `evidence:x`
/// "satisfied" via holder `evidence:a`, even though `work:w2`'s own
/// requirement of `evidence:x` remained genuinely uncovered — reproduced
/// against the live evaluator during #24's review, before the allowlist fix
/// below. #34 scoped `compute_satisfied_requirement_ids` to require every
/// holder, so this fixture no longer produces that coarse answer; it is kept
/// because it still exercises the renderer's allowlist (see the comment on
/// `space_reason_text_never_annotates_an_evidence_missing_finding_even_at_a_shared_requirement`
/// below for why that allowlist still matters).
fn two_holder_evidence_missing_fixture() -> Value {
    let space_id = "space:two-holder-evidence-missing";
    let source_boundary = json!({
        "id": "source_boundary:two-holder-evidence-missing",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["test.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture declares its own coverage claims",
        "information_loss": []
    });
    json!({
        "schema": "highergraphen.case.space.v1",
        "schema_version": 1,
        "case_space_id": "case_space:two-holder-evidence-missing",
        "space_id": space_id,
        "case_cells": [
            {
                "id": "work:w1", "cell_type": "work", "lifecycle": "active",
                "space_id": space_id, "title": "W1",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.9, "review_status": "reviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "work:w2", "cell_type": "work", "lifecycle": "active",
                "space_id": space_id, "title": "W2",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.9, "review_status": "reviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:a", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "A (intermediate evidence)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "inferred"},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:x", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "X (shared sub-evidence)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "inferred"},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:y", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "Y (trusted, covers A only)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "source_backed"},
                "provenance": {"confidence": 0.9, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "t"}}
            },
            {
                "id": "capability:test-mutation", "cell_type": "custom:capability", "lifecycle": "accepted",
                "space_id": space_id, "title": "Authorize test mutations",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {
                    "actor_ids": ["actor:test-mutation-cli"],
                    "operations": ["evidence-attach", "review", "cell-transition"]
                },
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "document", "title": "t"}}
            }
        ],
        "case_relations": [
            {
                "id": "relation:w1-requires-a", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "work:w1", "to_id": "evidence:a",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:a-requires-x", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "evidence:a", "to_id": "evidence:x",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:w2-requires-x", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "work:w2", "to_id": "evidence:x",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:y-satisfies-a", "relation_type": "satisfies_evidence_requirement",
                "relation_strength": "diagnostic", "from_id": "evidence:y", "to_id": "evidence:a",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            }
        ],
        "morphism_log": [
            {
                "schema": "highergraphen.case.morphism_log_entry.v1", "schema_version": 1,
                "case_space_id": "case_space:two-holder-evidence-missing", "sequence": 1,
                "entry_id": "morphism_log_entry:genesis", "morphism_id": "morphism:genesis",
                "target_revision_id": "revision:two-holder-evidence-missing-base",
                "morphism": {
                    "morphism_id": "morphism:genesis", "morphism_type": "create",
                    "target_revision_id": "revision:two-holder-evidence-missing-base",
                    "added_ids": [], "updated_ids": [], "retired_ids": [], "preserved_ids": [],
                    "violated_invariant_ids": [], "review_status": "accepted",
                    "evidence_ids": [], "source_ids": ["source:test"],
                    "metadata": {
                        "lift_semantics": "test_fixture_to_case_space",
                        "source_boundary_id": "source_boundary:two-holder-evidence-missing",
                        "source_boundary": source_boundary
                    }
                },
                "actor_id": "actor:test-author", "recorded_at": "2026-08-02T00:00:00Z",
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}},
                "source_ids": ["source:test"], "replay_checksum": ""
            }
        ],
        "projections": [],
        "revision": {
            "revision_id": "revision:two-holder-evidence-missing-base",
            "case_space_id": "case_space:two-holder-evidence-missing",
            "applied_entry_ids": ["morphism_log_entry:genesis"],
            "applied_morphism_ids": ["morphism:genesis"],
            "checksum": "", "created_at": "2026-08-02T00:00:00Z",
            "source_ids": ["source:test"], "metadata": {}
        },
        "close_policy_id": null,
        "metadata": {"source_boundary": source_boundary}
    })
}

#[test]
fn space_reason_text_never_annotates_an_evidence_missing_finding_even_at_a_shared_requirement() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture_path = directory.join("two-holder-evidence-missing-fixture.case.space.json");
    write_json_value(&fixture_path, &two_holder_evidence_missing_fixture());
    import_native_case_space_from_input(
        &directory,
        &fixture_path,
        "revision:two-holder-evidence-missing-base",
    );
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:two-holder-evidence-missing";

    let json_report = run_cli(&[
        "space",
        "reason",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
    ]);
    assert!(
        json_report.status.success(),
        "stderr: {}",
        stderr(&json_report)
    );
    let evaluation = &stdout_json(&json_report)["result"]["evaluation"];
    // Issue #34: `evidence:x`'s own gap now correctly reads unsatisfied,
    // because `work:w2`'s requirement of it is still blocking and
    // `compute_satisfied_requirement_ids` requires every holder, not just
    // one. Before #34 this read `true`, via holder `evidence:a` alone — the
    // coarse union this fixture was built to catch.
    let x_gap = evaluation["review_gaps"]
        .as_array()
        .expect("review gaps")
        .iter()
        .find(|gap| gap["target_id"] == json!("evidence:x"))
        .expect("evidence:x review gap");
    assert_eq!(x_gap["requirement_satisfied"], json!(false));
    assert!(evaluation["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .any(|obstruction| obstruction["witness_ids"] == json!(["evidence:x"])));

    let text_report = run_cli(&[
        "space",
        "reason",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--format",
        "text",
    ]);
    assert!(
        text_report.status.success(),
        "stderr: {}",
        stderr(&text_report)
    );
    let text = stdout(&text_report);
    // The `evidence_missing` finding for work:w2 must state the obstruction
    // plainly, with no `requirement_satisfied` annotation at all. Before #34
    // this mattered because the flag could disagree with the finding —
    // asserting both "none is available" and "requirement_satisfied=true" in
    // the same line was exactly the contradiction #24 exists to stop. After
    // #34, `INV-EVID-001` (`docs/specs/requirement-satisfaction.fsl`, proved
    // by k-induction) makes that disagreement impossible: a `true` flag and
    // a blocking `missing_evidence` obstruction naming the same requirement
    // are now mutually exclusive by construction, so the renderer's
    // allowlist in `push_evidence_finding` is redundant with respect to the
    // evaluator's current strictness. It stays anyway, and this test stays
    // with it, because the reason for the exclusion was never the
    // evaluator's strictness — it is that an `EvidenceMissing` finding's
    // subject is a *(holder, requirement)* pair while `requirement_satisfied`
    // names the requirement alone, and joining across a subject mismatch is
    // wrong regardless of how strict the flag currently happens to be. A
    // reader who sees the allowlist can never fire under today's evaluator
    // must not conclude it can be deleted — the same reasoning
    // `native_halt.rs::is_clearable_by_review`'s doc comment gives for
    // keeping its own constant comparison after its second producer was
    // deleted.
    assert!(text.contains(
        "work:w2 requires source-backed or accepted evidence evidence:x, but none is available. \
         [review_status=unreviewed]\n"
    ));
    // The gap itself still reports its own `requirement_satisfied` — now
    // correctly `false`, since #34 scopes it to every holder and work:w2's
    // holder of evidence:x is still blocked. That fact is not hidden, only
    // kept off a finding it does not describe.
    assert!(text.contains("[target=evidence:x]"));
    assert!(text.contains("[requirement_satisfied=false]"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn assurance_axis_does_not_launder_an_unreviewed_claim_through_an_unrelated_satisfies_target() {
    // The exclusion #20 added must key on an actually-satisfied
    // `requires_evidence` target, not on bare membership in the coverage set
    // `--satisfies` writes to. `--satisfies` accepts any evidence cell as a
    // target (`is_coverage_target` checks only `cell_type: evidence`), so
    // without that distinction an actor holding only `evidence-attach` could
    // attach an unrelated, never-reviewed claim, name it as the
    // `--satisfies` target of a second claim, and have any reviewer's
    // `review accept` of that *second* claim — never of the first — clear
    // the first claim's own review gap out of the Assurance axis.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture_path = directory.join("assurance-oracle-fixture.case.space.json");
    write_json_value(&fixture_path, &assurance_placeholder_fixture());
    import_native_case_space_from_input(
        &directory,
        &fixture_path,
        "revision:assurance-placeholder-base",
    );
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:assurance-placeholder-fixture";
    let gate_flags = [
        "--actor-id",
        "actor:test-mutation-cli",
        "--capability-id",
        "capability:test-mutation",
        "--operation-scope-id",
        case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:assurance-placeholder-fixture",
    ];
    let reason = || {
        run_cli(&[
            "space",
            "reason",
            "--store",
            &store,
            "--case-space-id",
            case_space_id,
            "--format",
            "json",
        ])
    };
    let attach = |claim_id: &str, claim_path: &Path, satisfies: Option<&str>, base: &str| {
        write_json_value(
            claim_path,
            &json!({
                "id": claim_id, "cell_type": "evidence", "lifecycle": "active",
                "space_id": "space:assurance-placeholder-fixture", "title": "Attached claim",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "doc"}}
            }),
        );
        let mut args = vec![
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            case_space_id,
            "--base-revision-id",
            base,
            "--input",
            claim_path.to_str().expect("claim path"),
        ];
        if let Some(target) = satisfies {
            args.extend(["--satisfies", target]);
        }
        args.extend(["--format", "json"]);
        args.extend(gate_flags);
        let attached = run_cli(&args);
        assert!(attached.status.success(), "stderr: {}", stderr(&attached));
        stdout_json(&attached)["result"]["record"]["current_revision_id"]
            .as_str()
            .expect("attached revision")
            .to_owned()
    };

    // Baseline: the documented placeholder pattern, already covered and
    // reviewed, reports `accepted` — the same setup and outcome as
    // `assurance_reaches_accepted_only_after_a_requirement_placeholder_is_covered_by_reviewed_evidence`.
    let after_placeholder_attach = attach(
        "evidence:placeholder-claim",
        &directory.join("placeholder-claim.evidence.json"),
        Some("evidence:placeholder-slot"),
        "revision:assurance-placeholder-base",
    );
    let mut review_args = vec![
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--target-id",
        "evidence:placeholder-claim",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "reviewed the placeholder claim",
        "--base-revision-id",
        after_placeholder_attach.as_str(),
        "--evidence-id",
        "evidence:placeholder-claim",
        "--format",
        "json",
    ];
    review_args.extend(gate_flags);
    let placeholder_reviewed = run_cli(&review_args);
    assert!(
        placeholder_reviewed.status.success(),
        "stderr: {}",
        stderr(&placeholder_reviewed)
    );
    let baseline_revision = stdout_json(&placeholder_reviewed)["result"]["record"]
        ["current_revision_id"]
        .as_str()
        .expect("baseline revision")
        .to_owned();
    assert_eq!(
        stdout_json(&reason())["result"]["evaluation"]["assurance"],
        json!("accepted")
    );

    // Step 1: attach a claim nothing requires and nothing satisfies. Its own
    // review gap is unresolved, so assurance must leave `accepted`.
    let after_a1 = attach(
        "evidence:oracle-a1",
        &directory.join("oracle-a1.evidence.json"),
        None,
        &baseline_revision,
    );
    let after_a1_evaluation = stdout_json(&reason())["result"]["evaluation"].clone();
    assert_eq!(
        after_a1_evaluation["assurance"],
        json!("review_required"),
        "an unreviewed claim that nothing requires must not stay hidden behind `accepted`"
    );

    // Step 2: attach a second claim naming the first as its `--satisfies`
    // target. `evidence:oracle-a1` is not a `requires_evidence` target of
    // anything — this is exactly the coverage claim `is_coverage_target`
    // allows and the fix must not treat as a satisfied requirement.
    let after_a2 = attach(
        "evidence:oracle-a2",
        &directory.join("oracle-a2.evidence.json"),
        Some("evidence:oracle-a1"),
        &after_a1,
    );
    assert_eq!(
        stdout_json(&reason())["result"]["evaluation"]["assurance"],
        json!("review_required")
    );

    // Step 3: review accept the *second* claim only. `evidence:oracle-a1`
    // itself is still never reviewed by anyone.
    let mut promote_a2 = vec![
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--target-id",
        "evidence:oracle-a2",
        "--reviewer-id",
        "reviewer:human",
        "--reason",
        "reviewed the second claim only",
        "--base-revision-id",
        after_a2.as_str(),
        "--evidence-id",
        "evidence:oracle-a2",
        "--format",
        "json",
    ];
    promote_a2.extend(gate_flags);
    let a2_reviewed = run_cli(&promote_a2);
    assert!(
        a2_reviewed.status.success(),
        "stderr: {}",
        stderr(&a2_reviewed)
    );

    let after_evaluation = stdout_json(&reason())["result"]["evaluation"].clone();
    assert_eq!(
        after_evaluation["assurance"],
        json!("review_required"),
        "reviewing an unrelated second claim must not launder the first, never-reviewed claim \
         out of the Assurance axis just because it named the first as a `--satisfies` target"
    );
    assert!(
        after_evaluation["evidence_findings"]["unreviewed_inference_ids"]
            .as_array()
            .expect("unreviewed inference ids")
            .contains(&json!("evidence:oracle-a1")),
        "the never-reviewed claim must still surface as an unreviewed inference"
    );
    assert!(
        after_evaluation["review_gaps"]
            .as_array()
            .expect("review gaps")
            .iter()
            .any(|gap| gap["gap_type"] == json!("unreviewed_inference")
                && gap["target_id"] == json!("evidence:oracle-a1")),
        "the never-reviewed claim's own review gap must remain open"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn generic_morphism_refuses_caller_declared_evidence_trust_on_an_added_cell() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:added-evidence-trust-base");

    // `evidence attach` overwrites the boundary its input names, because the
    // boundary decides whether a cell satisfies a hard requirement with no
    // review. A morphism payload reaches the same state, so it has to answer to
    // the same rule — one caller-written string used to be the whole difference
    // between a blocking obstruction and a cleared one.
    let added_evidence = |boundary: &str| {
        json!({
            "id": "evidence:declared-trust",
            "cell_type": "evidence",
            "lifecycle": "active",
            "space_id": "space:casegraphen",
            "title": "Evidence that names its own boundary",
            "source_ids": ["source:native-cli"],
            "structure_ids": [],
            "metadata": {"evidence_boundary": boundary},
            "provenance": {
                "confidence": 1.0,
                "review_status": "unreviewed",
                "source": {"kind": "document", "title": "caller supplied"}
            }
        })
    };
    let morphism = |boundary: &str| {
        json!({
            "morphism_id": "morphism:added-evidence-trust",
            "morphism_type": "update",
            "source_revision_id": "revision:added-evidence-trust-base",
            "target_revision_id": "revision:added-evidence-trust",
            "added_ids": ["evidence:declared-trust"],
            "updated_ids": [],
            "retired_ids": [],
            "preserved_ids": [],
            "violated_invariant_ids": [],
            "review_status": "unreviewed",
            "evidence_ids": [],
            "source_ids": ["source:native-cli"],
            "metadata": {"payload": {"added_cells": [added_evidence(boundary)]}}
        })
    };
    let morphism_path = directory.join("added-evidence-trust.case_morphism.json");
    let propose = |boundary: &str| {
        fs::write(
            &morphism_path,
            serde_json::to_string_pretty(&morphism(boundary)).expect("serialize morphism"),
        )
        .expect("write morphism");
        run_cli(&[
            "morphism",
            "propose",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--input",
            morphism_path.to_str().expect("morphism path"),
            "--format",
            "json",
        ])
    };

    for boundary in ["source_backed", "review_promoted"] {
        let refused = propose(boundary);
        assert!(
            !refused.status.success(),
            "evidence_boundary {boundary} was accepted on an added cell"
        );
        assert!(
            stderr(&refused).contains(
                "evidence entering after genesis is untrusted, so only inferred and \
                 worker_output are accepted"
            ),
            "stderr: {}",
            stderr(&refused)
        );
    }

    // The rule must not be wider than the defect: the two spellings this tool
    // itself mints after genesis still pass.
    for boundary in ["inferred", "worker_output"] {
        let accepted = propose(boundary);
        assert!(
            accepted.status.success(),
            "evidence_boundary {boundary} was refused; stderr: {}",
            stderr(&accepted)
        );
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn generic_morphisms_cannot_forge_plan_review_or_status() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:forged-plan-base");

    let forged_morphism_path = directory.join("forged-plan-review.case_morphism.json");
    write_native_metadata_morphism_with_metadata(
        &forged_morphism_path,
        "morphism:forged-plan-review",
        "revision:forged-plan-base",
        "revision:forged-plan-review",
        json!({
            "native_review_schema_version": 1,
            "review_id": "review:forged-plan-review",
            "target_kind": "plan",
            "target_id": "plan:forged-acceptance",
            "action": "accept",
            "outcome_review_status": "accepted",
            "reviewer_id": "reviewer:forged",
            "reviewed_at": "2026-07-30T00:00:00Z",
            "reason": "Generic morphisms must not forge canonical reviews.",
            "plan_content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }),
    );
    let forged_propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        forged_morphism_path.to_str().expect("forged morphism path"),
        "--format",
        "json",
    ]);
    assert!(!forged_propose.status.success());
    assert!(stderr(&forged_propose).contains("reserved canonical review metadata"));

    let safe_morphism_path = directory.join("safe-before-apply-tamper.case_morphism.json");
    write_native_metadata_morphism(
        &safe_morphism_path,
        "morphism:apply-tamper",
        "revision:forged-plan-base",
        "revision:apply-tamper",
    );
    let safe_propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        safe_morphism_path.to_str().expect("safe morphism path"),
        "--format",
        "json",
    ]);
    assert!(
        safe_propose.status.success(),
        "stderr: {}",
        stderr(&safe_propose)
    );
    let safe_propose_json = stdout_json(&safe_propose);
    let proposal_path = directory.join(
        safe_propose_json["result"]["proposal_path"]
            .as_str()
            .expect("proposal path"),
    );
    let mut proposal = json_file(proposal_path.clone());
    proposal["morphism"]["metadata"]["operation_gate"] = json!({
        "actor_id": "actor:forged",
        "operation": "plan-review"
    });
    fs::write(
        &proposal_path,
        serde_json::to_string_pretty(&proposal).expect("serialize tampered proposal"),
    )
    .expect("tamper proposal");
    let tampered_apply = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:apply-tamper",
            "--base-revision-id",
            "revision:forged-plan-base",
            "--reviewer-id",
            "reviewer:forged",
            "--reason",
            "Attempt reserved metadata at apply",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!tampered_apply.status.success());
    assert!(stderr(&tampered_apply).contains("reserved canonical review metadata"));

    let plan_input = directory.join("forged-acceptance.execution.plan.json");
    write_execution_plan(
        &plan_input,
        "plan:forged-acceptance",
        "revision:forged-plan-base",
        "work:review-native-contract",
    );
    let plan_propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input"),
        "--format",
        "json",
    ]);
    assert!(
        plan_propose.status.success(),
        "stderr: {}",
        stderr(&plan_propose)
    );
    let stored_plan_path = directory
        .join("plans")
        .join("plan~3aforged-acceptance.execution.plan.json");
    let mut stored_plan = json_file(stored_plan_path.clone());
    stored_plan["review_status"] = json!("accepted");
    fs::write(
        &stored_plan_path,
        serde_json::to_string_pretty(&stored_plan).expect("serialize forged stored plan"),
    )
    .expect("forge stored plan status");

    let forged_run = run_cli(&[
        "run",
        "--step",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:forged-acceptance",
        "--base-revision-id",
        "revision:forged-plan-base",
        "--actor-id",
        "actor:forged-run",
        "--gate-actor-id",
        "actor:forged-run",
        "--capability-id",
        "capability:dispatch",
        "--capability-id",
        "capability:native-integration-worker",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--enable-worker",
        "shell",
        "--format",
        "json",
    ]);
    assert!(!forged_run.status.success());
    assert!(stderr(&forged_run).contains("disagrees with log-derived status unreviewed"));
    assert!(stderr(&forged_run).contains("possible plan tampering"));
    // A forged stored review_status is `verified_plan_review_status`
    // re-verifying already-recorded state against the log, not live
    // authorization — `store_integrity` ("stop and investigate"), not
    // `invalid` ("fix the call and retry").
    assert_eq!(
        stderr_json(&forged_run)["error_code"],
        json!("store_integrity")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_execution_plan_propose_check_and_accept_with_gate() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:plan-accept-base");
    let input_path = directory.join("execution-plan.accept.json");
    write_execution_plan(
        &input_path,
        "plan:native-accept",
        "revision:plan-accept-base",
        "work:review-native-contract",
    );

    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        input_path.to_str().expect("plan path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let propose_json = stdout_json(&propose);
    let content_hash = propose_json["result"]["plan_content_hash"]
        .as_str()
        .expect("plan content hash")
        .to_owned();
    assert_eq!(content_hash.len(), 64);
    assert!(content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let stored_path = directory
        .join("plans")
        .join("plan~3anative-accept.execution.plan.json");
    assert_eq!(
        json_file(stored_path.clone())["review_status"],
        json!("unreviewed")
    );
    let stored_plan = json_file(stored_path.clone());
    let binding_hash = stored_plan["metadata"]["worker_binding_hashes"]
        ["worker_binding:native-integration"]
        .as_str()
        .expect("recorded worker binding hash");
    assert_eq!(binding_hash.len(), 64);
    assert_eq!(
        propose_json["result"]["plan"]["metadata"]["worker_binding_hashes"]
            ["worker_binding:native-integration"],
        json!(binding_hash)
    );

    let check = run_cli(&[
        "plan",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:native-accept",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    let check_json = stdout_json(&check);
    assert_eq!(
        check_json["result"]["frontier_cell_ids"],
        json!([
            "goal:native-case-contract",
            "case:native-contract-example",
            "work:review-native-contract"
        ])
    );
    assert_eq!(
        check_json["result"]["step_readiness"][0]["on_readiness_frontier"],
        json!(true)
    );
    assert_eq!(
        check_json["result"]["plan_content_hash"],
        json!(content_hash)
    );

    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:native-accept",
        "--reviewer-id",
        "reviewer:plan-accept",
        "--reason",
        "Accept the bounded native execution plan",
        "--base-revision-id",
        "revision:plan-accept-base",
        "--actor-id",
        "actor:plan-accept",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accept_json = stdout_json(&accept);
    let entry = &accept_json["result"]["entry"];
    assert_eq!(
        accept_json["metadata"]["command"],
        json!("casegraphen plan accept")
    );
    assert_eq!(entry["actor_id"], json!("actor:plan-accept"));
    assert_eq!(
        entry["morphism"]["metadata"],
        json!({
            "native_review_schema_version": 1,
            "review_id": entry["morphism"]["metadata"]["review_id"],
            "target_kind": "plan",
            "target_id": "plan:native-accept",
            "action": "accept",
            "outcome_review_status": "accepted",
            "reviewer_id": "reviewer:plan-accept",
            "reviewed_at": entry["morphism"]["metadata"]["reviewed_at"],
            "reason": "Accept the bounded native execution plan",
            "plan_content_hash": content_hash,
            "operation_gate": {
                "actor_id": "actor:plan-accept",
                "operation": "plan-review",
                "operation_scope_id": native_case_space_id(),
                "audience": "audit",
                "capability_ids": ["capability:plan-review"],
                "source_boundary_id": "source_boundary:native-case-management-contract",
            },
        })
    );
    assert_eq!(
        accept_json["result"]["operation_gate"]["operation"],
        json!("plan-review")
    );
    assert_eq!(json_file(stored_path)["review_status"], json!("accepted"));

    let history = run_native_case_store_command(&directory, "history");
    let history_json = stdout_json(&history);
    assert_eq!(
        history_json["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        2
    );
    assert_eq!(
        history_json["result"]["entries"][1]["morphism"]["metadata"]["plan_content_hash"],
        json!(content_hash)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_execution_plan_accept_requires_gate_and_unknown_work_is_rejected() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:plan-failure-base");
    let input_path = directory.join("execution-plan.no-gate.json");
    write_execution_plan(
        &input_path,
        "plan:no-gate",
        "revision:plan-failure-base",
        "work:review-native-contract",
    );
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        input_path.to_str().expect("plan path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));

    let no_gate = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:no-gate",
        "--reviewer-id",
        "reviewer:no-gate",
        "--reason",
        "This must not be accepted without an operation gate",
        "--base-revision-id",
        "revision:plan-failure-base",
        "--format",
        "json",
    ]);
    assert!(!no_gate.status.success());
    assert!(stderr(&no_gate).contains("--actor-id"));

    let fabricated_capability = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:no-gate",
        "--reviewer-id",
        "reviewer:fabricated-capability",
        "--reason",
        "A fabricated capability must not authorize plan review",
        "--base-revision-id",
        "revision:plan-failure-base",
        "--actor-id",
        "actor:fabricated-capability",
        "--capability-id",
        "capability:fabricated",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(!fabricated_capability.status.success());
    assert!(stderr(&fabricated_capability).contains("existing case cell"));
    // The plan surface's own gate check must classify identically to the
    // mutation surface's (issue #22 batch 2): both go through the same
    // `From<NativeOperationGateError>` conversion now, so a fabricated
    // capability here is `gate_violation` with witnesses, not `invalid`.
    let fabricated_capability_refusal = stderr_json(&fabricated_capability);
    assert_eq!(
        fabricated_capability_refusal["error_code"],
        json!("gate_violation")
    );
    assert!(
        fabricated_capability_refusal["data"]["witness_ids"]
            .as_array()
            .expect("witness_ids is an array")
            .contains(&json!("capability:fabricated")),
        "witness_ids: {}",
        fabricated_capability_refusal["data"]["witness_ids"]
    );

    let unknown_path = directory.join("execution-plan.unknown-work.json");
    write_execution_plan(
        &unknown_path,
        "plan:unknown-work",
        "revision:plan-failure-base",
        "work:not-present",
    );
    let unknown = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        unknown_path.to_str().expect("plan path"),
        "--format",
        "json",
    ]);
    assert!(!unknown.status.success());
    assert!(stderr(&unknown).contains("work:not-present"));
    assert!(stderr(&unknown).contains("missing work_cell_id"));

    let missing_requirement_path = directory.join("execution-plan.missing-requirement.json");
    write_execution_plan(
        &missing_requirement_path,
        "plan:missing-requirement",
        "revision:plan-failure-base",
        "work:review-native-contract",
    );
    let mut missing_requirement_plan = json_file(missing_requirement_path.clone());
    missing_requirement_plan["steps"][0]["success_evidence_requirement_ids"] =
        json!(["evidence:missing"]);
    fs::write(
        &missing_requirement_path,
        serde_json::to_string_pretty(&missing_requirement_plan)
            .expect("serialize missing requirement plan"),
    )
    .expect("write missing requirement plan");
    let missing_requirement = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        missing_requirement_path
            .to_str()
            .expect("missing requirement plan path"),
        "--format",
        "json",
    ]);
    assert!(!missing_requirement.status.success());
    assert!(stderr(&missing_requirement).contains("evidence:missing"));
    assert!(stderr(&missing_requirement).contains("not existing case cells"));

    let empty_success_path = directory.join("execution-plan.empty-success.json");
    write_execution_plan(
        &empty_success_path,
        "plan:empty-success",
        "revision:plan-failure-base",
        "work:review-native-contract",
    );
    let mut empty_success_plan = json_file(empty_success_path.clone());
    empty_success_plan["steps"][0]["success_evidence_requirement_ids"] = json!([]);
    fs::write(
        &empty_success_path,
        serde_json::to_string_pretty(&empty_success_plan)
            .expect("serialize empty success requirement plan"),
    )
    .expect("write empty success requirement plan");
    let empty_success = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        empty_success_path
            .to_str()
            .expect("empty success requirement plan path"),
        "--format",
        "json",
    ]);
    assert!(!empty_success.status.success());
    assert!(stderr(&empty_success).contains("success_evidence_requirement_ids must not be empty"));

    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn plan_propose_refuses_a_plan_authored_against_a_revision_that_is_no_longer_current() {
    // Distinct from the mutation surface's `stale_revision` (issue #22
    // batch 2): the subject here is the plan file itself, authored against
    // a base revision the case space has since moved past, not a caller's
    // concurrency token on an otherwise-current call. Recovery differs too
    // — regenerate the plan against the current revision, not retry the
    // same plan with a corrected `--base-revision-id`.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:plan-stale-base");

    let stale_path = directory.join("execution-plan.stale-base.json");
    write_execution_plan(
        &stale_path,
        "plan:stale-base",
        "revision:not-the-current-revision",
        "work:review-native-contract",
    );
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        stale_path.to_str().expect("plan path"),
        "--format",
        "json",
    ]);
    assert!(!propose.status.success());
    let refusal = stderr_json(&propose);
    assert_eq!(refusal["error_code"], json!("stale_plan_revision"));
    assert_eq!(refusal["data"]["plan_id"], json!("plan:stale-base"));
    assert_eq!(
        refusal["data"]["base_revision_id"],
        json!("revision:not-the-current-revision")
    );
    assert_eq!(
        refusal["data"]["current_revision_id"],
        json!("revision:plan-stale-base")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_execution_plan_propose_requires_registered_worker_bindings() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:plan-missing-binding");
    let input = directory.join("missing-binding.execution.plan.json");
    write_execution_plan_for_binding(
        &input,
        "plan:missing-binding",
        "revision:plan-missing-binding",
        "work:review-native-contract",
        "worker_binding:not-registered",
    );

    let output = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("worker_binding:not-registered"));
    assert!(stderr(&output).contains("not registered"));
    assert!(!directory
        .join("plans")
        .join("plan~3amissing-binding.execution.plan.json")
        .exists());
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_execution_plan_reject_records_review_and_rewrites_plan() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:plan-reject-base");
    let input_path = directory.join("execution-plan.reject.json");
    write_execution_plan(
        &input_path,
        "plan:native-reject",
        "revision:plan-reject-base",
        "work:review-native-contract",
    );
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        input_path.to_str().expect("plan path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let content_hash = stdout_json(&propose)["result"]["plan_content_hash"].clone();

    let reject = run_cli(&[
        "plan",
        "reject",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        "plan:native-reject",
        "--reviewer-id",
        "reviewer:plan-reject",
        "--reason",
        "Reject the execution plan",
        "--base-revision-id",
        "revision:plan-reject-base",
        "--actor-id",
        "actor:plan-reject",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    let reject_json = stdout_json(&reject);
    assert_eq!(
        reject_json["result"]["entry"]["actor_id"],
        json!("actor:plan-reject")
    );
    assert_eq!(
        reject_json["result"]["entry"]["morphism"]["metadata"]["target_kind"],
        json!("plan")
    );
    assert_eq!(
        reject_json["result"]["entry"]["morphism"]["metadata"]["action"],
        json!("reject")
    );
    assert_eq!(
        reject_json["result"]["entry"]["morphism"]["metadata"]["plan_content_hash"],
        content_hash
    );
    assert_eq!(
        json_file(
            directory
                .join("plans")
                .join("plan~3anative-reject.execution.plan.json")
        )["review_status"],
        json!("rejected")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn worker_binding_register_stores_pretty_json_and_content_hash() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let input = directory.join("register.worker.binding.json");
    write_worker_binding(
        &input,
        "worker_binding:register-integration",
        &directory,
        "printf 'registered\\n'",
    );

    let register = run_cli(&[
        "binding",
        "register",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("binding path"),
        "--format",
        "json",
    ]);

    assert!(register.status.success(), "stderr: {}", stderr(&register));
    let result = stdout_json(&register);
    assert_eq!(result["result"]["binding_status"], json!("registered"));
    let hash = result["result"]["binding_content_hash"]
        .as_str()
        .expect("binding content hash");
    assert_eq!(hash.len(), 64);
    let stored_path = directory.join(
        result["result"]["binding_path"]
            .as_str()
            .expect("binding path"),
    );
    let stored_text = fs::read_to_string(&stored_path).expect("read stored binding");
    assert!(stored_text.contains("\n  \"schema\""));
    assert!(stored_text.ends_with('\n'));

    let duplicate = run_cli(&[
        "binding",
        "register",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("binding path"),
        "--format",
        "json",
    ]);
    assert!(!duplicate.status.success());
    assert!(stderr(&duplicate).contains("already exists"));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_executes_one_accepted_plan_step_and_then_stops() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "happy",
        "printf 'successful-worker-output\\n'; printf 'diagnostic-worker-output\\n' >&2",
    );

    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    assert_eq!(first_json["result"]["status"], json!("step_executed"));
    assert_eq!(
        first_json["result"]["trace"]["step_id"],
        json!(fixture.step_id)
    );
    assert_eq!(
        first_json["result"]["trace"]["transition_applied"],
        json!(true)
    );
    assert_eq!(
        first_json["result"]["appended_entry_ids"]
            .as_array()
            .expect("appended entries")
            .len(),
        3
    );
    let trace_path = only_run_file(&directory, "execution.trace.json");
    let report_path = only_run_file(&directory, "worker.report.json");
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/execution.trace.schema.json"),
        &trace_path,
    );
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/worker.report.schema.json"),
        &report_path,
    );
    let trace = json_file(trace_path.clone());
    let run_directory = trace_path.parent().expect("run directory").to_owned();
    assert_eq!(
        trace["worker_report_content_hash"],
        json!(sha256_file(&report_path))
    );
    assert_eq!(
        trace["stdout_content_hash"],
        json!(sha256_file(&run_directory.join("stdout")))
    );
    assert_eq!(
        trace["stderr_content_hash"],
        json!(sha256_file(&run_directory.join("stderr")))
    );

    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let cells = replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells");
    let work = cells
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("work cell");
    assert_eq!(work["lifecycle"], json!("resolved"));
    let worker_evidence = cells
        .iter()
        .find(|cell| {
            cell["cell_type"] == json!("evidence")
                && cell["metadata"]["worker_report_id"].is_string()
        })
        .expect("worker evidence cell");
    assert_eq!(
        worker_evidence["provenance"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        worker_evidence["provenance"]["source"]["kind"],
        json!("custom:tool_captured_artifact")
    );
    assert_eq!(
        worker_evidence["metadata"]["evidence_boundary"],
        json!("worker_output")
    );
    let relations = replay["result"]["replay"]["case_space"]["case_relations"]
        .as_array()
        .expect("replayed relations");
    assert!(relations.iter().any(|relation| {
        relation["relation_type"] == json!("satisfies_evidence_requirement")
            && relation["from_id"] == worker_evidence["id"]
            && relation["relation_strength"] == json!("diagnostic")
    }));
    let evidence_entry = replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .find(|entry| {
            entry["morphism"]["morphism_type"] == json!("evidence_attach")
                && entry["morphism"]["metadata"]["trace_id"]
                    == first_json["result"]["trace"]["trace_id"]
        })
        .expect("worker evidence morphism");
    assert_eq!(
        evidence_entry["morphism"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        first_json["result"]["trace"]["operation_gate"]["capability_ids"],
        json!(["capability:dispatch", "capability:native-run-worker"])
    );
    assert_eq!(
        first_json["result"]["trace"]["unsatisfied_success_evidence_requirement_ids"],
        json!([])
    );

    let result_revision = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("result revision");
    let second = run_native_step_with_base(&directory, &fixture, result_revision, true, None);
    assert!(second.status.success(), "stderr: {}", stderr(&second));
    let second_json = stdout_json(&second);
    assert_eq!(
        second_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert!(second_json["result"]["trace"].is_null());
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_reports_clean_exit_descendant_containment_truthfully() {
    let utilities_available = dedicated_session_utilities_available();
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let sleeper_pid_path = directory.join("background-sleeper.pid");
    let script = format!(
        "exec /bin/sleep 400 >/dev/null 2>&1 & printf '%s\\n' \"$!\" > '{}'; exit 0",
        sleeper_pid_path.display()
    );
    let fixture = setup_native_run(&directory, "clean-exit-descendant", &script);

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("step_executed"));
    let expected_descendants_may_survive = !utilities_available;
    assert_eq!(
        value["result"]["worker_report_summary"]["descendants_may_survive"],
        json!(expected_descendants_may_survive)
    );
    let report = json_file(only_run_file(&directory, "worker.report.json"));
    assert_eq!(
        report["descendants_may_survive"],
        json!(expected_descendants_may_survive)
    );

    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let evidence = replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .find(|cell| cell["metadata"]["worker_report_id"] == report["report_id"])
        .expect("evidence anchored to worker report");
    assert!(evidence["source_ids"]
        .as_array()
        .expect("evidence source ids")
        .contains(&report["report_id"]));

    let sleeper_pid = fs::read_to_string(&sleeper_pid_path)
        .expect("read background sleeper pid")
        .trim()
        .parse::<u32>()
        .expect("background sleeper pid");
    if utilities_available {
        let exited = wait_for_process_exit(sleeper_pid, Duration::from_secs(5));
        if !exited {
            kill_process(sleeper_pid);
        }
        assert!(
            exited,
            "background sleeper {sleeper_pid} survived a clean worker exit"
        );
    } else {
        assert!(
            process_exists(sleeper_pid),
            "the no-containment fixture must leave a descendant to make the report observable"
        );
        kill_process(sleeper_pid);
        assert!(wait_for_process_exit(sleeper_pid, Duration::from_secs(5)));
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_distinguishes_an_empty_group_from_missing_containment() {
    let utilities_available = dedicated_session_utilities_available();
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(&directory, "clean-exit-empty-group", "exit 0");

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("step_executed"));
    let expected_descendants_may_survive = !utilities_available;
    assert_eq!(
        value["result"]["worker_report_summary"]["descendants_may_survive"],
        json!(expected_descendants_may_survive)
    );
    assert_eq!(
        json_file(only_run_file(&directory, "worker.report.json"))["descendants_may_survive"],
        json!(expected_descendants_may_survive)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_does_not_rebase_after_an_intervening_append() {
    // Issue #32: this used to bet that a whole external `cell transition`
    // invocation (spawn, load, evaluate, snapshot, append) finished inside a
    // fixed 0.5 s worker sleep — a wall-clock race a loaded machine can
    // lose. The worker now waits on a marker the test creates only after
    // the intervening append has actually landed, so there is no window to
    // miss.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("worker-started");
    let proceed = directory.join("worker-proceed");
    let script = format!(
        "printf 'started\\n' > '{}'; {}; printf 'worker-output\\n'",
        worker_started.display(),
        shell_wait_for_marker(&proceed)
    );
    let fixture = setup_native_run(&directory, "pinned-application-base", &script);
    let child = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_step_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            true,
            None,
            &["capability:dispatch", "capability:native-run-worker"],
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn casegraphen run --step");
    wait_for_file(&worker_started, "worker did not start before timeout");

    let intervening = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &fixture.accepted_revision_id,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "resolved",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(
        intervening.status.success(),
        "stderr: {}",
        stderr(&intervening)
    );
    signal_rendezvous_marker(&proceed);

    let output = child.wait_with_output().expect("wait for run --step");

    assert!(!output.status.success());
    // Issue #39: this used to assert the refusal's message contained
    // "entry sequence must be" — `store_integrity`'s message, pinned back
    // when the sequence check ran before the source-revision check and so
    // reported first. This is the purpose-built reproduction of that exact
    // race (an intervening append moves current out from under a pinned
    // base while a worker is still running), so it is also the case the
    // reclassification is *for*: the caller's own pinned base is what went
    // stale, which is `stale_revision`'s "re-read current_revision_id and
    // retry", not `store_integrity`'s "stop and investigate". Assert the
    // refusal's structured fields, not its message — `error_code` and
    // `data` are the stable contract (`native-cli.refusal.schema.json`),
    // `message` explicitly is not.
    let refusal = stderr_json(&output);
    assert_eq!(refusal["error_code"], json!("stale_revision"));
    assert!(
        refusal["data"]["current_revision_id"].is_string(),
        "refusal: {refusal:?}"
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert_eq!(
        replayed_work_lifecycle(&replay),
        "active",
        "run --step must not apply its transition against a revision newer than its pinned base"
    );
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_refuses_plan_whose_latest_review_is_reject() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(&directory, "accept-then-reject", "printf 'must-not-run\\n'");

    let reject = run_cli(&[
        "plan",
        "reject",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &fixture.plan_id,
        "--reviewer-id",
        "reviewer:run-plan-reject",
        "--reason",
        "A later rejection revokes the earlier acceptance",
        "--base-revision-id",
        &fixture.accepted_revision_id,
        "--actor-id",
        "actor:run-plan-reject",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    let rejected_revision = stdout_json(&reject)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("rejected revision")
        .to_owned();

    let output = run_native_step_with_base(&directory, &fixture, &rejected_revision, true, None);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("latest plan review"));
    assert!(stderr(&output).contains("Rejected"));
    assert!(!directory.join("runs").exists());
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_requires_shell_worker_opt_in() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(&directory, "disabled", "printf 'must-not-run\\n'");

    let output = run_native_step(&directory, &fixture, false, None);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("shell worker kind is disabled by default"));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_records_tampered_binding_as_domain_obstruction() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(&directory, "tampered", "printf 'original\\n'");
    let mut binding = json_file(fixture.binding_path.clone());
    binding["args"] = json!(["-c", "printf 'tampered\\n'"]);
    fs::write(
        &fixture.binding_path,
        serde_json::to_string_pretty(&binding).expect("serialize tampered binding"),
    )
    .expect("tamper binding");

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("binding_hash_mismatch")
    );
    assert_eq!(value["result"]["trace"]["transition_applied"], json!(false));
    let trace_path = only_run_file(&directory, "execution.trace.json");
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/execution.trace.schema.json"),
        &trace_path,
    );
    assert_eq!(
        fs::read(trace_path.parent().expect("run directory").join("stdout"))
            .expect("read anchored empty stdout"),
        b""
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert_eq!(
        replay["result"]["replay"]["current_revision_id"],
        value["result"]["trace"]["result_revision_id"]
    );
    assert_eq!(
        replayed_work_lifecycle(&replay),
        "active",
        "tampered binding must not dispatch or transition work"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_failed_anchor_append_leaves_the_trace_naming_only_what_was_written() {
    // The trace file must be written before the anchor append, because the
    // anchor hashes it — so it names an entry and a revision that do not exist
    // yet. When the append does not commit, the file used to keep naming them,
    // and "an anchored revision that is not in the store" is precisely the
    // signal residual risk 2 tells an operator means history was erased.
    // Ordinary lock contention was enough to produce it.
    //
    // Driven without timing: the anchor writes a snapshot at a path derived
    // from its revision, and an unscheduled sequence still reads any file
    // already there and requires it to agree
    // (`require_existing_snapshot_agrees_with_candidate`, the sibling of
    // `require_snapshot_absent`). Occupying that path with the genesis
    // snapshot fails the append on the store's own invariant rather than on a
    // serde message, which cannot change without the contract changing.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(&directory, "anchor-rewind", "printf 'rewind\\n'");
    let inspected = stdout_json(&run_native_case_store_command(&directory, "inspect"));
    let genesis_snapshot = directory.join(
        inspected["result"]["record"]["nearest_snapshot_path"]
            .as_str()
            .expect("nearest snapshot path"),
    );
    let snapshots = genesis_snapshot
        .parent()
        .expect("snapshots directory")
        .to_path_buf();

    // The name mirrors `path_segment`, applied at each nesting level: the trace
    // id embeds the escaped plan and step ids, the anchor revision embeds the
    // escaped trace id, and the file name escapes that. Written out rather
    // than learned because nothing reports it before the run that needs it to
    // already exist — and the test fails loudly if it drifts, since an
    // unoccupied path lets the append succeed.
    let escape = |id: &str| {
        id.chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                    character.to_string()
                } else {
                    format!("~{:02x}", character as u32)
                }
            })
            .collect::<String>()
    };
    let trace_id = format!(
        "execution_trace:{}:{}:1",
        escape(&fixture.plan_id),
        escape(&fixture.step_id)
    );
    let anchor_revision = format!("revision:execution-trace-anchor:{}", escape(&trace_id));
    let occupied = snapshots.join(format!("{}.case.space.json", escape(&anchor_revision)));
    fs::copy(&genesis_snapshot, &occupied).expect("occupy the anchor snapshot path");

    let output = run_native_step(&directory, &fixture, true, None);
    assert!(
        !output.status.success(),
        "the anchor append must fail with its snapshot path occupied; occupied={} stderr={}",
        occupied.display(),
        stderr(&output)
    );

    let trace = json_file(only_run_file(&directory, "execution.trace.json"));
    let log_path = imported_native_log_path(&directory, &inspected);
    let entries = fs::read_to_string(&log_path)
        .expect("read log")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("parse log entry"))
        .collect::<Vec<_>>();
    let entry_ids = entries
        .iter()
        .map(|entry| entry["entry_id"].clone())
        .collect::<Vec<_>>();
    let revisions = entries
        .iter()
        .map(|entry| entry["target_revision_id"].clone())
        .collect::<Vec<_>>();

    assert!(
        !revisions.contains(&json!(anchor_revision)),
        "the anchor must not have been appended"
    );
    for appended in trace["appended_entry_ids"]
        .as_array()
        .expect("appended entry ids")
    {
        assert!(
            entry_ids.contains(appended),
            "trace names an entry that is not in the log: {appended}"
        );
    }
    // And the transition's revision — which really was appended — is restored
    // rather than cleared: a trace still saying the transition applied must
    // name the revision it produced, since that is the field the audit chain
    // follows to the replay.
    assert!(
        revisions.contains(&trace["result_revision_id"]),
        "result_revision_id {} is not a revision in the log",
        trace["result_revision_id"]
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_detects_a_rewritten_anchored_trace() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "trace-tamper",
        "printf 'trace-anchor-output\\n'",
    );
    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    let trace_id = first_json["result"]["trace"]["trace_id"]
        .as_str()
        .expect("trace id")
        .to_owned();
    let result_revision_id = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("trace result revision")
        .to_owned();
    let trace_path = only_run_file(&directory, "execution.trace.json");
    let mut trace = json_file(trace_path.clone());
    trace["operation_gate"]["actor_id"] = json!("actor:rewritten-history");
    fs::write(
        &trace_path,
        serde_json::to_string_pretty(&trace).expect("serialize rewritten trace"),
    )
    .expect("rewrite trace");

    let second = run_native_step_with_base(&directory, &fixture, &result_revision_id, true, None);

    assert!(!second.status.success());
    let error = stderr(&second);
    assert!(error.contains(&trace_id), "{error}");
    assert!(error.contains("morphism-log content hash"), "{error}");
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_detects_a_rewritten_anchored_worker_report() {
    assert_worker_artifact_tamper_detected("worker.report.json", "worker report", |path| {
        let mut report = json_file(path.to_owned());
        report["timed_out"] = json!(true);
        fs::write(
            path,
            serde_json::to_vec_pretty(&report).expect("serialize rewritten worker report"),
        )
        .expect("rewrite worker report");
    });
}

#[test]
fn native_run_step_detects_rewritten_anchored_stderr() {
    assert_worker_artifact_tamper_detected("stderr", "stderr stream", |path| {
        fs::write(path, b"rewritten stderr\n").expect("rewrite stderr");
    });
}

#[test]
fn native_run_step_detects_rewritten_anchored_stdout() {
    assert_worker_artifact_tamper_detected("stdout", "stdout stream", |path| {
        fs::write(path, b"rewritten stdout\n").expect("rewrite stdout");
    });
}

#[test]
fn native_run_step_anchors_and_verifies_a_full_stream_beyond_the_retention_cap() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let chunk = "x".repeat(4096);
    let script =
        format!("i=0; while [ \"$i\" -lt 1025 ]; do printf '%s' '{chunk}'; i=$((i + 1)); done");
    let fixture = setup_native_run(&directory, "truncated-anchor", &script);

    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    let trace_path = only_run_file(&directory, "execution.trace.json");
    let run_directory = trace_path.parent().expect("run directory").to_owned();
    let stdout_path = run_directory.join("stdout");
    let trace = json_file(trace_path);
    let report = json_file(run_directory.join("worker.report.json"));
    let stdout_report = report["outputs"]
        .as_array()
        .expect("worker outputs")
        .iter()
        .find(|output| output["name"] == json!("stdout"))
        .expect("stdout report");
    let full_byte_len = 4096_u64 * 1025;

    assert_eq!(stdout_report["byte_len"], json!(full_byte_len));
    assert_eq!(
        stdout_report["retained_byte_len"],
        json!(4_u64 * 1024 * 1024)
    );
    assert_eq!(stdout_report["truncated"], json!(true));
    assert_eq!(
        fs::metadata(&stdout_path).expect("stdout metadata").len(),
        full_byte_len
    );
    assert_eq!(
        trace["stdout_content_hash"],
        json!(sha256_file(&stdout_path))
    );
    assert_eq!(
        trace["stdout_content_hash"], stdout_report["content_hash"],
        "the trace and report must carry the same tool-computed full-stream hash"
    );

    let result_revision_id = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("trace result revision");
    let verified = run_native_step_with_base(&directory, &fixture, result_revision_id, true, None);
    assert!(verified.status.success(), "stderr: {}", stderr(&verified));
    assert_eq!(
        stdout_json(&verified)["result"]["status"],
        json!("no_dispatchable_step")
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_publishes_tool_capture_over_a_worker_replaced_stdout_path() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "worker-replaced-stdout",
        concat!(
            "printf 'captured stdout'; ",
            "rm -f \"$CASEGRAPHEN_RUN_DIR/stdout\"; ",
            "printf 'worker path replacement' > \"$CASEGRAPHEN_RUN_DIR/stdout\""
        ),
    );

    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    let stdout_path = only_run_file(&directory, "stdout");
    assert_eq!(
        fs::read(&stdout_path).expect("read published stdout"),
        b"captured stdout"
    );
    assert_eq!(
        first_json["result"]["trace"]["stdout_content_hash"],
        json!(sha256_file(&stdout_path))
    );

    let result_revision_id = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("trace result revision");
    let verified = run_native_step_with_base(&directory, &fixture, result_revision_id, true, None);
    assert!(verified.status.success(), "stderr: {}", stderr(&verified));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_does_not_follow_a_worker_report_symlink_to_stdout() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let ln = if Path::new("/bin/ln").is_file() {
        "/bin/ln"
    } else {
        "/usr/bin/ln"
    };
    let script = format!(
        concat!(
            "printf 'captured stdout'; ",
            "rm -f \"$CASEGRAPHEN_RUN_DIR/worker.report.json\"; ",
            "{ln} -s stdout \"$CASEGRAPHEN_RUN_DIR/worker.report.json\""
        ),
        ln = ln
    );
    let fixture = setup_native_run(&directory, "worker-report-symlink", &script);

    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    let trace_path = only_run_file(&directory, "execution.trace.json");
    let run_directory = trace_path.parent().expect("run directory");
    let stdout_path = run_directory.join("stdout");
    let report_path = run_directory.join("worker.report.json");
    assert_eq!(
        fs::read(&stdout_path).expect("read published stdout"),
        b"captured stdout"
    );
    assert!(!fs::symlink_metadata(&report_path)
        .expect("worker report metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        first_json["result"]["trace"]["worker_report_content_hash"],
        json!(sha256_file(&report_path))
    );
    assert_eq!(
        first_json["result"]["trace"]["stdout_content_hash"],
        json!(sha256_file(&stdout_path))
    );

    let result_revision_id = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("trace result revision");
    let verified = run_native_step_with_base(&directory, &fixture, result_revision_id, true, None);
    assert!(verified.status.success(), "stderr: {}", stderr(&verified));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_does_not_anchor_a_post_worker_artifact_publication_failure() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "worker-report-publication-failure",
        concat!(
            "printf 'captured before report failure'; ",
            "rm -f \"$CASEGRAPHEN_RUN_DIR/worker.report.json\"; ",
            "mkdir \"$CASEGRAPHEN_RUN_DIR/worker.report.json\""
        ),
    );

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("worker.report.json"));
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert!(!replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .any(|entry| {
            entry["morphism"]["morphism_type"] == json!("custom:execution_trace_anchor")
        }));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_rejects_a_retargeted_command_symlink() {
    use std::os::unix::fs::symlink;

    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let command_link = directory.join("reviewed-command");
    symlink("/bin/sh", &command_link).expect("create reviewed command symlink");
    let fixture = setup_native_run_with_allowed_lifecycle_and_command(
        &directory,
        "retargeted-command",
        "printf 'must-not-run\\n'",
        "resolved",
        &command_link,
    );
    fs::remove_file(&command_link).expect("remove reviewed command symlink");
    let replacement = if Path::new("/bin/false").exists() {
        Path::new("/bin/false")
    } else {
        Path::new("/usr/bin/false")
    };
    symlink(replacement, &command_link).expect("retarget command symlink");

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("binding_identity_mismatch")
    );
    assert_eq!(
        fs::read(
            only_run_file(&directory, "execution.trace.json")
                .parent()
                .expect("run directory")
                .join("stdout")
        )
        .expect("read anchored empty stdout"),
        b""
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_spawn_error_still_leaves_an_anchored_failure_trace() {
    use std::os::unix::fs::PermissionsExt;

    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let command = directory.join("non-executable-worker");
    fs::write(&command, b"#!/bin/sh\nprintf 'must-not-run\\n'\n").expect("write worker file");
    let mut permissions = fs::metadata(&command)
        .expect("worker metadata")
        .permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&command, permissions).expect("make worker non-executable");
    let fixture = setup_native_run_with_allowed_lifecycle_and_command(
        &directory,
        "spawn-error",
        "",
        "resolved",
        &command,
    );

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(!output.status.success());
    let trace_path = only_run_file(&directory, "execution.trace.json");
    let trace = json_file(trace_path.clone());
    assert_eq!(trace["dispatch_state"], json!("failed"));
    assert_eq!(
        trace["obstructions"][0]["obstruction_type"],
        json!("dispatch_failed")
    );
    assert_eq!(trace["metadata"]["worker_invoked"], json!(false));
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/execution.trace.schema.json"),
        &trace_path,
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert!(replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .any(|entry| {
            entry["morphism"]["metadata"]["trace_id"] == trace["trace_id"]
                && entry["morphism"]["metadata"]["trace_content_hash"]
                    .as_str()
                    .is_some_and(|hash| hash.len() == 64)
        }));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_rejects_fabricated_and_incomplete_dispatch_capabilities() {
    let fabricated_directory = unique_temp_dir();
    fs::create_dir_all(&fabricated_directory).expect("create fabricated temp directory");
    let fabricated_fixture = setup_native_run(
        &fabricated_directory,
        "fabricated-gate",
        "printf 'must-not-run\\n'",
    );
    let fabricated = run_native_step_with_gate_capabilities(
        &fabricated_directory,
        &fabricated_fixture,
        &fabricated_fixture.accepted_revision_id,
        true,
        None,
        &["capability:fabricated", "capability:native-run-worker"],
    );
    assert!(
        fabricated.status.success(),
        "stderr: {}",
        stderr(&fabricated)
    );
    let fabricated_json = stdout_json(&fabricated);
    assert_eq!(
        fabricated_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert_eq!(
        fabricated_json["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("operation_gate_rejected")
    );
    assert!(
        fabricated_json["result"]["trace"]["obstructions"][0]["summary"]
            .as_str()
            .expect("gate summary")
            .contains("existing case cell")
    );
    assert!(only_run_file(&fabricated_directory, "execution.trace.json").is_file());
    fs::remove_dir_all(fabricated_directory).expect("remove fabricated temp directory");

    let incomplete_directory = unique_temp_dir();
    fs::create_dir_all(&incomplete_directory).expect("create incomplete temp directory");
    let incomplete_fixture = setup_native_run(
        &incomplete_directory,
        "incomplete-gate",
        "printf 'must-not-run\\n'",
    );
    let incomplete = run_native_step_with_gate_capabilities(
        &incomplete_directory,
        &incomplete_fixture,
        &incomplete_fixture.accepted_revision_id,
        true,
        None,
        &["capability:dispatch"],
    );
    assert!(
        incomplete.status.success(),
        "stderr: {}",
        stderr(&incomplete)
    );
    let incomplete_json = stdout_json(&incomplete);
    assert_eq!(
        incomplete_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert_eq!(
        incomplete_json["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("operation_gate_rejected")
    );
    assert_eq!(
        incomplete_json["result"]["trace"]["obstructions"][0]["witness_ids"],
        json!(["capability:native-run-worker"])
    );
    assert_eq!(
        incomplete_json["result"]["trace"]["operation_gate"]["capability_ids"],
        json!(["capability:dispatch"])
    );
    assert_eq!(
        replayed_work_lifecycle(&stdout_json(&run_native_case_store_command(
            &incomplete_directory,
            "replay"
        ))),
        "active"
    );
    fs::remove_dir_all(incomplete_directory).expect("remove incomplete temp directory");
}

#[test]
fn native_run_step_records_failed_worker_evidence_without_transition() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "failure",
        "printf 'failed-output'; printf 'failed-error' >&2; exit 1",
    );

    let mut strict_args = native_step_args(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        true,
        None,
        &["capability:dispatch", "capability:native-run-worker"],
    );
    strict_args.push("--strict".to_owned());
    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(strict_args)
        .output()
        .expect("run strict casegraphen run --step");

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("step_failed"));
    assert_eq!(value["result"]["trace"]["transition_applied"], json!(false));
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("worker_execution_failed")
    );
    assert_eq!(
        value["result"]["appended_entry_ids"]
            .as_array()
            .expect("appended entries")
            .len(),
        2
    );
    let run_directory = only_run_file(&directory, "execution.trace.json")
        .parent()
        .expect("run directory")
        .to_path_buf();
    assert_eq!(
        fs::read(run_directory.join("stdout")).expect("read stdout"),
        b"failed-output"
    );
    assert_eq!(
        fs::read(run_directory.join("stderr")).expect("read stderr"),
        b"failed-error"
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert_eq!(replayed_work_lifecycle(&replay), "active");
    assert!(replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .any(|cell| {
            cell["cell_type"] == json!("evidence")
                && cell["metadata"]["exit_status"] == json!(1)
                && cell["metadata"]["worker_report_id"].is_string()
        }));
    assert!(!replay["result"]["replay"]["case_space"]["case_relations"]
        .as_array()
        .expect("replayed relations")
        .iter()
        .any(|relation| {
            relation["relation_type"] == json!("satisfies_evidence_requirement")
                && relation["from_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("evidence:worker-output:"))
        }));
    let failed_evidence_entry = replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .find(|entry| {
            entry["morphism"]["morphism_type"] == json!("evidence_attach")
                && entry["morphism"]["metadata"]["trace_id"] == value["result"]["trace"]["trace_id"]
        })
        .expect("failed worker evidence morphism");
    assert_eq!(
        failed_evidence_entry["morphism"]["review_status"],
        json!("unreviewed")
    );

    let failed_revision = value["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("failed run result revision");
    let without_retry =
        run_native_step_with_base(&directory, &fixture, failed_revision, true, None);
    assert!(
        without_retry.status.success(),
        "stderr: {}",
        stderr(&without_retry)
    );
    let without_retry_json = stdout_json(&without_retry);
    assert_eq!(
        without_retry_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert_eq!(
        without_retry_json["result"]["obstructions"][0]["obstruction_type"],
        json!("retry_required")
    );
    let mut strict_retry_args = native_step_args(
        &directory,
        &fixture,
        failed_revision,
        true,
        None,
        &["capability:dispatch", "capability:native-run-worker"],
    );
    strict_retry_args.push("--strict".to_owned());
    let strict_retry = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(strict_retry_args)
        .output()
        .expect("run strict retry-required casegraphen run --step");
    assert_eq!(strict_retry.status.code(), Some(2));
    assert_eq!(strict_retry.stdout, without_retry.stdout);

    let retried = run_native_step_with_base(
        &directory,
        &fixture,
        failed_revision,
        true,
        Some(&fixture.step_id),
    );
    assert!(retried.status.success(), "stderr: {}", stderr(&retried));
    assert_eq!(
        stdout_json(&retried)["result"]["status"],
        json!("step_failed")
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_preserves_unauthorized_transition_as_unreviewed_proposal() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run_with_allowed_lifecycle(
        &directory,
        "unauthorized",
        "printf 'successful-but-not-authorized\\n'",
        "accepted",
    );

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["result"]["status"],
        json!("transition_not_authorized")
    );
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("transition_not_authorized")
    );
    assert_eq!(value["result"]["trace"]["transition_applied"], json!(false));
    let proposed = json_file(
        only_run_file(&directory, "execution.trace.json")
            .parent()
            .expect("run directory")
            .join("proposed.morphism.json"),
    );
    assert_eq!(proposed["review_status"], json!("unreviewed"));
    assert_eq!(
        proposed["metadata"]["authorization_source"],
        json!("accepted_execution_plan")
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert_eq!(replayed_work_lifecycle(&replay), "active");
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_blocks_successful_worker_when_success_evidence_is_unsatisfied() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let initial = setup_native_run(
        &directory,
        "unsatisfied-base",
        "printf 'diagnostic-only\\n'",
    );
    let binding_id = json_file(initial.binding_path.clone())["binding_id"]
        .as_str()
        .expect("binding id")
        .to_owned();
    let plan_id = "plan:run-unsatisfied-success";
    let plan_input = directory.join("unsatisfied-success.execution.plan.input.json");
    write_execution_plan_for_binding(
        &plan_input,
        plan_id,
        &initial.accepted_revision_id,
        "work:review-native-contract",
        &binding_id,
    );
    let mut plan = json_file(plan_input.clone());
    plan["steps"][0]["success_evidence_requirement_ids"] =
        json!(["review:native-contract-acceptance"]);
    fs::write(
        &plan_input,
        serde_json::to_string_pretty(&plan).expect("serialize unsatisfied plan"),
    )
    .expect("write unsatisfied plan");
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        plan_id,
        "--reviewer-id",
        "reviewer:unsatisfied-plan",
        "--reason",
        "Accept plan to exercise runtime success authorization",
        "--base-revision-id",
        &initial.accepted_revision_id,
        "--actor-id",
        "actor:unsatisfied-plan",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let fixture = NativeRunFixture {
        plan_id: plan_id.to_owned(),
        step_id: format!("step:{plan_id}"),
        accepted_revision_id: stdout_json(&accept)["result"]["record"]["current_revision_id"]
            .as_str()
            .expect("accepted revision")
            .to_owned(),
        binding_path: initial.binding_path,
    };

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["result"]["status"],
        json!("transition_not_authorized")
    );
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["obstruction_type"],
        json!("success_conditions_unsatisfied")
    );
    assert_eq!(
        value["result"]["trace"]["obstructions"][0]["blocking"],
        json!(true)
    );
    assert_eq!(
        value["result"]["trace"]["unsatisfied_success_evidence_requirement_ids"],
        json!(["review:native-contract-acceptance"])
    );
    assert!(!value["result"]["trace"]["transition_applied"]
        .as_bool()
        .expect("transition applied"));
    let proposed = json_file(
        only_run_file(&directory, "execution.trace.json")
            .parent()
            .expect("run directory")
            .join("proposed.morphism.json"),
    );
    assert_eq!(proposed["review_status"], json!("unreviewed"));
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert_eq!(replayed_work_lifecycle(&replay), "active");
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_executes_independent_steps_and_appends_in_plan_order() {
    // Issue #32: this used to bet that the "first" work cell's worker,
    // delayed by a fixed 1 s sleep, would still lose the race to "second"'s
    // near-instant worker — true on a quiet machine, not guaranteed on a
    // loaded one. "first" now waits on a marker "second" creates as its own
    // final act, so "second" finishing first is a fact this test drives
    // directly rather than a race it merely tends to win.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let completion_order = directory.join("worker-completion-order");
    let second_done = directory.join("worker-second-done");
    let first_script = format!(
        "{}\nprintf 'first\\n' >> '{}'\nprintf 'first-output\\n'",
        shell_wait_for_marker(&second_done),
        completion_order.display()
    );
    let second_script = format!(
        "printf 'second\\n' >> '{}'\nprintf 'second-output\\n'\ntouch '{}'",
        completion_order.display(),
        second_done.display()
    );
    let fixture = setup_native_frontier(
        &directory,
        "independent",
        &[
            ("work:frontier-first", first_script.as_str()),
            ("work:frontier-second", second_script.as_str()),
        ],
    );

    let output = run_native_frontier(&directory, &fixture, 2);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("round_executed"));
    let traces = value["result"]["traces"]
        .as_array()
        .expect("frontier traces");
    assert_eq!(traces.len(), 2);
    assert_eq!(traces[0]["step_id"], json!(fixture.step_ids[0]));
    assert_eq!(traces[1]["step_id"], json!(fixture.step_ids[1]));
    assert_eq!(traces[0]["transition_applied"], json!(true));
    assert_eq!(traces[1]["transition_applied"], json!(true));
    assert_eq!(
        fs::read_to_string(&completion_order).expect("worker completion order"),
        "second\nfirst\n",
        "the second worker must finish first to exercise serial plan-order application"
    );
    let appended_entry_ids = value["result"]["appended_entry_ids"]
        .as_array()
        .expect("frontier appended entries");
    assert_eq!(appended_entry_ids.len(), 6);
    assert_eq!(
        value["result"]["result_revision_id"],
        traces[1]["result_revision_id"]
    );

    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let cells = replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells");
    for work_cell_id in &fixture.work_cell_ids {
        assert_eq!(
            cells
                .iter()
                .find(|cell| cell["id"] == json!(work_cell_id))
                .expect("frontier work cell")["lifecycle"],
            json!("resolved")
        );
    }
    let log = replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log");
    let appended_from_log = log
        .iter()
        .filter(|entry| appended_entry_ids.contains(&entry["entry_id"]))
        .map(|entry| entry["entry_id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(appended_from_log, appended_entry_ids.clone());
    let applied_step_order = log
        .iter()
        .filter_map(|entry| {
            entry["morphism"]["metadata"]["step_id"]
                .as_str()
                .map(str::to_owned)
        })
        .filter(|step_id| fixture.step_ids.contains(step_id))
        .collect::<Vec<_>>();
    assert_eq!(
        applied_step_order,
        vec![
            fixture.step_ids[0].clone(),
            fixture.step_ids[0].clone(),
            fixture.step_ids[1].clone(),
            fixture.step_ids[1].clone(),
        ]
    );
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_continues_after_one_worker_fails() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "partial-failure",
        &[
            (
                "work:frontier-failing",
                "printf 'failed-output\\n'\nprintf 'failed-error\\n' >&2\nexit 1",
            ),
            ("work:frontier-succeeding", "printf 'successful-output\\n'"),
        ],
    );

    let mut strict_args = native_frontier_args(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        2,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
        true,
    );
    strict_args.push("--strict".to_owned());
    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(strict_args)
        .output()
        .expect("run strict casegraphen run --frontier");

    assert_eq!(output.status.code(), Some(2), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("round_executed"));
    let traces = value["result"]["traces"]
        .as_array()
        .expect("frontier traces");
    assert_eq!(traces.len(), 2);
    assert_eq!(traces[0]["step_id"], json!(fixture.step_ids[0]));
    assert_eq!(traces[0]["transition_applied"], json!(false));
    assert_eq!(
        traces[0]["obstructions"][0]["obstruction_type"],
        json!("worker_execution_failed")
    );
    assert_eq!(traces[1]["step_id"], json!(fixture.step_ids[1]));
    assert_eq!(traces[1]["transition_applied"], json!(true));
    assert_eq!(
        value["result"]["appended_entry_ids"]
            .as_array()
            .expect("frontier appended entries")
            .len(),
        5
    );

    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let cells = replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells");
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell["id"] == json!(fixture.work_cell_ids[0]))
            .expect("failing work cell")["lifecycle"],
        json!("active")
    );
    assert_eq!(
        cells
            .iter()
            .find(|cell| cell["id"] == json!(fixture.work_cell_ids[1]))
            .expect("succeeding work cell")["lifecycle"],
        json!("resolved")
    );
    assert!(cells.iter().any(|cell| {
        cell["cell_type"] == json!("evidence")
            && cell["metadata"]["exit_status"] == json!(1)
            && cell["metadata"]["trace_id"] == traces[0]["trace_id"]
    }));
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_selects_only_one_step_per_work_cell() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "shared-cell",
        &[
            ("work:frontier-shared", "printf 'first-output\\n'"),
            ("work:frontier-shared", "printf 'must-not-run\\n'"),
        ],
    );

    let output = run_native_frontier(&directory, &fixture, 2);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("round_executed"));
    assert_eq!(
        value["result"]["traces"]
            .as_array()
            .expect("frontier traces")
            .len(),
        1
    );
    assert_eq!(
        value["result"]["traces"][0]["step_id"],
        json!(fixture.step_ids[0])
    );
    assert_eq!(value["result"]["step_reasons"][1]["eligible"], json!(false));
    assert_eq!(
        value["result"]["step_reasons"][1]["reasons"],
        json!(["work_cell_already_selected_this_round"])
    );
    assert_eq!(
        fs::read_dir(directory.join("runs"))
            .expect("read frontier run directories")
            .count(),
        1
    );
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_selects_a_later_same_cell_step_when_the_first_is_ineligible() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "shared-cell-first-ineligible",
        &[
            ("work:frontier-shared-fallback", "printf 'must-not-run\\n'"),
            ("work:frontier-shared-fallback", "printf 'fallback-ran\\n'"),
        ],
    );
    let first_binding = directory
        .join("bindings")
        .join("worker_binding~3afrontier-shared-cell-first-ineligible-1.worker.binding.json");
    let mut tampered = json_file(first_binding.clone());
    tampered["metadata"]["tampered_after_plan_acceptance"] = json!(true);
    fs::write(
        &first_binding,
        serde_json::to_string_pretty(&tampered).expect("serialize tampered first binding"),
    )
    .expect("tamper first binding");

    let output = run_native_frontier(&directory, &fixture, 2);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("round_executed"));
    assert_eq!(
        value["result"]["traces"]
            .as_array()
            .expect("frontier traces")
            .len(),
        2
    );
    assert_eq!(
        value["result"]["traces"][0]["step_id"],
        json!(fixture.step_ids[0])
    );
    assert_eq!(
        value["result"]["traces"][0]["dispatch_state"],
        json!("failed")
    );
    assert_eq!(
        value["result"]["traces"][0]["obstructions"][0]["obstruction_type"],
        json!("binding_hash_mismatch")
    );
    assert_eq!(
        value["result"]["traces"][1]["step_id"],
        json!(fixture.step_ids[1])
    );
    assert_eq!(
        value["result"]["step_reasons"][0]["reasons"],
        json!(["binding_hash_mismatch"])
    );
    assert_eq!(value["result"]["step_reasons"][1]["eligible"], json!(true));
    assert_eq!(value["result"]["step_reasons"][1]["reasons"], json!([]));
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_treats_uncovered_binding_capabilities_as_ineligible() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "uncovered-capability",
        &[("work:frontier-uncovered", "printf 'must-not-run\\n'")],
    );

    let output = run_native_frontier_with(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        4,
        &["capability:dispatch"],
        &[],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    assert_eq!(
        value["result"]["traces"][0]["dispatch_state"],
        json!("failed")
    );
    assert_eq!(
        value["result"]["traces"][0]["obstructions"][0]["obstruction_type"],
        json!("operation_gate_rejected")
    );
    assert_eq!(
        value["result"]["appended_entry_ids"]
            .as_array()
            .expect("trace anchor entry")
            .len(),
        1
    );
    assert_eq!(
        value["result"]["step_reasons"][0]["reasons"],
        json!(["operation_gate_rejected"])
    );
    assert!(only_run_file(&directory, "execution.trace.json").is_file());
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let trace_id = value["result"]["traces"][0]["trace_id"]
        .as_str()
        .expect("rejected trace id");
    assert!(replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .any(|entry| {
            entry["morphism"]["morphism_type"] == json!("custom:execution_trace_anchor")
                && entry["morphism"]["metadata"]["trace_id"] == json!(trace_id)
        }));
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_tampered_binding_writes_anchored_failed_trace_and_burns_attempt() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "tampered-binding-audit",
        &[("work:frontier-tampered-binding", "printf 'must-not-run\n'")],
    );
    let binding_path = directory
        .join("bindings")
        .join("worker_binding~3afrontier-tampered-binding-audit-1.worker.binding.json");
    let original_binding = fs::read(&binding_path).expect("read original binding");
    let mut tampered = json_file(binding_path.clone());
    tampered["metadata"]["tampered_after_plan_acceptance"] = json!(true);
    fs::write(
        &binding_path,
        serde_json::to_string_pretty(&tampered).expect("serialize tampered binding"),
    )
    .expect("tamper binding");

    let refused = run_native_frontier(&directory, &fixture, 1);

    assert!(refused.status.success(), "stderr: {}", stderr(&refused));
    let refused_json = stdout_json(&refused);
    assert_eq!(
        refused_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    let trace = &refused_json["result"]["traces"][0];
    assert_eq!(trace["step_id"], json!(fixture.step_ids[0]));
    assert_eq!(trace["dispatch_state"], json!("failed"));
    assert_eq!(trace["transition_applied"], json!(false));
    assert_eq!(
        trace["obstructions"][0]["obstruction_type"],
        json!("binding_hash_mismatch")
    );
    let trace_id = trace["trace_id"].as_str().expect("failed trace id");
    let trace_path = only_run_file(&directory, "execution.trace.json");
    assert_eq!(json_file(trace_path)["trace_id"], json!(trace_id));
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    assert!(replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .any(|entry| {
            entry["morphism"]["morphism_type"] == json!("custom:execution_trace_anchor")
                && entry["morphism"]["metadata"]["trace_id"] == json!(trace_id)
        }));

    fs::write(&binding_path, original_binding).expect("restore accepted binding");
    let current_revision = refused_json["result"]["result_revision_id"]
        .as_str()
        .expect("trace anchor revision");
    let without_retry = run_native_frontier_with(
        &directory,
        &fixture,
        current_revision,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
    );
    assert!(
        without_retry.status.success(),
        "stderr: {}",
        stderr(&without_retry)
    );
    let without_retry_json = stdout_json(&without_retry);
    assert_eq!(
        without_retry_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert_eq!(without_retry_json["result"]["traces"], json!([]));
    assert_eq!(
        without_retry_json["result"]["step_reasons"][0]["reasons"],
        json!(["prior_failed_trace_requires_retry"])
    );
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_reports_each_disabled_worker_without_aborting_the_round() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "disabled-worker-round",
        &[
            ("work:frontier-disabled-1", "printf 'one\n'"),
            ("work:frontier-disabled-2", "printf 'two\n'"),
            ("work:frontier-disabled-3", "printf 'three\n'"),
            ("work:frontier-disabled-4", "printf 'four\n'"),
        ],
    );

    let output = run_native_frontier_without_worker_opt_in(&directory, &fixture, 4);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    let traces = value["result"]["traces"]
        .as_array()
        .expect("per-step failed traces");
    assert_eq!(traces.len(), 4);
    assert!(traces.iter().all(|trace| {
        trace["dispatch_state"] == json!("failed")
            && trace["transition_applied"] == json!(false)
            && trace["obstructions"][0]["obstruction_type"] == json!("dispatch_failed")
            && trace["obstructions"][0]["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("--enable-worker shell"))
    }));
    assert_eq!(
        value["result"]["appended_entry_ids"]
            .as_array()
            .expect("one trace anchor per failed step")
            .len(),
        4
    );
    for (index, reason) in value["result"]["step_reasons"]
        .as_array()
        .expect("step reasons")
        .iter()
        .enumerate()
    {
        assert_eq!(reason["step_id"], json!(fixture.step_ids[index]));
        assert_eq!(reason["reasons"], json!(["dispatch_failed"]));
    }
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_refuses_transition_when_cell_leaves_frontier_during_round() {
    // Issue #32: this used to bet that a whole external `morphism apply`
    // invocation finished inside a fixed 0.5 s worker sleep — a wall-clock
    // race a loaded machine can lose. The worker now waits on a marker the
    // test creates only after the intervening apply has actually landed.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("frontier-membership-worker-started");
    let proceed = directory.join("frontier-membership-worker-proceed");
    let script = format!(
        "printf 'started\\n' > '{}'\n{}\nprintf 'worker-output\\n'",
        worker_started.display(),
        shell_wait_for_marker(&proceed)
    );
    let fixture = setup_native_frontier(
        &directory,
        "membership-recheck",
        &[("work:frontier-membership-recheck", script.as_str())],
    );
    let morphism_path = directory.join("frontier-membership-intervening.case_morphism.json");
    let morphism = json!({
        "morphism_id": "morphism:frontier-membership-intervening",
        "morphism_type": "create",
        "source_revision_id": fixture.accepted_revision_id,
        "target_revision_id": "revision:frontier-membership-intervening",
        "added_ids": [
            "evidence:frontier-membership-required",
            "relation:frontier-membership-requires-evidence"
        ],
        "updated_ids": [],
        "retired_ids": [],
        "preserved_ids": ["work:frontier-membership-recheck"],
        "violated_invariant_ids": [],
        "review_status": "unreviewed",
        "evidence_ids": [],
        "source_ids": ["source:frontier-membership-test"],
        "metadata": {
            "payload": {
                "added_cells": [{
                    "id": "evidence:frontier-membership-required",
                    "cell_type": "evidence",
                    "space_id": "space:higher-graphen-casegraphen",
                    "title": "Evidence required after dispatch selection",
                    "lifecycle": "proposed",
                    "source_ids": ["source:frontier-membership-test"],
                    "structure_ids": [],
                    "provenance": {
                        "source": {
                            "kind": "human",
                            "title": "Frontier membership concurrency regression"
                        },
                        "confidence": 1.0,
                        "review_status": "unreviewed"
                    },
                    "metadata": {"evidence_boundary": "inferred"}
                }],
                "added_relations": [{
                    "id": "relation:frontier-membership-requires-evidence",
                    "relation_type": "requires_evidence",
                    "relation_strength": "hard",
                    "from_id": "work:frontier-membership-recheck",
                    "to_id": "evidence:frontier-membership-required",
                    "evidence_ids": [],
                    "source_ids": ["source:frontier-membership-test"],
                    "provenance": {
                        "source": {
                            "kind": "human",
                            "title": "Frontier membership concurrency regression"
                        },
                        "confidence": 1.0,
                        "review_status": "unreviewed"
                    },
                    "metadata": {}
                }]
            }
        }
    });
    fs::write(
        &morphism_path,
        serde_json::to_string_pretty(&morphism).expect("serialize intervening morphism"),
    )
    .expect("write intervening morphism");
    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));

    let child = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            1,
            &["capability:dispatch", "capability:native-run-worker"],
            &[],
            true,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn casegraphen run --frontier");
    wait_for_file(
        &worker_started,
        "frontier worker did not start before timeout",
    );

    let apply = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            directory.to_str().expect("store path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:frontier-membership-intervening",
            "--base-revision-id",
            &fixture.accepted_revision_id,
            "--reviewer-id",
            "reviewer:frontier-membership",
            "--reason",
            "Add a blocking evidence requirement during frontier dispatch",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    signal_rendezvous_marker(&proceed);

    let output = child.wait_with_output().expect("wait for frontier round");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("round_executed"));
    assert_eq!(
        value["result"]["traces"][0]["transition_applied"],
        json!(false)
    );
    assert!(value["result"]["traces"][0]["obstructions"]
        .as_array()
        .expect("trace obstructions")
        .iter()
        .any(|obstruction| {
            obstruction["obstruction_type"] == json!("work_cell_not_on_frontier")
        }));
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let work_cell = replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:frontier-membership-recheck"))
        .expect("frontier membership work cell");
    assert_eq!(work_cell["lifecycle"], json!("active"));
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_traces_record_each_application_base_revision() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "trace-application-bases",
        &[
            ("work:frontier-base-1", "printf 'one\n'"),
            ("work:frontier-base-2", "printf 'two\n'"),
            ("work:frontier-base-3", "printf 'three\n'"),
        ],
    );

    let output = run_native_frontier(&directory, &fixture, 3);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    let traces = value["result"]["traces"]
        .as_array()
        .expect("frontier traces");
    assert_eq!(traces.len(), 3);
    assert_eq!(
        traces[0]["base_revision_id"],
        json!(fixture.accepted_revision_id)
    );
    for index in 1..traces.len() {
        assert_eq!(
            traces[index]["base_revision_id"],
            traces[index - 1]["result_revision_id"],
            "each trace must name the revision present when its evidence morphism was computed"
        );
    }
    let mut base_revision_ids = traces
        .iter()
        .map(|trace| {
            trace["base_revision_id"]
                .as_str()
                .expect("trace base revision")
                .to_owned()
        })
        .collect::<Vec<_>>();
    base_revision_ids.sort();
    base_revision_ids.dedup();
    assert_eq!(base_revision_ids.len(), traces.len());
    for trace in traces {
        assert_eq!(
            trace["information_loss"][0]["represented_ids"],
            json!([trace["base_revision_id"].clone()])
        );
    }
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_supersedes_a_killed_dispatcher_trace() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("stale-started-worker-started");
    let worker_finished = directory.join("stale-started-worker-finished");
    let script = format!(
        "printf 'started\\n' > '{}'\nsleep 1\nprintf 'finished\\n' > '{}'\nprintf 'worker-output\\n'",
        worker_started.display(),
        worker_finished.display()
    );
    let fixture = setup_native_frontier(
        &directory,
        "stale-started-retry",
        &[("work:frontier-stale-started", script.as_str())],
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            1,
            &["capability:dispatch", "capability:native-run-worker"],
            &[],
            true,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn casegraphen run --frontier");
    wait_for_file(
        &worker_started,
        "frontier worker did not start before timeout",
    );
    child.kill().expect("kill frontier round");
    let killed = child.wait_with_output().expect("wait for killed round");
    assert!(!killed.status.success());
    wait_for_file(
        &worker_finished,
        "killed dispatcher's worker did not finish before supersession",
    );
    let started_trace_path = only_run_file(&directory, "execution.trace.json");
    let started_trace = json_file(started_trace_path);
    let started_trace_id = started_trace["trace_id"]
        .as_str()
        .expect("started trace id")
        .to_owned();
    assert_eq!(started_trace["dispatch_state"], json!("started"));
    assert_eq!(
        started_trace["metadata"]["reserved_base_revision_id"],
        json!(fixture.accepted_revision_id)
    );

    let intervening = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &fixture.accepted_revision_id,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "resolved",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(
        intervening.status.success(),
        "stderr: {}",
        stderr(&intervening)
    );
    let intervening_revision = stdout_json(&intervening)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("intervening revision")
        .to_owned();

    let recovered = run_native_frontier_with_superseded_traces(
        &directory,
        &fixture,
        &intervening_revision,
        &[started_trace_id.as_str()],
    );

    assert!(recovered.status.success(), "stderr: {}", stderr(&recovered));
    let recovered_json = stdout_json(&recovered);
    assert_eq!(recovered_json["result"]["status"], json!("round_executed"));
    assert_eq!(
        recovered_json["result"]["traces"][0]["transition_applied"],
        json!(true)
    );
    assert_eq!(
        recovered_json["result"]["traces"][0]["metadata"]["superseded_trace_ids"],
        json!([started_trace_id])
    );
    assert_eq!(
        fs::read_dir(directory.join("runs"))
            .expect("read retry run directories")
            .count(),
        2
    );
    let replay = stdout_json(&run_native_case_store_command(&directory, "replay"));
    let recovered_trace_id = recovered_json["result"]["traces"][0]["trace_id"]
        .as_str()
        .expect("recovered trace id");
    let replayed_anchor = replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("replayed morphism log")
        .iter()
        .find(|entry| {
            entry["morphism"]["morphism_type"] == json!("custom:execution_trace_anchor")
                && entry["morphism"]["metadata"]["trace_id"] == json!(recovered_trace_id)
        })
        .expect("replayed superseding trace anchor");
    let replayed_trace_path = replayed_anchor["morphism"]["metadata"]["trace_path"]
        .as_str()
        .expect("replayed trace path");
    let replayed_trace = json_file(directory.join(replayed_trace_path));
    assert_eq!(
        replayed_trace["metadata"]["superseded_trace_ids"],
        json!([started_trace_id])
    );
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// `space history --format text` folds a superseded dispatch's log entries
/// per the three rules in `native_cli_text.rs::render_case_history`'s doc
/// comment. Reuses the kill-and-supersede setup above (ADR 0014's own
/// scenario) because that is the only real path that ever records
/// `metadata.superseded_trace_ids` — a plain `--retry-step` after a failure
/// does not.
#[cfg(unix)]
#[test]
fn space_history_text_folds_only_the_supersession_the_new_trace_actually_names() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("history-fold-worker-started");
    let worker_finished = directory.join("history-fold-worker-finished");
    let script = format!(
        "printf 'started\\n' > '{}'\nsleep 1\nprintf 'finished\\n' > '{}'\nprintf 'worker-output\\n'",
        worker_started.display(),
        worker_finished.display()
    );
    let fixture = setup_native_frontier(
        &directory,
        "history-fold",
        &[("work:frontier-history-fold", script.as_str())],
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            1,
            &["capability:dispatch", "capability:native-run-worker"],
            &[],
            true,
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn casegraphen run --frontier");
    wait_for_file(
        &worker_started,
        "frontier worker did not start before timeout",
    );
    child.kill().expect("kill frontier round");
    let killed = child.wait_with_output().expect("wait for killed round");
    assert!(!killed.status.success());
    wait_for_file(
        &worker_finished,
        "killed dispatcher's worker did not finish before supersession",
    );
    let started_trace_path = only_run_file(&directory, "execution.trace.json");
    let started_trace = json_file(started_trace_path);
    let started_trace_id = started_trace["trace_id"]
        .as_str()
        .expect("started trace id")
        .to_owned();

    let intervening = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &fixture.accepted_revision_id,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "resolved",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(
        intervening.status.success(),
        "stderr: {}",
        stderr(&intervening)
    );
    let intervening_revision = stdout_json(&intervening)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("intervening revision")
        .to_owned();

    let recovered = run_native_frontier_with_superseded_traces(
        &directory,
        &fixture,
        &intervening_revision,
        &[started_trace_id.as_str()],
    );
    assert!(recovered.status.success(), "stderr: {}", stderr(&recovered));
    let recovered_json = stdout_json(&recovered);
    let recovered_trace_id = recovered_json["result"]["traces"][0]["trace_id"]
        .as_str()
        .expect("recovered trace id")
        .to_owned();
    assert_native_store_valid_and_rebuilds(&directory);

    let log_path = find_morphism_log_path(&directory);
    let log_bytes_before = fs::read(&log_path).expect("read morphism log before rendering");

    // The started (killed) trace never reached `write_and_anchor_trace`, so
    // it has no morphism log entry at all — only the surviving trace does.
    // `space history --format json` answers for the log alone (ADR 0011)
    // and so names only the one id the log actually has.
    let history_json = run_native_case_store_command(&directory, "history");
    let json_trace_ids = stdout_json(&history_json)["result"]["entries"]
        .as_array()
        .expect("history entries")
        .iter()
        .filter_map(|entry| entry["morphism"]["metadata"]["trace_id"].as_str())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        json_trace_ids,
        std::collections::BTreeSet::from([recovered_trace_id.clone()])
    );

    let history_text = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "text",
    ]);
    assert!(
        history_text.status.success(),
        "stderr: {}",
        stderr(&history_text)
    );
    let text = stdout(&history_text);
    assert!(text.contains(&started_trace_id));
    assert!(text.contains(&recovered_trace_id));
    assert!(
        text.contains(&format!(
            "(2 attempts: {started_trace_id}, {recovered_trace_id})"
        )),
        "expected exactly the two named trace ids folded into one line: {text}"
    );
    // The fold is a projection, not a loss: every trace id the unfolded log
    // names is still visible in the text (here, trivially, since the log
    // names only one). The annotation additionally surfaces the superseded
    // id from the surviving trace's own file — information ADR 0014 already
    // records but the bare log never carried.
    let text_trace_ids = json_trace_ids
        .iter()
        .filter(|trace_id| text.contains(trace_id.as_str()))
        .count();
    assert_eq!(text_trace_ids, json_trace_ids.len());

    // Rendering is read-only: the log on disk is byte-identical, and the
    // store still validates.
    let log_bytes_after = fs::read(&log_path).expect("read morphism log after rendering");
    assert_eq!(log_bytes_before, log_bytes_after);
    assert_native_store_valid_and_rebuilds(&directory);

    // A stray file under `runs/` that no anchor in the log names — nothing
    // recorded is in doubt, so this degrades the fold rather than refusing
    // (rule 2), and the recovered entry's own line is unaffected.
    let stray_run_directory = directory.join("runs").join("stray-unanchored-trace");
    fs::create_dir_all(&stray_run_directory).expect("create stray run directory");
    fs::write(
        stray_run_directory.join("execution.trace.json"),
        "{not valid json",
    )
    .expect("write stray malformed trace file");
    let history_text_degraded = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "text",
    ]);
    assert!(
        history_text_degraded.status.success(),
        "a stray unanchored file must degrade the fold, not fail the command: stderr: {}",
        stderr(&history_text_degraded)
    );
    let degraded_text = stdout(&history_text_degraded);
    assert!(degraded_text.contains("Execution traces unreadable, rendering entries unfolded:"));
    assert!(!degraded_text.contains("attempts:"));
    fs::remove_dir_all(&stray_run_directory).expect("remove stray run directory");

    // Corrupting the *anchored* surviving trace's own file is a different
    // failure entirely: the log's own anchor recorded this file's content
    // hash, so a mismatch here is the log disagreeing with the file it
    // points at — CLAUDE.md's "integrity mismatches are tool failures". This
    // must refuse exactly like `run --frontier`/`operate` would, never
    // degrade into a rendering note (that would turn a tamper signal into a
    // quiet, exit-0 omission). Run directory names are
    // `path_helpers::path_segment`-escaped, not the raw trace id, so find
    // the file by content rather than assuming a path.
    let recovered_trace_path = run_files(&directory, "execution.trace.json")
        .into_iter()
        .find(|path| json_file(path.clone())["trace_id"] == json!(recovered_trace_id))
        .expect("recovered trace file");
    fs::write(&recovered_trace_path, "{not valid json")
        .expect("corrupt the anchored surviving trace file");
    let history_text_tampered = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "text",
    ]);
    assert!(
        !history_text_tampered.status.success(),
        "an anchored trace's content-hash mismatch must refuse, not render a degraded view"
    );
    // `--format text` renders a refusal as prose in the command's own
    // resolved format (`cli.rs::refusal_text`), not the JSON envelope.
    assert!(stderr(&history_text_tampered).contains("may have been rewritten"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Two independent steps in the same round each anchor their own trace and
/// neither trace names the other in `metadata.superseded_trace_ids` — the
/// fold must not collapse them just because their log entries sit next to
/// each other (rule 1: adjacency is not supersession).
#[cfg(unix)]
#[test]
fn space_history_text_does_not_fold_adjacent_but_unrelated_traces() {
    // Issue #32: neither worker's completion order matters to this test —
    // it only checks that both anchors render and are not folded together
    // — so the fixed `sleep 1` one of them used to carry was a wall-clock
    // assumption nothing here actually depended on. Removed rather than
    // converted.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "history-adjacent",
        &[
            (
                "work:frontier-history-adjacent-first",
                "printf 'first-output\\n'",
            ),
            (
                "work:frontier-history-adjacent-second",
                "printf 'second-output\\n'",
            ),
        ],
    );
    let output = run_native_frontier(&directory, &fixture, 2);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let traces = stdout_json(&output)["result"]["traces"]
        .as_array()
        .expect("frontier traces")
        .clone();
    assert_eq!(traces.len(), 2);
    let first_trace_id = traces[0]["trace_id"].as_str().expect("first trace id");
    let second_trace_id = traces[1]["trace_id"].as_str().expect("second trace id");

    let history_text = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "text",
    ]);
    assert!(
        history_text.status.success(),
        "stderr: {}",
        stderr(&history_text)
    );
    let text = stdout(&history_text);
    assert!(text.contains(first_trace_id));
    assert!(text.contains(second_trace_id));
    assert!(
        !text.contains("attempts:"),
        "two unrelated anchors must render as two lines, not one fold: {text}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// #24 C3: `sections::review_gaps` mints one `UnreviewedMorphism` gap per
/// unreviewed log entry, and every worker transition morphism is minted
/// unreviewed — so a store with a few successful dispatches already has
/// enough of them to prove the compact view groups rather than reprints the
/// identical explanation once per entry.
#[cfg(unix)]
#[test]
fn space_reason_text_groups_unreviewed_morphism_gaps_by_count_not_by_line() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "review-gap-grouping",
        &[
            (
                "work:frontier-review-gap-grouping-first",
                "printf 'first-output\\n'",
            ),
            (
                "work:frontier-review-gap-grouping-second",
                "printf 'second-output\\n'",
            ),
        ],
    );
    let output = run_native_frontier(&directory, &fixture, 2);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let json_report = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        json_report.status.success(),
        "stderr: {}",
        stderr(&json_report)
    );
    let review_gaps = stdout_json(&json_report)["result"]["evaluation"]["review_gaps"]
        .as_array()
        .expect("review gaps")
        .clone();
    let morphism_gaps = review_gaps
        .iter()
        .filter(|gap| gap["gap_type"] == json!("unreviewed_morphism"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        morphism_gaps.len() >= 2,
        "fixture must produce at least two unreviewed morphism gaps to test grouping: {morphism_gaps:?}"
    );

    let text_report = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "text",
    ]);
    assert!(
        text_report.status.success(),
        "stderr: {}",
        stderr(&text_report)
    );
    let text = stdout(&text_report);
    // Grouped, not reprinted: the exact count appears once, next to the
    // type name, and every target id is still present in that group's
    // `targets` line — nothing hidden, nothing filtered.
    assert!(text.contains(&format!(
        "unreviewed_morphism: {} gap(s)",
        morphism_gaps.len()
    )));
    for gap in &morphism_gaps {
        let target_id = gap["target_id"].as_str().expect("target id");
        assert!(text.contains(target_id));
    }
    assert_eq!(
        text.matches("Generated morphisms do not count as accepted evolution until reviewed.")
            .count(),
        1,
        "the constant explanation must appear once per group, not once per gap: {text}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_retry_does_not_release_a_live_dispatch_after_revision_moves() {
    // Issue #32 (one of the tests actually observed flaking): this used to
    // bet that spawning and completing two whole separate `run --step`
    // invocations (the sibling, then the refused retry attempt) fit inside
    // a fixed 3 s worker sleep — a wall-clock race a loaded machine can
    // lose, and did. The slow worker now waits on a marker the test creates
    // only after both invocations have finished and the "still live" check
    // has passed, so there is no window to miss on that axis.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let slow_started = directory.join("live-slow-worker-started");
    let slow_pids = directory.join("live-slow-worker-pids");
    let slow_proceed = directory.join("live-slow-worker-proceed");
    let slow_script = format!(
        "printf '%s\\n' \"$$\" >> '{}'\nprintf 'started\\n' > '{}'\n{}\nprintf 'slow-output\\n'",
        slow_pids.display(),
        slow_started.display(),
        shell_wait_for_marker(&slow_proceed)
    );
    // Issue #39: `setup_native_frontier` (via `write_pinned_worker_binding`)
    // pins every worker to a 5 s `timeout_ms`. That is not a race with
    // `sibling`'s dispatch selection — `select_steps` already excludes an
    // in-progress step by its live run directory, so `sibling` reliably
    // lands on `work:live-sibling` — it is a race with the *test's own*
    // wall-clock: `apply_step_result` only appends a trace's evidence/anchor
    // once its worker resolves (success, failure, *or timeout*), so as long
    // as `live_run`'s worker cannot resolve before `signal_rendezvous_marker`
    // is called below, the revision the whole test builds on
    // (`fixture.accepted_revision_id`) cannot move out from under `sibling`
    // or the refused retry check. Under load, `sibling`'s own spawn-and-
    // append plus the refused-retry check can together take longer than a
    // pinned 5 s, so `work:live-slow`'s own worker timed out mid-test,
    // appended its own (failed) evidence and anchor, and moved
    // `current_revision_id` while `sibling`'s append was still in flight —
    // reproducing the exact "two writers racing for the same next sequence"
    // shape this issue is about, from the timeout rather than from
    // dispatch-time ordering. Give only this worker a timeout well past
    // anything the deterministic parts of this test could plausibly take
    // (`setup_native_frontier_with_timeouts` rather than mutating the
    // registered binding after the fact: the plan captures the binding's
    // content hash at proposal time, so a post-registration rewrite is
    // correctly refused as tampering, not a shortcut worth taking), so the
    // *only* way it resolves is the explicit `signal_rendezvous_marker` call
    // below — a real event, not a race against a fixed budget.
    let fixture = setup_native_frontier_with_timeouts(
        &directory,
        "live-dispatch-retry",
        &[
            ("work:live-slow", slow_script.as_str()),
            ("work:live-sibling", "printf 'sibling-output\\n'"),
        ],
        &[30_000, 5_000],
    );
    let live_run = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_step_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            None,
            &[],
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn live slow run --step");
    wait_for_file(&slow_started, "slow worker did not start before timeout");

    let sibling = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_step_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            None,
            &[],
        ))
        .output()
        .expect("run sibling step");
    assert!(sibling.status.success(), "stderr: {}", stderr(&sibling));
    let sibling_json = stdout_json(&sibling);
    assert_eq!(sibling_json["result"]["status"], json!("step_executed"));
    assert_eq!(
        sibling_json["result"]["trace"]["step_id"],
        json!(fixture.step_ids[1])
    );
    let moved_revision = sibling_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("sibling result revision");

    let refused = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_step_args(
            &directory,
            &fixture,
            moved_revision,
            Some(&fixture.step_ids[0]),
            &[],
        ))
        .output()
        .expect("retry live slow step");
    assert!(refused.status.success(), "stderr: {}", stderr(&refused));
    let refused_json = stdout_json(&refused);
    assert_eq!(
        refused_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert!(refused_json["result"]["obstructions"]
        .as_array()
        .expect("run obstructions")
        .iter()
        .any(|obstruction| obstruction["obstruction_type"] == json!("dispatch_in_progress")));
    assert_eq!(
        fs::read_to_string(&slow_pids)
            .expect("read slow worker pids")
            .lines()
            .count(),
        1,
        "retry must not launch a second worker"
    );
    let slow_pid = fs::read_to_string(&slow_pids)
        .expect("read slow worker pid")
        .trim()
        .parse::<u32>()
        .expect("slow worker pid");
    assert!(
        process_exists(slow_pid),
        "the original worker must still be live when retry is refused"
    );
    signal_rendezvous_marker(&slow_proceed);

    let live_output = live_run.wait_with_output().expect("wait for live slow run");
    assert!(
        !live_output.status.success(),
        "the original run must observe the sibling's stale revision"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_refuses_invalid_supersede_trace_assertions() {
    let unknown_directory = unique_temp_dir();
    fs::create_dir_all(&unknown_directory).expect("create unknown temp directory");
    let unknown_fixture = setup_native_run(
        &unknown_directory,
        "unknown-supersede",
        "printf 'must-not-run\\n'",
    );
    let unknown = run_native_step_with_superseded_traces(
        &unknown_directory,
        &unknown_fixture,
        &unknown_fixture.accepted_revision_id,
        &["execution_trace:unknown"],
    );
    assert!(!unknown.status.success());
    assert!(
        stderr(&unknown).contains("is unknown"),
        "{}",
        stderr(&unknown)
    );
    fs::remove_dir_all(unknown_directory).expect("remove unknown temp directory");

    let failed_directory = unique_temp_dir();
    fs::create_dir_all(&failed_directory).expect("create failed temp directory");
    let failed_fixture = setup_native_run(&failed_directory, "failed-supersede", "exit 7");
    let failed_run = run_native_step(&failed_directory, &failed_fixture, true, None);
    assert!(
        failed_run.status.success(),
        "stderr: {}",
        stderr(&failed_run)
    );
    let failed_json = stdout_json(&failed_run);
    let failed_trace_id = failed_json["result"]["trace"]["trace_id"]
        .as_str()
        .expect("failed trace id");
    let failed_revision = failed_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("failed trace revision");
    let failed = run_native_step_with_superseded_traces(
        &failed_directory,
        &failed_fixture,
        failed_revision,
        &[failed_trace_id],
    );
    assert!(!failed.status.success());
    assert!(
        stderr(&failed).contains("already failed"),
        "{}",
        stderr(&failed)
    );

    let failed_trace_path = only_run_file(&failed_directory, "execution.trace.json");
    let mut different_step_trace = json_file(failed_trace_path);
    different_step_trace["trace_id"] = json!("execution-trace-different-step");
    different_step_trace["step_id"] = json!("step:different");
    different_step_trace["dispatch_state"] = json!("started");
    different_step_trace["transition_applied"] = json!(false);
    different_step_trace["result_revision_id"] = Value::Null;
    let different_step_directory = failed_directory
        .join("runs")
        .join("execution-trace-different-step");
    fs::create_dir(&different_step_directory).expect("create different-step run directory");
    fs::write(
        different_step_directory.join("execution.trace.json"),
        serde_json::to_vec_pretty(&different_step_trace).expect("serialize different-step trace"),
    )
    .expect("write different-step trace");
    let different_step = run_native_step_with_superseded_traces(
        &failed_directory,
        &failed_fixture,
        failed_revision,
        &["execution-trace-different-step"],
    );
    assert!(!different_step.status.success());
    assert!(
        stderr(&different_step).contains("is not a step of plan"),
        "{}",
        stderr(&different_step)
    );

    different_step_trace["trace_id"] = json!("execution-trace-different-plan");
    different_step_trace["plan_id"] = json!("plan:different");
    different_step_trace["step_id"] = json!(failed_fixture.step_id);
    let different_plan_directory = failed_directory
        .join("runs")
        .join("execution-trace-different-plan");
    fs::create_dir(&different_plan_directory).expect("create different-plan run directory");
    fs::write(
        different_plan_directory.join("execution.trace.json"),
        serde_json::to_vec_pretty(&different_step_trace).expect("serialize different-plan trace"),
    )
    .expect("write different-plan trace");
    let different_plan = run_native_step_with_superseded_traces(
        &failed_directory,
        &failed_fixture,
        failed_revision,
        &["execution-trace-different-plan"],
    );
    assert!(!different_plan.status.success());
    assert!(
        stderr(&different_plan).contains("belongs to plan plan:different"),
        "{}",
        stderr(&different_plan)
    );
    fs::remove_dir_all(failed_directory).expect("remove failed temp directory");

    let applied_directory = unique_temp_dir();
    fs::create_dir_all(&applied_directory).expect("create applied temp directory");
    let applied_fixture = setup_native_run(
        &applied_directory,
        "applied-supersede",
        "printf 'applied\\n'",
    );
    let applied_run = run_native_step(&applied_directory, &applied_fixture, true, None);
    assert!(
        applied_run.status.success(),
        "stderr: {}",
        stderr(&applied_run)
    );
    let applied_json = stdout_json(&applied_run);
    let applied_trace_id = applied_json["result"]["trace"]["trace_id"]
        .as_str()
        .expect("applied trace id");
    let applied_revision = applied_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("applied trace revision");
    let applied = run_native_step_with_superseded_traces(
        &applied_directory,
        &applied_fixture,
        applied_revision,
        &[applied_trace_id],
    );
    assert!(!applied.status.success());
    assert!(
        stderr(&applied).contains("already applied"),
        "{}",
        stderr(&applied)
    );
    fs::remove_dir_all(applied_directory).expect("remove applied temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_step_asserting_trace_a_does_not_release_later_trace_b() {
    // Issue #32: both dispatches of this work cell run the identical
    // script (the whole point is retrying/superseding the *same* step), so
    // the two invocations cannot be told apart from the outside. The first
    // invocation's own completion timing was never fragile — nothing races
    // it, it just needs to finish on its own before `second` is even
    // spawned, and `wait_for_line_count`'s generous bound already covers
    // that. The second invocation's completion *was* fragile: this used to
    // bet that the whole `refused` retry-attempt invocation completed
    // inside the same fixed 1 s sleep the first invocation used, a
    // wall-clock race a loaded machine can lose. The script now tells the
    // two apart by checking whether `finishes` already has an entry — true
    // only for a later invocation, since the first invocation's own write
    // to `finishes` is what the test waits for before ever spawning a
    // second one — and only a later invocation waits on a proceed marker,
    // created once the `refused` check has actually run.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let starts = directory.join("successive-dispatch-starts");
    let finishes = directory.join("successive-dispatch-finishes");
    let later_proceed = directory.join("successive-dispatch-later-proceed");
    let script = format!(
        "printf '%s\\n' \"$$\" >> '{}'\nif [ -s '{}' ]; then {}; else sleep 1; fi\nprintf '%s\\n' \"$$\" >> '{}'\nprintf 'worker-output\\n'",
        starts.display(),
        finishes.display(),
        shell_wait_for_marker(&later_proceed),
        finishes.display()
    );
    let fixture = setup_native_run(&directory, "successive-started", &script);
    let mut first = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_step_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            true,
            None,
            &["capability:dispatch", "capability:native-run-worker"],
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first dispatch");
    wait_for_line_count(&starts, 1, "first dispatch did not start");
    first.kill().expect("kill first dispatcher");
    let killed = first.wait_with_output().expect("wait for first dispatcher");
    assert!(!killed.status.success());
    wait_for_line_count(&finishes, 1, "first worker did not finish");
    let trace_a = json_file(only_run_file(&directory, "execution.trace.json"))["trace_id"]
        .as_str()
        .expect("trace A id")
        .to_owned();

    let mut second_args = native_step_args(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        true,
        None,
        &["capability:dispatch", "capability:native-run-worker"],
    );
    append_supersede_trace_args(&mut second_args, &[trace_a.as_str()]);
    let second = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(second_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn second dispatch");
    wait_for_line_count(&starts, 2, "second dispatch did not start");

    let refused = run_native_step_with_superseded_traces(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        &[trace_a.as_str()],
    );
    assert!(refused.status.success(), "stderr: {}", stderr(&refused));
    let refused_json = stdout_json(&refused);
    assert_eq!(
        refused_json["result"]["status"],
        json!("no_dispatchable_step")
    );
    assert!(refused_json["result"]["obstructions"]
        .as_array()
        .expect("run obstructions")
        .iter()
        .any(|obstruction| obstruction["obstruction_type"] == json!("dispatch_in_progress")));
    assert_eq!(
        fs::read_to_string(&starts)
            .expect("read dispatch starts")
            .lines()
            .count(),
        2,
        "asserting trace A must not start a third worker while trace B is live"
    );
    signal_rendezvous_marker(&later_proceed);

    let second_output = second.wait_with_output().expect("wait for second dispatch");
    assert!(
        second_output.status.success(),
        "stderr: {}",
        stderr(&second_output)
    );
    assert_eq!(run_files(&directory, "execution.trace.json").len(), 2);
    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_max_parallel_one_has_the_same_serial_log_order() {
    let serial_directory = unique_temp_dir();
    let parallel_directory = unique_temp_dir();
    fs::create_dir_all(&serial_directory).expect("create serial temp directory");
    fs::create_dir_all(&parallel_directory).expect("create parallel temp directory");
    let workers = [
        (
            "work:frontier-compare-first",
            "sleep 0.1\nprintf 'first\\n'",
        ),
        (
            "work:frontier-compare-second",
            "sleep 0.1\nprintf 'second\\n'",
        ),
    ];
    let serial_fixture = setup_native_frontier(&serial_directory, "parallel-limit", &workers);
    let parallel_fixture = setup_native_frontier(&parallel_directory, "parallel-limit", &workers);

    let serial = run_native_frontier(&serial_directory, &serial_fixture, 1);
    let parallel = run_native_frontier(&parallel_directory, &parallel_fixture, 2);

    assert!(serial.status.success(), "stderr: {}", stderr(&serial));
    assert!(parallel.status.success(), "stderr: {}", stderr(&parallel));
    let serial_value = stdout_json(&serial);
    let parallel_value = stdout_json(&parallel);
    assert_eq!(
        serial_value["result"]["appended_entry_ids"],
        parallel_value["result"]["appended_entry_ids"]
    );
    assert_eq!(
        serial_value["result"]["result_revision_id"],
        parallel_value["result"]["result_revision_id"]
    );
    assert_eq!(
        serial_value["result"]["traces"][0]["step_id"],
        parallel_value["result"]["traces"][0]["step_id"]
    );
    assert_eq!(
        serial_value["result"]["traces"][1]["step_id"],
        parallel_value["result"]["traces"][1]["step_id"]
    );
    assert_native_store_valid_and_rebuilds(&serial_directory);
    assert_native_store_valid_and_rebuilds(&parallel_directory);
    fs::remove_dir_all(serial_directory).expect("remove serial temp directory");
    fs::remove_dir_all(parallel_directory).expect("remove parallel temp directory");
}

#[test]
fn native_run_step_and_frontier_are_mutually_exclusive() {
    let output = run_cli(&["run", "--step", "--frontier", "--format", "json"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("exactly one of --step or --frontier"));

    let zero_parallel = run_cli(&[
        "run",
        "--frontier",
        "--store",
        "unused",
        "--case-space-id",
        "case_space:unused",
        "--plan-id",
        "plan:unused",
        "--base-revision-id",
        "revision:unused",
        "--actor-id",
        "actor:unused",
        "--capability-id",
        "capability:unused",
        "--operation-scope-id",
        "case_space:unused",
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:unused",
        "--max-parallel",
        "0",
        "--format",
        "json",
    ]);
    assert!(!zero_parallel.status.success());
    assert!(stderr(&zero_parallel).contains("--max-parallel must be at least 1"));
}

#[test]
fn unsupported_audience_value_carries_the_accepted_set_structurally() {
    // `--audience` is an enum-valued flag the parser already holds the
    // closed set for (issue #22): a bad value hands the accepted set back
    // in `data`, not only enumerated inside `message` prose.
    let output = run_cli(&[
        "run",
        "--frontier",
        "--store",
        "unused",
        "--case-space-id",
        "case_space:unused",
        "--plan-id",
        "plan:unused",
        "--base-revision-id",
        "revision:unused",
        "--actor-id",
        "actor:unused",
        "--capability-id",
        "capability:unused",
        "--operation-scope-id",
        "case_space:unused",
        "--audience",
        "not-a-real-audience",
        "--source-boundary-id",
        "source_boundary:unused",
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    let refusal = stderr_json(&output);
    assert_eq!(refusal["error_code"], json!("usage"));
    assert_eq!(refusal["data"]["flag"], json!("--audience"));
    assert_eq!(refusal["data"]["value"], json!("not-a-real-audience"));
    assert_eq!(
        refusal["data"]["accepted_values"],
        json!(["human_review", "ai_agent", "audit", "system", "migration"])
    );
}

#[test]
fn native_typed_morphism_materializes_payload_end_to_end() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-typed-base");

    let morphism_path = directory.join("typed-add.case_morphism.json");
    let morphism = json!({
        "morphism_id": "morphism:native-typed-add",
        "morphism_type": "create",
        "source_revision_id": "revision:native-typed-base",
        "target_revision_id": "revision:native-typed-added",
        "added_ids": [
            "work:native-typed-reducer",
            "relation:native-typed-depends-on-goal"
        ],
        "updated_ids": [],
        "retired_ids": [],
        "preserved_ids": ["goal:native-case-contract"],
        "violated_invariant_ids": [],
        "review_status": "unreviewed",
        "evidence_ids": [],
        "source_ids": ["source:native-typed-integration"],
        "metadata": {
            "payload": {
                "added_cells": [
                    {
                        "id": "work:native-typed-reducer",
                        "cell_type": "work",
                        "space_id": "space:higher-graphen-casegraphen",
                        "title": "Exercise typed morphism reducers",
                        "lifecycle": "proposed",
                        "source_ids": ["source:native-typed-integration"],
                        "structure_ids": [],
                        "provenance": {
                            "source": {
                                "kind": "human",
                                "title": "Typed reducer integration test"
                            },
                            "confidence": 1.0,
                            "review_status": "unreviewed"
                        },
                        "metadata": {}
                    }
                ],
                "added_relations": [
                    {
                        "id": "relation:native-typed-depends-on-goal",
                        "relation_type": "depends_on",
                        "relation_strength": "hard",
                        "from_id": "work:native-typed-reducer",
                        "to_id": "evidence:native-schema-json-valid",
                        "evidence_ids": [],
                        "source_ids": ["source:native-typed-integration"],
                        "provenance": {
                            "source": {
                                "kind": "human",
                                "title": "Typed reducer integration test"
                            },
                            "confidence": 1.0,
                            "review_status": "unreviewed"
                        },
                        "metadata": {}
                    }
                ]
            }
        }
    });
    fs::write(
        &morphism_path,
        serde_json::to_string_pretty(&morphism).expect("serialize typed morphism"),
    )
    .expect("write typed morphism");

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));

    let check = run_cli(&[
        "morphism",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-typed-add",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    assert_eq!(stdout_json(&check)["result"]["valid"], json!(true));
    assert_eq!(stdout_json(&check)["result"]["applicable"], json!(true));

    let apply = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:native-typed-add",
            "--base-revision-id",
            "revision:native-typed-base",
            "--reviewer-id",
            "reviewer:native-typed-integration",
            "--reason",
            "Accept typed reducer integration morphism",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let apply_json = stdout_json(&apply);
    let previous_hash = apply_json["result"]["entry"]["previous_entry_hash"]
        .as_str()
        .expect("previous entry hash");
    assert_eq!(previous_hash.len(), 64);
    assert!(previous_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let replay = run_cli(&[
        "space",
        "replay",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(replay.status.success(), "stderr: {}", stderr(&replay));
    let replay_json = stdout_json(&replay);
    let replay_space = &replay_json["result"]["replay"]["case_space"];
    assert!(replay_space["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .any(|cell| cell["id"] == json!("work:native-typed-reducer")));
    assert!(replay_space["case_relations"]
        .as_array()
        .expect("replayed relations")
        .iter()
        .any(|relation| {
            relation["id"] == json!("relation:native-typed-depends-on-goal")
                && relation["relation_type"] == json!("depends_on")
        }));

    let history = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(history.status.success(), "stderr: {}", stderr(&history));
    let history_json = stdout_json(&history);
    assert_eq!(
        history_json["result"]["entries"][1]["morphism"]["metadata"]["payload"]["added_cells"][0]
            ["id"],
        json!("work:native-typed-reducer")
    );
    assert_eq!(
        history_json["result"]["entries"][1]["morphism"]["metadata"]["payload"]["added_relations"]
            [0]["id"],
        json!("relation:native-typed-depends-on-goal")
    );

    let validate = run_cli(&[
        "space",
        "validate",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(validate.status.success(), "stderr: {}", stderr(&validate));
    assert_eq!(
        stdout_json(&validate)["result"]["validation"]["valid"],
        json!(true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_review_accept_appends_history_and_satisfies_close_review() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let native_path = directory.join("native.unreviewed-morphism.json");
    let mut native_case = json_file(native_case_fixture());
    native_case["morphism_log"][0]["morphism"]["review_status"] = json!("unreviewed");
    fs::write(
        &native_path,
        serde_json::to_string_pretty(&native_case).expect("serialize native case"),
    )
    .expect("write native case");
    import_native_case_space_from_input(&directory, &native_path, "revision:native-review-base");

    let accept_args = [
        "review",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "morphism:create-native-contract",
        "--reviewer-id",
        "reviewer:native-review-cli",
        "--reason",
        "Accept the imported morphism after explicit review",
        "--base-revision-id",
        "revision:native-review-base",
        "--evidence-id",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
    ];
    let ungated_accept = run_cli(&accept_args);
    assert!(!ungated_accept.status.success());
    assert!(stderr(&ungated_accept).contains("--actor-id <id> is required for review"));

    let accept = run_cli_with_mutation_gate(&accept_args, "actor:native-mutation-cli");
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accept_json = stdout_json(&accept);
    let entry = &accept_json["result"]["entry"];
    assert_eq!(
        accept_json["metadata"]["command"],
        json!("casegraphen review accept")
    );
    assert_eq!(entry["actor_id"], json!("actor:native-mutation-cli"));
    assert_eq!(
        entry["morphism"]["metadata"],
        json!({
            "native_review_schema_version": 1,
            "review_id": entry["morphism"]["metadata"]["review_id"],
            "target_kind": "morphism",
            "target_id": "morphism:create-native-contract",
            "action": "accept",
            "outcome_review_status": "accepted",
            "reviewer_id": "reviewer:native-review-cli",
            "reviewed_at": entry["morphism"]["metadata"]["reviewed_at"],
            "reason": "Accept the imported morphism after explicit review",
            "operation_gate": {
                "actor_id": "actor:native-mutation-cli",
                "operation": "review",
                "operation_scope_id": native_case_space_id(),
                "audience": "audit",
                "capability_ids": ["capability:durable-mutation"],
                "source_boundary_id": "source_boundary:native-case-management-contract"
            }
        })
    );
    assert_eq!(
        entry["morphism"]["evidence_ids"],
        json!(["evidence:native-schema-json-valid"])
    );
    let current_revision = accept_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("review revision")
        .to_owned();

    let history = run_cli(&[
        "space",
        "history",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(history.status.success(), "stderr: {}", stderr(&history));
    assert!(stdout_json(&history)["result"]["entries"]
        .as_array()
        .expect("history entries")
        .iter()
        .any(|candidate| {
            candidate["morphism"]["metadata"]["target_id"]
                == json!("morphism:create-native-contract")
                && candidate["morphism"]["metadata"]["action"] == json!("accept")
        }));

    let close_check = run_cli(&[
        "invariant",
        "close-check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        &current_revision,
        "--validation-evidence-id",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
    ]);
    assert!(
        close_check.status.success(),
        "stderr: {}",
        stderr(&close_check)
    );
    assert!(
        stdout_json(&close_check)["result"]["close_check"]["invariant_results"]
            .as_array()
            .expect("close invariants")
            .iter()
            .any(|invariant| {
                invariant["invariant_id"] == json!("close:native-morphisms-reviewed")
                    && invariant["passed"] == json!(true)
            })
    );

    let empty_reason = run_cli_with_mutation_gate(
        &[
            "review",
            "reject",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "goal:native-case-contract",
            "--reviewer-id",
            "reviewer:native-review-cli",
            "--reason",
            "   ",
            "--base-revision-id",
            &current_revision,
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!empty_reason.status.success());
    assert!(stderr(&empty_reason).contains("review reason must not be empty"));

    let stale = run_cli_with_mutation_gate(
        &[
            "review",
            "reopen",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "goal:native-case-contract",
            "--reviewer-id",
            "reviewer:native-review-cli",
            "--reason",
            "Exercise stale base handling",
            "--base-revision-id",
            "revision:native-review-base",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!stale.status.success());
    assert!(stderr(&stale).contains(&format!(
        "base revision revision:native-review-base is stale; current revision is {current_revision}"
    )));
    // A stale base revision carries the current revision back structurally
    // (issue #22), not only in prose: recovery is re-reading
    // `data.current_revision_id` and retrying, which `error_code` marks as
    // a different kind of answer from a plain usage or business-rule
    // refusal.
    let stale_refusal = stderr_json(&stale);
    assert_eq!(stale_refusal["error_code"], json!("stale_revision"));
    assert_eq!(
        stale_refusal["data"]["base_revision_id"],
        json!("revision:native-review-base")
    );
    assert_eq!(
        stale_refusal["data"]["current_revision_id"],
        json!(current_revision)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_refusal_after_a_successful_parse_renders_in_the_format_parsing_resolved() {
    // Regression: `--reason` (a free-text flag) takes the very next token
    // unconditionally as its value, with no quoting distinction between "a
    // flag" and "data" — pre-existing behaviour, not something issue #22
    // introduced. If that token is literally the string `--format`, a
    // *second*, later `--format <value>` is the one the parser actually
    // recognizes, and parsing genuinely succeeds with that value.
    //
    // Before the fix, `main_entry` re-derived the refusal's render format
    // from a raw-argv scan for every refusal, even when `Command::parse`
    // had already succeeded and `command.format()` was known with
    // certainty. That scan finds the *first* `--format` token pair — whose
    // value is the unrecognized literal string "--format" — and never
    // revisits the second, real one, so it silently fell back to the
    // default (json) even when parsing had resolved text.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:format-survives-failure");
    let store = directory.to_str().expect("temp path").to_owned();

    let trick_args = |case_space_id: &str, format: &str| {
        run_cli(&[
            "space",
            "reason",
            "--store",
            &store,
            "--case-space-id",
            case_space_id,
            "--reason",
            "--format",
            "--format",
            format,
        ])
    };

    // First prove the trick genuinely parses, rather than merely looking
    // plausible: against the case space that *does* exist, it succeeds and
    // renders in the format the second `--format` names.
    let succeeds = trick_args(native_case_space_id(), "text");
    assert!(succeeds.status.success(), "stderr: {}", stderr(&succeeds));
    assert!(
        !stdout(&succeeds).trim_start().starts_with('{'),
        "a command parsed with format text must render as prose: {}",
        stdout(&succeeds)
    );

    // Now the regression case: the same trick, but execution fails (a
    // second case-space-id that was never imported) *after* parsing already
    // resolved format to text. The refusal must be prose, not JSON.
    let text_refusal = trick_args("case_space:does-not-exist", "text");
    assert!(!text_refusal.status.success());
    let text_refusal_stderr = stderr(&text_refusal);
    assert!(
        !text_refusal_stderr.trim_start().starts_with('{'),
        "a refusal from a command that parsed with format text must render as prose, not JSON: {text_refusal_stderr}"
    );
    assert!(text_refusal_stderr.contains("missing native case space case_space:does-not-exist"));

    // Mirror case: the same trick with the second `--format` naming json
    // must still render JSON — the fix is "use whatever `Command::parse`
    // actually resolved", not "the renderer now always prefers text".
    let json_refusal = trick_args("case_space:does-not-exist", "json");
    assert!(!json_refusal.status.success());
    assert_eq!(
        stderr_json(&json_refusal)["error_code"],
        json!("missing_case_space")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_evidence_attach_materializes_cell_relation_and_content_hash() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-evidence-base");
    let input_path = directory.join("attached-evidence-cell.json");
    let output_path = directory.join("evidence-attach.report.json");
    let mut evidence_cell = json_file(native_case_fixture())["case_cells"][3].clone();
    evidence_cell["id"] = json!("evidence:attached-cli");
    evidence_cell["title"] = json!("Attached CLI evidence");
    evidence_cell["lifecycle"] = json!("active");
    evidence_cell["provenance"]["review_status"] = json!("unreviewed");
    evidence_cell["source_ids"] = json!(["source:attached-cli"]);
    evidence_cell["metadata"] =
        json!({"evidence_boundary": "source_backed", "content_hash": "caller-bogus-hash"});
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&evidence_cell).expect("serialize evidence cell"),
    )
    .expect("write evidence cell");

    let attach_args = [
        "evidence",
        "attach",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:native-evidence-base",
        "--input",
        input_path.to_str().expect("evidence path"),
        "--satisfies",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ];
    let ungated_attach = run_cli(&attach_args);
    assert!(!ungated_attach.status.success());
    assert!(stderr(&ungated_attach).contains("--actor-id <id> is required for evidence attach"));

    let attach = run_cli_with_mutation_gate(&attach_args, "actor:native-evidence-cli");
    assert!(attach.status.success(), "stderr: {}", stderr(&attach));
    assert!(stdout(&attach).is_empty());
    let attach_json = json_file(output_path);
    let entry = &attach_json["result"]["entry"];
    let attached_cell = &entry["morphism"]["metadata"]["payload"]["added_cells"][0];
    let relation = &entry["morphism"]["metadata"]["payload"]["added_relations"][0];
    assert_eq!(
        attach_json["metadata"]["command"],
        json!("casegraphen evidence attach")
    );
    assert_eq!(entry["actor_id"], json!("actor:native-evidence-cli"));
    assert_eq!(entry["morphism"]["morphism_type"], json!("evidence_attach"));
    assert_eq!(
        entry["morphism"]["metadata"]["operation_gate"]["operation"],
        json!("evidence-attach")
    );
    assert_eq!(
        entry["morphism"]["added_ids"],
        json!([
            "evidence:attached-cli",
            "relation:evidence:evidence~3aattached-cli:1"
        ])
    );
    assert_eq!(
        attached_cell["provenance"]["review_status"],
        json!("unreviewed")
    );
    // `inferred` is the shared trust rule's own spelling for "needs an
    // accepted review"; the previous `attached_unverified` was an unrecognized
    // string that only happened to fall through to the same treatment.
    assert_eq!(
        attached_cell["metadata"]["evidence_boundary"],
        json!("inferred")
    );
    let content_hash = attached_cell["metadata"]["content_hash"]
        .as_str()
        .expect("content hash");
    assert_ne!(content_hash, "caller-bogus-hash");
    assert_eq!(content_hash.len(), 64);
    assert!(content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(
        relation,
        &json!({
            "id": "relation:evidence:evidence~3aattached-cli:1",
            "relation_type": "satisfies_evidence_requirement",
            "relation_strength": "diagnostic",
            "from_id": "evidence:attached-cli",
            "to_id": "evidence:native-schema-json-valid",
            "evidence_ids": ["evidence:attached-cli"],
            "source_ids": ["source:attached-cli"],
            "provenance": attached_cell["provenance"],
            "metadata": {}
        })
    );

    let replay = run_native_case_store_command(&directory, "replay");
    let replay_space = &stdout_json(&replay)["result"]["replay"]["case_space"];
    assert!(replay_space["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .any(|cell| {
            cell["id"] == json!("evidence:attached-cli")
                && cell["metadata"]["content_hash"] == json!(content_hash)
                && cell["provenance"]["review_status"] == json!("unreviewed")
        }));
    assert!(replay_space["case_relations"]
        .as_array()
        .expect("replayed relations")
        .iter()
        .any(|candidate| candidate == relation));

    let reason = run_native_case_store_command(&directory, "reason");
    assert!(
        !stdout_json(&reason)["result"]["evaluation"]["evidence_findings"]
            ["source_backed_evidence_ids"]
            .as_array()
            .expect("reason evidence ids")
            .contains(&json!("evidence:attached-cli"))
    );
    let evidence = run_native_case_store_command(&directory, "evidence");
    assert!(
        !stdout_json(&evidence)["result"]["evidence_findings"]["source_backed_evidence_ids"]
            .as_array()
            .expect("evidence ids")
            .contains(&json!("evidence:attached-cli"))
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn execution_topology_review_cli_binds_store_artifact_and_enables_reviewed_compilation() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:topology-review-base");

    let mut topology_value = json_file(repo_path(
        "schemas/experimental/execution.topology.file-review.example.json",
    ));
    topology_value["topology_id"] = json!("topology:native-cli-reviewed");
    topology_value["case_space_id"] = json!(native_case_space_id());
    let topology: ExecutionTopology =
        serde_json::from_value(topology_value.clone()).expect("typed topology");
    let topology_hash = execution_topology_content_hash(&topology).expect("topology hash");
    let verification_policies = topology
        .verification_policy_ids
        .iter()
        .map(|id| {
            let mut value = json_file(repo_path(
                "schemas/experimental/verification.policy.example.json",
            ));
            value["verification_policy_id"] = json!(id);
            (id.clone(), value)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let budget_policies = topology
        .budget_policy_ids
        .iter()
        .map(|id| (id.clone(), json!({"policy_id": id, "max_cost": 10})))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expansion_policies = std::collections::BTreeMap::new();
    let policy_manifest = casegraphen::deployment_policy::deployment_policy_manifest(
        &topology,
        &topology_hash,
        &verification_policies,
        &budget_policies,
        &expansion_policies,
    );
    let policy_manifest_hash =
        casegraphen::deployment_policy::deployment_policy_manifest_content_hash(&policy_manifest)
            .expect("policy manifest hash");
    let policy_manifest_path = directory.join("deployment-policy-manifest.json");
    fs::write(
        &policy_manifest_path,
        serde_json::to_vec_pretty(&policy_manifest).expect("policy manifest bytes"),
    )
    .expect("write policy manifest");
    let topology_path = directory.join("execution.topology.json");
    let topology_bytes = serde_json::to_vec_pretty(&topology_value).expect("topology bytes");
    fs::write(&topology_path, &topology_bytes).expect("write topology artifact");
    let artifact_hash = format!("{:x}", Sha256::digest(&topology_bytes));
    let artifact_id = format!("artifact:sha256-{artifact_hash}");

    let mut claim = json_file(native_case_fixture())["case_cells"][3].clone();
    claim["id"] = json!("evidence:execution-topology");
    claim["title"] = json!("Execution topology proposal");
    claim["lifecycle"] = json!("active");
    claim["provenance"]["review_status"] = json!("unreviewed");
    claim["metadata"] = json!({
        "evidence_boundary": "inferred",
        "topology_id": topology.topology_id,
        "execution_topology_content_hash": topology_hash,
        "artifact_id": artifact_id,
        "case_space_id": native_case_space_id(),
        "policy_manifest_content_hash": policy_manifest_hash
    });
    let claim_path = directory.join("execution-topology-claim.json");
    fs::write(
        &claim_path,
        serde_json::to_vec_pretty(&claim).expect("claim bytes"),
    )
    .expect("write claim");

    let attach_args = [
        "evidence",
        "attach",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:topology-review-base",
        "--input",
        claim_path.to_str().expect("claim path"),
        "--artifact",
        topology_path.to_str().expect("topology path"),
        "--format",
        "json",
    ];
    let attach = run_cli_with_mutation_gate(&attach_args, "actor:native-evidence-cli");
    assert!(attach.status.success(), "stderr: {}", stderr(&attach));
    let attached_revision = stdout_json(&attach)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();

    let review_args = [
        "topology-review",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:execution-topology",
        "--input",
        topology_path.to_str().expect("topology path"),
        "--policy-manifest",
        policy_manifest_path.to_str().expect("policy manifest path"),
        "--reviewer-id",
        "reviewer:topology",
        "--reason",
        "Reviewed the exact topology artifact.",
        "--base-revision-id",
        &attached_revision,
        "--format",
        "json",
    ];
    let accepted = run_cli_with_mutation_gate(&review_args, "actor:native-mutation-cli");
    assert!(accepted.status.success(), "stderr: {}", stderr(&accepted));
    let accepted_json = stdout_json(&accepted);
    let accepted_revision = accepted_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted revision")
        .to_owned();
    let metadata = &accepted_json["result"]["entry"]["morphism"]["metadata"];
    assert_eq!(metadata["target_kind"], json!("execution_topology"));
    assert_eq!(
        metadata["execution_topology_binding"]["topology_content_hash"],
        json!(topology_hash)
    );
    assert_eq!(
        metadata["execution_topology_binding"]["artifact_id"],
        json!(artifact_id)
    );
    assert_eq!(
        metadata["execution_topology_binding"]["observed_base_revision_id"],
        json!(attached_revision)
    );

    let inspected = run_cli(&[
        "topology-review",
        "inspect",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--target-id",
        "evidence:execution-topology",
        "--format",
        "json",
    ]);
    assert!(inspected.status.success(), "stderr: {}", stderr(&inspected));
    assert_eq!(
        stdout_json(&inspected)["result"]["current_status"],
        json!("accepted")
    );

    let replay = NativeCaseStore::new(directory.clone())
        .replay_current_case_space(&higher_graphen_core::Id::new(native_case_space_id()).unwrap())
        .expect("store replay");
    let mode = reviewed_compilation_mode(&replay.case_space, "evidence:execution-topology")
        .expect("content-bound reviewed mode");
    let transition = AllowedTransitionClass {
        morphism_type: CaseMorphismType::Update,
        target_cell_types: vec![CaseCellType::Work],
        to_lifecycles: vec![CaseCellLifecycle::Resolved],
    };
    let request = CompilerRequest {
        mode,
        target: CompilationTarget::GenericJsonlV0,
        case_space_id: native_case_space_id().to_owned(),
        base_revision_id: accepted_revision.clone(),
        plan_id: "plan:reviewed-topology-e2e".to_owned(),
        node_plan_mappings: topology
            .nodes
            .iter()
            .map(|node| NodePlanMapping {
                node_id: node.node_id.clone(),
                worker_binding_id: format!("worker_binding:{}", node.node_id),
                success_evidence_requirement_ids: vec![format!(
                    "evidence_requirement:{}",
                    node.node_id
                )],
                allowed_transition_classes: vec![transition.clone()],
            })
            .collect(),
        verification_policies: verification_policies.clone(),
        budget_policies: budget_policies.clone(),
        expansion_policies: expansion_policies.clone(),
    };
    let bundle = compile_execution_topology(&topology, &request)
        .expect("store-produced reviewed topology compiles");
    assert_eq!(bundle.manifest.mode, "reviewed");

    let run_disposition = |action: &str, base_revision: &str| {
        let args = [
            "topology-review",
            action,
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:execution-topology",
            "--input",
            topology_path.to_str().expect("topology path"),
            "--policy-manifest",
            policy_manifest_path.to_str().expect("policy manifest path"),
            "--reviewer-id",
            "reviewer:topology",
            "--reason",
            "Exercise the explicit topology review lifecycle.",
            "--base-revision-id",
            base_revision,
            "--format",
            "json",
        ];
        run_cli_with_mutation_gate(&args, "actor:native-mutation-cli")
    };
    let reopened = run_disposition("reopen", &accepted_revision);
    assert!(reopened.status.success(), "stderr: {}", stderr(&reopened));
    let reopened_revision = stdout_json(&reopened)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("reopened revision")
        .to_owned();
    let rejected = run_disposition("reject", &reopened_revision);
    assert!(rejected.status.success(), "stderr: {}", stderr(&rejected));

    let mut changed_topology = topology.clone();
    changed_topology.nodes[0]
        .purpose
        .push_str(" changed after review");
    let refusal = compile_execution_topology(&changed_topology, &request)
        .expect_err("same claim id cannot authorize changed topology bytes");
    assert!(refusal
        .unsupported_semantics
        .iter()
        .any(|finding| finding.code == "reviewed_topology_hash_mismatch"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// `parse_topology_review` used to answer "which review operations exist?"
/// in two places — an outer `"accept" | "reject" | "reopen"` arm and an
/// inner match converting the same three strings to `ReviewAction` — with
/// the inner match's `_` falling to `unreachable!()`. Adding a fourth action
/// to only one of the two would have compiled and then panicked at runtime.
/// The two were collapsed into one inner match; this pins that an unknown
/// topology-review operation still refuses cleanly (not a panic, not a
/// crash exit) with the exact usage message and error_code unchanged, and
/// that "inspect" and the real actions are unaffected by the collapse.
#[test]
fn topology_review_unsupported_operation_is_refused_not_panicked() {
    let refused = run_cli(&[
        "topology-review",
        "bogus-operation",
        "--store",
        "/nonexistent-store",
        "--format",
        "json",
    ]);
    assert!(!refused.status.success());
    assert_eq!(stderr_json(&refused)["error_code"], json!("usage"));
    assert_eq!(
        stderr_json(&refused)["message"],
        json!("unsupported native topology-review command")
    );
}

#[test]
fn a_refusal_after_a_landed_mutation_reports_completed_through() {
    // Regression: `--output` naming a directory that does not exist fails
    // *after* the append already landed. Before the fix, the refusal threw
    // away the very revision id the just-built report already held,
    // leaving the caller to replay the store and diff to find out what
    // happened — exactly the hand-driven reconstruction ADR 0016 exists to
    // delete.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:completed-through-base");
    let input_path = directory.join("attached-evidence-cell.json");
    let mut evidence_cell = json_file(native_case_fixture())["case_cells"][3].clone();
    evidence_cell["id"] = json!("evidence:completed-through");
    evidence_cell["title"] = json!("Completed-through evidence");
    evidence_cell["lifecycle"] = json!("active");
    evidence_cell["provenance"]["review_status"] = json!("unreviewed");
    evidence_cell["source_ids"] = json!(["source:completed-through"]);
    evidence_cell["metadata"] =
        json!({"evidence_boundary": "source_backed", "content_hash": "caller-bogus-hash"});
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&evidence_cell).expect("serialize evidence cell"),
    )
    .expect("write evidence cell");

    let history_before = stdout_json(&run_native_case_store_command(&directory, "history"))
        ["result"]["entries"]
        .as_array()
        .expect("history entries")
        .len();

    let attach_args = [
        "evidence",
        "attach",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:completed-through-base",
        "--input",
        input_path.to_str().expect("evidence path"),
        "--satisfies",
        "evidence:native-schema-json-valid",
        "--format",
        "json",
        "--output",
        "/nonexistent-dir-completed-through-xyz/report.json",
    ];
    let attach = run_cli_with_mutation_gate(&attach_args, "actor:native-evidence-cli");
    assert!(!attach.status.success());
    let refusal = stderr_json(&attach);
    assert_eq!(refusal["error_code"], json!("io_error"));

    let history_after_output = stdout_json(&run_native_case_store_command(&directory, "history"));
    let history_after = history_after_output["result"]["entries"]
        .as_array()
        .expect("history entries");
    assert_eq!(
        history_after.len(),
        history_before + 1,
        "the append must have landed despite the --output failure"
    );
    let new_head = history_after.last().expect("new entry")["target_revision_id"].clone();
    assert_ne!(new_head, Value::Null);
    assert_eq!(
        refusal["completed_through"], new_head,
        "the refusal must report the revision the mutation actually reached"
    );

    let refusal_path = directory.join("completed-through.refusal.json");
    fs::write(
        &refusal_path,
        serde_json::to_string_pretty(&refusal).expect("serialize refusal"),
    )
    .expect("write refusal fixture");
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/native-cli.refusal.schema.json"),
        &refusal_path,
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_evidence_attach_refuses_rather_than_breaking_an_aged_case_lock() {
    // ADR 0017 / issue #30: the tool never infers a live lock is abandoned
    // from file age alone. This test used to be
    // `stderr_stays_one_json_object_when_a_refusal_follows_a_broken_stale_lock`,
    // which forged a lock aged past the old 60s staleness threshold and
    // asserted it got *broken*, letting the append through to a second,
    // unrelated failure — before ADR 0017 that was correct behaviour; after
    // it, the exact same setup must refuse instead, which is what this test
    // now proves. It also keeps the property the old test was named for:
    // `stderr_json` panics if stderr is not exactly one line of valid JSON,
    // so a `LockUnavailable` refusal still only ever produces one JSON
    // object on stderr.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:aged-lock-base");
    let imported_json = stdout_json(&imported);
    let log_path = imported_native_log_path(&directory, &imported_json);
    let lock_path = log_path.with_file_name(".lock");
    let lock_contents = "token=forged-aged-lock\n";
    fs::write(&lock_path, lock_contents).expect("forge a lock file");
    fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .expect("open forged lock")
        .set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH))
        .expect("age forged lock far past the old 60s staleness threshold");

    let log_before = fs::read_to_string(&log_path).expect("read log before refused acquire");
    let revision_before = stdout_json(&run_native_case_store_command(&directory, "inspect"))
        ["result"]["record"]["current_revision_id"]
        .clone();

    let input_path = directory.join("attached-evidence-cell.json");
    let mut evidence_cell = json_file(native_case_fixture())["case_cells"][3].clone();
    evidence_cell["id"] = json!("evidence:aged-lock");
    evidence_cell["title"] = json!("Aged-lock evidence");
    evidence_cell["lifecycle"] = json!("active");
    evidence_cell["provenance"]["review_status"] = json!("unreviewed");
    evidence_cell["source_ids"] = json!(["source:aged-lock"]);
    evidence_cell["metadata"] =
        json!({"evidence_boundary": "source_backed", "content_hash": "caller-bogus-hash"});
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&evidence_cell).expect("serialize evidence cell"),
    )
    .expect("write evidence cell");

    // This waits out the real `LOCK_WAIT_BUDGET` (30s) on purpose, matching
    // `append_fails_while_case_lock_is_held_without_corrupting_history` in
    // `src/native_store/tests.rs`: shrinking that budget to make the test
    // faster would stop testing the timing this refusal depends on.
    let attach = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:aged-lock-base",
            "--input",
            input_path.to_str().expect("evidence path"),
            "--satisfies",
            "evidence:native-schema-json-valid",
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(!attach.status.success());

    // `stderr_json` itself is part of the assertion: it panics if stderr is
    // not exactly one line of valid JSON.
    let refusal = stderr_json(&attach);
    assert_eq!(refusal["error_code"], json!("lock_unavailable"));

    assert!(
        lock_path.exists(),
        "an aged lock must not be broken — recovery is an operator's own act, not an inference"
    );
    assert_eq!(
        fs::read_to_string(&lock_path).expect("read lock after refusal"),
        lock_contents,
        "the lock file must be byte-identical after a refusal"
    );

    let revision_after = stdout_json(&run_native_case_store_command(&directory, "inspect"))
        ["result"]["record"]["current_revision_id"]
        .clone();
    assert_eq!(
        revision_after, revision_before,
        "a refused lock acquisition must not move the store"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read log after refused acquire"),
        log_before,
        "a refused lock acquisition must not change the log"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_evidence_attach_batches_cells_and_coverage_in_one_revision() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:batch-evidence-base");
    let store = directory.to_str().expect("temp path").to_owned();

    // A coverage target must be an evidence cell, so the second requirement is
    // installed as an ordinary placeholder rather than aimed at a work or goal
    // cell — the fixture used to name a goal, which is what let coverage
    // discharge every requirement that cell had.
    let setup_path = directory.join("batch-second-requirement.case_morphism.json");
    write_json_value(
        &setup_path,
        &json!({
            "morphism_id": "morphism:batch-second-requirement",
            "morphism_type": "create",
            "source_revision_id": "revision:batch-evidence-base",
            "target_revision_id": "revision:batch-second-requirement",
            "added_ids": ["evidence:batch-second-requirement"],
            "updated_ids": [], "retired_ids": [], "preserved_ids": [],
            "evidence_ids": [], "source_ids": ["source:native-cli"],
            "violated_invariant_ids": [], "review_status": "unreviewed",
            "metadata": {"payload": {"added_cells": [{
                "id": "evidence:batch-second-requirement", "cell_type": "evidence",
                "lifecycle": "proposed",
                "space_id": json_file(native_case_fixture())["space_id"].clone(),
                "title": "Required: the second batch proof",
                "source_ids": ["source:native-cli"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            }], "added_relations": [], "updated_cells": [], "updated_relations": []}}
        }),
    );
    assert!(run_cli(&[
        "morphism",
        "propose",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--input",
        setup_path.to_str().expect("setup path"),
        "--format",
        "json",
    ])
    .status
    .success());
    assert!(run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:batch-second-requirement",
            "--base-revision-id",
            "revision:batch-evidence-base",
            "--reviewer-id",
            "reviewer:batch",
            "--reason",
            "install the second requirement placeholder",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    )
    .status
    .success());

    let first_path = directory.join("batch-first.evidence.json");
    let second_path = directory.join("batch-second.evidence.json");
    write_json_value(
        &first_path,
        &native_attached_evidence("evidence:batch-first", "unreviewed"),
    );
    write_json_value(
        &second_path,
        &native_attached_evidence("evidence:batch-second", "unreviewed"),
    );

    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:batch-second-requirement",
            "--input",
            first_path.to_str().expect("first evidence path"),
            "--satisfies",
            "evidence:native-schema-json-valid",
            "--input",
            second_path.to_str().expect("second evidence path"),
            "--satisfies",
            "evidence:batch-second-requirement",
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );

    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let entry = &stdout_json(&attached)["result"]["entry"];
    let payload = &entry["morphism"]["metadata"]["payload"];
    assert_eq!(
        entry["morphism"]["morphism_id"],
        json!("morphism:evidence-attach:evidence~3abatch-first:3")
    );
    assert_eq!(
        entry["morphism"]["target_revision_id"],
        json!("revision:evidence-attach:evidence~3abatch-first:3")
    );
    assert_eq!(
        entry["morphism"]["added_ids"],
        json!([
            "evidence:batch-first",
            "relation:evidence:evidence~3abatch-first:1",
            "evidence:batch-second",
            "relation:evidence:evidence~3abatch-second:1"
        ])
    );
    assert_eq!(
        entry["morphism"]["evidence_ids"],
        json!(["evidence:batch-first", "evidence:batch-second"])
    );
    assert_eq!(
        payload["added_cells"]
            .as_array()
            .expect("attached cells")
            .len(),
        2
    );
    assert_eq!(
        payload["added_relations"],
        json!([
            {
                "id": "relation:evidence:evidence~3abatch-first:1",
                "relation_type": "satisfies_evidence_requirement",
                "relation_strength": "diagnostic",
                "from_id": "evidence:batch-first",
                "to_id": "evidence:native-schema-json-valid",
                "evidence_ids": ["evidence:batch-first"],
                "source_ids": ["source:native-cli"],
                "provenance": payload["added_cells"][0]["provenance"],
                "metadata": {}
            },
            {
                "id": "relation:evidence:evidence~3abatch-second:1",
                "relation_type": "satisfies_evidence_requirement",
                "relation_strength": "diagnostic",
                "from_id": "evidence:batch-second",
                "to_id": "evidence:batch-second-requirement",
                "evidence_ids": ["evidence:batch-second"],
                "source_ids": ["source:native-cli"],
                "provenance": payload["added_cells"][1]["provenance"],
                "metadata": {}
            }
        ])
    );
    for (cell, path) in payload["added_cells"]
        .as_array()
        .expect("attached cells")
        .iter()
        .zip([&first_path, &second_path])
    {
        assert_eq!(cell["metadata"]["evidence_boundary"], json!("inferred"));
        assert_eq!(cell["metadata"]["content_hash"], json!(sha256_file(path)));
    }
    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        3,
        "genesis, the requirement setup, and one batch attach must be the whole log"
    );
    let validation = run_native_case_store_command(&directory, "validate");
    assert_eq!(
        stdout_json(&validation)["result"]["validation"]["valid"],
        json!(true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn evidence_attach_with_artifact_mints_a_cell_and_relation_kept_out_of_findings_and_frontier() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:artifact-attach-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let claim_path = directory.join("claim-with-artifact.evidence.json");
    write_json_value(
        &claim_path,
        &native_attached_evidence("evidence:claim-with-artifact", "unreviewed"),
    );
    let artifact_path = directory.join("build.log");
    fs::write(&artifact_path, b"a captured worker log\n").expect("write artifact file");

    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:artifact-attach-base",
            "--input",
            claim_path.to_str().expect("claim path"),
            "--satisfies",
            "evidence:native-schema-json-valid",
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let entry = &stdout_json(&attached)["result"]["entry"];
    let payload = &entry["morphism"]["metadata"]["payload"];
    let added_cells = payload["added_cells"].as_array().expect("added cells");
    assert_eq!(
        added_cells.len(),
        2,
        "the claim and the artifact it cites belong in the one evidence-attach morphism"
    );
    let content_hash = sha256_file(&artifact_path);
    let artifact_id = format!("artifact:sha256-{content_hash}");
    let artifact_cell = added_cells
        .iter()
        .find(|cell| cell["id"] == json!(artifact_id))
        .expect("artifact cell present in the payload");
    assert_eq!(artifact_cell["cell_type"], json!("custom:artifact"));
    assert_eq!(artifact_cell["lifecycle"], json!("resolved"));
    assert_eq!(
        artifact_cell["provenance"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(artifact_cell["title"], json!("build.log"));
    assert_eq!(artifact_cell["source_ids"], json!(["source:native-cli"]));
    assert_eq!(
        artifact_cell["metadata"]["content_hash"],
        json!(content_hash)
    );
    // The recorded uri is the canonicalized path that was actually hashed
    // (issue #21), not necessarily the string `--artifact` named — a temp
    // directory under a symlinked root (`/var` -> `/private/var` on macOS)
    // canonicalizes to a different string than it was given.
    let canonical_artifact_path =
        fs::canonicalize(&artifact_path).expect("canonicalize artifact path");
    assert_eq!(
        artifact_cell["metadata"]["artifact_uri"],
        json!(canonical_artifact_path
            .to_str()
            .expect("canonical artifact path"))
    );
    // Evidence-produced-by-this-morphism names the claim, not what it cites.
    assert_eq!(
        entry["morphism"]["evidence_ids"],
        json!(["evidence:claim-with-artifact"])
    );

    let derives_from = payload["added_relations"]
        .as_array()
        .expect("added relations")
        .iter()
        .find(|relation| relation["relation_type"] == json!("derives_from"))
        .expect("derives_from relation present in the payload");
    assert_eq!(
        derives_from["from_id"],
        json!("evidence:claim-with-artifact")
    );
    assert_eq!(derives_from["to_id"], json!(artifact_id));
    assert_eq!(derives_from["relation_strength"], json!("diagnostic"));

    let reason = run_native_case_store_command(&directory, "reason");
    let evaluation = &stdout_json(&reason)["result"]["evaluation"];
    let unreviewed_inference_ids = evaluation["evidence_findings"]["unreviewed_inference_ids"]
        .as_array()
        .expect("unreviewed inference ids");
    assert!(
        unreviewed_inference_ids.contains(&json!("evidence:claim-with-artifact")),
        "the claim is an inferred, unreviewed evidence cell"
    );
    assert!(
        !unreviewed_inference_ids.contains(&json!(artifact_id)),
        "the artifact is an observation, not a claim, so it must not surface as an \
         unreviewed-inference finding"
    );
    let frontier_cell_ids = evaluation["frontier_cell_ids"]
        .as_array()
        .expect("frontier cell ids");
    assert!(
        !frontier_cell_ids.contains(&json!(artifact_id)),
        "a resolved, non-evidence cell type still must not join the frontier"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn evidence_attach_dedupes_repeated_artifact_bytes_across_separate_attaches() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:artifact-dedupe-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let artifact_path = directory.join("shared.xcresult");
    fs::write(&artifact_path, b"the same captured bytes\n").expect("write artifact file");
    let content_hash = sha256_file(&artifact_path);
    let artifact_id = format!("artifact:sha256-{content_hash}");

    let first_claim_path = directory.join("first-claim.evidence.json");
    write_json_value(
        &first_claim_path,
        &native_attached_evidence("evidence:artifact-dedupe-first", "unreviewed"),
    );
    let first_attach = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:artifact-dedupe-base",
            "--input",
            first_claim_path.to_str().expect("first claim path"),
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(
        first_attach.status.success(),
        "stderr: {}",
        stderr(&first_attach)
    );
    let first_revision = stdout_json(&first_attach)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("first attach revision")
        .to_owned();
    assert!(
        stdout_json(&first_attach)["result"]["entry"]["morphism"]["metadata"]["payload"]
            ["added_cells"]
            .as_array()
            .expect("first added cells")
            .iter()
            .any(|cell| cell["id"] == json!(artifact_id))
    );

    let second_claim_path = directory.join("second-claim.evidence.json");
    write_json_value(
        &second_claim_path,
        &native_attached_evidence("evidence:artifact-dedupe-second", "unreviewed"),
    );
    let second_attach = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &first_revision,
            "--input",
            second_claim_path.to_str().expect("second claim path"),
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(
        second_attach.status.success(),
        "stderr: {}",
        stderr(&second_attach)
    );
    let second_payload =
        &stdout_json(&second_attach)["result"]["entry"]["morphism"]["metadata"]["payload"];
    assert_eq!(
        second_payload["added_cells"]
            .as_array()
            .expect("second added cells"),
        &vec![json!(second_payload["added_cells"][0])],
        "the second attach must add only its own claim, not another copy of the artifact"
    );
    assert_eq!(
        second_payload["added_cells"][0]["id"],
        json!("evidence:artifact-dedupe-second")
    );
    let second_derives_from = second_payload["added_relations"]
        .as_array()
        .expect("second added relations")
        .iter()
        .find(|relation| relation["relation_type"] == json!("derives_from"))
        .expect("second derives_from relation");
    assert_eq!(second_derives_from["to_id"], json!(artifact_id));

    let replay = run_native_case_store_command(&directory, "replay");
    let replayed_cells = stdout_json(&replay)["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells")
        .clone();
    assert_eq!(
        replayed_cells
            .iter()
            .filter(|cell| cell["id"] == json!(artifact_id))
            .count(),
        1,
        "only one artifact cell must ever exist for one content hash"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn morphism_propose_refuses_an_added_artifact_cell_and_a_derives_from_relation_into_one() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:artifact-propose-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let space_id = json_file(native_case_fixture())["space_id"].clone();

    let propose = |value: &Value, morphism_path: &Path| {
        write_json_value(morphism_path, value);
        run_cli(&[
            "morphism",
            "propose",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--input",
            morphism_path.to_str().expect("morphism path"),
            "--format",
            "json",
        ])
    };

    let forged_artifact_cell = json!({
        "id": "artifact:sha256-forged0000000000000000000000000000000000000000000000000000000",
        "cell_type": "custom:artifact",
        "lifecycle": "resolved",
        "space_id": space_id,
        "title": "Forged artifact",
        "source_ids": [],
        "structure_ids": [],
        "metadata": {
            "content_hash": "forged0000000000000000000000000000000000000000000000000000000",
            "artifact_uri": "nowhere"
        },
        "provenance": {
            "confidence": 1.0,
            "review_status": "unreviewed",
            "source": {"kind": "human", "title": "t"}
        }
    });
    let add_artifact_path = directory.join("add-artifact.case_morphism.json");
    let add_artifact = json!({
        "morphism_id": "morphism:forged-artifact-add",
        "morphism_type": "update",
        "source_revision_id": "revision:artifact-propose-base",
        "target_revision_id": "revision:forged-artifact-add",
        "added_ids": [forged_artifact_cell["id"]],
        "updated_ids": [], "retired_ids": [], "preserved_ids": [],
        "evidence_ids": [], "source_ids": ["source:native-cli"],
        "violated_invariant_ids": [], "review_status": "unreviewed",
        "metadata": {"payload": {"added_cells": [forged_artifact_cell]}}
    });
    let add_refused = propose(&add_artifact, &add_artifact_path);
    assert!(!add_refused.status.success());
    assert!(
        stderr(&add_refused).contains("custom:artifact cells enter only through evidence attach"),
        "stderr: {}",
        stderr(&add_refused)
    );

    // A real artifact to target with a forged `derives_from` relation.
    let claim_path = directory.join("propose-claim.evidence.json");
    write_json_value(
        &claim_path,
        &native_attached_evidence("evidence:artifact-propose-claim", "unreviewed"),
    );
    let artifact_path = directory.join("propose.log");
    fs::write(&artifact_path, b"artifact for a propose refusal test\n").expect("write artifact");
    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:artifact-propose-base",
            "--input",
            claim_path.to_str().expect("claim path"),
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();
    let artifact_id = format!("artifact:sha256-{}", sha256_file(&artifact_path));

    let forged_relation_path = directory.join("add-derives-from.case_morphism.json");
    let forged_relation = json!({
        "morphism_id": "morphism:forged-derives-from",
        "morphism_type": "relate",
        "source_revision_id": attached_revision,
        "target_revision_id": "revision:forged-derives-from",
        "added_ids": ["relation:forged-derives-from"],
        "updated_ids": [], "retired_ids": [], "preserved_ids": [],
        "evidence_ids": [], "source_ids": ["source:native-cli"],
        "violated_invariant_ids": [], "review_status": "unreviewed",
        "metadata": {"payload": {"added_relations": [{
            "id": "relation:forged-derives-from",
            "relation_type": "derives_from",
            "relation_strength": "diagnostic",
            "from_id": "evidence:artifact-propose-claim",
            "to_id": artifact_id,
            "evidence_ids": [],
            "source_ids": ["source:native-cli"],
            "metadata": {},
            "provenance": {"confidence": 1.0, "review_status": "unreviewed",
                           "source": {"kind": "human", "title": "t"}}
        }]}}
    });
    let relation_refused = propose(&forged_relation, &forged_relation_path);
    assert!(!relation_refused.status.success());
    assert!(
        stderr(&relation_refused).contains("a derives_from relation into artifact cell")
            && stderr(&relation_refused).contains("enters only through evidence attach"),
        "stderr: {}",
        stderr(&relation_refused)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn genesis_lift_refuses_a_snapshot_containing_an_artifact_cell() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let mut fixture = json_file(native_case_fixture());
    let space_id = fixture["space_id"].clone();
    fixture["case_cells"]
        .as_array_mut()
        .expect("case cells array")
        .push(json!({
            "id": "artifact:sha256-genesis00000000000000000000000000000000000000000000000000000",
            "cell_type": "custom:artifact",
            "lifecycle": "resolved",
            "space_id": space_id,
            "title": "Artifact smuggled through genesis",
            "source_ids": [],
            "structure_ids": [],
            "metadata": {
                "content_hash": "genesis00000000000000000000000000000000000000000000000000000",
                "artifact_uri": "nowhere"
            },
            "provenance": {
                "confidence": 1.0,
                "review_status": "unreviewed",
                "source": {"kind": "human", "title": "t"}
            }
        }));
    let fixture_path = directory.join("genesis-with-artifact.case.space.json");
    write_json_value(&fixture_path, &fixture);

    let lifted = run_cli(&[
        "lift",
        "native",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        fixture_path.to_str().expect("fixture path"),
        "--revision-id",
        "revision:genesis-with-artifact",
        "--format",
        "json",
    ]);
    assert!(!lifted.status.success());
    assert!(
        stderr(&lifted).contains("custom:artifact cells enter only through evidence attach"),
        "stderr: {}",
        stderr(&lifted)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cell_transition_and_review_accept_refuse_an_artifact_cell() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:artifact-review-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let claim_path = directory.join("review-claim.evidence.json");
    write_json_value(
        &claim_path,
        &native_attached_evidence("evidence:artifact-review-claim", "unreviewed"),
    );
    let artifact_path = directory.join("review.log");
    fs::write(&artifact_path, b"artifact for a review refusal test\n").expect("write artifact");
    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:artifact-review-base",
            "--input",
            claim_path.to_str().expect("claim path"),
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();
    let artifact_id = format!("artifact:sha256-{}", sha256_file(&artifact_path));

    let transitioned = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &attached_revision,
            "--cell-id",
            &artifact_id,
            "--to",
            "accepted",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!transitioned.status.success());
    assert!(
        stderr(&transitioned).contains("an artifact is an immutable observation"),
        "stderr: {}",
        stderr(&transitioned)
    );

    let reviewed = run_cli_with_mutation_gate(
        &[
            "review",
            "accept",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            &artifact_id,
            "--reviewer-id",
            "reviewer:human",
            "--reason",
            "trying to review the observation itself",
            "--base-revision-id",
            &attached_revision,
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!reviewed.status.success());
    assert!(
        stderr(&reviewed).contains("an observation, not a claim"),
        "stderr: {}",
        stderr(&reviewed)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn generic_update_morphism_touching_an_artifact_cell_is_refused() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:artifact-update-base");
    let store = directory.to_str().expect("temp path").to_owned();

    let claim_path = directory.join("update-claim.evidence.json");
    write_json_value(
        &claim_path,
        &native_attached_evidence("evidence:artifact-update-claim", "unreviewed"),
    );
    let artifact_path = directory.join("update.log");
    fs::write(&artifact_path, b"artifact for an update refusal test\n").expect("write artifact");
    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:artifact-update-base",
            "--input",
            claim_path.to_str().expect("claim path"),
            "--artifact",
            artifact_path.to_str().expect("artifact path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();
    let payload = &stdout_json(&attached)["result"]["entry"]["morphism"]["metadata"]["payload"];
    let artifact_id = format!("artifact:sha256-{}", sha256_file(&artifact_path));
    let mut updated_artifact_cell = payload["added_cells"]
        .as_array()
        .expect("added cells")
        .iter()
        .find(|cell| cell["id"] == json!(artifact_id))
        .expect("artifact cell")
        .clone();
    updated_artifact_cell["title"] = json!("Renamed after the fact");

    let update_path = directory.join("update-artifact.case_morphism.json");
    let update = json!({
        "morphism_id": "morphism:forged-artifact-update",
        "morphism_type": "update",
        "source_revision_id": attached_revision,
        "target_revision_id": "revision:forged-artifact-update",
        "added_ids": [], "retired_ids": [], "preserved_ids": [],
        "updated_ids": [artifact_id],
        "evidence_ids": [], "source_ids": ["source:native-cli"],
        "violated_invariant_ids": [], "review_status": "unreviewed",
        "metadata": {"payload": {"updated_cells": [updated_artifact_cell]}}
    });
    write_json_value(&update_path, &update);
    let proposed = run_cli(&[
        "morphism",
        "propose",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--input",
        update_path.to_str().expect("update morphism path"),
        "--format",
        "json",
    ]);
    assert!(!proposed.status.success());
    assert!(
        stderr(&proposed).contains("an artifact is an immutable observation"),
        "stderr: {}",
        stderr(&proposed)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn batch_coverage_is_read_per_evidence_by_the_evaluator() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:batch-coverage-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let setup_path = directory.join("batch-coverage-setup.case_morphism.json");
    write_json_value(
        &setup_path,
        &json!({
            "morphism_id": "morphism:batch-coverage-setup",
            "morphism_type": "create",
            "source_revision_id": "revision:batch-coverage-base",
            "target_revision_id": "revision:batch-coverage-setup",
            "added_ids": [
                "work:batch-coverage-a", "work:batch-coverage-b",
                "evidence:batch-requirement-a", "evidence:batch-requirement-b",
                "relation:batch-requires-a", "relation:batch-requires-b"
            ],
            "updated_ids": [], "retired_ids": [], "preserved_ids": [],
            "evidence_ids": [], "source_ids": ["source:native-cli"],
            "violated_invariant_ids": [], "review_status": "unreviewed",
            "metadata": {"payload": {
                "added_cells": [
                    native_work_cell("work:batch-coverage-a", "Batch coverage A"),
                    native_work_cell("work:batch-coverage-b", "Batch coverage B"),
                    native_evidence_requirement(
                        "evidence:batch-requirement-a",
                        "Batch requirement A"
                    ),
                    native_evidence_requirement(
                        "evidence:batch-requirement-b",
                        "Batch requirement B"
                    )
                ],
                "added_relations": [
                    native_requires_evidence_relation(
                        "relation:batch-requires-a",
                        "work:batch-coverage-a",
                        "evidence:batch-requirement-a"
                    ),
                    native_requires_evidence_relation(
                        "relation:batch-requires-b",
                        "work:batch-coverage-b",
                        "evidence:batch-requirement-b"
                    )
                ],
                "updated_cells": [], "updated_relations": []
            }}
        }),
    );
    let proposed = run_cli(&[
        "morphism",
        "propose",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--input",
        setup_path.to_str().expect("setup path"),
        "--format",
        "json",
    ]);
    assert!(proposed.status.success(), "stderr: {}", stderr(&proposed));
    let applied = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:batch-coverage-setup",
            "--base-revision-id",
            "revision:batch-coverage-base",
            "--reviewer-id",
            "reviewer:batch-coverage",
            "--reason",
            "install two independent evidence requirements",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(applied.status.success(), "stderr: {}", stderr(&applied));

    let first_path = directory.join("coverage-a.evidence.json");
    let second_path = directory.join("coverage-b.evidence.json");
    write_json_value(
        &first_path,
        &native_attached_evidence("evidence:batch-coverage-a", "unreviewed"),
    );
    write_json_value(
        &second_path,
        &native_attached_evidence("evidence:batch-coverage-b", "unreviewed"),
    );
    let attached = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:batch-coverage-setup",
            "--input",
            first_path.to_str().expect("first evidence path"),
            "--satisfies",
            "evidence:batch-requirement-a",
            "--input",
            second_path.to_str().expect("second evidence path"),
            "--satisfies",
            "evidence:batch-requirement-b",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();
    let accepted = run_cli_with_mutation_gate(
        &[
            "review",
            "accept",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--target-id",
            "evidence:batch-coverage-a",
            "--reviewer-id",
            "reviewer:batch-coverage",
            "--reason",
            "reviewed only evidence A",
            "--base-revision-id",
            &attached_revision,
            "--evidence-id",
            "evidence:batch-coverage-a",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(accepted.status.success(), "stderr: {}", stderr(&accepted));
    assert_eq!(
        stdout_json(&accepted)["result"]["activated_coverage"],
        json!(["evidence:batch-requirement-a"])
    );

    let listed = run_cli(&[
        "obstruction",
        "list",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));
    let missing_witnesses = stdout_json(&listed)["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .filter(|obstruction| obstruction["obstruction_type"] == json!("missing_evidence"))
        .flat_map(|obstruction| {
            obstruction["witness_ids"]
                .as_array()
                .expect("witness ids")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(!missing_witnesses.contains(&"evidence:batch-requirement-a".to_owned()));
    assert!(missing_witnesses.contains(&"evidence:batch-requirement-b".to_owned()));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn batch_evidence_attach_refuses_everything_when_the_second_input_is_invalid() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:batch-refusal-base");
    let first_path = directory.join("valid-first.evidence.json");
    let second_path = directory.join("invalid-second.evidence.json");
    write_json_value(
        &first_path,
        &native_attached_evidence("evidence:valid-first", "unreviewed"),
    );
    fs::write(&second_path, b"{").expect("write invalid evidence");

    let attach = |second_target: &str| {
        run_cli_with_mutation_gate(
            &[
                "evidence",
                "attach",
                "--store",
                directory.to_str().expect("temp path"),
                "--case-space-id",
                native_case_space_id(),
                "--base-revision-id",
                "revision:batch-refusal-base",
                "--input",
                first_path.to_str().expect("first evidence path"),
                "--satisfies",
                "evidence:native-schema-json-valid",
                "--input",
                second_path.to_str().expect("second evidence path"),
                "--satisfies",
                second_target,
                "--format",
                "json",
            ],
            "actor:native-evidence-cli",
        )
    };
    let refused = attach("evidence:native-schema-json-valid");

    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains(second_path.to_str().expect("second evidence path")),
        "stderr: {}",
        stderr(&refused)
    );
    write_json_value(
        &second_path,
        &native_attached_evidence("relation:evidence:evidence~3avalid-first:1", "unreviewed"),
    );
    let collision = attach("evidence:native-schema-json-valid");
    assert!(!collision.status.success());
    assert!(
        stderr(&collision).contains(second_path.to_str().expect("second evidence path")),
        "stderr: {}",
        stderr(&collision)
    );
    write_json_value(
        &second_path,
        &native_attached_evidence("evidence:valid-second", "unreviewed"),
    );
    let non_cell_target = attach("relation:case-covers-goal");
    assert!(!non_cell_target.status.success());
    assert!(
        stderr(&non_cell_target).contains(second_path.to_str().expect("second evidence path")),
        "stderr: {}",
        stderr(&non_cell_target)
    );
    let inspect = run_native_case_store_command(&directory, "inspect");
    assert_eq!(
        stdout_json(&inspect)["result"]["record"]["current_revision_id"],
        json!("revision:batch-refusal-base")
    );
    let history = run_native_case_store_command(&directory, "history");
    assert_eq!(
        stdout_json(&history)["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn evidence_attach_refuses_satisfies_before_any_input() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:satisfies-order-base");
    let input_path = directory.join("ordered.evidence.json");
    write_json_value(
        &input_path,
        &native_attached_evidence("evidence:ordered", "unreviewed"),
    );

    let refused = run_cli_with_mutation_gate(
        &[
            "evidence",
            "attach",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:satisfies-order-base",
            "--satisfies",
            "goal:native-case-contract",
            "--input",
            input_path.to_str().expect("evidence path"),
            "--format",
            "json",
        ],
        "actor:native-evidence-cli",
    );

    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("--satisfies must follow the --input it belongs to"),
        "stderr: {}",
        stderr(&refused)
    );
    let inspect = run_native_case_store_command(&directory, "inspect");
    assert_eq!(
        stdout_json(&inspect)["result"]["record"]["current_revision_id"],
        json!("revision:satisfies-order-base")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn evidence_attach_appends_exactly_one_revision_or_none_for_any_input_list() {
    arbtest::arbtest(
        |u: &mut arbtest::arbitrary::Unstructured<'_>| -> arbtest::arbitrary::Result<()> {
            let selectors = Vec::<u8>::arbitrary(u)?;
            let selectors = selectors.into_iter().take(4).collect::<Vec<_>>();
            let directory = unique_temp_dir();
            fs::create_dir_all(&directory).expect("create temp directory");
            import_native_case_space(&directory, "revision:batch-property-base");
            let mut args = vec![
                "evidence".to_owned(),
                "attach".to_owned(),
                "--store".to_owned(),
                directory.to_string_lossy().into_owned(),
                "--case-space-id".to_owned(),
                native_case_space_id().to_owned(),
                "--base-revision-id".to_owned(),
                "revision:batch-property-base".to_owned(),
            ];
            let mut every_input_valid = !selectors.is_empty();
            for (index, selector) in selectors.iter().enumerate() {
                let path = directory.join(format!("property-{index}.evidence.json"));
                let target = match selector % 4 {
                    0 => {
                        fs::write(&path, b"{").expect("write invalid JSON");
                        every_input_valid = false;
                        "evidence:native-schema-json-valid"
                    }
                    1 => {
                        write_json_value(
                            &path,
                            &native_attached_evidence(
                                &format!("evidence:property-{index}"),
                                "unreviewed",
                            ),
                        );
                        every_input_valid = false;
                        "evidence:does-not-exist"
                    }
                    2 => {
                        write_json_value(
                            &path,
                            &native_attached_evidence(
                                &format!("evidence:property-{index}"),
                                "accepted",
                            ),
                        );
                        every_input_valid = false;
                        "evidence:native-schema-json-valid"
                    }
                    _ => {
                        write_json_value(
                            &path,
                            &native_attached_evidence(
                                &format!("evidence:property-{index}"),
                                "unreviewed",
                            ),
                        );
                        "evidence:native-schema-json-valid"
                    }
                };
                args.extend([
                    "--input".to_owned(),
                    path.to_string_lossy().into_owned(),
                    "--satisfies".to_owned(),
                    target.to_owned(),
                ]);
            }
            args.extend([
                "--actor-id".to_owned(),
                "actor:native-evidence-cli".to_owned(),
                "--capability-id".to_owned(),
                "capability:durable-mutation".to_owned(),
                "--operation-scope-id".to_owned(),
                native_case_space_id().to_owned(),
                "--audience".to_owned(),
                "audit".to_owned(),
                "--source-boundary-id".to_owned(),
                "source_boundary:native-case-management-contract".to_owned(),
                "--format".to_owned(),
                "json".to_owned(),
            ]);

            let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
                .args(&args)
                .output()
                .expect("run property evidence attach");
            assert_eq!(
                output.status.success(),
                every_input_valid,
                "stderr: {}",
                stderr(&output)
            );
            let history = run_native_case_store_command(&directory, "history");
            let appended = stdout_json(&history)["result"]["entries"]
                .as_array()
                .expect("history entries")
                .len()
                - 1;
            assert_eq!(appended, usize::from(every_input_valid));
            let inspect = run_native_case_store_command(&directory, "inspect");
            let current_revision =
                &stdout_json(&inspect)["result"]["record"]["current_revision_id"];
            if every_input_valid {
                assert_ne!(current_revision, &json!("revision:batch-property-base"));
            } else {
                assert_eq!(current_revision, &json!("revision:batch-property-base"));
            }

            fs::remove_dir_all(directory).expect("remove temp directory");
            Ok(())
        },
    )
    .budget_ms(1_200)
    .size_max(32);
}

#[test]
fn gate_profile_supplies_all_fields_and_records_only_expanded_values() {
    let directory = setup_profiled_evidence_attach("all-fields");
    let profile_name = "audit-profile-name-must-not-be-recorded";
    let profile_path = write_gate_profiles(
        &directory,
        json!([full_gate_profile(
            profile_name,
            "actor:native-evidence-cli",
            "capability:durable-mutation"
        )]),
    );

    let attach = run_profiled_evidence_attach(&directory, &profile_path, profile_name, &[]);

    assert!(attach.status.success(), "stderr: {}", stderr(&attach));
    let gate = replayed_attached_evidence_gate(&directory);
    assert_eq!(
        gate,
        json!({
            "actor_id": "actor:native-evidence-cli",
            "operation": "evidence-attach",
            "operation_scope_id": native_case_space_id(),
            "audience": "audit",
            "capability_ids": ["capability:durable-mutation"],
            "source_boundary_id": "source_boundary:native-case-management-contract"
        })
    );
    assert!(!serde_json::to_string(&gate)
        .expect("serialize recorded gate")
        .contains(profile_name));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn explicit_gate_flag_overrides_the_profile_for_that_field() {
    let directory = setup_profiled_evidence_attach("flag-wins");
    let profile_path = write_gate_profiles(
        &directory,
        json!([full_gate_profile(
            "override",
            "actor:native-mutation-cli",
            "capability:durable-mutation"
        )]),
    );

    let attach = run_profiled_evidence_attach(
        &directory,
        &profile_path,
        "override",
        &["--actor-id", "actor:native-evidence-cli"],
    );

    assert!(attach.status.success(), "stderr: {}", stderr(&attach));
    assert_eq!(
        replayed_attached_evidence_gate(&directory)["actor_id"],
        json!("actor:native-evidence-cli")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn profile_nonexistent_capability_is_refused_exactly_like_the_flag() {
    let directory = setup_profiled_evidence_attach("bad-capability");
    let profile_path = write_gate_profiles(
        &directory,
        json!([full_gate_profile(
            "bad-capability",
            "actor:native-evidence-cli",
            "capability:not-present"
        )]),
    );

    let from_profile =
        run_profiled_evidence_attach(&directory, &profile_path, "bad-capability", &[]);
    let from_flags = run_profiled_evidence_attach(
        &directory,
        &profile_path,
        "bad-capability",
        &[
            "--capability-id",
            "capability:not-present",
            "--actor-id",
            "actor:native-evidence-cli",
            "--operation-scope-id",
            native_case_space_id(),
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:native-case-management-contract",
        ],
    );

    assert!(!from_profile.status.success());
    assert!(!from_flags.status.success());
    assert_eq!(stderr(&from_profile), stderr(&from_flags));
    assert!(stderr(&from_profile)
        .contains("capability capability:not-present does not resolve to an existing case cell"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn partial_gate_profile_combines_with_remaining_flags() {
    let directory = setup_profiled_evidence_attach("partial");
    let profile_path = write_gate_profiles(
        &directory,
        json!([{
            "name": "partial",
            "actor_id": "actor:native-evidence-cli",
            "capability_ids": ["capability:durable-mutation"]
        }]),
    );

    let attach = run_profiled_evidence_attach(
        &directory,
        &profile_path,
        "partial",
        &[
            "--operation-scope-id",
            native_case_space_id(),
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:native-case-management-contract",
        ],
    );

    assert!(attach.status.success(), "stderr: {}", stderr(&attach));
    assert_eq!(
        replayed_attached_evidence_gate(&directory)["capability_ids"],
        json!(["capability:durable-mutation"])
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn gate_field_missing_from_both_profile_and_flags_is_refused() {
    let directory = setup_profiled_evidence_attach("missing");
    let profile_path = write_gate_profiles(
        &directory,
        json!([{
            "name": "missing-actor",
            "capability_ids": ["capability:durable-mutation"],
            "operation_scope_id": native_case_space_id(),
            "audience": "audit",
            "source_boundary_id": "source_boundary:native-case-management-contract"
        }]),
    );

    let attach = run_profiled_evidence_attach(&directory, &profile_path, "missing-actor", &[]);

    assert!(!attach.status.success());
    assert!(stderr(&attach).contains("--actor-id <id> is required for evidence attach"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn gate_profile_file_strictly_refuses_unknown_fields_through_the_binary() {
    let directory = setup_profiled_evidence_attach("unknown-field");
    let profile_path = write_gate_profiles(
        &directory,
        json!([{
            "name": "unknown-field",
            "actor_id": "actor:native-evidence-cli",
            "capability_ids": ["capability:durable-mutation"],
            "operation_scope_id": native_case_space_id(),
            "audience": "audit",
            "source_boundary_id": "source_boundary:native-case-management-contract",
            "trusted": true
        }]),
    );

    let attach = run_profiled_evidence_attach(&directory, &profile_path, "unknown-field", &[]);

    assert!(!attach.status.success());
    assert!(stderr(&attach).contains("unknown field `trusted`"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

fn setup_profiled_evidence_attach(label: &str) -> PathBuf {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-evidence-base");
    let input_path = directory.join("profiled-evidence-cell.json");
    let mut evidence_cell = json_file(native_case_fixture())["case_cells"][3].clone();
    evidence_cell["id"] = json!(format!("evidence:profile-{label}"));
    evidence_cell["title"] = json!(format!("Profile evidence {label}"));
    evidence_cell["lifecycle"] = json!("active");
    evidence_cell["provenance"]["review_status"] = json!("unreviewed");
    evidence_cell["source_ids"] = json!([format!("source:profile-{label}")]);
    evidence_cell["metadata"] = json!({"evidence_boundary": "source_backed"});
    fs::write(
        input_path,
        serde_json::to_string_pretty(&evidence_cell).expect("serialize evidence cell"),
    )
    .expect("write evidence cell");
    directory
}

fn full_gate_profile(name: &str, actor_id: &str, capability_id: &str) -> Value {
    json!({
        "name": name,
        "actor_id": actor_id,
        "capability_ids": [capability_id],
        "operation_scope_id": native_case_space_id(),
        "audience": "audit",
        "source_boundary_id": "source_boundary:native-case-management-contract"
    })
}

fn write_gate_profiles(directory: &Path, profiles: Value) -> PathBuf {
    let path = directory.join("operation-gate-profiles.json");
    let document = json!({
        "schema": "highergraphen.case.operation_gate_profiles.v1",
        "schema_version": 1,
        "profiles": profiles
    });
    fs::write(
        &path,
        serde_json::to_string_pretty(&document).expect("serialize gate profiles"),
    )
    .expect("write gate profiles");
    path
}

fn run_profiled_evidence_attach(
    directory: &Path,
    profile_path: &Path,
    profile_name: &str,
    gate_flags: &[&str],
) -> Output {
    let input_path = directory.join("profiled-evidence-cell.json");
    let mut args = vec![
        "evidence".to_owned(),
        "attach".to_owned(),
        "--store".to_owned(),
        directory.display().to_string(),
        "--case-space-id".to_owned(),
        native_case_space_id().to_owned(),
        "--base-revision-id".to_owned(),
        "revision:native-evidence-base".to_owned(),
        "--input".to_owned(),
        input_path.display().to_string(),
        "--gate-profile".to_owned(),
        profile_name.to_owned(),
        "--gate-profile-file".to_owned(),
        profile_path.display().to_string(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    args.extend(gate_flags.iter().map(|argument| (*argument).to_owned()));
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run profiled evidence attach")
}

fn replayed_attached_evidence_gate(directory: &Path) -> Value {
    let replay = stdout_json(&run_native_case_store_command(directory, "replay"));
    replay["result"]["replay"]["case_space"]["morphism_log"]
        .as_array()
        .expect("morphism log")
        .iter()
        .rev()
        .find(|entry| entry["morphism"]["morphism_type"] == json!("evidence_attach"))
        .expect("evidence attach entry")["morphism"]["metadata"]["operation_gate"]
        .clone()
}

#[test]
fn native_cell_transition_delegates_lifecycle_legality_to_reducer() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-transition-base");
    let original_goal = json_file(native_case_fixture())["case_cells"][0].clone();

    let resolve_args = [
        "cell",
        "transition",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--base-revision-id",
        "revision:native-transition-base",
        "--cell-id",
        "goal:native-case-contract",
        "--to",
        "resolved",
        "--reason",
        "The goal is complete",
        "--format",
        "json",
    ];
    let ungated_resolve = run_cli(&resolve_args);
    assert!(!ungated_resolve.status.success());
    assert!(stderr(&ungated_resolve).contains("--actor-id <id> is required for cell transition"));

    let resolve = run_cli_with_mutation_gate(&resolve_args, "actor:native-transition-cli");
    assert!(resolve.status.success(), "stderr: {}", stderr(&resolve));
    let resolve_json = stdout_json(&resolve);
    assert_eq!(
        resolve_json["metadata"]["command"],
        json!("casegraphen cell transition")
    );
    assert_eq!(
        resolve_json["result"]["entry"]["actor_id"],
        json!("actor:native-transition-cli")
    );
    assert_eq!(
        resolve_json["result"]["entry"]["morphism"]["metadata"]["operation_gate"]["operation"],
        json!("cell-transition")
    );
    assert_eq!(
        resolve_json["result"]["entry"]["morphism"]["metadata"]["transition"],
        json!({
            "from": "active",
            "to": "resolved",
            "reason": "The goal is complete"
        })
    );
    let mut expected_goal = original_goal;
    expected_goal["lifecycle"] = json!("resolved");
    assert_eq!(
        resolve_json["result"]["entry"]["morphism"]["metadata"]["payload"]["updated_cells"][0],
        expected_goal
    );
    let resolved_revision = resolve_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("resolved revision")
        .to_owned();

    let retire = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &resolved_revision,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "retired",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(retire.status.success(), "stderr: {}", stderr(&retire));
    let retire_json = stdout_json(&retire);
    let retired_revision = retire_json["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("retired revision")
        .to_owned();
    assert_eq!(
        retire_json["result"]["entry"]["morphism"]["metadata"]["transition"]["reason"],
        Value::Null
    );

    let capability_transition = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &retired_revision,
            "--cell-id",
            "capability:durable-mutation",
            "--to",
            "retired",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(!capability_transition.status.success());
    assert!(stderr(&capability_transition).contains(
        "cannot update capability cell capability:durable-mutation: custom:capability cells are \
         administered only at lift/import time inside the declared source boundary"
    ));

    let illegal = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &retired_revision,
            "--cell-id",
            "goal:native-case-contract",
            "--to",
            "active",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(!illegal.status.success());
    assert!(stderr(&illegal).contains(
        "cannot transition cell goal:native-case-contract lifecycle from Retired to Active"
    ));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_morphism_check_uses_metadata_core_extensions_as_applicability_gate() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let morphism_path = directory.join("blocked.case_morphism.json");
    write_native_metadata_morphism_with_metadata(
        &morphism_path,
        "morphism:native-cli-coreext-blocked",
        "revision:native-cli-imported",
        "revision:native-cli-coreext-blocked",
        json!({
            "higher_graphen_extensions": invalid_core_extensions("morphism:native-cli-coreext-blocked")
        }),
    );

    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));

    let check = run_cli(&[
        "morphism",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-cli-coreext-blocked",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    let value = stdout_json(&check);
    assert_eq!(value["result"]["valid"], json!(true));
    assert_eq!(value["result"]["applicable"], json!(false));
    assert_eq!(
        value["result"]["core_extensions"]["validation"]["blocked_count"],
        json!(1)
    );
    let temporal_checks = value["result"]["mathematical_diagnostics"]["temporal_checks"]
        .as_array()
        .expect("temporal checks");
    assert!(temporal_checks.iter().any(|check| check["id"]
        == json!(
            "temporal:morphism-transition-eventual:case_space:native-case-management-contract"
        )
        && check["report"]["status"] == json!("satisfied")));
    assert!(temporal_checks.iter().any(|check| check["id"]
        == json!("temporal:morphism-target-terminal:case_space:native-case-management-contract")
        && check["report"]["status"] == json!("satisfied")));
}

#[test]
fn native_morphism_check_rejects_unsupported_proposal_version() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let morphism_path = directory.join("versioned.case_morphism.json");
    write_native_metadata_morphism(
        &morphism_path,
        "morphism:native-cli-versioned",
        "revision:native-cli-imported",
        "revision:native-cli-versioned",
    );
    let propose = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));

    let proposal_path = directory.join(
        stdout_json(&propose)["result"]["proposal_path"]
            .as_str()
            .expect("proposal path"),
    );
    let mut proposal = json_file(proposal_path.clone());
    proposal["schema_version"] = json!(2);
    fs::write(
        &proposal_path,
        serde_json::to_string_pretty(&proposal).expect("serialize proposal"),
    )
    .expect("write proposal");

    let check = run_cli(&[
        "morphism",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:native-cli-versioned",
        "--format",
        "json",
    ]);
    assert!(!check.status.success());
    assert!(stdout(&check).is_empty());
    assert!(stderr(&check).contains("unsupported proposal schema version"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_cli_invalid_targets_exit_nonzero() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let blank_title = run_cli(&[
        "space",
        "new",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        "case_space:blank-title",
        "--space-id",
        "space:blank-title",
        "--title",
        "   ",
        "--revision-id",
        "revision:blank-title",
        "--format",
        "json",
    ]);
    assert!(!blank_title.status.success());
    assert!(stderr(&blank_title).contains("case title must not be empty"));

    let missing_case = run_cli(&[
        "space",
        "inspect",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        "case_space:does-not-exist",
        "--format",
        "json",
    ]);
    assert!(!missing_case.status.success());
    assert!(stdout(&missing_case).is_empty());
    assert!(stderr(&missing_case).contains("missing native case space"));

    let missing_morphism = run_cli(&[
        "morphism",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--morphism-id",
        "morphism:does-not-exist",
        "--format",
        "json",
    ]);
    assert!(!missing_morphism.status.success());
    assert!(stdout(&missing_morphism).is_empty());
    assert!(stderr(&missing_morphism).contains("No such file"));

    let stale_morphism_path = directory.join("stale.case_morphism.json");
    write_native_metadata_morphism(
        &stale_morphism_path,
        "morphism:native-cli-stale",
        "revision:stale",
        "revision:native-cli-stale-target",
    );
    let stale = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        stale_morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(!stale.status.success());
    assert!(stderr(&stale).contains("does not match current revision"));

    let apply_morphism_path = directory.join("missing-review.case_morphism.json");
    write_native_metadata_morphism(
        &apply_morphism_path,
        "morphism:native-cli-missing-review",
        "revision:native-cli-imported",
        "revision:native-cli-missing-review",
    );
    let propose_apply = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        apply_morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(
        propose_apply.status.success(),
        "stderr: {}",
        stderr(&propose_apply)
    );
    let missing_apply_reason = run_cli_with_mutation_gate(
        &[
            "morphism",
            "apply",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:native-cli-missing-review",
            "--base-revision-id",
            "revision:native-cli-imported",
            "--reviewer-id",
            "reviewer:native-cli",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!missing_apply_reason.status.success());
    assert!(stderr(&missing_apply_reason).contains("--reason <text> is required"));

    let reject_morphism_path = directory.join("same-revision-reject.case_morphism.json");
    write_native_metadata_morphism(
        &reject_morphism_path,
        "morphism:native-cli-same-revision-reject",
        "revision:native-cli-imported",
        "revision:native-cli-reject-target",
    );
    let propose_reject = run_cli(&[
        "morphism",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        reject_morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(
        propose_reject.status.success(),
        "stderr: {}",
        stderr(&propose_reject)
    );
    let same_revision_reject = run_cli_with_mutation_gate(
        &[
            "morphism",
            "reject",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--morphism-id",
            "morphism:native-cli-same-revision-reject",
            "--reviewer-id",
            "reviewer:native-cli",
            "--reason",
            "Reject without advancing revision",
            "--revision-id",
            "revision:native-cli-imported",
            "--format",
            "json",
        ],
        "actor:native-mutation-cli",
    );
    assert!(!same_revision_reject.status.success());
    assert!(stderr(&same_revision_reject).contains("must advance the revision"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lift_native_derives_the_genesis_materialization_from_the_authored_state() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    // An author writes the state and a genesis stub: who, from what, under
    // which boundary. The reconstructive copy — payload, added_ids, and the
    // immutable shell — is the tool's to derive. Documenting it as the author's
    // job once cost a reader a hand-written generator for 30 cells and 28
    // relations, so this pins that a stub is enough.
    let mut genesis: Value = serde_json::from_str(
        &fs::read_to_string(repo_path(
            "docs/guides/release-decision/genesis.case.space.json",
        ))
        .expect("read the walkthrough genesis"),
    )
    .expect("walkthrough genesis parses");
    let stub_metadata = genesis["morphism_log"][0]["morphism"]["metadata"]
        .as_object_mut()
        .expect("genesis morphism metadata");
    stub_metadata.remove("payload");
    stub_metadata.remove("genesis_case_space");
    genesis["morphism_log"][0]["morphism"]["added_ids"] = json!([]);
    let stub_path = directory.join("genesis-stub.case.space.json");
    fs::write(
        &stub_path,
        serde_json::to_vec_pretty(&genesis).expect("serialize the stub"),
    )
    .expect("write the stub");

    let output = run_cli(&[
        "lift",
        "native",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        stub_path.to_str().expect("stub path"),
        "--revision-id",
        "revision:genesis-stub",
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let lifted = stdout_json(&output);
    let case_space = &lifted["result"]["case_space"];
    let genesis_morphism = &case_space["morphism_log"][0]["morphism"];
    assert_eq!(
        genesis_morphism["metadata"]["payload"]["added_cells"],
        case_space["case_cells"]
    );
    assert_eq!(
        genesis_morphism["metadata"]["payload"]["added_relations"],
        case_space["case_relations"]
    );
    assert_eq!(
        genesis_morphism["added_ids"]
            .as_array()
            .expect("derived added ids")
            .len(),
        case_space["case_cells"]
            .as_array()
            .expect("case cells")
            .len()
            + case_space["case_relations"]
                .as_array()
                .expect("case relations")
                .len()
    );
    assert_eq!(
        genesis_morphism["metadata"]["genesis_case_space"]["space_id"],
        case_space["space_id"]
    );

    // Derived is not the same as adequate: delete the snapshot so the fold has
    // to run from an empty case space against the derived genesis alone.
    let relative_snapshot = lifted["result"]["record"]["nearest_snapshot_path"]
        .as_str()
        .expect("nearest snapshot path");
    fs::remove_file(directory.join(relative_snapshot)).expect("delete nearest snapshot");
    let case_space_id = case_space["case_space_id"].as_str().expect("case space id");
    let rebuild = run_cli(&[
        "space",
        "rebuild",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
    ]);
    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));
    let validation = run_cli(&[
        "space",
        "validate",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
    ]);
    assert_eq!(
        stdout_json(&validation)["result"]["validation"]["valid"],
        json!(true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn rebuild_repairs_a_head_that_lags_the_log_and_refuses_one_that_does_not() {
    // A crash between the log append and the head write — Ctrl-C is enough,
    // reproduced 9/9 — left the head one entry behind, and every command then
    // refused, including `space rebuild --adopt-existing-log`, the documented
    // recovery. The only thing that worked was deleting the head by hand,
    // which is the primitive residual risk 2 calls an untraceable rollback and
    // is indistinguishable from one afterwards.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let imported = import_native_case_space(&directory, "revision:lagging-head-base");
    let store = directory.to_str().expect("temp path").to_owned();
    let log_path = imported_native_log_path(&directory, &stdout_json(&imported));
    let head_path = log_path.with_file_name("morphism_log.head.json");
    let genesis_head = fs::read_to_string(&head_path).expect("read genesis head");

    // Append a second entry the honest way, then put the head back to genesis:
    // exactly the state a crash between the two writes leaves.
    let transitioned = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            "revision:lagging-head-base",
            "--cell-id",
            "work:review-native-contract",
            "--to",
            "active",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(
        transitioned.status.success(),
        "stderr: {}",
        stderr(&transitioned)
    );
    let current_head = fs::read_to_string(&head_path).expect("read advanced head");
    fs::write(&head_path, &genesis_head).expect("rewind the head");

    let refused_read = run_cli(&[
        "space",
        "validate",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        !refused_read.status.success(),
        "a lagging head must still stop every read"
    );

    let repaired = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(repaired.status.success(), "stderr: {}", stderr(&repaired));
    assert_eq!(
        stdout_json(&repaired)["result"]["rebuild"]["head_adopted"],
        json!(true)
    );
    assert_eq!(
        fs::read_to_string(&head_path).expect("read repaired head"),
        current_head,
        "the repaired head must name the log's tail"
    );
    for operation in ["validate", "inspect", "replay"] {
        let read = run_cli(&[
            "space",
            operation,
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--format",
            "json",
        ]);
        assert!(
            read.status.success(),
            "space {operation} stderr: {}",
            stderr(&read)
        );
    }

    // A lag of more than one is not a crash signature either: append_morphism
    // holds the case lock across exactly one append and one head write, so a
    // crash leaves the head behind by one entry and never more. Two entries
    // appended with the head held back must refuse, or the repair is wider
    // than the only thing that produces the state it repairs.
    let advanced_head = fs::read_to_string(&head_path).expect("read head after repair");
    for cell in ["work:review-native-contract", "work:review-native-contract"] {
        let revision = stdout_json(&run_cli(&[
            "space",
            "inspect",
            "--store",
            &store,
            "--case-space-id",
            native_case_space_id(),
            "--format",
            "json",
        ]))["result"]["record"]["current_revision_id"]
            .as_str()
            .expect("current revision")
            .to_owned();
        let stepped = run_cli_with_mutation_gate(
            &[
                "cell",
                "transition",
                "--store",
                &store,
                "--case-space-id",
                native_case_space_id(),
                "--base-revision-id",
                &revision,
                "--cell-id",
                cell,
                "--to",
                "active",
                "--format",
                "json",
            ],
            "actor:native-transition-cli",
        );
        assert!(stepped.status.success(), "stderr: {}", stderr(&stepped));
    }
    fs::write(&head_path, &advanced_head).expect("hold the head two entries back");
    let over_lagged = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        !over_lagged.status.success(),
        "a head two entries behind is not a crash signature and must be refused"
    );
    // Put it back where the repair left it so the rollback case below starts
    // from a readable store.
    let entries_now = fs::read_to_string(&log_path).expect("read log");
    let tail = entries_now
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    assert!(tail >= 4, "expected the two extra appends to be in the log");

    // A head *ahead* of the log is the rollback signature, not a crash, and it
    // is what residual risk 2 exists to catch. Truncating the log to produce
    // it must keep refusing.
    let entries = fs::read_to_string(&log_path).expect("read log");
    let kept = entries
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(1)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&log_path, format!("{kept}\n")).expect("truncate the log");
    let rolled_back = run_cli(&[
        "space",
        "rebuild",
        "--adopt-existing-log",
        "--store",
        &store,
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(
        !rolled_back.status.success(),
        "a head ahead of the log is a rollback and must be refused"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn coverage_can_only_be_aimed_at_an_evidence_cell() {
    // The evaluator treats coverage recorded against a work cell as satisfying
    // every evidence and proof requirement that cell has, so `--satisfies` on a
    // work cell discharged requirements no morphism named and no reviewer saw.
    // Reproduced against the walkthrough store: one attach aimed at
    // work:tag-release plus one review accept cleared a blocking hard
    // requirement on evidence:changelog-updated. `run --step` already answered
    // this question with "evidence cells only"; both now read one rule.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let store = directory.to_str().expect("temp path").to_owned();
    import_native_case_space_from_input(
        &directory,
        &repo_path("docs/guides/release-decision/genesis.case.space.json"),
        "revision:coverage-target",
    );
    let case_space_id = "case_space:casegraphen-release-0-9-0";
    let evidence_path = directory.join("aimed.evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&json!({
            "id": "evidence:aimed", "cell_type": "evidence", "lifecycle": "active",
            "space_id": "space:casegraphen", "title": "my own uploaded file",
            "source_ids": ["source:release-intent"], "structure_ids": [], "metadata": {},
            "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                           "source": {"kind": "document", "title": "ci"}}
        }))
        .expect("serialize evidence"),
    )
    .expect("write evidence");

    let attach = |target: &str| {
        run_cli(&[
            "evidence",
            "attach",
            "--store",
            &store,
            "--case-space-id",
            case_space_id,
            "--base-revision-id",
            "revision:coverage-target",
            "--input",
            evidence_path.to_str().expect("evidence path"),
            "--satisfies",
            target,
            "--actor-id",
            "actor:release-manager",
            "--capability-id",
            "capability:release-durable-mutation",
            "--operation-scope-id",
            case_space_id,
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:release-0-9-0-intent",
            "--format",
            "json",
        ])
    };

    for target in ["work:tag-release", "capability:release-durable-mutation"] {
        let refused = attach(target);
        assert!(
            !refused.status.success(),
            "coverage was accepted against {target}"
        );
        assert!(
            stderr(&refused).contains("is not an evidence cell"),
            "stderr: {}",
            stderr(&refused)
        );
    }
    assert!(
        attach("evidence:changelog-updated").status.success(),
        "the canonical evidence target must still be accepted"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_strict_parse_refusal_names_the_object_that_refused() {
    // ADR 0010. Line and column say where in the file; they do not say which
    // closed object rejected the field, which is what a caller has to fix —
    // and the report that prompted this had a caller writing a normalization
    // script to work it out.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let mut genesis: Value = serde_json::from_str(
        &fs::read_to_string(repo_path(
            "docs/guides/release-decision/genesis.case.space.json",
        ))
        .expect("read the walkthrough genesis"),
    )
    .expect("walkthrough genesis parses");
    genesis["case_cells"][3]["provenance"]["source"]["bogus_field"] = json!("x");
    let input = directory.join("unknown-field.case.space.json");
    fs::write(
        &input,
        serde_json::to_vec_pretty(&genesis).expect("serialize genesis"),
    )
    .expect("write genesis");

    let output = run_cli(&[
        "lift",
        "native",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("input path"),
        "--revision-id",
        "revision:strict-parse",
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("case_cells[3].provenance.source.bogus_field"),
        "refusal did not locate the field: {message}"
    );
    assert!(
        message.contains("unknown field"),
        "refusal did not keep serde's reason: {message}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn review_accept_reports_the_coverage_it_activates() {
    // A reviewer is shown one target id, but accepting it makes every coverage
    // pair the attach recorded live at once. The report echoes that set so the
    // record shows what the decision actually covered.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let store = directory.to_str().expect("temp path").to_owned();
    import_native_case_space_from_input(
        &directory,
        &repo_path("docs/guides/release-decision/genesis.case.space.json"),
        "revision:coverage-report",
    );
    let case_space_id = "case_space:casegraphen-release-0-9-0";
    let gate = |args: &[&str]| {
        let mut gated = args.to_vec();
        gated.extend([
            "--actor-id",
            "actor:release-manager",
            "--capability-id",
            "capability:release-durable-mutation",
            "--operation-scope-id",
            case_space_id,
            "--audience",
            "audit",
            "--source-boundary-id",
            "source_boundary:release-0-9-0-intent",
            "--format",
            "json",
        ]);
        run_cli(&gated)
    };

    let evidence_path = directory.join("gate-run.evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&json!({
            "id": "evidence:gate-run", "cell_type": "evidence", "lifecycle": "active",
            "space_id": "space:casegraphen", "title": "Gate run output",
            "source_ids": ["source:release-intent"], "structure_ids": [], "metadata": {},
            "provenance": {"confidence": 0.6, "review_status": "unreviewed",
                           "source": {"kind": "document", "title": "ci"}}
        }))
        .expect("serialize evidence"),
    )
    .expect("write evidence");
    let attached = gate(&[
        "evidence",
        "attach",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--base-revision-id",
        "revision:coverage-report",
        "--input",
        evidence_path.to_str().expect("evidence path"),
        "--satisfies",
        "evidence:schema-id-gate-clean",
        "--satisfies",
        "evidence:changelog-updated",
    ]);
    assert!(attached.status.success(), "stderr: {}", stderr(&attached));
    let attached_revision = stdout_json(&attached)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("attached revision")
        .to_owned();

    let accepted = gate(&[
        "review",
        "accept",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--target-id",
        "evidence:gate-run",
        "--reviewer-id",
        "reviewer:release-manager",
        "--reason",
        "verified the gate output",
        "--base-revision-id",
        &attached_revision,
    ]);
    assert!(accepted.status.success(), "stderr: {}", stderr(&accepted));
    assert_eq!(
        stdout_json(&accepted)["result"]["activated_coverage"],
        json!([
            "evidence:changelog-updated",
            "evidence:schema-id-gate-clean"
        ])
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn genesis_coverage_cannot_reserve_an_id_that_does_not_exist_yet() {
    // `structure_ids` is a free-form string list — the shipped example uses
    // file paths — and genesis is the declared trust root, so a genesis
    // evidence cell naming an id nothing has created must not cover the cell
    // that later takes that id. Reproduced against the walkthrough genesis
    // before the fix: the requirement was born satisfied and `work:planned`
    // never appeared as blocked.
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let store = directory.to_str().expect("temp path").to_owned();

    let mut genesis: Value = serde_json::from_str(
        &fs::read_to_string(repo_path(
            "docs/guides/release-decision/genesis.case.space.json",
        ))
        .expect("read the walkthrough genesis"),
    )
    .expect("walkthrough genesis parses");
    let space_id = genesis["case_cells"][0]["space_id"].clone();
    genesis["case_cells"]
        .as_array_mut()
        .expect("genesis cells")
        .push(json!({
            "id": "evidence:planned-proof", "cell_type": "evidence", "lifecycle": "accepted",
            "space_id": space_id, "title": "Proof claimed at genesis",
            "source_ids": ["source:release-intent"],
            "structure_ids": ["evidence:planned-need"],
            "metadata": {"evidence_boundary": "source_backed"},
            "provenance": {"confidence": 0.99, "review_status": "accepted",
                           "source": {"kind": "document", "title": "release record"}}
        }));
    let genesis_path = directory.join("reserving-genesis.case.space.json");
    fs::write(
        &genesis_path,
        serde_json::to_vec_pretty(&genesis).expect("serialize genesis"),
    )
    .expect("write genesis");
    let imported =
        import_native_case_space_from_input(&directory, &genesis_path, "revision:reserving");
    let case_space_id = stdout_json(&imported)["result"]["record"]["case_space_id"]
        .as_str()
        .expect("case space id")
        .to_owned();

    // The id the genesis claim reserved is created now, as a placeholder for a
    // hard requirement, exactly as an author would model planned work.
    let morphism_path = directory.join("planned.case_morphism.json");
    fs::write(
        &morphism_path,
        serde_json::to_string_pretty(&json!({
            "morphism_id": "morphism:add-planned", "morphism_type": "create",
            "source_revision_id": "revision:reserving", "target_revision_id": "revision:planned",
            "added_ids": ["work:planned", "evidence:planned-need", "relation:planned-requires"],
            "updated_ids": [], "retired_ids": [], "preserved_ids": [],
            "evidence_ids": [], "source_ids": ["source:release-intent"],
            "violated_invariant_ids": [], "review_status": "unreviewed",
            "metadata": {"payload": {
                "added_cells": [
                    {"id": "work:planned", "cell_type": "work", "lifecycle": "active",
                     "space_id": space_id, "title": "Planned work",
                     "source_ids": ["source:release-intent"], "structure_ids": [], "metadata": {},
                     "provenance": {"confidence": 0.9, "review_status": "reviewed",
                                    "source": {"kind": "human", "title": "plan"}}},
                    {"id": "evidence:planned-need", "cell_type": "evidence", "lifecycle": "proposed",
                     "space_id": space_id, "title": "Required: the planned proof",
                     "source_ids": ["source:release-intent"], "structure_ids": [], "metadata": {},
                     "provenance": {"confidence": 0.3, "review_status": "unreviewed",
                                    "source": {"kind": "human", "title": "plan"}}}
                ],
                "added_relations": [
                    {"id": "relation:planned-requires", "relation_type": "requires_evidence",
                     "relation_strength": "hard", "from_id": "work:planned",
                     "to_id": "evidence:planned-need", "evidence_ids": [],
                     "source_ids": ["source:release-intent"], "metadata": {},
                     "provenance": {"confidence": 1.0, "review_status": "accepted",
                                    "source": {"kind": "human", "title": "plan"}}}
                ],
                "updated_cells": [], "updated_relations": []
            }}
        }))
        .expect("serialize morphism"),
    )
    .expect("write morphism");
    let proposed = run_cli(&[
        "morphism",
        "propose",
        "--store",
        &store,
        "--case-space-id",
        &case_space_id,
        "--input",
        morphism_path.to_str().expect("morphism path"),
        "--format",
        "json",
    ]);
    assert!(proposed.status.success(), "stderr: {}", stderr(&proposed));
    let applied = run_cli(&[
        "morphism",
        "apply",
        "--store",
        &store,
        "--case-space-id",
        &case_space_id,
        "--morphism-id",
        "morphism:add-planned",
        "--base-revision-id",
        "revision:reserving",
        "--reviewer-id",
        "reviewer:release-manager",
        "--reason",
        "add the planned work and its requirement",
        "--actor-id",
        "actor:release-manager",
        "--capability-id",
        "capability:release-durable-mutation",
        "--operation-scope-id",
        &case_space_id,
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:release-0-9-0-intent",
        "--format",
        "json",
    ]);
    assert!(applied.status.success(), "stderr: {}", stderr(&applied));

    let listed = run_cli(&[
        "obstruction",
        "list",
        "--store",
        &store,
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));
    assert!(
        stdout_json(&listed)["result"]["obstructions"]
            .as_array()
            .expect("obstructions")
            .iter()
            .any(|obstruction| {
                obstruction["obstruction_type"] == json!("missing_evidence")
                    && obstruction["affected_ids"]
                        .as_array()
                        .expect("affected ids")
                        .contains(&json!("work:planned"))
            }),
        "a genesis structure_ids entry naming a then-nonexistent id satisfied a later requirement"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lift_workflow_materializes_the_graph_into_a_replayable_case_space() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let output = run_cli(&[
        "lift",
        "workflow",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        repo_path("schemas/casegraphen/workflow.graph.example.json")
            .to_str()
            .expect("workflow example path"),
        "--revision-id",
        "revision:workflow-lift-genesis",
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let lifted = stdout_json(&output);
    let case_space_id = lifted["result"]["record"]["case_space_id"]
        .as_str()
        .expect("lifted case space id")
        .to_owned();
    let cells = lifted["result"]["case_space"]["case_cells"]
        .as_array()
        .expect("materialized cells");

    // Work items became cells; the stored `blocked` state was discarded in
    // favour of derivation and preserved as metadata.
    let blocked_item = cells
        .iter()
        .find(|cell| cell["id"] == json!("task:implement-workflow-engine"))
        .expect("blocked work item cell");
    assert_eq!(blocked_item["cell_type"], json!("work"));
    assert_eq!(blocked_item["lifecycle"], json!("active"));
    assert_eq!(blocked_item["metadata"]["workflow_state"], json!("blocked"));

    // Evidence boundaries were normalized through the shared trust rule.
    let inference = cells
        .iter()
        .find(|cell| cell["id"] == json!("evidence:workflow-gap-inference"))
        .expect("worker output evidence cell");
    assert_eq!(
        inference["metadata"]["evidence_boundary"],
        json!("inferred")
    );
    assert_eq!(
        inference["metadata"]["workflow_evidence_boundary"],
        json!("worker_output")
    );
    assert_eq!(
        inference["provenance"]["review_status"],
        json!("unreviewed")
    );

    // A graph declaring `source_backed_evidence` was declaring its own trust:
    // that boundary is acceptable with no review at all, so it cleared a hard
    // evidence requirement by typing a string into the import. The graph is a
    // document, not an authored genesis, so its claim is kept where readers
    // can see it and no decision reads it.
    let declared_source_backed = cells
        .iter()
        .find(|cell| cell["id"] == json!("evidence:workflow-target-doc"))
        .expect("source-backed evidence cell");
    assert_eq!(
        declared_source_backed["metadata"]["evidence_boundary"],
        json!("inferred")
    );
    assert_eq!(
        declared_source_backed["metadata"]["workflow_evidence_boundary"],
        json!("source_backed")
    );

    // Every lifted cell enters unreviewed, not only the evidence ones: the
    // evaluator counts a cell as complete when its review status is accepted,
    // so a graph declaring that satisfied hard dependencies on it before the
    // work started.
    assert!(
        cells
            .iter()
            .all(|cell| cell["provenance"]["review_status"] != json!("accepted")),
        "a lifted cell kept the graph's accepted review status"
    );

    // A requirement target that named no declared record became an
    // unreviewed placeholder that cannot satisfy a hard requirement.
    let placeholder = cells
        .iter()
        .find(|cell| cell["id"] == json!("evidence:json-parse-check-output"))
        .expect("placeholder evidence cell");
    assert_eq!(placeholder["lifecycle"], json!("proposed"));
    assert_eq!(
        placeholder["provenance"]["review_status"],
        json!("unreviewed")
    );
    assert!(placeholder["metadata"]["evidence_boundary"].is_null());

    // The one native evaluator derives readiness over the lifted graph:
    // the stored-blocked item is re-blocked by its dependency relation.
    let frontier = stdout_json(&run_cli(&[
        "space",
        "frontier",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]));
    // Nothing is ready, and that is the honest reading of an imported graph:
    // this tool has verified none of its evidence. The dependency structure is
    // what the analysis space is for, and it still derives — the fixture used
    // to show this item ready only because the graph declared its own evidence
    // source-backed.
    assert_eq!(frontier["result"]["frontier_cell_ids"], json!([]));
    let obstructions = stdout_json(&run_cli(&[
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]));
    assert!(obstructions["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .any(|obstruction| {
            obstruction["obstruction_type"] == json!("unresolved_dependency")
                && obstruction["affected_ids"] == json!(["task:implement-workflow-engine"])
        }));

    // The lifted genesis is reconstructive like any other.
    let validation = stdout_json(&run_cli(&[
        "space",
        "validate",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]));
    assert_eq!(validation["result"]["validation"]["valid"], json!(true));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lift_github_issues_materializes_the_snapshot_into_a_rebuildable_case_space() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let output = run_cli(&[
        "lift",
        "github-issues",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        repo_path("schemas/casegraphen/github.issue-snapshot.example.json")
            .to_str()
            .expect("GitHub issue snapshot example path"),
        "--revision-id",
        "revision:github-issues-lift-genesis",
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let lifted = stdout_json(&output);
    let case_space = &lifted["result"]["case_space"];
    let case_space_id = case_space["case_space_id"]
        .as_str()
        .expect("case space id")
        .to_owned();
    let cells = case_space["case_cells"].as_array().expect("case cells");
    let relations = case_space["case_relations"]
        .as_array()
        .expect("case relations");
    let cell = |id: &str| {
        cells
            .iter()
            .find(|cell| cell["id"] == json!(id))
            .unwrap_or_else(|| panic!("missing cell {id}"))
    };

    assert_eq!(cell("work:issue-101")["lifecycle"], json!("active"));
    assert_eq!(cell("work:issue-102")["lifecycle"], json!("resolved"));
    assert_eq!(cell("work:issue-103")["lifecycle"], json!("retired"));
    assert_eq!(cell("work:issue-101")["cell_type"], json!("work"));
    assert_eq!(
        cell("work:issue-101")["source_ids"],
        json!(["source:github:CAPHTECH/casegraphen"])
    );
    assert_eq!(
        cell("work:issue-101")["provenance"]["source"]["kind"],
        json!("api")
    );
    assert_eq!(
        cell("work:issue-101")["provenance"]["review_status"],
        json!("reviewed")
    );
    assert_eq!(
        cell("work:issue-101")["metadata"]["github_labels"],
        json!(["enhancement"])
    );

    assert_eq!(
        cell("goal:milestone-release-1-0")["cell_type"],
        json!("goal")
    );
    let covers = relations
        .iter()
        .find(|relation| {
            relation["relation_type"] == json!("covers")
                && relation["from_id"] == json!("work:issue-101")
                && relation["to_id"] == json!("goal:milestone-release-1-0")
        })
        .expect("diagnostic milestone covers relation");
    assert_eq!(covers["relation_strength"], json!("diagnostic"));

    let pull_request = cell("evidence:github-pr-42");
    assert_eq!(pull_request["cell_type"], json!("evidence"));
    assert_eq!(
        pull_request["provenance"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        pull_request["metadata"]["evidence_boundary"],
        json!("inferred")
    );
    let verifies = relations
        .iter()
        .find(|relation| {
            relation["relation_type"] == json!("verifies")
                && relation["from_id"] == json!("evidence:github-pr-42")
                && relation["to_id"] == json!("work:issue-102")
        })
        .expect("diagnostic PR verifies relation");
    assert_eq!(verifies["relation_strength"], json!("diagnostic"));

    let task_list_dependency = relations
        .iter()
        .find(|relation| {
            relation["relation_type"] == json!("depends_on")
                && relation["from_id"] == json!("work:issue-101")
                && relation["to_id"] == json!("work:issue-102")
        })
        .expect("soft task-list dependency");
    assert_eq!(task_list_dependency["relation_strength"], json!("soft"));
    assert!(cells
        .iter()
        .all(|cell| cell["cell_type"] != json!("custom:capability")));

    // The lifted id is the one an operator can type from the input they just
    // lifted. It carries no path encoding: the store escapes it once when it
    // needs a directory, and escaping it here too produced an id spelled
    // `case_space:github~3aOWNER~2fREPO` under a twice-escaped directory.
    assert_eq!(case_space_id, "case_space:github:CAPHTECH/casegraphen");

    let source_boundary = &case_space["metadata"]["source_boundary"];
    assert_eq!(
        source_boundary["id"],
        json!("source_boundary:case_space:github:CAPHTECH/casegraphen")
    );
    // The gate compares `--source-boundary-id` against the first of these and
    // falls back to the second, so they are minted once and must not drift.
    assert_eq!(
        case_space["morphism_log"][0]["morphism"]["metadata"]["source_boundary_id"],
        source_boundary["id"]
    );
    assert_eq!(
        source_boundary["included_sources"][0]["repository"],
        json!("CAPHTECH/casegraphen")
    );
    assert!(source_boundary["included_sources"][0]["query"]
        .as_str()
        .expect("recorded query")
        .starts_with("gh issue list --repo CAPHTECH/casegraphen"));
    assert!(source_boundary["information_loss"]
        .as_array()
        .expect("information loss")
        .iter()
        .any(|loss| loss["skipped_issue_numbers"] == json!([999])));
    // The example carries `gh` fields this adapter does not map — a label
    // `url`, a milestone `state`, and the `id` on a closing pull-request
    // reference that used to refuse the whole snapshot. Accepting them is only
    // honest if the boundary says they were dropped.
    assert!(
        source_boundary["information_loss"]
            .as_array()
            .expect("information loss")
            .iter()
            .any(|loss| loss["description"]
                .as_str()
                .is_some_and(|text| text.contains("that this adapter does not map"))),
        "the boundary must declare the unmapped mirrored fields it ignored"
    );

    // Delete the disposable snapshot so rebuild must fold the real genesis
    // payload from an empty case space and recreate it.
    let relative_snapshot = lifted["result"]["record"]["nearest_snapshot_path"]
        .as_str()
        .expect("nearest snapshot path");
    fs::remove_file(directory.join(relative_snapshot)).expect("delete nearest snapshot");
    let rebuild = run_cli(&[
        "space",
        "rebuild",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]);
    assert!(rebuild.status.success(), "stderr: {}", stderr(&rebuild));
    assert_eq!(
        stdout_json(&rebuild)["result"]["rebuild"]["revisions"][0]["snapshot_status"],
        json!("rebuilt")
    );

    let validation = run_cli(&[
        "space",
        "validate",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ]);
    assert!(
        validation.status.success(),
        "stderr: {}",
        stderr(&validation)
    );
    assert_eq!(
        stdout_json(&validation)["result"]["validation"]["valid"],
        json!(true)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn strict_exit_codes_distinguish_domain_findings_from_tool_failures() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let graph_id = "workflow_graph:strict-exit";
    let mut graph = workflow_attack_graph(graph_id, Vec::new());
    graph["workflow_relations"] = json!([{
        "id": "relation:strict-contradiction",
        "relation_type": "contradicts",
        "from_id": "task:goal",
        "to_id": "proof:needed",
        "evidence_ids": [],
        "source_ids": ["source:s1"],
        "provenance": {
            "source": {"kind": "document"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        }
    }]);
    let lifted = lift_workflow_graph(&directory, &graph, "strict-exit");
    assert!(lifted.status.success(), "stderr: {}", stderr(&lifted));
    let case_space_id = format!("case_space:{graph_id}");
    let current_revision_id = stdout_json(&lifted)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("lifted revision")
        .to_owned();

    let obstruction_args = [
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--format",
        "json",
    ];
    let lenient = run_cli(&obstruction_args);
    assert_eq!(
        lenient.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&lenient)
    );
    assert!(!stdout_json(&lenient)["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .is_empty());

    let strict = run_cli(&[
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(strict.status.code(), Some(2), "stderr: {}", stderr(&strict));
    assert_eq!(strict.stdout, lenient.stdout);

    for (namespace, operation) in [("space", "reason"), ("invariant", "check")] {
        let strict_report = run_cli(&[
            namespace,
            operation,
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            &case_space_id,
            "--strict",
            "--format",
            "json",
        ]);
        assert_eq!(
            strict_report.status.code(),
            Some(2),
            "{namespace} {operation} stderr: {}",
            stderr(&strict_report)
        );
    }

    let strict_close = run_cli(&[
        "invariant",
        "close-check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--base-revision-id",
        &current_revision_id,
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(
        strict_close.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&strict_close)
    );

    let clean_case_space_id = "case_space:strict-clean";
    let created = run_cli(&[
        "space",
        "new",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        clean_case_space_id,
        "--space-id",
        "space:strict-clean",
        "--title",
        "Strict clean space",
        "--revision-id",
        "revision:strict-clean",
        "--format",
        "json",
    ]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let clean = run_cli(&[
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        clean_case_space_id,
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(clean.status.code(), Some(0), "stderr: {}", stderr(&clean));
    assert!(stdout_json(&clean)["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .is_empty());

    let stale_close = run_cli(&[
        "invariant",
        "close-check",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        clean_case_space_id,
        "--base-revision-id",
        "revision:strict-stale",
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(
        stale_close.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&stale_close)
    );
    let stale_close_json = stdout_json(&stale_close);
    assert_eq!(
        stale_close_json["result"]["close_check"]["closeable"],
        json!(false)
    );
    assert!(
        stale_close_json["result"]["close_check"]["invariant_results"]
            .as_array()
            .expect("close invariants")
            .iter()
            .any(|invariant| invariant["invariant_id"]
                == json!("close:native-base-revision-matches")
                && invariant["passed"] == json!(false))
    );

    // A missing case space is a refusal with its own error_code, not just
    // exit 1 (issue #22): `--store` names a directory that was never
    // created, so `NativeStoreError::MissingCase` is what surfaces.
    let missing_store = directory.join("missing-store");
    let tool_failure = run_cli(&[
        "obstruction",
        "list",
        "--store",
        missing_store.to_str().expect("missing store path"),
        "--case-space-id",
        "case_space:missing",
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(
        tool_failure.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&tool_failure)
    );
    let missing_refusal = stderr_json(&tool_failure);
    assert_eq!(missing_refusal["error_code"], json!("missing_case_space"));
    assert_eq!(
        missing_refusal["data"]["case_space_id"],
        json!("case_space:missing")
    );

    // `space inspect` does not accept `--strict` (only report-with-findings
    // commands do): an unsupported-flag refusal, a different error_code
    // from the missing-case-space one above.
    let unsupported = run_cli(&[
        "space",
        "inspect",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        &case_space_id,
        "--strict",
        "--format",
        "json",
    ]);
    assert_eq!(unsupported.status.code(), Some(1));
    let unsupported_refusal = stderr_json(&unsupported);
    assert_eq!(unsupported_refusal["error_code"], json!("usage"));
    assert_ne!(
        unsupported_refusal["error_code"], missing_refusal["error_code"],
        "an unknown flag and a missing case space must not share an error_code"
    );
    assert!(unsupported_refusal["message"]
        .as_str()
        .expect("message is a string")
        .contains("unsupported native argument \"--strict\" for space"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// A workflow graph with one task, one proof, and one extra work item the test
/// supplies. The task hard-requires evidence that no record declares and a
/// proof that the graph declares, so a clean lift always carries a
/// `missing_evidence` and a `missing_proof` obstruction.
fn workflow_attack_graph(graph_id: &str, extra_items: Vec<Value>) -> Value {
    let provenance = json!({
        "source": {"kind": "document"}, "confidence": 0.9, "review_status": "unreviewed"
    });
    let item = |id: &str, item_type: &str, extra: Value| {
        let mut value = json!({
            "id": id, "space_id": "space:attack", "item_type": item_type, "title": id,
            "state": "todo", "case_ids": [], "hard_dependency_ids": [],
            "external_wait_ids": [], "evidence_requirement_ids": [],
            "proof_requirement_ids": [], "source_ids": ["source:s1"],
            "provenance": provenance.clone(), "metadata": {}
        });
        for (key, extra_value) in extra.as_object().expect("extra object") {
            value[key] = extra_value.clone();
        }
        value
    };
    let mut items = vec![
        item(
            "task:goal",
            "task",
            json!({
                "evidence_requirement_ids": ["evidence:real-doc"],
                "proof_requirement_ids": ["proof:needed"]
            }),
        ),
        item("proof:needed", "proof", json!({})),
    ];
    items.extend(extra_items);
    json!({
        "schema": "highergraphen.case.workflow.graph.v1", "schema_version": 1,
        "workflow_graph_id": graph_id, "case_graph_id": "case_graph:attack",
        "space_id": "space:attack", "work_items": items, "workflow_relations": [],
        "readiness_rules": [], "evidence_records": [], "transition_records": [],
        "projection_profiles": [], "correspondence_records": [], "metadata": {}
    })
}

fn lift_workflow_graph(directory: &Path, graph: &Value, name: &str) -> Output {
    let input = directory.join(format!("{name}.workflow.graph.json"));
    fs::write(
        &input,
        serde_json::to_string_pretty(graph).expect("serialize graph"),
    )
    .expect("write graph");
    run_cli(&[
        "lift",
        "workflow",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("input path"),
        "--revision-id",
        "revision:attack-genesis",
        "--format",
        "json",
    ])
}

fn obstruction_types(directory: &Path, case_space_id: &str) -> Vec<String> {
    let output = run_cli(&[
        "obstruction",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    stdout_json(&output)["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .map(|obstruction| {
            obstruction["obstruction_type"]
                .as_str()
                .expect("obstruction type")
                .to_owned()
        })
        .collect()
}

#[test]
fn lift_workflow_refuses_a_cell_colliding_with_a_genesis_structural_id() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    // The genesis morphism id is derived from the graph's own
    // workflow_graph_id, so a caller can name it exactly. The store's
    // reference checks do not index morphism, entry, or revision ids; the
    // evaluator's do, so importing such a space used to succeed and then make
    // every derived read fail permanently with no repair path.
    let graph = workflow_attack_graph(
        "workflow_graph:collide",
        vec![json!({
            "id": "morphism:create:case_space~3aworkflow_graph~3acollide",
            "space_id": "space:attack", "item_type": "task", "title": "collision",
            "state": "todo", "case_ids": [], "hard_dependency_ids": [],
            "external_wait_ids": [], "evidence_requirement_ids": [],
            "proof_requirement_ids": [], "source_ids": ["source:s1"],
            "provenance": {"source": {"kind": "document"}, "confidence": 0.9,
                           "review_status": "unreviewed"},
            "metadata": {}
        })],
    );
    let output = lift_workflow_graph(&directory, &graph, "collide");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("not evaluable"),
        "stderr: {}",
        stderr(&output)
    );
    // Refused before any filesystem write, so the case-space id is not burned.
    assert!(!directory.join("native_case_spaces").exists());

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lift_workflow_refuses_caller_declared_evidence_trust_from_a_work_item() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let baseline = lift_workflow_graph(
        &directory,
        &workflow_attack_graph("workflow_graph:baseline", Vec::new()),
        "baseline",
    );
    assert!(baseline.status.success(), "stderr: {}", stderr(&baseline));
    let mut expected = obstruction_types(&directory, "case_space:workflow_graph:baseline");
    expected.sort();
    assert_eq!(expected, vec!["missing_evidence", "missing_proof"]);

    // One evidence-typed work item claiming, in caller-supplied fields alone,
    // both that it is source-backed and that it covers the requirement and the
    // requiring cell. No evidence record, no relation, no review.
    let graph = workflow_attack_graph(
        "workflow_graph:selfdeclared",
        vec![json!({
            "id": "evidence:blank", "space_id": "space:attack", "item_type": "evidence",
            "title": "self-declared", "state": "todo",
            "case_ids": ["task:goal", "evidence:real-doc", "proof:needed"],
            "hard_dependency_ids": [], "external_wait_ids": [],
            "evidence_requirement_ids": [], "proof_requirement_ids": [],
            "source_ids": ["source:s1"],
            "provenance": {"source": {"kind": "document"}, "confidence": 0.9,
                           "review_status": "accepted"},
            "metadata": {"evidence_boundary": "source_backed"}
        })],
    );
    let output = lift_workflow_graph(&directory, &graph, "selfdeclared");
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let mut actual = obstruction_types(&directory, "case_space:workflow_graph:selfdeclared");
    actual.sort();
    assert_eq!(
        actual, expected,
        "caller-declared case_ids or evidence_boundary satisfied a hard requirement"
    );
    let cells = stdout_json(&output)["result"]["case_space"]["case_cells"]
        .as_array()
        .expect("cells")
        .clone();
    let blank = cells
        .iter()
        .find(|cell| cell["id"] == json!("evidence:blank"))
        .expect("lifted evidence-typed item");
    assert_eq!(blank["structure_ids"], json!([]));
    assert_eq!(blank["metadata"]["evidence_boundary"], json!("inferred"));
    assert_eq!(blank["provenance"]["review_status"], json!("unreviewed"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn lift_workflow_refuses_the_legacy_accepted_evidence_label_without_a_review() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    // `accepted_evidence` normalizes to the review-promoted boundary, which the
    // shared trust rule accepts when the cell's own review status is accepted —
    // and on this path the caller declares that status too.
    let mut graph = workflow_attack_graph("workflow_graph:legacylabel", Vec::new());
    graph["work_items"][0]["evidence_requirement_ids"] = json!(["evidence:claim"]);
    graph["evidence_records"] = json!([{
        "id": "evidence:claim", "evidence_type": "ai_inference",
        "evidence_boundary": "accepted_evidence", "summary": "a claim",
        "supports_ids": [], "contradicts_ids": [], "source_ids": ["source:s1"],
        "provenance": {"source": {"kind": "document"}, "confidence": 0.9,
                       "review_status": "accepted"}
    }]);
    let output = lift_workflow_graph(&directory, &graph, "legacylabel");
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    assert!(
        obstruction_types(&directory, "case_space:workflow_graph:legacylabel")
            .contains(&"missing_evidence".to_owned()),
        "a caller-declared accepted review promoted its own evidence"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn a_failed_import_rolls_back_instead_of_burning_the_case_space_id() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let graph = workflow_attack_graph("workflow_graph:rollback", Vec::new());
    let input = directory.join("rollback.workflow.graph.json");
    fs::write(
        &input,
        serde_json::to_string_pretty(&graph).expect("serialize graph"),
    )
    .expect("write graph");

    // A snapshot file name over NAME_MAX (255 on both APFS and ext4) fails the
    // write after the case directory exists — the window that used to leave a
    // logless directory behind.
    let long_revision = format!("revision:{}", "a".repeat(250));
    let failed = run_cli(&[
        "lift",
        "workflow",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("input path"),
        "--revision-id",
        &long_revision,
        "--format",
        "json",
    ]);
    assert!(
        !failed.status.success(),
        "expected the over-long snapshot name to fail the write"
    );
    let case_dir = directory
        .join("native_case_spaces")
        .join("case_space~3aworkflow_graph~3arollback");
    assert!(
        !case_dir.exists(),
        "a failed import left {} behind",
        case_dir.display()
    );

    // The id is still usable and the store is still listable.
    let retry = lift_workflow_graph(&directory, &graph, "rollback-retry");
    assert!(retry.status.success(), "stderr: {}", stderr(&retry));
    let listed = run_cli(&[
        "space",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--format",
        "json",
    ]);
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn schema_and_fixture_files_are_valid_json() {
    for path in schema_fixture_paths() {
        let text = fs::read_to_string(&path).expect("read JSON file");
        serde_json::from_str::<Value>(&text).unwrap_or_else(|error| {
            panic!("{} should be valid JSON: {error}", path.display());
        });
    }
}

#[test]
fn native_schema_examples_validate_against_json_schemas() {
    for (schema, example) in native_schema_example_pairs() {
        assert_jsonschema_valid(&schema, &example);
    }
}

fn assert_jsonschema_valid(schema: &Path, instance: &Path) {
    // A base URI resolves the relative cross-file `$ref`s some schemas use to
    // reuse another schema's `$defs` instead of duplicating them (e.g.
    // native.morphism-propose-input.schema.json referencing case_morphism
    // properties in native.case.space.schema.json). Without it, `python3 -m
    // jsonschema` treats a relative `$ref` as unretrievable.
    let schema_directory = schema
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .expect("schema file has a canonicalizable parent directory");
    let base_uri = format!("file://{}/", schema_directory.display());
    let output = Command::new("python3")
        .args([
            "-m",
            "jsonschema",
            "--base-uri",
            &base_uri,
            schema.to_str().expect("schema path"),
            "--instance",
            instance.to_str().expect("instance path"),
        ])
        .output()
        .expect("run python jsonschema validator");

    assert!(
        output.status.success(),
        "{} should validate against {}\nstdout: {}\nstderr: {}",
        instance.display(),
        schema.display(),
        stdout(&output),
        stderr(&output)
    );
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen CLI")
}

fn run_cli_with_mutation_gate(args: &[&str], actor_id: &str) -> Output {
    let mut gated_args = args.to_vec();
    gated_args.extend([
        "--actor-id",
        actor_id,
        "--capability-id",
        "capability:durable-mutation",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
    ]);
    run_cli(&gated_args)
}

fn import_native_case_space(directory: &Path, revision_id: &str) -> Output {
    import_native_case_space_from_input(directory, &native_case_fixture(), revision_id)
}

fn import_native_case_space_from_input(
    directory: &Path,
    input: &Path,
    revision_id: &str,
) -> Output {
    let output = run_cli(&[
        "lift",
        "native",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        input.to_str().expect("native fixture path"),
        "--revision-id",
        revision_id,
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    output
}

fn imported_native_log_path(directory: &Path, imported: &Value) -> PathBuf {
    directory.join(
        imported["result"]["record"]["log_path"]
            .as_str()
            .expect("native morphism log path"),
    )
}

fn run_native_case_store_command(directory: &Path, command: &str) -> Output {
    run_native_store_command(directory, "space", command)
}

fn run_native_store_command(directory: &Path, namespace: &str, command: &str) -> Output {
    let output = run_cli(&[
        namespace,
        command,
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    output
}

fn write_native_metadata_morphism(
    path: &Path,
    morphism_id: &str,
    source_revision_id: &str,
    target_revision_id: &str,
) {
    write_native_metadata_morphism_with_metadata(
        path,
        morphism_id,
        source_revision_id,
        target_revision_id,
        json!({}),
    );
}

fn write_native_metadata_morphism_with_metadata(
    path: &Path,
    morphism_id: &str,
    source_revision_id: &str,
    target_revision_id: &str,
    metadata: Value,
) {
    // `update`, not `review`: a generic proposal may not declare a morphism type
    // the tool mints for itself, and these fixtures exercise generic propose and
    // apply. Declaring `review` here made every one of them depend on a rule
    // that no longer exists.
    let morphism = json!({
        "morphism_id": morphism_id,
        "morphism_type": "update",
        "source_revision_id": source_revision_id,
        "target_revision_id": target_revision_id,
        "added_ids": [],
        "updated_ids": [],
        "retired_ids": [],
        "preserved_ids": ["goal:native-case-contract"],
        "violated_invariant_ids": [],
        "review_status": "unreviewed",
        "evidence_ids": [],
        "source_ids": ["source:native-cli-test"],
        "metadata": metadata
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&morphism).expect("serialize morphism"),
    )
    .expect("write native morphism");
}

fn write_execution_plan(path: &Path, plan_id: &str, base_revision_id: &str, work_cell_id: &str) {
    let store = path.parent().expect("execution plan store directory");
    let registered_binding = store
        .join("bindings")
        .join("worker_binding~3anative-integration.worker.binding.json");
    if !registered_binding.exists() {
        let binding_input = store.join("native-integration.worker.binding.input.json");
        write_worker_binding(
            &binding_input,
            "worker_binding:native-integration",
            store,
            "printf 'native integration worker\\n'",
        );
        let register = run_cli(&[
            "binding",
            "register",
            "--store",
            store.to_str().expect("store path"),
            "--input",
            binding_input.to_str().expect("binding input path"),
            "--format",
            "json",
        ]);
        assert!(register.status.success(), "stderr: {}", stderr(&register));
    }
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": base_revision_id,
        "steps": [
            {
                "step_id": format!("step:{plan_id}"),
                "work_cell_id": work_cell_id,
                "worker_binding_id": "worker_binding:native-integration",
                "success_evidence_requirement_ids": [
                    "evidence:native-schema-json-valid"
                ],
                "allowed_transition_classes": [
                    {
                        "morphism_type": "update",
                        "target_cell_types": ["work"],
                        "to_lifecycles": ["resolved"]
                    }
                ]
            }
        ],
        "provenance": {
            "source": {
                "kind": "human",
                "title": "Native execution plan integration test"
            },
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&plan).expect("serialize execution plan"),
    )
    .expect("write execution plan");
}

struct NativeRunFixture {
    plan_id: String,
    step_id: String,
    accepted_revision_id: String,
    binding_path: PathBuf,
}

struct NativeFrontierFixture {
    plan_id: String,
    step_ids: Vec<String>,
    work_cell_ids: Vec<String>,
    accepted_revision_id: String,
}

#[cfg(unix)]
fn setup_native_frontier(
    directory: &Path,
    suffix: &str,
    workers: &[(&str, &str)],
) -> NativeFrontierFixture {
    setup_native_frontier_with_timeouts(directory, suffix, workers, &vec![5_000; workers.len()])
}

/// The general form `setup_native_frontier` above delegates to with every
/// worker pinned to the ordinary 5 s `timeout_ms`: lets one test give a
/// specific worker a different budget (issue #39 — a worker deliberately
/// held "live" by a test-controlled marker must not also be racing a
/// pinned timeout meant for workers that are expected to resolve quickly on
/// their own) without touching `setup_native_frontier`'s signature or its
/// other 21 call sites in this file.
#[cfg(unix)]
fn setup_native_frontier_with_timeouts(
    directory: &Path,
    suffix: &str,
    workers: &[(&str, &str)],
    worker_timeouts_ms: &[u64],
) -> NativeFrontierFixture {
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        worker_timeouts_ms.len(),
        workers.len(),
        "one timeout per worker"
    );

    let input_path = directory.join(format!("{suffix}.frontier.native.input.json"));
    let mut input = json_file(native_case_fixture());
    let work_template = input["case_cells"]
        .as_array()
        .expect("native case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("native work template")
        .clone();
    let mut work_cell_ids = Vec::new();
    for (work_cell_id, _) in workers {
        if work_cell_ids.iter().any(|id| id == work_cell_id) {
            continue;
        }
        let mut work_cell = work_template.clone();
        work_cell["id"] = json!(work_cell_id);
        work_cell["title"] = json!(format!("Frontier work {work_cell_id}"));
        work_cell["lifecycle"] = json!("active");
        work_cell["structure_ids"] = json!([]);
        input["case_cells"]
            .as_array_mut()
            .expect("native case cells")
            .push(work_cell);
        work_cell_ids.push((*work_cell_id).to_owned());
    }
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&input).expect("serialize frontier case space"),
    )
    .expect("write frontier case space");
    let import_revision = format!("revision:frontier-{suffix}-import");
    import_native_case_space_from_input(directory, &input_path, &import_revision);

    let plan_id = format!("plan:frontier-{suffix}");
    let mut step_ids = Vec::new();
    let mut steps = Vec::new();
    for (index, (work_cell_id, script_body)) in workers.iter().enumerate() {
        let number = index + 1;
        let script_path = directory.join(format!("{suffix}-worker-{number}.sh"));
        fs::write(&script_path, format!("#!/bin/sh\nset -eu\n{script_body}\n"))
            .expect("write pinned frontier worker");
        let mut permissions = fs::metadata(&script_path)
            .expect("frontier worker metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make frontier worker executable");

        let binding_id = format!("worker_binding:frontier-{suffix}-{number}");
        let binding_input = directory.join(format!("{suffix}-worker-{number}.binding.input.json"));
        write_pinned_worker_binding_with_timeout(
            &binding_input,
            &binding_id,
            directory,
            &script_path,
            worker_timeouts_ms[index],
        );
        let register = run_cli(&[
            "binding",
            "register",
            "--store",
            directory.to_str().expect("store path"),
            "--input",
            binding_input.to_str().expect("binding input path"),
            "--format",
            "json",
        ]);
        assert!(register.status.success(), "stderr: {}", stderr(&register));

        let step_id = format!("step:{plan_id}:{number}");
        step_ids.push(step_id.clone());
        steps.push(json!({
            "step_id": step_id,
            "work_cell_id": work_cell_id,
            "worker_binding_id": binding_id,
            "success_evidence_requirement_ids": [
                "evidence:native-schema-json-valid"
            ],
            "allowed_transition_classes": [
                {
                    "morphism_type": "update",
                    "target_cell_types": ["work"],
                    "to_lifecycles": ["resolved"]
                }
            ]
        }));
    }
    let plan_input = directory.join(format!("{suffix}.frontier.execution.plan.input.json"));
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": import_revision,
        "steps": steps,
        "provenance": {
            "source": {
                "kind": "human",
                "title": "Native frontier integration plan"
            },
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    fs::write(
        &plan_input,
        serde_json::to_string_pretty(&plan).expect("serialize frontier plan"),
    )
    .expect("write frontier plan");
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &plan_id,
        "--reviewer-id",
        "reviewer:frontier-plan",
        "--reason",
        "Accept frontier worker execution plan",
        "--base-revision-id",
        &import_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accepted_revision_id = stdout_json(&accept)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted frontier revision")
        .to_owned();
    NativeFrontierFixture {
        plan_id,
        step_ids,
        work_cell_ids,
        accepted_revision_id,
    }
}

#[cfg(unix)]
fn write_pinned_worker_binding(
    path: &Path,
    binding_id: &str,
    working_directory: &Path,
    command: &Path,
) {
    write_pinned_worker_binding_with_timeout(path, binding_id, working_directory, command, 5_000);
}

#[cfg(unix)]
fn write_pinned_worker_binding_with_timeout(
    path: &Path,
    binding_id: &str,
    working_directory: &Path,
    command: &Path,
    timeout_ms: u64,
) {
    let binding = json!({
        "schema": "highergraphen.case.workflow.worker_binding.v1",
        "schema_version": 1,
        "binding_id": binding_id,
        "worker_kind": "shell",
        "command": command,
        "args": [],
        "working_directory": working_directory,
        "resolved_command_path": "/caller/value/is/overwritten",
        "resolved_working_directory": "/caller/value/is/overwritten",
        "command_content_hash": "0000000000000000000000000000000000000000000000000000000000000000",
        "env_allowlist": [],
        "timeout_ms": timeout_ms,
        "capability_ids": ["capability:native-run-worker"],
        "metadata": {}
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&binding).expect("serialize pinned worker binding"),
    )
    .expect("write pinned worker binding");
}

fn setup_native_run(directory: &Path, suffix: &str, script: &str) -> NativeRunFixture {
    setup_native_run_with_allowed_lifecycle(directory, suffix, script, "resolved")
}

fn setup_native_run_with_allowed_lifecycle(
    directory: &Path,
    suffix: &str,
    script: &str,
    allowed_lifecycle: &str,
) -> NativeRunFixture {
    setup_native_run_with_allowed_lifecycle_and_command(
        directory,
        suffix,
        script,
        allowed_lifecycle,
        Path::new("/bin/sh"),
    )
}

fn setup_native_run_with_allowed_lifecycle_and_command(
    directory: &Path,
    suffix: &str,
    script: &str,
    allowed_lifecycle: &str,
    command: &Path,
) -> NativeRunFixture {
    let import_revision = format!("revision:run-{suffix}-import");
    import_native_case_space(directory, &import_revision);
    let activate = run_cli_with_mutation_gate(
        &[
            "cell",
            "transition",
            "--store",
            directory.to_str().expect("temp path"),
            "--case-space-id",
            native_case_space_id(),
            "--base-revision-id",
            &import_revision,
            "--cell-id",
            "work:review-native-contract",
            "--to",
            "active",
            "--format",
            "json",
        ],
        "actor:native-transition-cli",
    );
    assert!(activate.status.success(), "stderr: {}", stderr(&activate));
    let active_revision = stdout_json(&activate)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("active revision")
        .to_owned();

    let binding_id = format!("worker_binding:run-{suffix}");
    let binding_input = directory.join(format!("{suffix}.worker.binding.input.json"));
    write_worker_binding_with_command(&binding_input, &binding_id, directory, command, script);
    let register = run_cli(&[
        "binding",
        "register",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        binding_input.to_str().expect("binding input path"),
        "--format",
        "json",
    ]);
    assert!(register.status.success(), "stderr: {}", stderr(&register));
    let register_json = stdout_json(&register);
    let binding_path = directory.join(
        register_json["result"]["binding_path"]
            .as_str()
            .expect("registered binding path"),
    );

    let plan_id = format!("plan:run-{suffix}");
    let step_id = format!("step:{plan_id}");
    let plan_input = directory.join(format!("{suffix}.execution.plan.input.json"));
    write_execution_plan_for_binding_with_lifecycle(
        &plan_input,
        &plan_id,
        &active_revision,
        "work:review-native-contract",
        &binding_id,
        allowed_lifecycle,
    );
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &plan_id,
        "--reviewer-id",
        "reviewer:run-plan",
        "--reason",
        "Accept one-step worker execution plan",
        "--base-revision-id",
        &active_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accepted_revision_id = stdout_json(&accept)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted revision")
        .to_owned();

    NativeRunFixture {
        plan_id,
        step_id,
        accepted_revision_id,
        binding_path,
    }
}

fn write_worker_binding(path: &Path, binding_id: &str, working_directory: &Path, script: &str) {
    write_worker_binding_with_command(
        path,
        binding_id,
        working_directory,
        Path::new("/bin/sh"),
        script,
    );
}

fn write_worker_binding_with_command(
    path: &Path,
    binding_id: &str,
    working_directory: &Path,
    command: &Path,
    script: &str,
) {
    let binding = json!({
        "schema": "highergraphen.case.workflow.worker_binding.v1",
        "schema_version": 1,
        "binding_id": binding_id,
        "worker_kind": "shell",
        "command": command,
        "args": ["-c", script],
        "working_directory": working_directory,
        "resolved_command_path": "/caller/value/is/overwritten",
        "resolved_working_directory": "/caller/value/is/overwritten",
        "command_content_hash": "0000000000000000000000000000000000000000000000000000000000000000",
        "env_allowlist": [],
        "timeout_ms": 5000,
        "capability_ids": ["capability:native-run-worker"],
        "metadata": {}
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&binding).expect("serialize worker binding"),
    )
    .expect("write worker binding");
}

fn write_execution_plan_for_binding(
    path: &Path,
    plan_id: &str,
    base_revision_id: &str,
    work_cell_id: &str,
    binding_id: &str,
) {
    write_execution_plan_for_binding_with_lifecycle(
        path,
        plan_id,
        base_revision_id,
        work_cell_id,
        binding_id,
        "resolved",
    );
}

fn write_execution_plan_for_binding_with_lifecycle(
    path: &Path,
    plan_id: &str,
    base_revision_id: &str,
    work_cell_id: &str,
    binding_id: &str,
    allowed_lifecycle: &str,
) {
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": base_revision_id,
        "steps": [
            {
                "step_id": format!("step:{plan_id}"),
                "work_cell_id": work_cell_id,
                "worker_binding_id": binding_id,
                "success_evidence_requirement_ids": [
                    "evidence:native-schema-json-valid"
                ],
                "allowed_transition_classes": [
                    {
                        "morphism_type": "update",
                        "target_cell_types": ["work"],
                        "to_lifecycles": [allowed_lifecycle]
                    }
                ]
            }
        ],
        "provenance": {
            "source": {
                "kind": "human",
                "title": "Native run integration plan"
            },
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    fs::write(
        path,
        serde_json::to_string_pretty(&plan).expect("serialize execution plan"),
    )
    .expect("write execution plan");
}

fn run_native_step(
    directory: &Path,
    fixture: &NativeRunFixture,
    enable_shell: bool,
    retry_step_id: Option<&str>,
) -> Output {
    run_native_step_with_base(
        directory,
        fixture,
        &fixture.accepted_revision_id,
        enable_shell,
        retry_step_id,
    )
}

fn run_native_step_with_base(
    directory: &Path,
    fixture: &NativeRunFixture,
    base_revision_id: &str,
    enable_shell: bool,
    retry_step_id: Option<&str>,
) -> Output {
    run_native_step_with_gate_capabilities(
        directory,
        fixture,
        base_revision_id,
        enable_shell,
        retry_step_id,
        &["capability:dispatch", "capability:native-run-worker"],
    )
}

fn run_native_step_with_superseded_traces(
    directory: &Path,
    fixture: &NativeRunFixture,
    base_revision_id: &str,
    supersede_trace_ids: &[&str],
) -> Output {
    let mut args = native_step_args(
        directory,
        fixture,
        base_revision_id,
        true,
        None,
        &["capability:dispatch", "capability:native-run-worker"],
    );
    append_supersede_trace_args(&mut args, supersede_trace_ids);
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen run --step with superseded traces")
}

fn run_native_step_with_gate_capabilities(
    directory: &Path,
    fixture: &NativeRunFixture,
    base_revision_id: &str,
    enable_shell: bool,
    retry_step_id: Option<&str>,
    capability_ids: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_step_args(
            directory,
            fixture,
            base_revision_id,
            enable_shell,
            retry_step_id,
            capability_ids,
        ))
        .output()
        .expect("run casegraphen run --step")
}

fn native_step_args(
    directory: &Path,
    fixture: &NativeRunFixture,
    base_revision_id: &str,
    enable_shell: bool,
    retry_step_id: Option<&str>,
    capability_ids: &[&str],
) -> Vec<String> {
    let mut args = vec![
        "run".to_owned(),
        "--step".to_owned(),
        "--store".to_owned(),
        directory.display().to_string(),
        "--case-space-id".to_owned(),
        native_case_space_id().to_owned(),
        "--plan-id".to_owned(),
        fixture.plan_id.clone(),
        "--base-revision-id".to_owned(),
        base_revision_id.to_owned(),
        "--actor-id".to_owned(),
        "actor:native-run".to_owned(),
        "--gate-actor-id".to_owned(),
        "actor:native-run".to_owned(),
        "--operation-scope-id".to_owned(),
        native_case_space_id().to_owned(),
        "--audience".to_owned(),
        "audit".to_owned(),
        "--source-boundary-id".to_owned(),
        "source_boundary:native-case-management-contract".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    for capability_id in capability_ids {
        args.extend(["--capability-id".to_owned(), (*capability_id).to_owned()]);
    }
    if enable_shell {
        args.extend(["--enable-worker".to_owned(), "shell".to_owned()]);
    }
    if let Some(step_id) = retry_step_id {
        args.extend(["--retry-step".to_owned(), step_id.to_owned()]);
    }
    args
}

#[cfg(unix)]
fn run_native_frontier(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    max_parallel: usize,
) -> Output {
    run_native_frontier_with(
        directory,
        fixture,
        &fixture.accepted_revision_id,
        max_parallel,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
    )
}

#[cfg(unix)]
fn run_native_frontier_with(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    base_revision_id: &str,
    max_parallel: usize,
    capability_ids: &[&str],
    retry_step_ids: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_args(
            directory,
            fixture,
            base_revision_id,
            max_parallel,
            capability_ids,
            retry_step_ids,
            true,
        ))
        .output()
        .expect("run casegraphen run --frontier")
}

#[cfg(unix)]
fn run_native_frontier_with_superseded_traces(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    base_revision_id: &str,
    supersede_trace_ids: &[&str],
) -> Output {
    let mut args = native_frontier_args(
        directory,
        fixture,
        base_revision_id,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
        true,
    );
    append_supersede_trace_args(&mut args, supersede_trace_ids);
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen run --frontier with superseded traces")
}

#[cfg(unix)]
fn run_native_frontier_without_worker_opt_in(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    max_parallel: usize,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_frontier_args(
            directory,
            fixture,
            &fixture.accepted_revision_id,
            max_parallel,
            &["capability:dispatch", "capability:native-run-worker"],
            &[],
            false,
        ))
        .output()
        .expect("run casegraphen run --frontier without worker opt-in")
}

#[cfg(unix)]
fn native_frontier_args(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    base_revision_id: &str,
    max_parallel: usize,
    capability_ids: &[&str],
    retry_step_ids: &[&str],
    enable_shell: bool,
) -> Vec<String> {
    let mut args = vec![
        "run".to_owned(),
        "--frontier".to_owned(),
        "--store".to_owned(),
        directory.display().to_string(),
        "--case-space-id".to_owned(),
        native_case_space_id().to_owned(),
        "--plan-id".to_owned(),
        fixture.plan_id.clone(),
        "--base-revision-id".to_owned(),
        base_revision_id.to_owned(),
        "--actor-id".to_owned(),
        "actor:native-run".to_owned(),
        "--operation-scope-id".to_owned(),
        native_case_space_id().to_owned(),
        "--audience".to_owned(),
        "audit".to_owned(),
        "--source-boundary-id".to_owned(),
        "source_boundary:native-case-management-contract".to_owned(),
        "--max-parallel".to_owned(),
        max_parallel.to_string(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    for capability_id in capability_ids {
        args.extend(["--capability-id".to_owned(), (*capability_id).to_owned()]);
    }
    if enable_shell {
        args.extend(["--enable-worker".to_owned(), "shell".to_owned()]);
    }
    for step_id in retry_step_ids {
        args.extend(["--retry-step".to_owned(), (*step_id).to_owned()]);
    }
    args
}

fn append_supersede_trace_args(args: &mut Vec<String>, supersede_trace_ids: &[&str]) {
    for trace_id in supersede_trace_ids {
        args.extend(["--supersede-trace".to_owned(), (*trace_id).to_owned()]);
    }
}

#[cfg(unix)]
fn native_frontier_step_args(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    base_revision_id: &str,
    retry_step_id: Option<&str>,
    supersede_trace_ids: &[&str],
) -> Vec<String> {
    let mut args = native_frontier_args(
        directory,
        fixture,
        base_revision_id,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
        true,
    );
    args[1] = "--step".to_owned();
    let max_parallel = args
        .iter()
        .position(|argument| argument == "--max-parallel")
        .expect("frontier args include max parallel");
    args.drain(max_parallel..=max_parallel + 1);
    if let Some(step_id) = retry_step_id {
        args.extend(["--retry-step".to_owned(), step_id.to_owned()]);
    }
    append_supersede_trace_args(&mut args, supersede_trace_ids);
    args
}

#[cfg(unix)]
fn native_operate_args(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    base_revision_id: &str,
    max_parallel: usize,
    max_rounds: usize,
    capability_ids: &[&str],
    retry_step_ids: &[&str],
) -> Vec<String> {
    let mut args = vec![
        "operate".to_owned(),
        "--store".to_owned(),
        directory.display().to_string(),
        "--case-space-id".to_owned(),
        native_case_space_id().to_owned(),
        "--plan-id".to_owned(),
        fixture.plan_id.clone(),
        "--base-revision-id".to_owned(),
        base_revision_id.to_owned(),
        "--actor-id".to_owned(),
        "actor:native-run".to_owned(),
        "--operation-scope-id".to_owned(),
        native_case_space_id().to_owned(),
        "--audience".to_owned(),
        "audit".to_owned(),
        "--source-boundary-id".to_owned(),
        "source_boundary:native-case-management-contract".to_owned(),
        "--max-parallel".to_owned(),
        max_parallel.to_string(),
        "--max-rounds".to_owned(),
        max_rounds.to_string(),
        "--enable-worker".to_owned(),
        "shell".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    for capability_id in capability_ids {
        args.extend(["--capability-id".to_owned(), (*capability_id).to_owned()]);
    }
    for step_id in retry_step_ids {
        args.extend(["--retry-step".to_owned(), (*step_id).to_owned()]);
    }
    args
}

#[cfg(unix)]
fn run_native_operate(
    directory: &Path,
    fixture: &NativeFrontierFixture,
    max_parallel: usize,
    max_rounds: usize,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_operate_args(
            directory,
            fixture,
            &fixture.accepted_revision_id,
            max_parallel,
            max_rounds,
            &["capability:dispatch", "capability:native-run-worker"],
            &[],
        ))
        .output()
        .expect("run casegraphen operate")
}

/// A single-step fixture whose one work cell carries an extra readiness
/// obstruction the plan's own machinery never touches, so the step is never
/// dispatchable at all — the halt this produces is visible before any worker
/// ever runs. `relation_type` names the readiness rule under test
/// (`requires_evidence` for `needs_evidence`, `waits_for` for
/// `needs_external`); `blocking_target_id` is a real cell of a matching
/// type that is present but deliberately never satisfied — trusted evidence
/// and lifecycle completion are both about log-derived and lifecycle state,
/// not mere existence, so an id that resolves to nothing would fail the
/// import's dangling-reference check before readiness ever runs, and an id
/// that resolves to something already complete would satisfy the
/// requirement instead of blocking it.
#[cfg(unix)]
fn setup_native_operate_blocked_fixture(
    directory: &Path,
    suffix: &str,
    relation_type: &str,
    blocking_target_id: &str,
    blocking_target_cell_type: &str,
) -> NativeFrontierFixture {
    use std::os::unix::fs::PermissionsExt;

    let input_path = directory.join(format!("{suffix}.operate-blocked.native.input.json"));
    let mut input = json_file(native_case_fixture());
    let work_template = input["case_cells"]
        .as_array()
        .expect("native case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("native work template")
        .clone();
    let work_cell_id = format!("work:operate-blocked-{suffix}");
    let space_id = input["space_id"].clone();
    let mut work_cell = work_template.clone();
    work_cell["id"] = json!(work_cell_id);
    work_cell["title"] = json!(format!("Operate blocked fixture {suffix}"));
    work_cell["lifecycle"] = json!("active");
    work_cell["structure_ids"] = json!([]);
    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .push(work_cell);
    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .push(json!({
            "id": blocking_target_id,
            "cell_type": blocking_target_cell_type,
            "space_id": space_id,
            "title": format!("Operate blocked fixture {suffix} target"),
            "summary": Value::Null,
            "lifecycle": "active",
            "source_ids": [],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }));
    let relation_id = format!("relation:operate-blocked-{suffix}");
    input["case_relations"]
        .as_array_mut()
        .expect("native case relations")
        .push(json!({
            "id": relation_id,
            "relation_type": relation_type,
            "relation_strength": "hard",
            "from_id": work_cell_id,
            "to_id": blocking_target_id,
            "evidence_ids": [],
            "source_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "accepted"
            },
            "metadata": {}
        }));
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&input).expect("serialize blocked fixture case space"),
    )
    .expect("write blocked fixture case space");
    let import_revision = format!("revision:operate-blocked-{suffix}-import");
    import_native_case_space_from_input(directory, &input_path, &import_revision);

    let script_path = directory.join(format!("{suffix}-blocked-worker.sh"));
    fs::write(
        &script_path,
        "#!/bin/sh\nset -eu\nprintf 'never invoked\\n'\n",
    )
    .expect("write unused blocked worker");
    let mut permissions = fs::metadata(&script_path)
        .expect("blocked worker metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("make blocked worker executable");
    let binding_id = format!("worker_binding:operate-blocked-{suffix}");
    let binding_input = directory.join(format!("{suffix}-blocked-worker.binding.input.json"));
    write_pinned_worker_binding(&binding_input, &binding_id, directory, &script_path);
    let register = run_cli(&[
        "binding",
        "register",
        "--store",
        directory.to_str().expect("store path"),
        "--input",
        binding_input.to_str().expect("binding input path"),
        "--format",
        "json",
    ]);
    assert!(register.status.success(), "stderr: {}", stderr(&register));

    let plan_id = format!("plan:operate-blocked-{suffix}");
    let step_id = format!("step:{plan_id}:1");
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": import_revision,
        "steps": [{
            "step_id": step_id,
            "work_cell_id": work_cell_id,
            "worker_binding_id": binding_id,
            "success_evidence_requirement_ids": ["evidence:native-schema-json-valid"],
            "allowed_transition_classes": [{
                "morphism_type": "update",
                "target_cell_types": ["work"],
                "to_lifecycles": ["resolved"]
            }]
        }],
        "provenance": {
            "source": {"kind": "human", "title": "Operate blocked fixture plan"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    let plan_input = directory.join(format!("{suffix}-blocked.execution.plan.input.json"));
    fs::write(
        &plan_input,
        serde_json::to_string_pretty(&plan).expect("serialize blocked fixture plan"),
    )
    .expect("write blocked fixture plan");
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &plan_id,
        "--reviewer-id",
        "reviewer:operate-blocked-plan",
        "--reason",
        "Accept operate blocked fixture plan",
        "--base-revision-id",
        &import_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accepted_revision_id = stdout_json(&accept)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted blocked fixture revision")
        .to_owned();
    NativeFrontierFixture {
        plan_id,
        step_ids: vec![step_id],
        work_cell_ids: vec![work_cell_id],
        accepted_revision_id,
    }
}

#[test]
fn native_run_step_reports_needs_retry_decision_halt_after_a_worker_failure() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        "halt-retry",
        "printf 'failed-output'; printf 'failed-error' >&2; exit 1",
    );

    let output = run_native_step(&directory, &fixture, true, None);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("step_failed"));
    assert_eq!(
        value["result"]["halt"]["halt"],
        json!("needs_retry_decision")
    );
    assert_eq!(
        value["result"]["halt"]["target_ids"],
        json!([fixture.step_id])
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_run_step_reports_needs_plan_review_halt_for_an_unauthorized_transition() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run_with_allowed_lifecycle(
        &directory,
        "halt-plan-review",
        "printf 'successful-but-not-authorized\\n'",
        "accepted",
    );

    let output = run_native_step(&directory, &fixture, true, None);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["result"]["status"],
        json!("transition_not_authorized")
    );
    assert_eq!(value["result"]["halt"]["halt"], json!("needs_plan_review"));
    assert_eq!(
        value["result"]["halt"]["target_ids"],
        json!([fixture.step_id])
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_reports_needs_evidence_halt_when_nothing_is_dispatchable() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    // The `requires_evidence` target is a requirement placeholder
    // (`skills/casegraphen-operate/references/authoring.md`), not itself an
    // evidence cell: an `evidence`-typed target here would additionally
    // register as an untrusted (default-boundary) inference and produce an
    // `UnreviewedInference` review gap, which correctly outranks
    // `needs_evidence` under `derive_halt`'s priority order and would test
    // the wrong halt.
    let fixture = setup_native_operate_blocked_fixture(
        &directory,
        "evidence",
        "requires_evidence",
        "goal:operate-blocked-evidence-missing",
        "goal",
    );

    let output = run_native_frontier(&directory, &fixture, 1);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    assert_eq!(value["result"]["halt"]["halt"], json!("needs_evidence"));
    // The unsatisfied requirement `evidence attach --satisfies` takes, not
    // the work cell it blocks: a halt names what clears it.
    assert_eq!(
        value["result"]["halt"]["target_ids"],
        json!(["goal:operate-blocked-evidence-missing"])
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn native_run_frontier_reports_needs_external_halt_when_nothing_is_dispatchable() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_operate_blocked_fixture(
        &directory,
        "external",
        "waits_for",
        "event:operate-blocked-external-wait",
        "event",
    );

    let output = run_native_frontier(&directory, &fixture, 1);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("no_dispatchable_step"));
    assert_eq!(value["result"]["halt"]["halt"], json!("needs_external"));
    assert_eq!(
        value["result"]["halt"]["target_ids"],
        json!(fixture.work_cell_ids)
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #28 end to end through the real binary: `space reason`'s
/// `readiness.waiting_cell_ids` must include a cell blocked only by an
/// unresolved `waits_for` target, and must exclude a cell that is *also*
/// missing evidence — the all-or-nothing rule the unit tests in
/// `src/native_eval/tests.rs` pin at the function level.
#[test]
fn space_reason_reports_waiting_cell_ids_for_external_waits_and_excludes_mixed_blockers() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let input_path = directory.join("issue28.native.input.json");
    let mut input = json_file(native_case_fixture());
    let work_template = input["case_cells"]
        .as_array()
        .expect("native case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("native work template")
        .clone();
    let space_id = input["space_id"].clone();

    let waiting_only_id = "work:issue28-waiting-only";
    let mut waiting_only = work_template.clone();
    waiting_only["id"] = json!(waiting_only_id);
    waiting_only["title"] = json!("Issue 28 waiting-only fixture");
    waiting_only["lifecycle"] = json!("active");
    waiting_only["structure_ids"] = json!([]);

    let mixed_id = "work:issue28-mixed-blockers";
    let mut mixed = work_template.clone();
    mixed["id"] = json!(mixed_id);
    mixed["title"] = json!("Issue 28 mixed-blockers fixture");
    mixed["lifecycle"] = json!("active");
    mixed["structure_ids"] = json!([]);

    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .extend([waiting_only, mixed]);
    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .push(json!({
            "id": "event:issue28-external-wait",
            "cell_type": "event",
            "space_id": space_id,
            "title": "Issue 28 unresolved wait target",
            "summary": Value::Null,
            "lifecycle": "active",
            "source_ids": [],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }));
    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .push(json!({
            "id": "evidence:issue28-missing-requirement",
            "cell_type": "evidence",
            "space_id": space_id,
            "title": "Issue 28 unsatisfied evidence requirement",
            "summary": Value::Null,
            "lifecycle": "proposed",
            "source_ids": [],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }));
    input["case_relations"]
        .as_array_mut()
        .expect("native case relations")
        .extend([
            json!({
                "id": "relation:issue28-waiting-only-waits-for-event",
                "relation_type": "waits_for",
                "relation_strength": "hard",
                "from_id": waiting_only_id,
                "to_id": "event:issue28-external-wait",
                "evidence_ids": [],
                "source_ids": [],
                "provenance": {
                    "source": {"kind": "human"},
                    "confidence": 1.0,
                    "review_status": "accepted"
                },
                "metadata": {}
            }),
            json!({
                "id": "relation:issue28-mixed-waits-for-event",
                "relation_type": "waits_for",
                "relation_strength": "hard",
                "from_id": mixed_id,
                "to_id": "event:issue28-external-wait",
                "evidence_ids": [],
                "source_ids": [],
                "provenance": {
                    "source": {"kind": "human"},
                    "confidence": 1.0,
                    "review_status": "accepted"
                },
                "metadata": {}
            }),
            json!({
                "id": "relation:issue28-mixed-requires-evidence",
                "relation_type": "requires_evidence",
                "relation_strength": "hard",
                "from_id": mixed_id,
                "to_id": "evidence:issue28-missing-requirement",
                "evidence_ids": [],
                "source_ids": [],
                "provenance": {
                    "source": {"kind": "human"},
                    "confidence": 1.0,
                    "review_status": "accepted"
                },
                "metadata": {}
            }),
        ]);

    fs::write(
        &input_path,
        serde_json::to_string_pretty(&input).expect("serialize issue28 fixture case space"),
    )
    .expect("write issue28 fixture case space");
    import_native_case_space_from_input(&directory, &input_path, "revision:issue28-import");

    let report = run_cli(&[
        "space",
        "reason",
        "--store",
        directory.to_str().expect("temp path"),
        "--case-space-id",
        native_case_space_id(),
        "--format",
        "json",
    ]);
    assert!(report.status.success(), "stderr: {}", stderr(&report));
    let waiting_cell_ids = stdout_json(&report)["result"]["evaluation"]["readiness"]
        ["waiting_cell_ids"]
        .as_array()
        .expect("waiting cell ids")
        .clone();
    assert!(
        waiting_cell_ids.contains(&json!(waiting_only_id)),
        "waiting_cell_ids: {waiting_cell_ids:?}"
    );
    assert!(
        !waiting_cell_ids.contains(&json!(mixed_id)),
        "a cell also missing evidence must not read as purely waiting: {waiting_cell_ids:?}"
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Two work cells and two plan steps, the second `depends_on` the first, so
/// the second is not dispatchable until the first resolves. This is what
/// makes a genuine second round necessary — `setup_native_frontier`'s
/// independent work cells all dispatch in one round regardless of
/// `--max-parallel`, since `select_steps` selects every eligible step across
/// the whole plan at once and `--max-parallel` only bounds how many run
/// concurrently within that round, not across rounds.
#[cfg(unix)]
fn setup_native_operate_dependency_fixture(
    directory: &Path,
    suffix: &str,
) -> NativeFrontierFixture {
    use std::os::unix::fs::PermissionsExt;

    let input_path = directory.join(format!("{suffix}.operate-dependency.native.input.json"));
    let mut input = json_file(native_case_fixture());
    let work_template = input["case_cells"]
        .as_array()
        .expect("native case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("native work template")
        .clone();
    let upstream_id = format!("work:operate-dependency-{suffix}-upstream");
    let downstream_id = format!("work:operate-dependency-{suffix}-downstream");
    for cell_id in [&upstream_id, &downstream_id] {
        let mut cell = work_template.clone();
        cell["id"] = json!(cell_id);
        cell["title"] = json!(format!("Operate dependency fixture {cell_id}"));
        cell["lifecycle"] = json!("active");
        cell["structure_ids"] = json!([]);
        input["case_cells"]
            .as_array_mut()
            .expect("native case cells")
            .push(cell);
    }
    input["case_relations"]
        .as_array_mut()
        .expect("native case relations")
        .push(json!({
            "id": format!("relation:operate-dependency-{suffix}"),
            "relation_type": "depends_on",
            "relation_strength": "hard",
            "from_id": downstream_id,
            "to_id": upstream_id,
            "evidence_ids": [],
            "source_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "accepted"
            },
            "metadata": {}
        }));
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&input).expect("serialize dependency fixture case space"),
    )
    .expect("write dependency fixture case space");
    let import_revision = format!("revision:operate-dependency-{suffix}-import");
    import_native_case_space_from_input(directory, &input_path, &import_revision);

    let mut step_ids = Vec::new();
    let mut steps = Vec::new();
    for (number, work_cell_id) in [&upstream_id, &downstream_id].into_iter().enumerate() {
        let script_path = directory.join(format!("{suffix}-dependency-worker-{number}.sh"));
        fs::write(&script_path, "#!/bin/sh\nset -eu\nprintf 'ok\\n'\n")
            .expect("write dependency fixture worker");
        let mut permissions = fs::metadata(&script_path)
            .expect("dependency worker metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make dependency worker executable");
        let binding_id = format!("worker_binding:operate-dependency-{suffix}-{number}");
        let binding_input = directory.join(format!(
            "{suffix}-dependency-worker-{number}.binding.input.json"
        ));
        write_pinned_worker_binding(&binding_input, &binding_id, directory, &script_path);
        let register = run_cli(&[
            "binding",
            "register",
            "--store",
            directory.to_str().expect("store path"),
            "--input",
            binding_input.to_str().expect("binding input path"),
            "--format",
            "json",
        ]);
        assert!(register.status.success(), "stderr: {}", stderr(&register));
        let step_id = format!("step:operate-dependency-{suffix}:{number}");
        step_ids.push(step_id.clone());
        steps.push(json!({
            "step_id": step_id,
            "work_cell_id": work_cell_id,
            "worker_binding_id": binding_id,
            "success_evidence_requirement_ids": ["evidence:native-schema-json-valid"],
            "allowed_transition_classes": [{
                "morphism_type": "update",
                "target_cell_types": ["work"],
                "to_lifecycles": ["resolved"]
            }]
        }));
    }
    let plan_id = format!("plan:operate-dependency-{suffix}");
    let plan_input = directory.join(format!("{suffix}-dependency.execution.plan.input.json"));
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": import_revision,
        "steps": steps,
        "provenance": {
            "source": {"kind": "human", "title": "Operate dependency fixture plan"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    fs::write(
        &plan_input,
        serde_json::to_string_pretty(&plan).expect("serialize dependency fixture plan"),
    )
    .expect("write dependency fixture plan");
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &plan_id,
        "--reviewer-id",
        "reviewer:operate-dependency-plan",
        "--reason",
        "Accept operate dependency fixture plan",
        "--base-revision-id",
        &import_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accepted_revision_id = stdout_json(&accept)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted dependency fixture revision")
        .to_owned();
    NativeFrontierFixture {
        plan_id,
        step_ids,
        work_cell_ids: vec![upstream_id, downstream_id],
        accepted_revision_id,
    }
}

#[cfg(unix)]
#[test]
fn operate_halts_on_round_budget_exhausted_when_dispatchable_work_remains() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_operate_dependency_fixture(&directory, "budget");

    let output = run_native_operate(&directory, &fixture, 4, 1);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["rounds_used"], json!(1));
    assert_eq!(
        value["result"]["rounds"][0]["status"],
        json!("round_executed")
    );
    assert_eq!(
        value["result"]["halt"]["halt"],
        json!("round_budget_exhausted")
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn operate_executes_two_independent_steps_then_halts_on_nothing_eligible() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "operate-two",
        &[
            ("work:operate-two-a", "printf 'a\\n'"),
            ("work:operate-two-b", "printf 'b\\n'"),
        ],
    );

    let output = run_native_operate(&directory, &fixture, 2, 2);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["rounds_used"], json!(1));
    assert_eq!(
        value["result"]["rounds"].as_array().expect("rounds").len(),
        1
    );
    assert_eq!(
        value["result"]["rounds"][0]["status"],
        json!("round_executed")
    );
    // rounds_used bounds rounds, not work: both independent steps dispatched
    // concurrently inside that one round (--max-parallel 2), so the actual
    // spawn count this invocation used is 2, not rounds_used's 1.
    assert_eq!(value["result"]["steps_dispatched"], json!(2));
    assert_eq!(value["result"]["halt"]["halt"], json!("nothing_eligible"));
    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[cfg(unix)]
#[test]
fn operate_halts_on_needs_retry_decision_after_a_worker_failure_and_stops_the_loop() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "operate-fail",
        &[("work:operate-fail-a", "exit 1")],
    );

    let output = run_native_operate(&directory, &fixture, 1, 3);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["rounds_used"], json!(1));
    assert_eq!(
        value["result"]["halt"]["halt"],
        json!("needs_retry_decision")
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// `--retry-step` on `operate` is refused, not consumed. Regression test for
/// the adversarial-execution-reviewer finding: `--retry-step` named on
/// `operate` used to stay exempt from `select_steps`'s `prior_failed` gate
/// on every round of the invocation, so a step whose worker always fails
/// was dispatched again automatically each round — an auto-retry loop
/// bounded only by `--max-rounds`, the retry engine ADR 0002 excludes and
/// ADR 0004 kept excluding. Consuming the consent after one attempt would
/// have fixed the loop but needed per-round bookkeeping
/// `docs/specs/operate-halt.fsl` does not model; refusing the flag outright
/// keeps `--retry-step` exactly the between-invocations act the spec already
/// models — run `run --frontier --retry-step <id>` explicitly, then
/// `operate`.
///
/// A/B this by reverting the refusal in
/// `src/native_cli/parser.rs::parse_operate` — this test then fails because
/// the command succeeds, dispatches the always-failing worker, and the
/// marker file exists.
#[cfg(unix)]
#[test]
fn operate_refuses_retry_step_with_a_usage_error_and_spawns_no_worker() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    // The worker's `working_directory` is `directory` itself
    // (`setup_native_frontier`), so a bare relative filename lands there.
    let marker = directory.join("worker-ran.marker");
    let fixture = setup_native_frontier(
        &directory,
        "operate-refuses-retry",
        &[("work:operate-refuses-retry-a", "touch worker-ran.marker")],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(native_operate_args(
            &directory,
            &fixture,
            &fixture.accepted_revision_id,
            1,
            4,
            &["capability:dispatch", "capability:native-run-worker"],
            &[&fixture.step_ids[0]],
        ))
        .output()
        .expect("run casegraphen operate");
    assert!(
        !output.status.success(),
        "operate --retry-step must be refused: {}",
        stdout_json(&output)
    );
    let refusal = stderr_json(&output);
    assert_eq!(refusal["error_code"], json!("usage"));
    assert!(
        !marker.is_file(),
        "a refused --retry-step must dispatch nothing; the worker ran anyway"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// A two-step fixture whose upstream step is freely dispatchable and whose
/// downstream step is blocked by an unaccepted `accepts` relation to a
/// review cell — `native_eval.rs::required_review_relations` /
/// `review_satisfied`, the same producer `needs_review`'s
/// `is_clearable_by_review` keys on. Modelled on
/// `setup_native_operate_dependency_fixture`, with the `depends_on` relation
/// to a second work cell replaced by an `accepts` relation to a review cell,
/// so `operate` gets one real round of dispatch before the halt this
/// produces is reachable — unlike `setup_native_operate_blocked_fixture`'s
/// single-cell fixtures, this exercises INV-OPERATE-002 against a morphism
/// log that is not empty.
#[cfg(unix)]
fn setup_native_operate_review_seam_fixture(
    directory: &Path,
    suffix: &str,
) -> NativeFrontierFixture {
    use std::os::unix::fs::PermissionsExt;

    let input_path = directory.join(format!("{suffix}.operate-review-seam.native.input.json"));
    let mut input = json_file(native_case_fixture());
    let work_template = input["case_cells"]
        .as_array()
        .expect("native case cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .expect("native work template")
        .clone();
    let upstream_id = format!("work:operate-review-seam-{suffix}-upstream");
    let downstream_id = format!("work:operate-review-seam-{suffix}-downstream");
    for cell_id in [&upstream_id, &downstream_id] {
        let mut cell = work_template.clone();
        cell["id"] = json!(cell_id);
        cell["title"] = json!(format!("Operate review seam fixture {cell_id}"));
        cell["lifecycle"] = json!("active");
        cell["structure_ids"] = json!([]);
        input["case_cells"]
            .as_array_mut()
            .expect("native case cells")
            .push(cell);
    }
    let review_id = format!("review:operate-review-seam-{suffix}-blocking");
    let space_id = input["space_id"].clone();
    input["case_cells"]
        .as_array_mut()
        .expect("native case cells")
        .push(json!({
            "id": review_id,
            "cell_type": "review",
            "space_id": space_id,
            "title": format!("Operate review seam fixture {suffix} blocking review"),
            "summary": Value::Null,
            "lifecycle": "active",
            "source_ids": [],
            "structure_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "metadata": {}
        }));
    input["case_relations"]
        .as_array_mut()
        .expect("native case relations")
        .push(json!({
            "id": format!("relation:operate-review-seam-{suffix}"),
            "relation_type": "accepts",
            "relation_strength": "hard",
            "from_id": downstream_id,
            "to_id": review_id,
            "evidence_ids": [],
            "source_ids": [],
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "accepted"
            },
            "metadata": {}
        }));
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&input).expect("serialize review seam fixture case space"),
    )
    .expect("write review seam fixture case space");
    let import_revision = format!("revision:operate-review-seam-{suffix}-import");
    import_native_case_space_from_input(directory, &input_path, &import_revision);

    let mut step_ids = Vec::new();
    let mut steps = Vec::new();
    for (number, work_cell_id) in [&upstream_id, &downstream_id].into_iter().enumerate() {
        let script_path = directory.join(format!("{suffix}-review-seam-worker-{number}.sh"));
        fs::write(&script_path, "#!/bin/sh\nset -eu\nprintf 'ok\\n'\n")
            .expect("write review seam fixture worker");
        let mut permissions = fs::metadata(&script_path)
            .expect("review seam worker metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("make review seam worker executable");
        let binding_id = format!("worker_binding:operate-review-seam-{suffix}-{number}");
        let binding_input = directory.join(format!(
            "{suffix}-review-seam-worker-{number}.binding.input.json"
        ));
        write_pinned_worker_binding(&binding_input, &binding_id, directory, &script_path);
        let register = run_cli(&[
            "binding",
            "register",
            "--store",
            directory.to_str().expect("store path"),
            "--input",
            binding_input.to_str().expect("binding input path"),
            "--format",
            "json",
        ]);
        assert!(register.status.success(), "stderr: {}", stderr(&register));
        let step_id = format!("step:operate-review-seam-{suffix}:{number}");
        step_ids.push(step_id.clone());
        steps.push(json!({
            "step_id": step_id,
            "work_cell_id": work_cell_id,
            "worker_binding_id": binding_id,
            "success_evidence_requirement_ids": ["evidence:native-schema-json-valid"],
            "allowed_transition_classes": [{
                "morphism_type": "update",
                "target_cell_types": ["work"],
                "to_lifecycles": ["resolved"]
            }]
        }));
    }
    let plan_id = format!("plan:operate-review-seam-{suffix}");
    let plan_input = directory.join(format!("{suffix}-review-seam.execution.plan.input.json"));
    let plan = json!({
        "schema": "highergraphen.case.workflow.execution_plan.v1",
        "schema_version": 1,
        "plan_id": plan_id,
        "case_space_id": native_case_space_id(),
        "base_revision_id": import_revision,
        "steps": steps,
        "provenance": {
            "source": {"kind": "human", "title": "Operate review seam fixture plan"},
            "confidence": 1.0,
            "review_status": "unreviewed"
        },
        "review_status": "unreviewed",
        "metadata": {}
    });
    fs::write(
        &plan_input,
        serde_json::to_string_pretty(&plan).expect("serialize review seam fixture plan"),
    )
    .expect("write review seam fixture plan");
    let propose = run_cli(&[
        "plan",
        "propose",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--input",
        plan_input.to_str().expect("plan input path"),
        "--format",
        "json",
    ]);
    assert!(propose.status.success(), "stderr: {}", stderr(&propose));
    let accept = run_cli(&[
        "plan",
        "accept",
        "--store",
        directory.to_str().expect("store path"),
        "--case-space-id",
        native_case_space_id(),
        "--plan-id",
        &plan_id,
        "--reviewer-id",
        "reviewer:operate-review-seam-plan",
        "--reason",
        "Accept operate review seam fixture plan",
        "--base-revision-id",
        &import_revision,
        "--actor-id",
        "actor:run-plan-review",
        "--capability-id",
        "capability:plan-review",
        "--operation-scope-id",
        native_case_space_id(),
        "--audience",
        "audit",
        "--source-boundary-id",
        "source_boundary:native-case-management-contract",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));
    let accepted_revision_id = stdout_json(&accept)["result"]["record"]["current_revision_id"]
        .as_str()
        .expect("accepted review seam fixture revision")
        .to_owned();
    NativeFrontierFixture {
        plan_id,
        step_ids,
        work_cell_ids: vec![upstream_id, downstream_id],
        accepted_revision_id,
    }
}

/// `INV-OPERATE-002` (`docs/specs/operate-halt.fsl`): no actor accepts the
/// claim it dispatched. ADR 0016 decision 4 states the stronger form —
/// "the actor seam is a halt, never a step"; `operate` never performs a
/// review itself, under any circumstance, including when a step it just
/// finished dispatching is immediately followed by another step blocked on
/// exactly that seam. A source-reading argument that `operate`'s dispatch
/// path never calls into `review_apply` is real but does not survive a
/// refactor, so this is an outcome check through the real binary instead:
/// drive `operate` through one real dispatch to a `needs_review` halt, then
/// read the actual persisted morphism log back and assert it contains no
/// `review` morphism at all. A hypothetical auto-accept added behind a flag
/// (the alternative ADR 0016 decision 4 rejects) would append exactly such
/// an entry, and this test would catch it; a grep over the source would not.
#[cfg(unix)]
#[test]
fn operate_halts_on_needs_review_and_appends_no_review_morphism() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_operate_review_seam_fixture(&directory, "seam");
    let history_before = run_native_case_store_command(&directory, "history");
    let entries_before_len = stdout_json(&history_before)["result"]["entries"]
        .as_array()
        .expect("history entries before operate")
        .len();

    let output = run_native_operate(&directory, &fixture, 2, 2);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["rounds_used"], json!(1));
    assert_eq!(
        value["result"]["rounds"][0]["status"],
        json!("round_executed")
    );
    assert_eq!(value["result"]["halt"]["halt"], json!("needs_review"));
    // The review cell an independent actor must accept, not the work cell it
    // gates: `review accept --target-id` takes the former, and against the
    // latter it appends a `waiver` that clears nothing.
    assert_eq!(
        value["result"]["halt"]["target_ids"],
        json!(["review:operate-review-seam-seam-blocking"])
    );

    // The outcome check: read the actual morphism log this invocation wrote
    // — only the entries appended since setup finished, not the plan-accept
    // and genesis entries setup itself legitimately recorded — and confirm
    // not one of them is a review morphism, independent of anything the
    // report claims about itself.
    let history = run_native_case_store_command(&directory, "history");
    let entries = stdout_json(&history)["result"]["entries"]
        .as_array()
        .expect("history entries")
        .clone();
    let appended_by_operate = &entries[entries_before_len..];
    assert!(
        !appended_by_operate.is_empty(),
        "the upstream step's dispatch must have appended at least one entry"
    );
    let review_entries = appended_by_operate
        .iter()
        .filter(|entry| entry["morphism"]["morphism_type"] == json!("review"))
        .count();
    assert_eq!(
        review_entries, 0,
        "operate must never append a review morphism itself: {appended_by_operate:?}"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #32: this is a "this should never actually take this long"
/// detector, not a synchronization primitive a test's own timing races
/// against. `wait_for_file`/`wait_for_line_count` poll for an event a
/// spawned process is expected to produce almost immediately (writing a
/// marker file at the very start of a worker script); the bound only exists
/// to fail loudly, with a clear message, if that event genuinely never
/// happens (a real bug) rather than hanging the test suite forever. It must
/// not be read as "this event normally takes up to N seconds" — under a
/// loaded machine, process spawn plus store setup before a worker even
/// starts can itself take a noticeable fraction of a generous bound, which
/// is exactly why this is generous rather than tight.
const EVENT_SHOULD_HAVE_HAPPENED_BY_NOW: Duration = Duration::from_secs(30);

fn wait_for_file(path: &Path, timeout_message: &str) {
    let wait_started_at = Instant::now();
    while !path.is_file() && wait_started_at.elapsed() < EVENT_SHOULD_HAVE_HAPPENED_BY_NOW {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.is_file(), "{timeout_message}");
}

fn wait_for_line_count(path: &Path, expected: usize, timeout_message: &str) {
    let wait_started_at = Instant::now();
    while fs::read_to_string(path).map_or(0, |contents| contents.lines().count()) < expected
        && wait_started_at.elapsed() < EVENT_SHOULD_HAVE_HAPPENED_BY_NOW
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(path).map_or(0, |contents| contents.lines().count()),
        expected,
        "{timeout_message}"
    );
}

/// The worker-side half of a file-based test rendezvous (issue #32):
/// embedded into a generated shell script, this polls for `marker`'s
/// existence with its own bounded, generous timeout so a genuine bug — the
/// marker never appearing — still fails the worker loudly instead of
/// hanging the test suite forever, rather than an unbounded wait.
///
/// The bound here (120 s) is deliberately longer than
/// `EVENT_SHOULD_HAVE_HAPPENED_BY_NOW`'s 30 s: that one waits for a worker
/// to announce it started, effectively instant even under load, while this
/// one waits for the *test* to run one or more whole external CLI
/// invocations (spawn, load the store, evaluate, snapshot, append) before
/// signalling — measured under three concurrent full-suite runs
/// (`cargo test --test command` x3, real reproduction of "concurrent cargo
/// processes competing for the target-dir lock") to occasionally exceed
/// 30 s; 120 s did not reproduce a timeout in that same stress test. If this
/// still times out under some future load, the fix is a more generous bound
/// again, not a shorter one disguised as a shorter sleep.
///
/// Pairs with a test creating `marker` after its own action finishes
/// (`signal_rendezvous_marker`) once the worker's `started` write has
/// already been observed via `wait_for_file`/`wait_for_line_count` — so a
/// test's timing depends on a real event, never on a fixed sleep long
/// enough to usually win a race. It can equally pair with a *sibling*
/// worker creating the marker as its own completion signal, for a
/// worker-to-worker rendezvous with no test-side action in between.
fn shell_wait_for_marker(marker: &Path) -> String {
    format!(
        "i=0; while [ ! -f '{}' ] && [ \"$i\" -lt 1200 ]; do sleep 0.1; i=$((i+1)); done",
        marker.display()
    )
}

/// The test-side half of the rendezvous `shell_wait_for_marker` waits for.
fn signal_rendezvous_marker(marker: &Path) {
    fs::write(marker, "").expect("write rendezvous marker");
}

#[cfg(unix)]
fn dedicated_session_utilities_available() -> bool {
    use std::os::unix::fs::PermissionsExt;

    let executable = |path: &Path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    [
        "/usr/bin/setsid",
        "/bin/setsid",
        "/usr/local/bin/setsid",
        "/opt/homebrew/bin/setsid",
    ]
    .iter()
    .any(|path| executable(Path::new(path)))
        && ["/bin/kill", "/usr/bin/kill"]
            .iter()
            .any(|path| executable(Path::new(path)))
}

#[cfg(unix)]
fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while process_exists(pid) && started.elapsed() < timeout {
        thread::sleep(Duration::from_millis(10));
    }
    !process_exists(pid)
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", "--"])
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn kill_process(pid: u32) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", "--"])
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn assert_native_store_valid_and_rebuilds(directory: &Path) {
    let validate = run_native_case_store_command(directory, "validate");
    assert_eq!(
        stdout_json(&validate)["result"]["validation"]["valid"],
        json!(true)
    );
    let rebuild = run_native_case_store_command(directory, "rebuild");
    let rebuild_json = stdout_json(&rebuild);
    assert!(rebuild_json["result"]["rebuild"]["revisions"]
        .as_array()
        .expect("rebuilt revisions")
        .iter()
        .all(|revision| matches!(
            revision["snapshot_status"].as_str(),
            Some("agrees" | "not_scheduled")
        )));
}

fn only_run_file(directory: &Path, file_name: &str) -> PathBuf {
    let mut matches = run_files(directory, file_name);
    assert_eq!(matches.len(), 1, "expected one {file_name}");
    matches.remove(0)
}

/// Locates the one case space's `morphism_log.jsonl` under a temp store
/// directory without reimplementing `path_helpers::path_segment`'s escaping
/// — every fixture in this file drives exactly one case space per store.
fn find_morphism_log_path(directory: &Path) -> PathBuf {
    let root = directory.join("native_case_spaces");
    let mut matches = fs::read_dir(&root)
        .expect("read native_case_spaces directory")
        .map(|entry| entry.expect("case space directory entry").path())
        .map(|case_space_dir| case_space_dir.join("morphism_log.jsonl"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one case space's morphism_log.jsonl under {}",
        root.display()
    );
    matches.remove(0)
}

fn run_files(directory: &Path, file_name: &str) -> Vec<PathBuf> {
    let mut matches = fs::read_dir(directory.join("runs"))
        .expect("read runs directory")
        .map(|entry| entry.expect("run entry").path().join(file_name))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    matches.sort();
    matches
}

fn assert_worker_artifact_tamper_detected(
    file_name: &str,
    expected_label: &str,
    tamper: impl FnOnce(&Path),
) {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_run(
        &directory,
        &format!("{file_name}-tamper"),
        "printf 'anchored stdout\\n'; printf 'anchored stderr\\n' >&2",
    );
    let first = run_native_step(&directory, &fixture, true, None);
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    let first_json = stdout_json(&first);
    let trace_id = first_json["result"]["trace"]["trace_id"]
        .as_str()
        .expect("trace id");
    let result_revision_id = first_json["result"]["trace"]["result_revision_id"]
        .as_str()
        .expect("trace result revision");
    let trace_path = only_run_file(&directory, "execution.trace.json");
    tamper(&trace_path.parent().expect("run directory").join(file_name));

    let verified = run_native_step_with_base(&directory, &fixture, result_revision_id, true, None);

    assert!(!verified.status.success());
    let error = stderr(&verified);
    assert!(error.contains(trace_id), "{error}");
    assert!(error.contains(expected_label), "{error}");
    assert!(
        error.contains("does not match its recorded content hash"),
        "{error}"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!("{} should be readable for hashing: {error}", path.display())
    });
    format!("{:x}", Sha256::digest(bytes))
}

fn replayed_work_lifecycle(replay: &Value) -> &str {
    replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .and_then(|cell| cell["lifecycle"].as_str())
        .expect("work lifecycle")
}

fn native_attached_evidence(id: &str, review_status: &str) -> Value {
    json!({
        "id": id,
        "cell_type": "evidence",
        "lifecycle": "active",
        "space_id": "space:higher-graphen-casegraphen",
        "title": format!("Attached evidence {id}"),
        "source_ids": ["source:native-cli"],
        "structure_ids": [],
        "metadata": {
            "evidence_boundary": "source_backed",
            "content_hash": "caller-bogus-hash"
        },
        "provenance": {
            "confidence": 0.6,
            "review_status": review_status,
            "source": {"kind": "document", "title": "Batch evidence fixture"}
        }
    })
}

fn native_work_cell(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "cell_type": "work",
        "lifecycle": "active",
        "space_id": "space:higher-graphen-casegraphen",
        "title": title,
        "source_ids": ["source:native-cli"],
        "structure_ids": [],
        "metadata": {},
        "provenance": {
            "confidence": 0.8,
            "review_status": "unreviewed",
            "source": {"kind": "human", "title": "Batch coverage fixture"}
        }
    })
}

fn native_evidence_requirement(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "cell_type": "evidence",
        "lifecycle": "proposed",
        "space_id": "space:higher-graphen-casegraphen",
        "title": title,
        "source_ids": ["source:native-cli"],
        "structure_ids": [],
        "metadata": {},
        "provenance": {
            "confidence": 0.8,
            "review_status": "unreviewed",
            "source": {"kind": "human", "title": "Batch coverage fixture"}
        }
    })
}

fn native_requires_evidence_relation(id: &str, from_id: &str, to_id: &str) -> Value {
    json!({
        "id": id,
        "relation_type": "requires_evidence",
        "relation_strength": "hard",
        "from_id": from_id,
        "to_id": to_id,
        "evidence_ids": [],
        "source_ids": ["source:native-cli"],
        "metadata": {},
        "provenance": {
            "confidence": 1.0,
            "review_status": "accepted",
            "source": {"kind": "human", "title": "Batch coverage fixture"}
        }
    })
}

fn write_json_value(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize JSON fixture"),
    )
    .expect("write JSON fixture");
}

fn invalid_core_extensions(target_id: &str) -> Value {
    json!({
        "valuations": [
            {
                "id": "valuation:metadata-core-extension-block",
                "target": {
                    "ref": target_id
                },
                "order_type": "threshold_acceptance",
                "confidence": 0.5,
                "provenance": {
                    "source": {
                        "kind": "ai",
                        "title": "Metadata supplied core extension"
                    },
                    "confidence": 0.5,
                    "review_status": "unreviewed"
                },
                "review_status": "candidate"
            }
        ]
    })
}

fn stdout_json(output: &Output) -> Value {
    let stdout = stdout(output);
    assert_eq!(stdout.lines().count(), 1);
    serde_json::from_str(stdout.trim_end()).expect("stdout JSON")
}

/// Parses a `--format json` refusal from stderr (issue #22). A refusal
/// never touches stdout or `--output`, so this reads stderr, mirroring
/// `stdout_json` for the success path.
fn stderr_json(output: &Output) -> Value {
    let stderr = stderr(output);
    assert_eq!(stderr.lines().count(), 1, "stderr: {stderr}");
    serde_json::from_str(stderr.trim_end()).expect("stderr refusal JSON")
}

fn json_file(path: PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(&path).expect("read JSON file"))
        .unwrap_or_else(|error| panic!("{} should be valid JSON: {error}", path.display()))
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout utf8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr utf8")
}

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "casegraphen-cli-test-{}-{nanos}-{counter}",
        std::process::id()
    ))
}

fn projection_fixture() -> PathBuf {
    repo_path("schemas/casegraphen/projection.example.json")
}

fn native_case_fixture() -> PathBuf {
    repo_path("schemas/casegraphen/native.case.space.example.json")
}

fn native_case_space_id() -> &'static str {
    "case_space:native-case-management-contract"
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn schema_fixture_paths() -> Vec<PathBuf> {
    [
        "schemas/casegraphen/case.graph.example.json",
        "schemas/casegraphen/coverage.policy.example.json",
        "schemas/casegraphen/projection.example.json",
        "schemas/casegraphen/workflow.graph.example.json",
        "schemas/casegraphen/github.issue-snapshot.example.json",
        "schemas/casegraphen/native.case.space.example.json",
        "schemas/casegraphen/native.case.report.example.json",
        "schemas/casegraphen/execution.plan.example.json",
        "schemas/casegraphen/worker.binding.example.json",
        "schemas/casegraphen/worker.report.example.json",
        "schemas/casegraphen/execution.trace.example.json",
        "schemas/casegraphen/operation-gate-profiles.example.json",
        "schemas/casegraphen/evidence.packet.example.json",
        "schemas/casegraphen/report-schema-aliases.json",
        "schemas/casegraphen/case.graph.schema.json",
        "schemas/casegraphen/coverage.policy.schema.json",
        "schemas/casegraphen/projection.schema.json",
        "schemas/casegraphen/case.report.schema.json",
        "schemas/casegraphen/workflow.graph.schema.json",
        "schemas/casegraphen/github.issue-snapshot.schema.json",
        "schemas/casegraphen/native.case.space.schema.json",
        "schemas/casegraphen/native.morphism-log-entry.schema.json",
        "schemas/casegraphen/native.morphism-propose-input.schema.json",
        "schemas/casegraphen/native.morphism-propose-input.example.json",
        "schemas/casegraphen/native.case.report.schema.json",
        "schemas/casegraphen/execution.plan.schema.json",
        "schemas/casegraphen/worker.binding.schema.json",
        "schemas/casegraphen/worker.report.schema.json",
        "schemas/casegraphen/execution.trace.schema.json",
        "schemas/casegraphen/operation-gate-profiles.schema.json",
        "schemas/casegraphen/native-cli.report.schema.json",
        "schemas/casegraphen/evidence.packet.schema.json",
        "schemas/casegraphen/native-cli.refusal.schema.json",
        "schemas/casegraphen/native-cli.refusal.example.json",
    ]
    .iter()
    .map(|path| repo_path(path))
    .collect()
}

fn native_schema_example_pairs() -> Vec<(PathBuf, PathBuf)> {
    [
        (
            "schemas/casegraphen/github.issue-snapshot.schema.json",
            "schemas/casegraphen/github.issue-snapshot.example.json",
        ),
        (
            "schemas/casegraphen/native.case.space.schema.json",
            "schemas/casegraphen/native.case.space.example.json",
        ),
        (
            "schemas/casegraphen/native.case.report.schema.json",
            "schemas/casegraphen/native.case.report.example.json",
        ),
        (
            "schemas/casegraphen/native.morphism-propose-input.schema.json",
            "schemas/casegraphen/native.morphism-propose-input.example.json",
        ),
        (
            "schemas/casegraphen/execution.plan.schema.json",
            "schemas/casegraphen/execution.plan.example.json",
        ),
        (
            "schemas/casegraphen/worker.binding.schema.json",
            "schemas/casegraphen/worker.binding.example.json",
        ),
        (
            "schemas/casegraphen/worker.report.schema.json",
            "schemas/casegraphen/worker.report.example.json",
        ),
        (
            "schemas/casegraphen/execution.trace.schema.json",
            "schemas/casegraphen/execution.trace.example.json",
        ),
        (
            "schemas/casegraphen/operation-gate-profiles.schema.json",
            "schemas/casegraphen/operation-gate-profiles.example.json",
        ),
        (
            "schemas/casegraphen/evidence.packet.schema.json",
            "schemas/casegraphen/evidence.packet.example.json",
        ),
        (
            "schemas/casegraphen/native-cli.refusal.schema.json",
            "schemas/casegraphen/native-cli.refusal.example.json",
        ),
    ]
    .iter()
    .map(|(schema, example)| (repo_path(schema), repo_path(example)))
    .collect()
}

/// Rewrites the `--format json` an args-builder helper hardcodes to
/// `--format text`, for a `--format text` variant of a test the JSON form
/// already covers — issue #35 render-only tests reuse the exact fixture
/// setup an existing JSON-format test uses rather than duplicating it.
#[cfg(unix)]
fn set_format_text(args: &mut [String]) {
    let format_index = args
        .iter()
        .position(|argument| argument == "--format")
        .expect("args include --format");
    args[format_index + 1] = "text".to_owned();
}

/// Issue #35: `run --frontier --format text` renders `result.halt`/
/// `result.halts` as text instead of JSON. Same fixture as
/// `native_run_frontier_reports_needs_evidence_halt_when_nothing_is_dispatchable`
/// — `needs_evidence` is the halt member with a non-empty `next_operations`
/// (exactly one `evidence attach`, per `native_halt.rs::build_halt_report`).
#[cfg(unix)]
#[test]
fn native_run_frontier_format_text_renders_needs_evidence_halt_with_next_operations() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_operate_blocked_fixture(
        &directory,
        "text-evidence",
        "requires_evidence",
        "goal:operate-blocked-text-evidence-missing",
        "goal",
    );

    let mut args = native_frontier_args(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
        true,
    );
    set_format_text(&mut args);

    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(&args)
        .output()
        .expect("run casegraphen run --frontier --format text");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let rendered = stdout(&output);

    assert!(rendered.contains("Halt: needs_evidence"), "{rendered}");
    assert!(
        rendered.contains(&format!(
            "Completed through: {}",
            fixture.accepted_revision_id
        )),
        "{rendered}"
    );
    // The unsatisfied requirement id `evidence attach --satisfies` takes.
    assert!(
        rendered.contains("goal:operate-blocked-text-evidence-missing"),
        "{rendered}"
    );
    assert!(rendered.contains("evidence attach"), "{rendered}");
    assert!(
        rendered.contains(&format!("case_space_id: {}", native_case_space_id())),
        "{rendered}"
    );
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #35: `operate --format text`. Same fixture as
/// `operate_executes_two_independent_steps_then_halts_on_nothing_eligible`
/// — `nothing_eligible` is the halt member with empty `target_ids` and
/// empty `next_operations` (`native_halt.rs::build_halt_report`), so this
/// is the "no next_operations" counterpart to the `needs_evidence` test
/// above.
#[cfg(unix)]
#[test]
fn native_operate_format_text_renders_nothing_eligible_halt_without_next_operations() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture = setup_native_frontier(
        &directory,
        "text-nothing-eligible",
        &[
            ("work:text-nothing-eligible-a", "printf 'a\\n'"),
            ("work:text-nothing-eligible-b", "printf 'b\\n'"),
        ],
    );

    let mut args = native_operate_args(
        &directory,
        &fixture,
        &fixture.accepted_revision_id,
        2,
        2,
        &["capability:dispatch", "capability:native-run-worker"],
        &[],
    );
    set_format_text(&mut args);

    let output = Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(&args)
        .output()
        .expect("run casegraphen operate --format text");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let rendered = stdout(&output);

    assert!(rendered.contains("Halt: nothing_eligible"), "{rendered}");
    assert!(rendered.contains("Targets: (none)"), "{rendered}");
    assert!(rendered.contains("Next operations: (none)"), "{rendered}");
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #34's positive counterpart to `two_holder_evidence_missing_fixture`:
/// the same `work:w1` -> `evidence:a` -> `evidence:x`, `work:w2` -> `evidence:x`
/// shape, but now *every* holder of `evidence:x` is covered — `evidence:a`
/// through trusted `evidence:y` (as before) and `work:w2` directly through a
/// second trusted `evidence:z`. `docs/specs/requirement-satisfaction.fsl`'s
/// `satisfied_for_all()` requires every holder, not just one, so this fixture
/// is what proves the every-holder rule is not simply "always false now" —
/// it reads `true` exactly when it should.
fn two_holder_evidence_fully_covered_fixture() -> Value {
    let space_id = "space:two-holder-evidence-fully-covered";
    let source_boundary = json!({
        "id": "source_boundary:two-holder-evidence-fully-covered",
        "included_sources": ["source:test"],
        "excluded_sources": [],
        "adapters": ["test.fixture.v1"],
        "accepted_fact_policy": "fixture facts are accepted test input",
        "inference_policy": "fixture declares its own coverage claims",
        "information_loss": []
    });
    json!({
        "schema": "highergraphen.case.space.v1",
        "schema_version": 1,
        "case_space_id": "case_space:two-holder-evidence-fully-covered",
        "space_id": space_id,
        "case_cells": [
            {
                "id": "work:w1", "cell_type": "work", "lifecycle": "active",
                "space_id": space_id, "title": "W1",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.9, "review_status": "reviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "work:w2", "cell_type": "work", "lifecycle": "active",
                "space_id": space_id, "title": "W2",
                "source_ids": ["source:test"], "structure_ids": [], "metadata": {},
                "provenance": {"confidence": 0.9, "review_status": "reviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:a", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "A (intermediate evidence)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "inferred"},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:x", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "X (shared sub-evidence)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "inferred"},
                "provenance": {"confidence": 0.5, "review_status": "unreviewed",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "evidence:y", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "Y (trusted, covers A only)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "source_backed"},
                "provenance": {"confidence": 0.9, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "t"}}
            },
            {
                "id": "evidence:z", "cell_type": "evidence", "lifecycle": "active",
                "space_id": space_id, "title": "Z (trusted, covers X directly)",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {"evidence_boundary": "source_backed"},
                "provenance": {"confidence": 0.9, "review_status": "unreviewed",
                               "source": {"kind": "document", "title": "t"}}
            },
            {
                "id": "capability:test-mutation", "cell_type": "custom:capability", "lifecycle": "accepted",
                "space_id": space_id, "title": "Authorize test mutations",
                "source_ids": ["source:test"], "structure_ids": [],
                "metadata": {
                    "actor_ids": ["actor:test-mutation-cli"],
                    "operations": ["evidence-attach", "review", "cell-transition"]
                },
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "document", "title": "t"}}
            }
        ],
        "case_relations": [
            {
                "id": "relation:w1-requires-a", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "work:w1", "to_id": "evidence:a",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:a-requires-x", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "evidence:a", "to_id": "evidence:x",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:w2-requires-x", "relation_type": "requires_evidence",
                "relation_strength": "hard", "from_id": "work:w2", "to_id": "evidence:x",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:y-satisfies-a", "relation_type": "satisfies_evidence_requirement",
                "relation_strength": "diagnostic", "from_id": "evidence:y", "to_id": "evidence:a",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            },
            {
                "id": "relation:z-satisfies-x", "relation_type": "satisfies_evidence_requirement",
                "relation_strength": "diagnostic", "from_id": "evidence:z", "to_id": "evidence:x",
                "evidence_ids": [], "source_ids": ["source:test"], "metadata": {},
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}}
            }
        ],
        "morphism_log": [
            {
                "schema": "highergraphen.case.morphism_log_entry.v1", "schema_version": 1,
                "case_space_id": "case_space:two-holder-evidence-fully-covered", "sequence": 1,
                "entry_id": "morphism_log_entry:genesis", "morphism_id": "morphism:genesis",
                "target_revision_id": "revision:two-holder-evidence-fully-covered-base",
                "morphism": {
                    "morphism_id": "morphism:genesis", "morphism_type": "create",
                    "target_revision_id": "revision:two-holder-evidence-fully-covered-base",
                    "added_ids": [], "updated_ids": [], "retired_ids": [], "preserved_ids": [],
                    "violated_invariant_ids": [], "review_status": "accepted",
                    "evidence_ids": [], "source_ids": ["source:test"],
                    "metadata": {
                        "lift_semantics": "test_fixture_to_case_space",
                        "source_boundary_id": "source_boundary:two-holder-evidence-fully-covered",
                        "source_boundary": source_boundary
                    }
                },
                "actor_id": "actor:test-author", "recorded_at": "2026-08-02T00:00:00Z",
                "provenance": {"confidence": 1.0, "review_status": "accepted",
                               "source": {"kind": "human", "title": "t"}},
                "source_ids": ["source:test"], "replay_checksum": ""
            }
        ],
        "projections": [],
        "revision": {
            "revision_id": "revision:two-holder-evidence-fully-covered-base",
            "case_space_id": "case_space:two-holder-evidence-fully-covered",
            "applied_entry_ids": ["morphism_log_entry:genesis"],
            "applied_morphism_ids": ["morphism:genesis"],
            "checksum": "", "created_at": "2026-08-02T00:00:00Z",
            "source_ids": ["source:test"], "metadata": {}
        },
        "close_policy_id": null,
        "metadata": {"source_boundary": source_boundary}
    })
}

/// Issue #34's every-holder rule end to end through the real binary: unlike
/// `space_reason_text_never_annotates_an_evidence_missing_finding_even_at_a_shared_requirement`,
/// every holder of `evidence:x` is covered here, so `requirement_satisfied`
/// must read `true` and no obstruction may still name `evidence:x` — the
/// positive case the negative one alone cannot rule out.
#[test]
fn space_reason_reports_requirement_satisfied_true_once_every_holder_is_covered() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let fixture_path = directory.join("two-holder-evidence-fully-covered-fixture.case.space.json");
    write_json_value(&fixture_path, &two_holder_evidence_fully_covered_fixture());
    import_native_case_space_from_input(
        &directory,
        &fixture_path,
        "revision:two-holder-evidence-fully-covered-base",
    );
    let store = directory.to_str().expect("temp path").to_owned();
    let case_space_id = "case_space:two-holder-evidence-fully-covered";

    let json_report = run_cli(&[
        "space",
        "reason",
        "--store",
        &store,
        "--case-space-id",
        case_space_id,
        "--format",
        "json",
    ]);
    assert!(
        json_report.status.success(),
        "stderr: {}",
        stderr(&json_report)
    );
    let evaluation = &stdout_json(&json_report)["result"]["evaluation"];
    let x_gap = evaluation["review_gaps"]
        .as_array()
        .expect("review gaps")
        .iter()
        .find(|gap| gap["target_id"] == json!("evidence:x"))
        .expect("evidence:x review gap");
    assert_eq!(x_gap["requirement_satisfied"], json!(true));
    assert!(
        !evaluation["obstructions"]
            .as_array()
            .expect("obstructions")
            .iter()
            .any(|obstruction| obstruction["witness_ids"] == json!(["evidence:x"])),
        "no holder of evidence:x should still be blocked once every holder is covered: {:?}",
        evaluation["obstructions"]
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #33 / ADR 0018 end to end through the real binary: a step fails, is
/// retried with `--retry-step`, and succeeds — the retry-originated trace
/// must name the failed trace it retried in `metadata.retried_trace_ids`.
/// The worker script fails exactly once (a marker file records that the
/// first attempt happened) and succeeds on any later invocation, so the
/// retry is a real second dispatch, not a refusal.
#[cfg(unix)]
#[test]
fn native_run_frontier_retry_names_the_failed_trace_it_retried() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let attempted_marker = directory.join("retry-lineage-attempted");
    let script = format!(
        "if [ -f '{marker}' ]; then printf 'succeeded-on-retry\\n'; else touch '{marker}'; \
         printf 'failed-on-first-attempt\\n' >&2; exit 1; fi",
        marker = attempted_marker.display()
    );
    let fixture = setup_native_frontier(
        &directory,
        "retry-lineage",
        &[("work:retry-lineage", script.as_str())],
    );

    let failed = run_native_frontier(&directory, &fixture, 1);
    assert!(failed.status.success(), "stderr: {}", stderr(&failed));
    let failed_json = stdout_json(&failed);
    assert_eq!(failed_json["result"]["status"], json!("round_executed"));
    let failed_trace = &failed_json["result"]["traces"][0];
    assert_eq!(failed_trace["dispatch_state"], json!("failed"));
    assert!(
        failed_trace["metadata"].get("retried_trace_ids").is_none(),
        "the first-ever attempt of a step must record no retry lineage: {failed_trace}"
    );
    let failed_trace_id = failed_trace["trace_id"]
        .as_str()
        .expect("failed trace id")
        .to_owned();
    let revision_after_failure = failed_json["result"]["result_revision_id"]
        .as_str()
        .expect("revision after the failed attempt")
        .to_owned();

    let retried = run_native_frontier_with(
        &directory,
        &fixture,
        &revision_after_failure,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[&fixture.step_ids[0]],
    );
    assert!(retried.status.success(), "stderr: {}", stderr(&retried));
    let retried_json = stdout_json(&retried);
    assert_eq!(retried_json["result"]["status"], json!("round_executed"));
    let retried_trace = &retried_json["result"]["traces"][0];
    assert_eq!(retried_trace["dispatch_state"], json!("completed"));
    assert_eq!(
        retried_trace["metadata"]["retried_trace_ids"],
        json!([failed_trace_id]),
        "the retry-originated trace must name exactly the failed trace it retried: {retried_trace}"
    );

    // The anchored trace file on disk carries the same fact, not only the
    // command's own JSON report of it. Two runs now exist (the failure and
    // the retry), so the retried one is picked out by its trace_id rather
    // than assuming there is only one.
    let retried_trace_id = retried_trace["trace_id"]
        .as_str()
        .expect("retried trace id");
    let anchored_trace = run_files(&directory, "execution.trace.json")
        .into_iter()
        .map(json_file)
        .find(|trace| trace["trace_id"] == json!(retried_trace_id))
        .expect("the retried trace's own run directory carries an anchored trace file");
    assert_eq!(
        anchored_trace["metadata"]["retried_trace_ids"],
        json!([failed_trace_id])
    );

    assert_native_store_valid_and_rebuilds(&directory);
    fs::remove_dir_all(directory).expect("remove temp directory");
}

/// Issue #145: `skills/casegraphen-operate/SKILL.md` tells an operator that
/// running `lift` against a document they are unsure of is safe, because a
/// refused lift writes nothing — that is what makes the evaluator's half of
/// the rule set reachable at all, since `morphism.metadata` is an open object
/// the schema cannot constrain. A documented safety claim that nothing pins
/// rots the way the entry-ladder caution note did (#130). This pins it:
/// nothing is created anywhere under the store's parent, not only the store
/// path itself, so a temporary file or work directory beside it would fail
/// here too. It also pins that the refusal carries every violation as data
/// rather than only the first, which is the other half of the claim.
#[test]
fn a_refused_lift_reports_every_violation_and_writes_nothing() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp dir");
    let store = directory.join("store");

    // Schema-valid — `$defs.metadata` is `{"type": "object"}` — but the
    // evaluator requires `lift_semantics` and `source_boundary` on the first
    // morphism, so this is exactly the shape that validates clean and refuses.
    let fixture = json!({
        "schema": "highergraphen.case.space.v1",
        "schema_version": 1,
        "case_space_id": "case_space:refusal-writes-nothing",
        "space_id": "space:refusal-writes-nothing",
        "case_cells": [],
        "case_relations": [],
        "projections": [],
        "morphism_log": [{
            "schema": "highergraphen.case.morphism_log_entry.v1",
            "schema_version": 1,
            "case_space_id": "case_space:refusal-writes-nothing",
            "sequence": 1,
            "entry_id": "entry:1",
            "morphism_id": "morphism:1",
            "target_revision_id": "revision:1",
            "actor_id": "actor:test",
            "recorded_at": "2026-08-07T00:00:00Z",
            "provenance": {
                "source": {"kind": "human"},
                "confidence": 1.0,
                "review_status": "unreviewed"
            },
            "source_ids": [],
            "replay_checksum": "x",
            "morphism": {
                "morphism_id": "morphism:1",
                "morphism_type": "create",
                "target_revision_id": "revision:1",
                "added_ids": [], "updated_ids": [], "retired_ids": [],
                "preserved_ids": [], "violated_invariant_ids": [],
                "review_status": "unreviewed", "evidence_ids": [],
                "source_ids": [],
                "metadata": {}
            }
        }],
        "revision": {
            "revision_id": "revision:1",
            "case_space_id": "case_space:refusal-writes-nothing",
            "applied_entry_ids": ["entry:1"],
            "applied_morphism_ids": ["morphism:1"],
            "checksum": "x",
            "created_at": "2026-08-07T00:00:00Z",
            "source_ids": [], "metadata": {}
        },
        "metadata": {"source_boundary": {
            "id": "source_boundary:1",
            "included_sources": ["s"], "adapters": ["a"],
            "accepted_fact_policy": "p", "inference_policy": "p",
            "information_loss": []
        }}
    });
    let fixture_path = directory.join("evaluator-refuses.case.space.json");
    write_json_value(&fixture_path, &fixture);

    let lifted = run_cli(&[
        "lift",
        "native",
        "--store",
        store.to_str().expect("store path"),
        "--input",
        fixture_path.to_str().expect("fixture path"),
        "--revision-id",
        "revision:1",
        "--format",
        "json",
    ]);
    assert!(!lifted.status.success(), "stdout: {}", stdout(&lifted));

    let refusal: serde_json::Value =
        serde_json::from_str(stderr(&lifted).trim_end()).expect("refusal parses");
    assert_eq!(refusal["error_code"], json!("store_integrity"), "{refusal}");

    let violations = refusal["data"]["violations"]
        .as_array()
        .unwrap_or_else(|| panic!("refusal carries data.violations: {refusal}"));
    let fields: Vec<&str> = violations
        .iter()
        .filter_map(|violation| violation["field"].as_str())
        .collect();
    assert!(
        fields.contains(&"morphism.metadata.lift_semantics")
            && fields.contains(&"morphism.metadata.source_boundary"),
        "both violations must be reported in one refusal, not one per invocation: {refusal}"
    );
    for violation in violations {
        for key in ["code", "field", "message"] {
            assert!(
                violation[key].is_string(),
                "each violation carries {key} as a string: {refusal}"
            );
        }
    }

    let written: Vec<PathBuf> = fs::read_dir(&directory)
        .expect("read temp dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path != &fixture_path)
        .collect();
    assert!(
        written.is_empty(),
        "a refused lift must write nothing beside the input — SKILL.md tells \
         operators the run is safe on that basis: {written:?}"
    );

    fs::remove_dir_all(&directory).ok();
}
