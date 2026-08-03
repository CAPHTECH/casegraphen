#![allow(missing_docs)]

use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn temp(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "casegraphen-fresh-agent-release-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn fixture(directory: &Path, mode: &str) {
    let status = Command::new("python3")
        .args([
            "tests/fixtures/fresh-agent/build-release-fixture.py",
            "--output",
        ])
        .arg(directory)
        .args(["--mode", mode])
        .current_dir(root())
        .status()
        .unwrap();
    assert!(status.success());
}

fn aggregate(directory: &Path, output: &Path, manual: bool) -> std::process::Output {
    let mut command = Command::new("python3");
    command
        .args(["scripts/fresh-agent-release.py", "--runs-root"])
        .arg(directory)
        .args(["--output-dir"])
        .arg(output);
    if manual {
        command
            .args(["--manual-review"])
            .arg(directory.join("manual-review.json"));
    }
    command.current_dir(root()).output().unwrap()
}

fn report(output: &Path) -> Value {
    let pointer: Value =
        serde_json::from_slice(&fs::read(output.join("release-report.pointer.json")).unwrap())
            .unwrap();
    serde_json::from_slice(&fs::read(output.join(pointer["path"].as_str().unwrap())).unwrap())
        .unwrap()
}

#[test]
fn strict_baseline_is_the_exact_two_provider_ten_scenario_contract() {
    let output = Command::new("python3")
        .args(["scripts/fresh-agent-release.py", "--check-baseline"])
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2-provider x 10-scenario"));
}

#[test]
fn complete_matrix_passes_and_retains_content_addressed_evidence() {
    let directory = temp("pass-input");
    let output = temp("pass-parent").join("report");
    fixture(&directory, "pass");
    let result = aggregate(&directory, &output, true);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = report(&output);
    assert_eq!(report["status"], "pass");
    assert_eq!(report["promotion_eligible"], true);
    assert_eq!(report["accepted"], false);
    assert_eq!(report["matrix"].as_array().unwrap().len(), 20);
    assert!(report["evidence_inventory"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| {
            output
                .join(entry["retained_blob"].as_str().unwrap())
                .is_file()
                && entry["content_hash"]
                    .as_str()
                    .unwrap()
                    .starts_with("sha256:")
        }));
    assert!(report["failure_proposals"].as_array().unwrap().is_empty());
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn missing_unavailable_timeout_and_unresolved_review_cannot_pass() {
    for mode in ["missing", "provider_unavailable", "timeout"] {
        let directory = temp(&format!("{mode}-input"));
        let output = temp(&format!("{mode}-parent")).join("report");
        fixture(&directory, mode);
        let result = aggregate(&directory, &output, true);
        assert!(!result.status.success(), "{mode} unexpectedly passed");
        let report = report(&output);
        assert_eq!(report["status"], "fail");
        assert_eq!(report["promotion_eligible"], false);
        assert!(report["failure_proposals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|proposal| {
                proposal["accepted"] == false && proposal["review_status"] == "unreviewed"
            }));
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(output.parent().unwrap()).unwrap();
    }
    let directory = temp("manual-input");
    let output = temp("manual-parent").join("report");
    fixture(&directory, "pass");
    assert!(!aggregate(&directory, &output, false).status.success());
    assert!(report(&output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| { finding == "manual_judgments_unresolved" }));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn unobservable_cost_requires_a_run_bound_reviewer_limit() {
    let directory = temp("cost-waiver-input");
    fixture(&directory, "unobservable_cost");

    let valid_output = temp("cost-waiver-valid-parent").join("report");
    let valid = aggregate(&directory, &valid_output, true);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert_eq!(report(&valid_output)["status"], "pass");

    let review_path = directory.join("manual-review.json");
    let mut review: Value = serde_json::from_slice(&fs::read(&review_path).unwrap()).unwrap();
    review["cost_waivers"][0]
        .as_object_mut()
        .unwrap()
        .remove("maximum_usd");
    fs::write(&review_path, serde_json::to_vec_pretty(&review).unwrap()).unwrap();
    let invalid_output = temp("cost-waiver-invalid-parent").join("report");
    let invalid = aggregate(&directory, &invalid_output, true);
    assert!(!invalid.status.success());
    assert!(report(&invalid_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_cost_waiver:codex"));

    review["cost_waivers"][0]["maximum_usd"] = Value::from(10.0);
    fs::write(&review_path, serde_json::to_vec_pretty(&review).unwrap()).unwrap();
    let limited_output = temp("cost-waiver-limited-parent").join("report");
    let limited = aggregate(&directory, &limited_output, true);
    assert!(!limited.status.success());
    assert!(report(&limited_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "cost_waiver_limit_exceeded:codex"));

    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(valid_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(invalid_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(limited_output.parent().unwrap()).unwrap();
}
