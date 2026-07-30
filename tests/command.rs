#![allow(missing_docs)]

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
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
fn workflow_reason_emits_reasoning_report_for_workflow_fixture() {
    let output = run_cli(&[
        "workflow",
        "reason",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stderr(&output).is_empty());

    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.report.v1")
    );
    assert_eq!(value["report_type"], json!("case_workflow_reasoning"));
    assert_eq!(
        value["metadata"]["command"],
        json!("casegraphen workflow reason")
    );
    assert_eq!(
        value["metadata"]["tool_package"],
        json!("tools/casegraphen")
    );
    assert_eq!(value["result"]["status"], json!("obstructions_detected"));
    assert_eq!(
        value["result"]["readiness"]["ready_item_ids"],
        json!(["task:define-workflow-reasoning-contract"])
    );
    assert_eq!(
        value["result"]["completion_candidates"][0]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        value["projection"]["ai_view"]["audience"],
        json!("ai_agent")
    );
    assert!(value["core_extensions"]["witnesses"]
        .as_array()
        .expect("workflow core witnesses")
        .iter()
        .any(|witness| witness["id"] == json!("witness:evidence:workflow-target-doc")));
    assert!(value["core_extensions"]["policies"]
        .as_array()
        .expect("workflow core policies")
        .iter()
        .any(|policy| policy["policy_kind"] == json!("obligation")));
}

