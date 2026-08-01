#![allow(missing_docs)]

use arbtest::arbitrary::Arbitrary;
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
    for args in [["version"], ["--version"], ["-V"]] {
        let output = run_cli(&args);

        assert!(output.status.success(), "stderr: {}", stderr(&output));
        assert_eq!(
            stdout(&output).trim_end(),
            format!("casegraphen {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(stderr(&output).is_empty());
    }
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
        reason_json["result"]["evaluation"]["status"],
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
        "bb34c4e1e88ca67a5afb3e40068812b2b0f0886489acf460d01938181ade014a",
        "the JSON bytes changed from the pre-text-renderer baseline"
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

    let right_capability = transition("capability:durable-mutation");
    assert!(
        right_capability.status.success(),
        "stderr: {}",
        stderr(&right_capability)
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
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("worker-started");
    let script = format!(
        "printf 'started\\n' > '{}'; sleep 0.5; printf 'worker-output\\n'",
        worker_started.display()
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
    let wait_started_at = Instant::now();
    while !worker_started.is_file() && wait_started_at.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker_started.is_file(),
        "worker did not start before timeout"
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

    let output = child.wait_with_output().expect("wait for run --step");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("entry sequence must be"),
        "stderr: {}",
        stderr(&output)
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
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let completion_order = directory.join("worker-completion-order");
    let first_script = format!(
        "sleep 1\nprintf 'first\\n' >> '{}'\nprintf 'first-output\\n'",
        completion_order.display()
    );
    let second_script = format!(
        "printf 'second\\n' >> '{}'\nprintf 'second-output\\n'",
        completion_order.display()
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
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("frontier-membership-worker-started");
    let script = format!(
        "printf 'started\\n' > '{}'\nsleep 0.5\nprintf 'worker-output\\n'",
        worker_started.display()
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
fn native_run_frontier_retry_recovers_stale_started_trace() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let worker_started = directory.join("stale-started-worker-started");
    let script = format!(
        "printf 'started\\n' > '{}'\nsleep 1\nprintf 'worker-output\\n'",
        worker_started.display()
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
    let started_trace_path = only_run_file(&directory, "execution.trace.json");
    let started_trace = json_file(started_trace_path);
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

    let recovered = run_native_frontier_with(
        &directory,
        &fixture,
        &intervening_revision,
        1,
        &["capability:dispatch", "capability:native-run-worker"],
        &[fixture.step_ids[0].as_str()],
    );

    assert!(recovered.status.success(), "stderr: {}", stderr(&recovered));
    let recovered_json = stdout_json(&recovered);
    assert_eq!(recovered_json["result"]["status"], json!("round_executed"));
    assert_eq!(
        recovered_json["result"]["traces"][0]["transition_applied"],
        json!(true)
    );
    assert_eq!(
        fs::read_dir(directory.join("runs"))
            .expect("read retry run directories")
            .count(),
        2
    );
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
                        "to_id": "goal:native-case-contract",
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
        "goal:native-case-contract",
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
            "to_id": "goal:native-case-contract",
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
fn native_evidence_attach_batches_cells_and_coverage_in_one_revision() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:batch-evidence-base");
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
            "revision:batch-evidence-base",
            "--input",
            first_path.to_str().expect("first evidence path"),
            "--satisfies",
            "goal:native-case-contract",
            "--input",
            second_path.to_str().expect("second evidence path"),
            "--satisfies",
            "case:native-contract-example",
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
        json!("morphism:evidence-attach:evidence~3abatch-first:2")
    );
    assert_eq!(
        entry["morphism"]["target_revision_id"],
        json!("revision:evidence-attach:evidence~3abatch-first:2")
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
                "to_id": "goal:native-case-contract",
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
                "to_id": "case:native-contract-example",
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
        2,
        "genesis plus one batch attach must be the whole log"
    );
    let validation = run_native_case_store_command(&directory, "validate");
    assert_eq!(
        stdout_json(&validation)["result"]["validation"]["valid"],
        json!(true)
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
                "goal:native-case-contract",
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
    let refused = attach("case:native-contract-example");

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
    let collision = attach("case:native-contract-example");
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
                        "goal:native-case-contract"
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
                        "goal:does-not-exist"
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
                        "goal:native-case-contract"
                    }
                    _ => {
                        write_json_value(
                            &path,
                            &native_attached_evidence(
                                &format!("evidence:property-{index}"),
                                "unreviewed",
                            ),
                        );
                        "goal:native-case-contract"
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
    assert!(stderr(&unsupported).contains("unsupported native argument \"--strict\""));

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
    let output = Command::new("python3")
        .args([
            "-m",
            "jsonschema",
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
    use std::os::unix::fs::PermissionsExt;

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
        "timeout_ms": 5000,
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

fn wait_for_file(path: &Path, timeout_message: &str) {
    let wait_started_at = Instant::now();
    while !path.is_file() && wait_started_at.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.is_file(), "{timeout_message}");
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
    let mut matches = fs::read_dir(directory.join("runs"))
        .expect("read runs directory")
        .map(|entry| entry.expect("run entry").path().join(file_name))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    matches.sort();
    assert_eq!(matches.len(), 1, "expected one {file_name}");
    matches.remove(0)
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
        "schemas/casegraphen/report-schema-aliases.json",
        "schemas/casegraphen/case.graph.schema.json",
        "schemas/casegraphen/coverage.policy.schema.json",
        "schemas/casegraphen/projection.schema.json",
        "schemas/casegraphen/case.report.schema.json",
        "schemas/casegraphen/workflow.graph.schema.json",
        "schemas/casegraphen/github.issue-snapshot.schema.json",
        "schemas/casegraphen/native.case.space.schema.json",
        "schemas/casegraphen/native.morphism-log-entry.schema.json",
        "schemas/casegraphen/native.case.report.schema.json",
        "schemas/casegraphen/execution.plan.schema.json",
        "schemas/casegraphen/worker.binding.schema.json",
        "schemas/casegraphen/worker.report.schema.json",
        "schemas/casegraphen/execution.trace.schema.json",
        "schemas/casegraphen/operation-gate-profiles.schema.json",
        "schemas/casegraphen/native-cli.report.schema.json",
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
    ]
    .iter()
    .map(|(schema, example)| (repo_path(schema), repo_path(example)))
    .collect()
}
