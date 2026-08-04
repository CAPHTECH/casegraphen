#![allow(missing_docs)]

use casegraphen::runtime_protocol::{
    parse_runtime_node_report, reconcile_runtime_reports, ExpectedRuntimeNode,
    RuntimeGraphExpectation, RUNTIME_GRAPH_EXPECTATION_SCHEMA,
};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
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
fn promoted_safety_conditions_are_deterministic_without_duplicating_manual_review() {
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(root().join("evals/fresh-agent/scenarios.v0.json")).unwrap(),
    )
    .unwrap();
    for (scenario_id, required_values) in [
        (
            "evidence-requires-review",
            vec!["request_independent_review", "mutation_executed"],
        ),
        (
            "stale-revision-no-auto-rebase",
            vec!["re_read_and_redecide"],
        ),
        (
            "tool-failure-versus-domain-halt",
            vec![
                "inspect_tool_failure_before_retry",
                "stop_for_domain_review",
            ],
        ),
    ] {
        let scenario = manifest["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .find(|scenario| scenario["id"] == scenario_id)
            .unwrap();
        let deterministic = serde_json::to_string(&scenario["deterministic_evaluators"]).unwrap();
        let manual = serde_json::to_string(&scenario["manual_judgments"]).unwrap();
        for value in required_values {
            assert!(
                deterministic.contains(value),
                "{scenario_id}: missing {value}"
            );
            assert!(
                !manual.contains(value),
                "{scenario_id}: manual review duplicates deterministic value {value}"
            );
        }
    }
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
        schema: RUNTIME_GRAPH_EXPECTATION_SCHEMA.to_owned(),
        schema_version: 0,
        runtime_graph_id: base.runtime_graph_id.clone(),
        runtime_graph_content_hash: base.runtime_graph_content_hash.clone(),
        nodes: (0..200)
            .map(|index| ExpectedRuntimeNode {
                node_id: format!("node:{index:04}"),
                expected_output_schema_id: base.expected_output_schema_id.clone(),
                expected_parent_node_ids: Vec::new(),
            })
            .collect(),
        edges: Vec::new(),
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
            "--auth-mode",
            "cli-session",
            "--model",
            "codex-fixture-model",
            "--budget-usd",
            "1",
            "--expected-runner-version",
            "0.146.0",
            "--runner-package-identity",
            "@openai/codex@0.146.0",
            "--output-dir",
            run_root.to_str().unwrap(),
        ])
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
fn unauthenticated_cli_session_is_explicitly_unavailable() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("session-unavailable");
    let bin = output_root.path.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let codex = bin.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'codex-cli 0.146.0'; exit 0; fi\nif [ \"$1\" = \"login\" ] && [ \"$2\" = \"status\" ]; then exit 1; fi\nexit 2\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&codex).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex, permissions).unwrap();
    let output = Command::new("python3")
        .args([
            "scripts/fresh-agent-eval.py",
            "--runner-profile",
            "codex",
            "--auth-mode",
            "cli-session",
            "--model",
            "codex-fixture-model",
            "--budget-usd",
            "1",
            "--expected-runner-version",
            "0.146.0",
            "--runner-package-identity",
            "@openai/codex@0.146.0",
            "--output-dir",
            run_root.to_str().unwrap(),
        ])
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .current_dir(root())
        .output()
        .expect("run unauthenticated-session path");
    assert_eq!(output.status.code(), Some(3));
    let summary: Value =
        serde_json::from_str(&fs::read_to_string(run_root.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], "cli_session_unavailable");
    assert_eq!(summary["provider"]["authentication"]["mode"], "cli_session");
    assert_eq!(summary["provider"]["authentication"]["available"], false);
    assert_eq!(
        summary["provider"]["authentication"]["probe_output_retained"],
        false
    );
    assert_eq!(summary["results"].as_array().unwrap().len(), 0);
}

#[test]
fn cli_session_environment_removes_all_environment_credentials() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
environment = module.cli_session_environment()
print(json.dumps(sorted(key for key in environment if module.is_secret_key(key))))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .env("OPENAI_API_KEY", "selected")
        .env("ANTHROPIC_API_KEY", "unrelated")
        .env("GITHUB_TOKEN", "unrelated-host-token")
        .env("AWS_ACCESS_KEY_ID", "unrelated-cloud-access-key")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let keys: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(keys, serde_json::json!([]));
}

#[test]
fn cli_session_environment_excludes_agent_sockets_and_config_overrides() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
environment = module.cli_session_environment()
print(json.dumps(environment, sort_keys=True))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .env("HOME", "/tmp/provider-session-home")
        .env("SSH_AUTH_SOCK", "/tmp/agent.sock")
        .env("CODEX_HOME", "/tmp/ambient-codex")
        .env("CLAUDE_CONFIG_DIR", "/tmp/ambient-claude")
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let environment: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(environment["HOME"], "/tmp/provider-session-home");
    assert_eq!(environment["GIT_CONFIG_GLOBAL"], "/dev/null");
    assert!(environment.get("SSH_AUTH_SOCK").is_none());
    assert!(environment.get("CODEX_HOME").is_none());
    assert!(environment.get("CLAUDE_CONFIG_DIR").is_none());
}