#[test]
fn workflow_reason_uses_metadata_core_extensions_as_review_gate() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let input_path = directory.join("workflow.with-core-extension.json");
    let mut workflow = json_file(workflow_fixture());
    workflow["metadata"]["higher_graphen_extensions"] =
        invalid_core_extensions("task:define-workflow-reasoning-contract");
    fs::write(
        &input_path,
        serde_json::to_string_pretty(&workflow).expect("serialize workflow"),
    )
    .expect("write workflow");

    let output = run_cli(&[
        "workflow",
        "reason",
        "--input",
        input_path.to_str().expect("workflow path"),
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(value["result"]["status"], json!("review_required"));
    assert_eq!(
        value["core_extensions"]["validation"]["blocked_count"],
        json!(1)
    );
    assert_eq!(
        value["core_extensions"]["validation"]["findings"][0]["object_id"],
        json!("valuation:metadata-core-extension-block")
    );
}

#[test]
fn workflow_topology_emits_diagnostics_report() {
    let output = run_cli(&[
        "workflow",
        "history",
        "topology",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stderr(&output).is_empty());

    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.topology.report.v1")
    );
    assert_eq!(value["report_type"], json!("case_workflow_topology"));
    assert_eq!(
        value["metadata"]["command"],
        json!("casegraphen workflow history topology")
    );
    assert_eq!(
        value["result"]["topology"]["homology"]["coefficient_field"],
        json!("z2")
    );
    assert!(!value["result"]["source_mapping"]["nodes"]
        .as_array()
        .expect("topology nodes")
        .is_empty());
    assert!(value["result"].get("higher_order").is_none());

    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let output_path = directory.join("workflow.topology.report.json");

    let file_output = run_cli(&[
        "workflow",
        "history",
        "topology",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);

    assert!(
        file_output.status.success(),
        "stderr: {}",
        stderr(&file_output)
    );
    assert!(stdout(&file_output).is_empty());
    assert!(stderr(&file_output).is_empty());
    assert_eq!(
        json_file(output_path)["schema"],
        json!("highergraphen.case.workflow.topology.report.v1")
    );

    let higher_order = run_cli(&[
        "workflow",
        "history",
        "topology",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--higher-order",
        "--min-persistence-stages",
        "1",
    ]);
    assert!(
        higher_order.status.success(),
        "stderr: {}",
        stderr(&higher_order)
    );
    let higher_order_json = stdout_json(&higher_order);
    assert_eq!(
        higher_order_json["result"]["higher_order"]["options"]["min_persistence_stages"],
        json!(1)
    );
    assert!(
        !higher_order_json["result"]["higher_order"]["persistence"]["stages"]
            .as_array()
            .expect("higher-order stages")
            .is_empty()
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn workflow_topology_diff_command_reports_file_to_file_deltas() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let right_path = directory.join("right.workflow.graph.json");
    let output_path = directory.join("workflow.topology.diff.report.json");
    let mut workflow = json_file(workflow_fixture());

    let mut added_item = workflow["work_items"][0].clone();
    added_item["id"] = json!("task:topology-diff-added");
    added_item["title"] = json!("Topology diff added workflow item");
    added_item["state"] = json!("todo");
    added_item["hard_dependency_ids"] = json!([]);
    added_item["external_wait_ids"] = json!([]);
    added_item["evidence_requirement_ids"] = json!([]);
    added_item["proof_requirement_ids"] = json!([]);
    workflow["work_items"]
        .as_array_mut()
        .expect("work items")
        .push(added_item);

    let mut added_relation = workflow["workflow_relations"][0].clone();
    added_relation["id"] = json!("relation:topology-diff-added");
    added_relation["relation_type"] = json!("relates_to");
    added_relation["from_id"] = json!("task:topology-diff-added");
    added_relation["to_id"] = json!("task:define-workflow-reasoning-contract");
    added_relation["evidence_ids"] = json!([]);
    workflow["workflow_relations"]
        .as_array_mut()
        .expect("workflow relations")
        .push(added_relation);

    fs::write(
        &right_path,
        serde_json::to_string_pretty(&workflow).expect("serialize right workflow"),
    )
    .expect("write right workflow");

    let output = run_cli(&[
        "workflow",
        "history",
        "topology",
        "diff",
        "--left",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--right",
        right_path.to_str().expect("right workflow path"),
        "--format",
        "json",
        "--higher-order",
        "--max-dimension",
        "1",
        "--output",
        output_path.to_str().expect("output path"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).is_empty());
    let value = json_file(output_path);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.topology_diff.report.v1")
    );
    assert_eq!(value["report_type"], json!("case_workflow_topology_diff"));
    assert_eq!(
        value["metadata"]["command"],
        json!("casegraphen workflow history topology diff")
    );
    assert_eq!(
        value["result"]["scalar_deltas"]["vertex_count"]["delta"],
        json!(1)
    );
    assert_eq!(
        value["result"]["scalar_deltas"]["graph_edge_count"]["delta"],
        json!(1)
    );
    assert_eq!(
        value["result"]["source_mapping"]["added_source_node_ids"],
        json!(["task:topology-diff-added"])
    );
    assert_eq!(
        value["result"]["source_mapping"]["added_source_relation_ids"],
        json!(["relation:topology-diff-added"])
    );
    assert!(value["result"].get("higher_order").is_some());

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn workflow_validate_reports_semantic_violations_as_json() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let bad_workflow_path = directory.join("bad.workflow.graph.json");
    let mut workflow = json_file(workflow_fixture());
    workflow["workflow_relations"][0]["from_id"] = json!("task:missing-work-item");
    fs::write(
        &bad_workflow_path,
        serde_json::to_string_pretty(&workflow).expect("serialize bad workflow"),
    )
    .expect("write bad workflow");

    let output = run_cli(&[
        "workflow",
        "validate",
        "--input",
        bad_workflow_path.to_str().expect("bad workflow path"),
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stderr(&output).is_empty());

    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.validate.report.v1")
    );
    assert_eq!(value["report_type"], json!("case_workflow_validate"));
    assert_eq!(value["result"]["valid"], json!(false));
    assert!(value["result"]["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .any(|violation| violation["code"] == json!("dangling_reference")));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn workflow_reason_supports_output_file_without_stdout() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let output_path = directory.join("workflow.report.json");

    let output = run_cli(&[
        "workflow",
        "reason",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).is_empty());

    let value: Value =
        serde_json::from_str(&fs::read_to_string(&output_path).expect("read report"))
            .expect("report JSON");
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.report.v1")
    );
    assert_eq!(
        value["input"]["workflow_graph_id"],
        json!("workflow_graph:casegraphen-rewrite-contract")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn workflow_readiness_supports_output_file_without_stdout() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let output_path = directory.join("workflow.readiness.report.json");

    let output = run_cli(&[
        "workflow",
        "readiness",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).is_empty());

    let value: Value =
        serde_json::from_str(&fs::read_to_string(&output_path).expect("read report"))
            .expect("report JSON");
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.readiness.report.v1")
    );
    assert_eq!(
        value["result"]["ready_item_ids"],
        json!(["task:define-workflow-reasoning-contract"])
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn focused_workflow_commands_emit_section_reports() {
    let readiness = run_cli(&[
        "workflow",
        "readiness",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--projection",
        projection_fixture().to_str().expect("projection path"),
        "--format",
        "json",
    ]);
    assert!(readiness.status.success(), "stderr: {}", stderr(&readiness));
    let value = stdout_json(&readiness);
    assert_eq!(value["report_type"], json!("case_workflow_readiness"));
    assert_eq!(
        value["input"]["projection"],
        json!(projection_fixture().display().to_string())
    );
    assert_eq!(
        value["projection"]["audit_trace"]["information_loss"],
        json!(["Focused report contains the requested section; use workflow reason for the aggregate projection."])
    );
    assert_eq!(
        value["result"]["not_ready_items"][0]["work_item_id"],
        json!("proof:workflow-schema-parse-check")
    );

    let obstructions = stdout_json(&successful_workflow_command("obstructions"));
    assert_eq!(
        obstructions["schema"],
        json!("highergraphen.case.workflow.obstructions.report.v1")
    );
    assert!(obstructions["result"]["obstructions"]
        .as_array()
        .expect("obstructions")
        .iter()
        .any(|record| record["obstruction_type"] == json!("missing_evidence")));

    let completions = stdout_json(&successful_workflow_command("completions"));
    assert!(completions["result"]["completion_candidates"]
        .as_array()
        .expect("completion candidates")
        .iter()
        .any(|record| record["candidate_type"] == json!("missing_proof")));

    let evidence = stdout_json(&successful_workflow_command("evidence"));
    assert_eq!(
        evidence["result"]["inference_record_ids"],
        json!(["evidence:workflow-gap-inference"])
    );

    let project = run_cli(&[
        "workflow",
        "project",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--projection",
        projection_fixture().to_str().expect("projection path"),
        "--format",
        "json",
    ]);
    assert!(project.status.success(), "stderr: {}", stderr(&project));
    assert_eq!(
        stdout_json(&project)["result"]["projection_profile_id"],
        json!("projection:workflow-ai-review")
    );

    let correspond = run_cli(&[
        "workflow",
        "correspond",
        "--left",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--right",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
    ]);
    assert!(
        correspond.status.success(),
        "stderr: {}",
        stderr(&correspond)
    );
    assert_eq!(
        stdout_json(&correspond)["result"]["combined_correspondence"][0]["correspondence_type"],
        json!("similar_with_loss")
    );

    let evolution = stdout_json(&successful_workflow_command("evolution"));
    assert_eq!(
        evolution["result"]["transition_ids"],
        json!(["transition:foundation-docs-to-workflow-contract"])
    );
}

#[test]
fn cg_bridge_workflow_workspace_commands_round_trip_store_history() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let import = run_cli(&[
        "cg",
        "workflow",
        "import",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--revision-id",
        "revision:bridge-import",
        "--format",
        "json",
    ]);
    assert!(import.status.success(), "stderr: {}", stderr(&import));
    let imported = stdout_json(&import);
    assert_eq!(
        imported["schema"],
        json!("highergraphen.case.workflow.workspace_import.report.v1")
    );
    assert_eq!(
        imported["metadata"]["command"],
        json!("casegraphen cg workflow import")
    );
    assert_eq!(
        imported["result"]["current_revision_id"],
        json!("revision:bridge-import")
    );
    assert!(directory
        .join(
            imported["result"]["current_graph_path"]
                .as_str()
                .expect("current graph path")
        )
        .exists());

    let list = run_cli(&[
        "cg",
        "workflow",
        "list",
        "--store",
        directory.to_str().expect("temp path"),
        "--format",
        "json",
    ]);
    assert!(list.status.success(), "stderr: {}", stderr(&list));
    assert_eq!(
        stdout_json(&list)["result"]["workflow_graph_count"],
        json!(1)
    );

    let inspect = run_bridge_store_command(&directory, "inspect");
    assert_eq!(
        stdout_json(&inspect)["result"]["history_entry_count"],
        json!(1)
    );

    let history = run_bridge_store_command(&directory, "history");
    let history_json = stdout_json(&history);
    assert_eq!(
        history_json["result"]["entries"][0]["event_type"],
        json!("imported")
    );

    let replay = run_bridge_store_command(&directory, "replay");
    assert_eq!(
        stdout_json(&replay)["result"]["graph"]["workflow_graph_id"],
        json!("workflow_graph:casegraphen-rewrite-contract")
    );

    let output_path = directory.join("bridge.validate.report.json");
    let validate = run_cli(&[
        "cg",
        "workflow",
        "validate",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);
    assert!(validate.status.success(), "stderr: {}", stderr(&validate));
    assert!(stdout(&validate).is_empty());
    let validation = json_file(output_path);
    assert_eq!(validation["result"]["valid"], json!(true));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_readiness_supports_file_and_stored_workflow_graphs() {
    let file_based = run_cli(&[
        "cg",
        "workflow",
        "readiness",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
    ]);
    assert!(
        file_based.status.success(),
        "stderr: {}",
        stderr(&file_based)
    );
    let file_json = stdout_json(&file_based);
    assert_eq!(
        file_json["metadata"]["command"],
        json!("casegraphen cg workflow readiness")
    );
    assert_eq!(file_json["input"]["source"], json!("file"));
    assert_eq!(
        file_json["projection"]["audit_trace"]["information_loss"],
        json!(["Focused report contains the requested section; use workflow reason for the aggregate projection."])
    );
    assert_eq!(
        file_json["result"]["ready_item_ids"],
        json!(["task:define-workflow-reasoning-contract"])
    );

    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);
    let stored = run_cli(&[
        "cg",
        "workflow",
        "readiness",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--format",
        "json",
    ]);
    assert!(stored.status.success(), "stderr: {}", stderr(&stored));
    let stored_json = stdout_json(&stored);
    assert_eq!(stored_json["input"]["source"], json!("workspace_store"));
    assert_eq!(
        stored_json["projection"]["audit_trace"]["information_loss"],
        json!(["Focused report contains the requested section; use workflow reason for the aggregate projection."])
    );
    assert_eq!(
        stored_json["result"]["not_ready_items"][0]["work_item_id"],
        json!("proof:workflow-schema-parse-check")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_workflow_history_topology_uses_revision_filtration() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);

    let output = run_cli(&[
        "cg",
        "workflow",
        "history",
        "topology",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
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
        json!("casegraphen cg workflow history topology")
    );
    assert_eq!(
        value["result"]["topology"]["higher_order"]["filtration_source"],
        json!("workflow_history")
    );
    assert!(value["result"]["topology"]["higher_order"]["stage_sources"]
        .as_array()
        .expect("workflow stage sources")
        .iter()
        .any(|stage| stage["source_type"] == json!("workflow_revision")));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_completion_accept_records_review_without_promoting_inference() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);

    let output = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_evidence_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Reviewed the proposed evidence gap",
        "--revision-id",
        "revision:completion-accept",
        "--evidence-id",
        "evidence:workflow-target-doc",
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.completion_accept.report.v1")
    );
    assert_eq!(
        value["result"]["candidate_before_review"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        value["result"]["candidate_after_review"]["review_status"],
        json!("accepted")
    );
    assert_eq!(
        value["result"]["review_record"]["evidence_ids"],
        json!(["evidence:workflow-target-doc"])
    );
    assert_eq!(
        value["result"]["transition_record"]["transition_type"],
        json!("review_transition")
    );

    let replay = run_bridge_store_command(&directory, "replay");
    let graph = stdout_json(&replay)["result"]["graph"].clone();
    assert_eq!(
        graph["completion_reviews"][0]["candidate_snapshot"]["review_status"],
        json!("unreviewed")
    );
    assert!(!graph["evidence_records"]
        .as_array()
        .expect("evidence records")
        .iter()
        .any(|record| record["id"] == json!("evidence:json-parse-check-output")));

    let readiness = run_bridge_store_command(&directory, "readiness");
    let readiness_json = stdout_json(&readiness);
    assert!(readiness_json["result"]["not_ready_items"]
        .as_array()
        .expect("not ready items")
        .iter()
        .any(|item| item["obstruction_ids"]
            .as_array()
            .expect("obstruction ids")
            .contains(&json!(
                "obstruction:missing-evidence:proof-workflow-schema-parse-check:evidence-json-parse-check-output"
            ))));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_completion_reject_supports_output_file_and_invalid_target_errors() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);
    let output_path = directory.join("completion.reject.report.json");

    let reject = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "reject",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_proof_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Duplicate of existing proof task",
        "--revision-id",
        "revision:completion-reject",
        "--format",
        "json",
        "--output",
        output_path.to_str().expect("output path"),
    ]);

    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    assert!(stdout(&reject).is_empty());
    let value = json_file(output_path);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.completion_reject.report.v1")
    );
    assert_eq!(
        value["result"]["candidate_after_review"]["review_status"],
        json!("rejected")
    );
    assert_eq!(
        value["result"]["review_record"]["outcome_review_status"],
        json!("rejected")
    );

    let invalid = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        "candidate:does-not-exist",
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Invalid target smoke",
        "--revision-id",
        "revision:completion-invalid",
        "--format",
        "json",
    ]);

    assert!(!invalid.status.success());
    assert!(stdout(&invalid).is_empty());
    assert!(stderr(&invalid).contains("unknown completion candidate candidate:does-not-exist"));

    let invalid_evidence = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_proof_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Invalid linked evidence smoke",
        "--revision-id",
        "revision:completion-invalid-evidence",
        "--evidence-id",
        "evidence:does-not-exist",
        "--format",
        "json",
    ]);

    assert!(!invalid_evidence.status.success());
    assert!(stdout(&invalid_evidence).is_empty());
    assert!(stderr(&invalid_evidence)
        .contains("unknown linked evidence record evidence:does-not-exist"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_completion_reopen_restores_unreviewed_candidate_state() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);

    let accept = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_evidence_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Reviewed the proposed evidence gap",
        "--revision-id",
        "revision:completion-accept",
        "--evidence-id",
        "evidence:workflow-target-doc",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));

    let reopen = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "reopen",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_evidence_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Reopen after missing implementation evidence",
        "--revision-id",
        "revision:completion-reopen",
        "--format",
        "json",
    ]);

    assert!(reopen.status.success(), "stderr: {}", stderr(&reopen));
    let value = stdout_json(&reopen);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.completion_reopen.report.v1")
    );
    assert_eq!(value["result"]["action"], json!("reopen"));
    assert_eq!(
        value["result"]["candidate_before_review"]["review_status"],
        json!("accepted")
    );
    assert_eq!(
        value["result"]["candidate_after_review"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        value["result"]["review_record"]["outcome_review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        value["result"]["workspace_record"]["history_entry_count"],
        json!(3)
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_completion_patch_check_and_apply_flow() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);

    let accept = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_task_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Task candidate is a valid patch source",
        "--revision-id",
        "revision:patch-source-accepted",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));

    let patch = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "patch",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_task_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Convert accepted candidate into a reviewable patch transition",
        "--revision-id",
        "revision:completion-patch",
        "--transition-id",
        "transition:patch:test-missing-task",
        "--format",
        "json",
    ]);
    assert!(patch.status.success(), "stderr: {}", stderr(&patch));
    let patch_json = stdout_json(&patch);
    assert_eq!(
        patch_json["schema"],
        json!("highergraphen.case.workflow.completion_patch.report.v1")
    );
    assert_eq!(patch_json["result"]["applied"], json!(false));
    assert_eq!(
        patch_json["result"]["transition_record"]["provenance"]["review_status"],
        json!("unreviewed")
    );

    let check = run_cli(&[
        "cg",
        "workflow",
        "patch",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--transition-id",
        "transition:patch:test-missing-task",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    let check_json = stdout_json(&check);
    assert_eq!(check_json["result"]["valid"], json!(true));
    assert_eq!(check_json["result"]["applicable"], json!(true));

    let apply = run_cli(&[
        "cg",
        "workflow",
        "patch",
        "apply",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--transition-id",
        "transition:patch:test-missing-task",
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Apply reviewed patch transition",
        "--revision-id",
        "revision:patch-applied",
        "--format",
        "json",
    ]);
    assert!(apply.status.success(), "stderr: {}", stderr(&apply));
    let apply_json = stdout_json(&apply);
    assert_eq!(
        apply_json["schema"],
        json!("highergraphen.case.workflow.patch_apply.report.v1")
    );
    assert_eq!(
        apply_json["result"]["transition_after_review"]["provenance"]["review_status"],
        json!("accepted")
    );
    assert_eq!(apply_json["result"]["materialized_record_count"], json!(0));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn cg_bridge_patch_reject_records_review_without_materializing_patch() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_bridge_workflow(&directory);

    let accept = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "accept",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_task_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Task candidate is a valid patch source",
        "--revision-id",
        "revision:patch-source-accepted",
        "--format",
        "json",
    ]);
    assert!(accept.status.success(), "stderr: {}", stderr(&accept));

    let patch = run_cli(&[
        "cg",
        "workflow",
        "completion",
        "patch",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--candidate-id",
        missing_task_candidate_id(),
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Convert accepted candidate into a reviewable patch transition",
        "--revision-id",
        "revision:completion-patch",
        "--transition-id",
        "transition:patch:test-rejected-missing-task",
        "--format",
        "json",
    ]);
    assert!(patch.status.success(), "stderr: {}", stderr(&patch));

    let reject = run_cli(&[
        "cg",
        "workflow",
        "patch",
        "reject",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--transition-id",
        "transition:patch:test-rejected-missing-task",
        "--reviewer-id",
        "reviewer:workflow-lead",
        "--reason",
        "Reject patch until source proof is attached",
        "--revision-id",
        "revision:patch-rejected",
        "--format",
        "json",
    ]);

    assert!(reject.status.success(), "stderr: {}", stderr(&reject));
    let value = stdout_json(&reject);
    assert_eq!(
        value["schema"],
        json!("highergraphen.case.workflow.patch_reject.report.v1")
    );
    assert_eq!(value["result"]["action"], json!("reject"));
    assert_eq!(value["result"]["materialized_record_count"], json!(0));
    assert_eq!(
        value["result"]["transition_before_review"]["provenance"]["review_status"],
        json!("unreviewed")
    );
    assert_eq!(
        value["result"]["transition_after_review"]["provenance"]["review_status"],
        json!("rejected")
    );

    let check = run_cli(&[
        "cg",
        "workflow",
        "patch",
        "check",
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--transition-id",
        "transition:patch:test-rejected-missing-task",
        "--format",
        "json",
    ]);
    assert!(check.status.success(), "stderr: {}", stderr(&check));
    let check_json = stdout_json(&check);
    assert_eq!(check_json["result"]["valid"], json!(true));
    assert_eq!(check_json["result"]["applicable"], json!(false));
    assert_eq!(
        check_json["result"]["reason"],
        json!("Patch transition is rejected.")
    );

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_case_commands_create_import_list_inspect_history_and_replay() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");

    let created = run_cli(&[
        "case",
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
        "case",
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
        "case",
        "history",
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
fn generated_workflow_operation_report_validates_against_schema() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let report_path = directory.join("workflow.validate.report.json");

    let output = run_cli(&[
        "workflow",
        "validate",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--output",
        report_path.to_str().expect("report path"),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/workflow.operation.report.schema.json"),
        &report_path,
    );

    let topology_report_path = directory.join("workflow.topology.report.json");
    let topology = run_cli(&[
        "workflow",
        "history",
        "topology",
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
        "--higher-order",
        "--min-persistence-stages",
        "1",
        "--output",
        topology_report_path.to_str().expect("report path"),
    ]);
    assert!(topology.status.success(), "stderr: {}", stderr(&topology));
    assert_jsonschema_valid(
        &repo_path("schemas/casegraphen/workflow.operation.report.schema.json"),
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
        "case",
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
        "case",
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
        let output = run_native_case_store_command(&directory, command);
        assert!(
            output.status.success(),
            "{command} stderr: {}",
            stderr(&output)
        );
        let expected = match command {
            "obstructions" => "casegraphen obstruction list",
            "completions" => "casegraphen completion candidates",
            "evidence" => "casegraphen invariant evidence",
            "project" => "casegraphen projection apply",
            _ => unreachable!("test command set is fixed"),
        };
        assert_eq!(stdout_json(&output)["metadata"]["command"], json!(expected));
    }

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn native_case_topology_emits_domain_report() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    import_native_case_space(&directory, "revision:native-cli-imported");

    let output = run_cli(&[
        "case",
        "history",
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
        "case",
        "history",
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
        "case",
        "history",
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
        "case",
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
    let tampered_apply = run_cli(&[
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
    ]);
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
    let fixture = setup_native_run(&directory, "happy", "printf 'successful-worker-output\\n'");

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
    assert!(!trace_path
        .parent()
        .expect("run directory")
        .join("stdout")
        .exists());
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
    assert!(!only_run_file(&directory, "execution.trace.json")
        .parent()
        .expect("run directory")
        .join("stdout")
        .exists());
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

    let output = run_native_step(&directory, &fixture, true, None);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
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
        "case",
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
        "case",
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

    let empty_reason = run_cli(&[
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
    ]);
    assert!(!empty_reason.status.success());
    assert!(stderr(&empty_reason).contains("review reason must not be empty"));

    let stale = run_cli(&[
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
    ]);
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
    assert_eq!(
        attached_cell["metadata"]["evidence_boundary"],
        json!("attached_unverified")
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
        "case",
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
        "case",
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
    let missing_apply_reason = run_cli(&[
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
    ]);
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
    let same_revision_reject = run_cli(&[
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
    ]);
    assert!(!same_revision_reject.status.success());
    assert!(stderr(&same_revision_reject).contains("must advance the revision"));

    fs::remove_dir_all(directory).expect("remove temp directory");
}

#[test]
fn reference_workflow_reasoning_matches_checked_in_report() {
    let output = run_cli(&[
        "workflow",
        "reason",
        "--input",
        reference_workflow_fixture()
            .to_str()
            .expect("reference workflow path"),
        "--format",
        "json",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stderr(&output).is_empty());

    let value = stdout_json(&output);
    let reference = json_file(reference_workflow_report_fixture());
    assert_eq!(value, reference);

    assert_eq!(
        value["result"]["readiness"]["ready_item_ids"],
        json!(["task:define-workflow-reasoning-contract"])
    );
    assert_eq!(
        value["result"]["readiness"]["not_ready_items"][0]["work_item_id"],
        json!("proof:workflow-schema-parse-check")
    );

    let obstructions = value["result"]["obstructions"]
        .as_array()
        .expect("obstructions");
    assert!(obstructions
        .iter()
        .any(|record| record["obstruction_type"] == json!("missing_evidence")));
    assert!(obstructions
        .iter()
        .any(|record| record["obstruction_type"] == json!("missing_proof")));
    assert!(obstructions
        .iter()
        .any(|record| record["obstruction_type"] == json!("unresolved_dependency")));
    assert!(obstructions
        .iter()
        .any(|record| record["obstruction_type"] == json!("review_required")));

    let completion_candidates = value["result"]["completion_candidates"]
        .as_array()
        .expect("completion candidates");
    assert!(completion_candidates
        .iter()
        .any(|record| record["candidate_type"] == json!("missing_evidence")));
    assert!(completion_candidates
        .iter()
        .any(|record| record["candidate_type"] == json!("missing_proof")));
    assert!(completion_candidates
        .iter()
        .any(|record| record["candidate_type"] == json!("missing_task")));

    assert_eq!(
        value["result"]["evidence_findings"]["accepted_evidence_ids"],
        json!(["evidence:workflow-target-doc"])
    );
    assert_eq!(
        value["result"]["evidence_findings"]["inference_record_ids"],
        json!(["evidence:workflow-gap-inference"])
    );
    assert!(value["result"]["evidence_findings"]["findings"]
        .as_array()
        .expect("evidence findings")
        .iter()
        .any(|record| record["finding_type"] == json!("evidence_missing")));

    assert_eq!(
        value["result"]["projection"]["projection_profile_id"],
        json!("projection:workflow-ai-review")
    );
    assert_eq!(
        value["projection"]["ai_view"]["audience"],
        json!("ai_agent")
    );
    assert_eq!(
        value["projection"]["ai_view"]["information_loss"][0]["omitted_ids"],
        json!(["docs/specs/intermediate-tools/casegraphen-workflow-reasoning-engine.md"])
    );
    let ai_records = value["projection"]["ai_view"]["records"]
        .as_array()
        .expect("ai records");
    for record_type in [
        "readiness",
        "obstruction",
        "completion_candidate",
        "evidence_finding",
        "projection",
        "correspondence",
        "evolution",
    ] {
        assert!(
            ai_records
                .iter()
                .any(|record| record["record_type"] == json!(record_type)),
            "missing AI projection record type {record_type}"
        );
    }

    assert_eq!(
        value["result"]["correspondence"][0]["correspondence_type"],
        json!("similar_with_loss")
    );
    assert_eq!(
        value["result"]["evolution"]["transition_ids"],
        json!(["transition:foundation-docs-to-workflow-contract"])
    );
    assert_eq!(
        value["result"]["evolution"]["persisted_shape_ids"],
        json!([
            "schemas/casegraphen/case.graph.schema.json",
            "schemas/casegraphen/case.report.schema.json"
        ])
    );
}

#[test]
fn invalid_workflow_reference_errors_before_reasoning_report() {
    let directory = unique_temp_dir();
    fs::create_dir_all(&directory).expect("create temp directory");
    let bad_workflow_path = directory.join("bad.workflow.graph.json");
    let mut workflow = json_file(workflow_fixture());
    workflow["workflow_relations"][0]["from_id"] = json!("task:missing-work-item");
    fs::write(
        &bad_workflow_path,
        serde_json::to_string_pretty(&workflow).expect("serialize bad workflow"),
    )
    .expect("write bad workflow");

    let output = run_cli(&[
        "workflow",
        "reason",
        "--input",
        bad_workflow_path.to_str().expect("bad workflow path"),
        "--format",
        "json",
    ]);

    assert!(!output.status.success());
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("workflow validation failed"));
    assert!(stderr(&output).contains("dangling_reference"));

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

fn successful_workflow_command(command: &str) -> Output {
    let output = run_cli(&[
        "workflow",
        command,
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    output
}

fn import_bridge_workflow(directory: &Path) {
    let output = run_cli(&[
        "cg",
        "workflow",
        "import",
        "--store",
        directory.to_str().expect("temp path"),
        "--input",
        workflow_fixture().to_str().expect("workflow fixture path"),
        "--revision-id",
        "revision:bridge-import",
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
}

fn run_bridge_store_command(directory: &Path, command: &str) -> Output {
    let output = run_cli(&[
        "cg",
        "workflow",
        command,
        "--store",
        directory.to_str().expect("temp path"),
        "--workflow-graph-id",
        "workflow_graph:casegraphen-rewrite-contract",
        "--format",
        "json",
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    output
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
        "case",
        "import",
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

fn run_native_case_store_command(directory: &Path, command: &str) -> Output {
    let output = run_cli(&[
        "case",
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
    let morphism = json!({
        "morphism_id": morphism_id,
        "morphism_type": "review",
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
    Command::new(env!("CARGO_BIN_EXE_casegraphen"))
        .args(args)
        .output()
        .expect("run casegraphen run --step")
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

fn replayed_work_lifecycle(replay: &Value) -> &str {
    replay["result"]["replay"]["case_space"]["case_cells"]
        .as_array()
        .expect("replayed cells")
        .iter()
        .find(|cell| cell["id"] == json!("work:review-native-contract"))
        .and_then(|cell| cell["lifecycle"].as_str())
        .expect("work lifecycle")
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

fn workflow_fixture() -> PathBuf {
    repo_path("schemas/casegraphen/workflow.graph.example.json")
}

fn native_case_fixture() -> PathBuf {
    repo_path("schemas/casegraphen/native.case.space.example.json")
}

fn reference_workflow_fixture() -> PathBuf {
    repo_path("examples/casegraphen/reference/workflow.graph.json")
}

fn reference_workflow_report_fixture() -> PathBuf {
    repo_path("examples/casegraphen/reference/reports/workflow.reason.report.json")
}

fn missing_evidence_candidate_id() -> &'static str {
    "candidate:missing-evidence:obstruction-missing-evidence-proof-workflow-schema-parse-check-evidence-json-parse-check-output"
}

fn missing_proof_candidate_id() -> &'static str {
    "candidate:missing-proof:obstruction-missing-proof-task-implement-workflow-engine-proof-workflow-schema-parse-check"
}

fn missing_task_candidate_id() -> &'static str {
    "candidate:missing-task:obstruction-unresolved-dependency-task-implement-workflow-engine-task-define-workflow-reasoning-contract"
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
        "schemas/casegraphen/workflow.report.example.json",
        "schemas/casegraphen/native.case.space.example.json",
        "schemas/casegraphen/native.case.report.example.json",
        "schemas/casegraphen/execution.plan.example.json",
        "schemas/casegraphen/worker.binding.example.json",
        "schemas/casegraphen/worker.report.example.json",
        "schemas/casegraphen/execution.trace.example.json",
        "schemas/casegraphen/report-schema-aliases.json",
        "schemas/casegraphen/case.graph.schema.json",
        "schemas/casegraphen/coverage.policy.schema.json",
        "schemas/casegraphen/projection.schema.json",
        "schemas/casegraphen/case.report.schema.json",
        "schemas/casegraphen/workflow.graph.schema.json",
        "schemas/casegraphen/workflow.report.schema.json",
        "schemas/casegraphen/workflow.operation.report.schema.json",
        "schemas/casegraphen/native.case.space.schema.json",
        "schemas/casegraphen/native.case.report.schema.json",
        "schemas/casegraphen/execution.plan.schema.json",
        "schemas/casegraphen/worker.binding.schema.json",
        "schemas/casegraphen/worker.report.schema.json",
        "schemas/casegraphen/execution.trace.schema.json",
        "schemas/casegraphen/native-cli.report.schema.json",
        "examples/casegraphen/reference/workflow.graph.json",
        "examples/casegraphen/reference/reports/workflow.reason.report.json",
    ]
    .iter()
    .map(|path| repo_path(path))
    .collect()
}

fn native_schema_example_pairs() -> Vec<(PathBuf, PathBuf)> {
    [
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
    ]
    .iter()
    .map(|(schema, example)| (repo_path(schema), repo_path(example)))
    .collect()
}
