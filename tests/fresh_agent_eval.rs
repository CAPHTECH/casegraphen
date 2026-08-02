#![allow(missing_docs)]

use casegraphen::runtime_protocol::{
    parse_runtime_node_report, reconcile_runtime_reports, ExpectedRuntimeNode,
    RuntimeGraphExpectation,
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn manifest_contains_exactly_the_ten_release_scenarios_and_conforms() {
    let output = Command::new("python3")
        .args(["scripts/fresh-agent-eval.py", "--check-manifest"])
        .current_dir(root())
        .output()
        .expect("run manifest conformance without a model");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "validated 10 fresh-agent scenarios"
    );

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root().join("evals/fresh-agent/scenarios.v0.json")).unwrap(),
    )
    .unwrap();
    let scenarios = manifest["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), 10);
    assert!(scenarios.iter().all(|scenario| {
        !scenario["manual_judgments"].as_array().unwrap().is_empty()
            && !scenario["deterministic_evaluators"]
                .as_array()
                .unwrap()
                .is_empty()
    }));
}

#[test]
fn completeness_oracle_is_owned_by_the_canonical_runtime_reconciler() {
    let oracle: Value = serde_json::from_str(
        &fs::read_to_string(
            root().join("evals/fresh-agent/oracles/missing-one-of-200.expected.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let base = parse_runtime_node_report(include_str!(
        "../schemas/experimental/runtime.node_report.example.json"
    ))
    .unwrap();
    let expectation = RuntimeGraphExpectation {
        runtime_graph_id: base.runtime_graph_id.clone(),
        runtime_graph_content_hash: base.runtime_graph_content_hash.clone(),
        nodes: (0..200)
            .map(|index| ExpectedRuntimeNode {
                node_id: format!("node:{index:04}"),
                expected_output_schema_id: base.expected_output_schema_id.clone(),
            })
            .collect(),
    };
    let reports = (0..199)
        .map(|index| {
            let mut report = base.clone();
            report.report_id = format!("runtime_report:{index:04}");
            report.node_id = format!("node:{index:04}");
            report.attempt_id = format!("attempt:{index:04}:1");
            report.input_artifact_ids = vec![format!("artifact:input:{index:04}")];
            report.output_artifact_ids = vec![format!("artifact:output:{index:04}")];
            report
        })
        .collect::<Vec<_>>();
    let result = reconcile_runtime_reports(&expectation, &reports, &[]);
    assert_eq!(oracle["expected_node_count"], result.expected_node_count);
    assert_eq!(oracle["actual_report_count"], result.actual_report_count);
    assert_eq!(oracle["missing_report_count"], result.missing_report_count);
    assert_eq!(oracle["complete"], result.complete);
}

#[test]
fn harness_uses_a_fresh_task_local_workspace_and_captures_raw_output() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("run");
    let runner = serde_json::to_string(&vec![
        "python3",
        root()
            .join("tests/fixtures/fresh-agent/fake-runner.py")
            .to_str()
            .unwrap(),
    ])
    .unwrap();
    let output = Command::new("python3")
        .args([
            "scripts/fresh-agent-eval.py",
            "--runner-json",
            &runner,
            "--output-dir",
            run_root.to_str().unwrap(),
            "--scenario",
            "evidence-requires-review",
        ])
        .current_dir(root())
        .output()
        .expect("run fake fresh agent");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let scenario = run_root.join("evidence-requires-review");
    assert!(fs::read_to_string(scenario.join("raw.stdout"))
        .unwrap()
        .contains("workspace_isolated"));
    assert!(scenario
        .join("workspace/skill/casegraphen-operate/SKILL.md")
        .is_file());
    assert!(!scenario.join("workspace/.git").exists());
    let result: Value =
        serde_json::from_str(&fs::read_to_string(scenario.join("result.json")).unwrap()).unwrap();
    assert!(result["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evaluation| evaluation["kind"] == "json_assert" && evaluation["status"] == "pass"));
    assert!(result["evaluations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evaluation| evaluation["kind"] == "manual_judgment"
            && evaluation["status"] == "manual_required"));
}

struct TestOutputDirectory {
    path: PathBuf,
}

impl TestOutputDirectory {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "casegraphen-fresh-agent-eval-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestOutputDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