#[test]
fn auth_probe_classification_accepts_only_known_non_api_cli_sessions() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps({
  'claude_session': module.classify_cli_session('claude', json.dumps({'loggedIn': True, 'authMethod': 'claude.ai', 'email': 'not-retained@example.invalid'})),
  'claude_api_key': module.classify_cli_session('claude', json.dumps({'loggedIn': True, 'authMethod': 'api_key'})),
  'claude_unknown': module.classify_cli_session('claude', '{not-json'),
  'codex_session': module.classify_cli_session('codex', 'Logged in using ChatGPT\n'),
  'codex_api_key': module.classify_cli_session('codex', 'Logged in using an API key'),
  'codex_unknown': module.classify_cli_session('codex', 'Logged in')
}))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let classifications: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        classifications["claude_session"],
        "claude_subscription_session"
    );
    assert_eq!(classifications["codex_session"], "codex_chatgpt_session");
    for field in [
        "claude_api_key",
        "claude_unknown",
        "codex_api_key",
        "codex_unknown",
    ] {
        assert_eq!(classifications[field], Value::Null);
    }
    assert!(!String::from_utf8_lossy(&output.stdout).contains("not-retained@example.invalid"));
}

#[test]
fn observed_model_comparison_is_exact_and_absence_stays_explicit() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
def result(model=None):
    observation = {} if model is None else {'model': model}
    return {'usage_observations': [observation]}
print(json.dumps({
  'match': module.observed_models([result('declared-model')], 'declared-model'),
  'mismatch': module.observed_models([result('substituted-model')], 'declared-model'),
  'absent': module.observed_models([result()], 'declared-model')
}, sort_keys=True))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let observations: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(observations["match"]["matches_declared"], true);
    assert_eq!(observations["mismatch"]["matches_declared"], false);
    assert_eq!(observations["absent"]["observable"], false);
    assert_eq!(
        observations["absent"]["reported_models"],
        serde_json::json!([])
    );
}

