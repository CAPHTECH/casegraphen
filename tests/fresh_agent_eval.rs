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
    assert_eq!(result["provider"]["provider"], "custom");
    assert!(result["prompt_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(result["skill_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(result["declared_input_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
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

#[test]
fn unavailable_real_provider_is_reported_and_never_replaced_by_the_fake_runner() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("unavailable");
    let output = Command::new("/usr/bin/python3")
        .args([
            "scripts/fresh-agent-eval.py",
            "--runner-profile",
            "codex",
            "--budget-usd",
            "1",
            "--expected-runner-version",
            "0.146.0",
            "--runner-package-identity",
            "@openai/codex@0.146.0",
            "--output-dir",
            run_root.to_str().unwrap(),
        ])
        .env("OPENAI_API_KEY", "fixture-openai-credential")
        .env("PATH", "/definitely/no/provider/bin")
        .current_dir(root())
        .output()
        .expect("run unavailable-provider path");
    assert_eq!(output.status.code(), Some(3));
    let summary: Value =
        serde_json::from_str(&fs::read_to_string(run_root.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], "provider_unavailable");
    assert_eq!(summary["provider"]["provider"], "codex");
    assert_eq!(summary["results"].as_array().unwrap().len(), 0);
}

#[test]
fn missing_provider_credential_is_explicitly_unavailable() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("credential-unavailable");
    let output = Command::new("python3")
        .args([
            "scripts/fresh-agent-eval.py",
            "--runner-profile",
            "codex",
            "--budget-usd",
            "1",
            "--expected-runner-version",
            "0.146.0",
            "--runner-package-identity",
            "@openai/codex@0.146.0",
            "--output-dir",
            run_root.to_str().unwrap(),
        ])
        .env_remove("OPENAI_API_KEY")
        .current_dir(root())
        .output()
        .expect("run missing-credential path");
    assert_eq!(output.status.code(), Some(3));
    let summary: Value =
        serde_json::from_str(&fs::read_to_string(run_root.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], "credential_unavailable");
    assert_eq!(
        summary["provider"]["credential_environment"],
        "OPENAI_API_KEY"
    );
    assert_eq!(summary["results"].as_array().unwrap().len(), 0);
}

#[test]
fn provider_environment_contains_only_the_selected_provider_credential() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
environment = module.provider_environment('codex')
print(json.dumps(sorted(key for key in environment if module.is_secret_key(key))))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .env("OPENAI_API_KEY", "selected")
        .env("ANTHROPIC_API_KEY", "unrelated")
        .env("GITHUB_TOKEN", "unrelated-host-token")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let keys: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(keys, serde_json::json!(["OPENAI_API_KEY"]));
}

#[test]
fn credential_material_in_generated_files_is_failed_and_withheld() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("leak");
    let runner = serde_json::to_string(&vec![
        "python3",
        root()
            .join("tests/fixtures/fresh-agent/leaking-runner.py")
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
        .env("TEST_API_KEY", "fixture-secret-that-must-not-survive")
        .current_dir(root())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let scenario = run_root.join("evidence-requires-review");
    assert!(!scenario.join("workspace").exists());
    assert!(!fs::read_to_string(scenario.join("raw.stdout"))
        .unwrap()
        .contains("fixture-secret-that-must-not-survive"));
    let result: Value =
        serde_json::from_str(&fs::read_to_string(scenario.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["workspace_retained"], false);
    assert_eq!(result["credential_material_scan"]["status"], "fail");
}

#[test]
fn workflow_conformance_rejects_shared_job_secret_scope() {
    let output = Command::new("python3")
        .arg("scripts/fresh-agent-workflow-conformance.py")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let output_root = TestOutputDirectory::new();
    let invalid = output_root.path.join("shared-secrets.yml");
    let valid =
        fs::read_to_string(root().join(".github/workflows/fresh-agent-release-eval.yml")).unwrap();
    fs::write(
        &invalid,
        valid.replace(
            "    steps:\n",
            "    env:\n      OPENAI_API_KEY: shared\n      ANTHROPIC_API_KEY: shared\n    steps:\n",
        ),
    )
    .unwrap();
    let rejected = Command::new("python3")
        .args([
            "scripts/fresh-agent-workflow-conformance.py",
            "--workflow",
            invalid.to_str().unwrap(),
        ])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stdout).contains("job scope"));
}

#[test]
fn release_policy_names_both_real_providers_and_all_scenarios() {
    let policy: Value = serde_json::from_str(
        &fs::read_to_string(root().join("evals/fresh-agent/release-policy.v0.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        policy["required_providers"],
        serde_json::json!(["codex", "claude"])
    );
    assert_eq!(
        policy["required_scenario_ids"].as_array().unwrap().len(),
        10
    );
    assert_eq!(
        policy["runner_pins"]["codex"]["package_identity"],
        "@openai/codex@0.146.0"
    );
    assert_eq!(policy["runner_pins"]["claude"]["version"], "2.1.220");
    assert_eq!(
        policy["stable_promotion_threshold"]["deterministic_failures"],
        0
    );
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