#[test]
fn real_runner_profiles_do_not_bypass_tool_permissions_or_load_user_configuration() {
    let probe = r#"
import importlib.util, json, pathlib
path = pathlib.Path('scripts/fresh-agent-eval.py').resolve()
spec = importlib.util.spec_from_file_location('fresh_agent_eval', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(json.dumps(module.RUNNER_PROFILES))
"#;
    let output = Command::new("python3")
        .args(["-c", probe])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let profiles: Value = serde_json::from_slice(&output.stdout).unwrap();
    let codex = profiles["codex"].as_array().unwrap();
    let claude = profiles["claude"].as_array().unwrap();
    assert!(codex.iter().any(|value| value == "--ignore-user-config"));
    assert!(codex.iter().any(|value| value == "--ephemeral"));
    assert!(!claude.iter().any(|value| value == "bypassPermissions"));
    assert!(claude.iter().any(|value| value == "acceptEdits"));
    assert!(claude.iter().any(|value| value == "Read,Write,Edit"));
    assert!(claude.iter().any(|value| value == "--tools"));
    assert!(!claude.iter().any(|value| value == "--allowedTools"));
    assert!(claude.iter().any(|value| value == "project"));
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
        .env("TEST_SECRET", "fixture-secret-that-must-not-survive")
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
fn disk_session_canary_cannot_survive_in_retained_artifacts() {
    let output_root = TestOutputDirectory::new();
    let run_root = output_root.path.join("disk-leak");
    let session_home = output_root.path.join("provider-home");
    let credential = session_home.join(".codex/auth.json");
    fs::create_dir_all(credential.parent().unwrap()).unwrap();
    fs::write(&credential, "disk-session-canary-that-must-never-survive").unwrap();
    let runner = serde_json::to_string(&vec![
        "python3",
        root()
            .join("tests/fixtures/fresh-agent/disk-credential-leaking-runner.py")
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
        .env("HOME", &session_home)
        .env("CASEGRAPHEN_EVAL_CREDENTIAL_CANARY_FILE", &credential)
        .current_dir(root())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let scenario = run_root.join("evidence-requires-review");
    assert!(!scenario.join("workspace").exists());
    assert!(!fs::read_to_string(scenario.join("raw.stdout"))
        .unwrap()
        .contains("disk-session-canary-that-must-never-survive"));
    let result: Value =
        serde_json::from_str(&fs::read_to_string(scenario.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["credential_material_scan"]["status"], "fail");
    assert_eq!(
        result["credential_material_scan"]["output_match_detected"],
        true
    );
    assert_eq!(
        result["credential_material_scan"]["disk_canary_configured"],
        true
    );
}

#[test]
fn workflow_conformance_rejects_api_key_or_github_secret_injection() {
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
            "    env:\n      PROVIDER_API_KEY: ${{ secrets.PROVIDER_API_KEY }}\n    steps:\n",
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
    assert!(String::from_utf8_lossy(&rejected.stdout).contains("must not inject API keys"));

    let reject = |name: &str, contents: String, expected: &str| {
        let path = output_root.path.join(name);
        fs::write(&path, contents).unwrap();
        let result = Command::new("python3")
            .args([
                "scripts/fresh-agent-workflow-conformance.py",
                "--workflow",
                path.to_str().unwrap(),
            ])
            .current_dir(root())
            .output()
            .unwrap();
        assert!(!result.status.success(), "{name} unexpectedly conformed");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains(expected),
            "{name}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
    };

    let swapped = valid
        .replace(
            "runner_label: casegraphen-codex-cli-session",
            "runner_label: temporary-session-label",
        )
        .replace(
            "runner_label: casegraphen-claude-cli-session",
            "runner_label: casegraphen-codex-cli-session",
        )
        .replace(
            "runner_label: temporary-session-label",
            "runner_label: casegraphen-claude-cli-session",
        );
    reject(
        "swapped-labels.yml",
        swapped,
        "must exactly bind each provider",
    );
    reject(
        "shell-input.yml",
        valid.replace(
            "--model \"$CASEGRAPHEN_MODEL\"",
            "--model \"${{ inputs.codex_model }}\"",
        ),
        "only through step environment variables",
    );
    reject(
        "relative-binary.yml",
        valid.replace(
            "$GITHUB_WORKSPACE/fresh-agent-bundle/bin/casegraphen",
            "target/release/casegraphen",
        ),
        "absolute prepared casegraphen binary path",
    );
    reject(
        "credentialed-build.yml",
        valid.replace(
            "    timeout-minutes: 240\n    steps:\n",
            "    timeout-minutes: 240\n    steps:\n      - run: cargo build --release\n",
        ),
        "consume only the prepared evaluator artifact",
    );
    reject(
        "floating-action.yml",
        valid.replacen(
            "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
            "actions/checkout@v4",
            1,
        ),
        "immutable commit SHA",
    );
    reject(
        "missing-rust-toolchain-input.yml",
        valid.replace("        with:\n          toolchain: 1.80.0\n", ""),
        "must declare the repository toolchain input",
    );
    reject(
        "non-main-dispatch.yml",
        valid.replace("    if: github.ref == 'refs/heads/main'\n", ""),
        "refuse non-main workflow dispatch refs",
    );
    reject(
        "unprotected-provider-environment.yml",
        valid.replace(
            "    environment: fresh-agent-cli-session-${{ matrix.provider }}\n",
            "",
        ),
        "provider-specific protected environments",
    );
    reject(
        "short-lived-evaluator.yml",
        valid.replacen(
            "          retention-days: 90\n",
            "          retention-days: 1\n",
            1,
        ),
        "90-day review lifecycle",
    );
}

#[test]
fn release_evidence_lifecycle_requires_brokers_exact_artifacts_and_strict_finalization() {
    let output_root = TestOutputDirectory::new();
    let attestation =
        fs::read_to_string(root().join(".github/workflows/fresh-agent-host-attest.yml")).unwrap();
    let finalization =
        fs::read_to_string(root().join(".github/workflows/fresh-agent-release-finalize.yml"))
            .unwrap();

    let reject = |name: &str, flag: &str, contents: String, expected: &str| {
        let path = output_root.path.join(name);
        fs::write(&path, contents).unwrap();
        let result = Command::new("python3")
            .args(["scripts/fresh-agent-workflow-conformance.py", flag])
            .arg(&path)
            .current_dir(root())
            .output()
            .unwrap();
        assert!(!result.status.success(), "{name} unexpectedly conformed");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains(expected),
            "{name}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
    };

    reject(
        "attestation-api-key.yml",
        "--attestation-workflow",
        attestation.replace(
            "    steps:\n",
            "    env:\n      PROVIDER_API_KEY: forbidden\n    steps:\n",
        ),
        "must not use provider API keys",
    );
    reject(
        "attestation-wrong-runner.yml",
        "--attestation-workflow",
        attestation.replacen(
            "casegraphen-codex-attestation-broker",
            "casegraphen-codex-cli-session",
            1,
        ),
        "dedicated broker runner",
    );
    reject(
        "finalization-unbound-artifact.yml",
        "--finalization-workflow",
        finalization.replace(
            "fresh-agent-codex-${{ inputs.evaluated_commit_sha }}",
            "fresh-agent-codex-latest",
        ),
        "download exact artifact",
    );
    reject(
        "finalization-unprotected.yml",
        "--finalization-workflow",
        finalization.replace("    environment: fresh-agent-release-verifier\n", ""),
        "protected release-verifier environment",
    );
    reject(
        "finalization-fail-open.yml",
        "--finalization-workflow",
        finalization.replace("      - name: Require a passing strict aggregate\n        if: steps.aggregate.outcome != 'success'\n        run: exit 1\n", ""),
        "fail closed",
    );
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
        policy["runner_pins"]["codex"]["authentication_mode"],
        "cli_session"
    );
    assert_eq!(
        policy["runner_pins"]["claude"]["self_hosted_runner_label"],
        "casegraphen-claude-cli-session"
    );
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
