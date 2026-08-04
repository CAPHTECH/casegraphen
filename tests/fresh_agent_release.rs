#![allow(missing_docs)]

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

const REVIEWER_IDENTITY: &str = "reviewer:fixture-independent";
const REVIEWER_KEY_ID: &str = "fresh-agent-reviewer-fixture-v1";

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
        .arg(output)
        .args(["--host-attestation"])
        .arg(format!(
            "codex={}",
            directory.join("codex-host-attestation.json").display()
        ))
        .args(["--host-attestation"])
        .arg(format!(
            "claude={}",
            directory.join("claude-host-attestation.json").display()
        ))
        .args(["--attestation-public-key"])
        .arg(format!(
            "codex={}",
            directory
                .join("codex-host-attestation-public.pem")
                .display()
        ))
        .args(["--attestation-public-key"])
        .arg(format!(
            "claude={}",
            directory
                .join("claude-host-attestation-public.pem")
                .display()
        ))
        .args(["--manual-review-public-key"])
        .arg(directory.join("manual-review-public.pem"))
        .args(["--expected-reviewer-identity", REVIEWER_IDENTITY])
        .args(["--expected-reviewer-key-id", REVIEWER_KEY_ID])
        .args(["--expected-provenance"])
        .arg(format!(
            "codex={}",
            directory.join("codex-expected-provenance.json").display()
        ))
        .args(["--expected-provenance"])
        .arg(format!(
            "claude={}",
            directory.join("claude-expected-provenance.json").display()
        ));
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

fn generate_rsa_keypair(private_key: &Path, public_key: &Path) {
    let generated = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
        ])
        .arg(private_key)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let exported = Command::new("openssl")
        .args(["pkey", "-in"])
        .arg(private_key)
        .args(["-pubout", "-out"])
        .arg(public_key)
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
}

fn inject_duplicate_top_level_member(path: &Path, member: &str) {
    let original = fs::read_to_string(path).unwrap();
    assert!(original.starts_with('{'));
    fs::write(path, format!("{{{member},{}", &original[1..])).unwrap();
}

fn sign_review(
    directory: &Path,
    input: &Path,
    output: &Path,
    allowed_review_root: Option<&Path>,
) -> std::process::Output {
    let mut command = Command::new("python3");
    command
        .args([
            "scripts/fresh-agent-run-provenance.py",
            "sign-review",
            "--input",
        ])
        .arg(input)
        .args(["--output"])
        .arg(output)
        .args(["--private-key"])
        .arg(directory.join("manual-review-private.pem"))
        .args(["--reviewer-identity", REVIEWER_IDENTITY])
        .args(["--reviewer-key-id", REVIEWER_KEY_ID])
        .args(["--expected-provenance"])
        .arg(format!(
            "codex={}",
            directory.join("codex-expected-provenance.json").display()
        ))
        .args(["--expected-provenance"])
        .arg(format!(
            "claude={}",
            directory.join("claude-expected-provenance.json").display()
        ));
    if let Some(root) = allowed_review_root {
        command.args(["--allowed-review-root"]).arg(root);
    }
    command.current_dir(root()).output().unwrap()
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
fn caller_cli_session_assertion_without_host_attestation_cannot_promote() {
    let directory = temp("self-asserted-input");
    let output = temp("self-asserted-parent").join("report");
    fixture(&directory, "pass");
    let result = Command::new("python3")
        .args(["scripts/fresh-agent-release.py", "--runs-root"])
        .arg(&directory)
        .args(["--manual-review"])
        .arg(directory.join("manual-review.json"))
        .args(["--output-dir"])
        .arg(&output)
        .current_dir(root())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let report = report(&output);
    assert_eq!(report["promotion_eligible"], false);
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "missing_host_attestation:codex"));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn substituted_host_attestation_signature_fails_closed() {
    let directory = temp("attestation-substitution-input");
    let output = temp("attestation-substitution-parent").join("report");
    fixture(&directory, "pass");
    let path = directory.join("codex-host-attestation.json");
    let mut attestation: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    attestation["runner_instance_id_hash"] = Value::from(format!("sha256:{}", "f".repeat(64)));
    fs::write(&path, serde_json::to_vec_pretty(&attestation).unwrap()).unwrap();
    let result = aggregate(&directory, &output, true);
    assert!(!result.status.success());
    assert!(report(&output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_host_attestation_signature:codex"));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn wrong_provider_public_key_cannot_verify_or_forge_an_attestation() {
    let directory = temp("attestation-wrong-key-input");
    let output = temp("attestation-wrong-key-parent").join("report");
    fixture(&directory, "pass");
    fs::copy(
        directory.join("claude-host-attestation-public.pem"),
        directory.join("codex-host-attestation-public.pem"),
    )
    .unwrap();
    assert!(!aggregate(&directory, &output, true).status.success());
    assert!(report(&output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_host_attestation_signature:codex"));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn valid_signature_with_stale_source_coordinates_fails_closed() {
    let directory = temp("attestation-stale-provenance-input");
    let output = temp("attestation-stale-provenance-parent").join("report");
    fixture(&directory, "stale_provenance");
    assert!(!aggregate(&directory, &output, true).status.success());
    assert!(report(&output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "host_attestation_binding_mismatch:codex:provenance"));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn signed_manual_review_with_substituted_provider_provenance_fails_closed() {
    let directory = temp("manual-stale-provenance-input");
    let output = temp("manual-stale-provenance-parent").join("report");
    fixture(&directory, "manual_stale_provenance");
    let result = aggregate(&directory, &output, true);
    assert!(!result.status.success());
    let report = report(&output);
    assert_eq!(
        report["manual_review_authority"]["signature_verified"],
        true
    );
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "manual_review_provenance_binding_mismatch"));
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn unsigned_or_substituted_manual_review_cannot_supply_authority() {
    let unsigned_directory = temp("unsigned-review-input");
    let unsigned_output = temp("unsigned-review-parent").join("report");
    fixture(&unsigned_directory, "pass");
    let review_path = unsigned_directory.join("manual-review.json");
    let mut review: Value = serde_json::from_slice(&fs::read(&review_path).unwrap()).unwrap();
    review.as_object_mut().unwrap().remove("ed25519_signature");
    fs::write(&review_path, serde_json::to_vec_pretty(&review).unwrap()).unwrap();
    assert!(!aggregate(&unsigned_directory, &unsigned_output, true)
        .status
        .success());
    assert!(report(&unsigned_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_manual_review_signature"));

    let substituted_directory = temp("substituted-reviewer-input");
    let substituted_output = temp("substituted-reviewer-parent").join("report");
    fixture(&substituted_directory, "pass");
    let review_path = substituted_directory.join("manual-review.json");
    let mut review: Value = serde_json::from_slice(&fs::read(&review_path).unwrap()).unwrap();
    review["reviewer_identity"] = Value::from("reviewer:substituted");
    fs::write(&review_path, serde_json::to_vec_pretty(&review).unwrap()).unwrap();
    assert!(
        !aggregate(&substituted_directory, &substituted_output, true)
            .status
            .success()
    );
    let findings = report(&substituted_output)["findings"]
        .as_array()
        .unwrap()
        .clone();
    assert!(findings
        .iter()
        .any(|finding| finding == "invalid_manual_review_signature"));
    assert!(findings
        .iter()
        .any(|finding| finding == "manual_review_reviewer_identity_mismatch"));

    fs::remove_dir_all(unsigned_directory).unwrap();
    fs::remove_dir_all(unsigned_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(substituted_directory).unwrap();
    fs::remove_dir_all(substituted_output.parent().unwrap()).unwrap();
}

#[test]
fn rsa_keys_cannot_supply_ed25519_public_or_private_key_authority() {
    let public_directory = temp("rsa-public-key-input");
    let public_output = temp("rsa-public-key-parent").join("report");
    fixture(&public_directory, "pass");
    let rsa_private = public_directory.join("rsa-private.pem");
    let rsa_public = public_directory.join("rsa-public.pem");
    generate_rsa_keypair(&rsa_private, &rsa_public);

    fs::copy(
        &rsa_public,
        public_directory.join("codex-host-attestation-public.pem"),
    )
    .unwrap();
    fs::copy(
        &rsa_public,
        public_directory.join("manual-review-public.pem"),
    )
    .unwrap();
    assert!(!aggregate(&public_directory, &public_output, true)
        .status
        .success());
    let findings = report(&public_output)["findings"]
        .as_array()
        .unwrap()
        .clone();
    assert!(findings
        .iter()
        .any(|finding| finding == "invalid_host_attestation_signature:codex"));
    assert!(findings
        .iter()
        .any(|finding| finding == "invalid_manual_review_signature"));

    let private_directory = temp("rsa-private-key-input");
    fixture(&private_directory, "pass");
    let rsa_private = private_directory.join("rsa-private.pem");
    let rsa_public = private_directory.join("rsa-public.pem");
    generate_rsa_keypair(&rsa_private, &rsa_public);
    let evaluation_host_spki =
        fs::read_to_string(private_directory.join("codex-evaluation-host-public-spki-sha256.txt"))
            .unwrap();
    let result = Command::new("python3")
        .args(["scripts/fresh-agent-host-attest.py", "--summary"])
        .arg(private_directory.join("codex/summary.json"))
        .args(["--provider", "codex"])
        .args(["--private-key-file"])
        .arg(&rsa_private)
        .args([
            "--key-id",
            "codex-cli-session-host-v1",
            "--evaluation-host-proof",
        ])
        .arg(private_directory.join("codex-evaluation-host-proof.json"))
        .arg("--evaluation-host-public-key")
        .arg(private_directory.join("codex-evaluation-host-public.pem"))
        .args([
            "--evaluation-host-key-id",
            "codex-evaluation-host-fixture-v1",
            "--evaluation-host-public-key-spki-sha256",
            evaluation_host_spki.trim(),
            "--provenance-file",
        ])
        .arg(private_directory.join("codex-expected-provenance.json"))
        .arg("--output")
        .arg(private_directory.join("rsa-signed-attestation.json"))
        .current_dir(root())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("host attestation signing key must be Ed25519"));
    assert!(!private_directory
        .join("rsa-signed-attestation.json")
        .exists());

    fs::remove_dir_all(public_directory).unwrap();
    fs::remove_dir_all(public_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(private_directory).unwrap();
}

#[test]
fn duplicate_json_keys_in_manual_review_or_attestation_fail_closed() {
    let manual_directory = temp("duplicate-manual-json-input");
    let manual_output = temp("duplicate-manual-json-parent").join("report");
    fixture(&manual_directory, "pass");
    inject_duplicate_top_level_member(
        &manual_directory.join("manual-review.json"),
        r#""reviewer_identity":"reviewer:duplicate""#,
    );
    let manual = aggregate(&manual_directory, &manual_output, true);
    assert!(!manual.status.success());
    assert!(String::from_utf8_lossy(&manual.stderr).contains("duplicate JSON key"));
    assert!(!manual_output.join("release-report.pointer.json").exists());

    let attestation_directory = temp("duplicate-attestation-json-input");
    let attestation_output = temp("duplicate-attestation-json-parent").join("report");
    fixture(&attestation_directory, "pass");
    inject_duplicate_top_level_member(
        &attestation_directory.join("codex-host-attestation.json"),
        r#""provider":"claude""#,
    );
    let attestation = aggregate(&attestation_directory, &attestation_output, true);
    assert!(!attestation.status.success());
    assert!(report(&attestation_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "unreadable_host_attestation:codex"));

    fs::remove_dir_all(manual_directory).unwrap();
    fs::remove_dir_all(manual_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(attestation_directory).unwrap();
    fs::remove_dir_all(attestation_output.parent().unwrap()).unwrap();
}

#[test]
fn manual_review_signer_rejects_duplicate_json_keys() {
    let directory = temp("sign-review-duplicate-input");
    fixture(&directory, "pass");
    let review_root = directory.join("reviews");
    fs::create_dir(&review_root).unwrap();
    let input = review_root.join("unsigned.json");
    fs::write(
        &input,
        r#"{"run_content_hashes":{},"judgments":[],"judgments":[]}"#,
    )
    .unwrap();
    let output = directory.join("signed.json");
    let result = sign_review(&directory, &input, &output, Some(&review_root));
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate JSON key: judgments"));
    assert!(!output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn manual_review_signer_binds_exact_provider_provenance() {
    let directory = temp("sign-review-provenance-input");
    let output = temp("sign-review-provenance-parent").join("report");
    fixture(&directory, "pass");
    let review_root = directory.join("reviews");
    fs::create_dir(&review_root).unwrap();
    let unsigned_path = review_root.join("unsigned.json");
    let signed_path = directory.join("manual-review.json");
    let mut unsigned: Value = serde_json::from_slice(&fs::read(&signed_path).unwrap()).unwrap();
    unsigned
        .as_object_mut()
        .unwrap()
        .remove("ed25519_signature");
    unsigned
        .as_object_mut()
        .unwrap()
        .remove("expected_provider_provenance");
    fs::write(
        &unsigned_path,
        serde_json::to_vec_pretty(&unsigned).unwrap(),
    )
    .unwrap();
    let signed = sign_review(&directory, &unsigned_path, &signed_path, Some(&review_root));
    assert!(
        signed.status.success(),
        "{}",
        String::from_utf8_lossy(&signed.stderr)
    );
    let review: Value = serde_json::from_slice(&fs::read(&signed_path).unwrap()).unwrap();
    for provider in ["codex", "claude"] {
        let provenance: Value = serde_json::from_slice(
            &fs::read(directory.join(format!("{provider}-expected-provenance.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(review["expected_provider_provenance"][provider], provenance);
        assert!(
            review["expected_provider_provenance"][provider]["provider_artifact"]
                .get("id")
                .is_some()
        );
        assert!(
            review["expected_provider_provenance"][provider]["provider_artifact"]
                .get("name")
                .is_some()
        );
        assert!(
            review["expected_provider_provenance"][provider]["provider_artifact"]
                .get("digest")
                .is_some()
        );
    }
    let result = aggregate(&directory, &output, true);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn manual_review_signer_rejects_symlink_and_allowed_root_escape() {
    let directory = temp("sign-review-path-input");
    fixture(&directory, "pass");
    let review_root = directory.join("reviews");
    fs::create_dir(&review_root).unwrap();
    let outside = directory.join("outside.json");
    fs::write(&outside, r#"{"run_content_hashes":{},"judgments":[]}"#).unwrap();

    let linked = review_root.join("linked.json");
    symlink(&outside, &linked).unwrap();
    let linked_output = directory.join("linked-signed.json");
    let linked_result = sign_review(&directory, &linked, &linked_output, Some(&review_root));
    assert!(!linked_result.status.success());
    assert!(String::from_utf8_lossy(&linked_result.stderr)
        .contains("manual review input must not be a symlink"));
    assert!(!linked_output.exists());

    let escaped_output = directory.join("escaped-signed.json");
    let escaped_result = sign_review(&directory, &outside, &escaped_output, Some(&review_root));
    assert!(!escaped_result.status.success());
    assert!(String::from_utf8_lossy(&escaped_result.stderr)
        .contains("manual review input escapes the allowed review root"));
    assert!(!escaped_output.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn noncanonical_and_malformed_base64_signatures_fail_closed() {
    let noncanonical_directory = temp("noncanonical-signature-input");
    let noncanonical_output = temp("noncanonical-signature-parent").join("report");
    fixture(&noncanonical_directory, "pass");
    let manual_path = noncanonical_directory.join("manual-review.json");
    let mut manual: Value = serde_json::from_slice(&fs::read(&manual_path).unwrap()).unwrap();
    let signature = manual["ed25519_signature"].as_str().unwrap().to_owned();
    manual["ed25519_signature"] = Value::from(format!("{signature}="));
    fs::write(&manual_path, serde_json::to_vec_pretty(&manual).unwrap()).unwrap();
    assert!(
        !aggregate(&noncanonical_directory, &noncanonical_output, true)
            .status
            .success()
    );
    assert!(report(&noncanonical_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_manual_review_signature"));

    let malformed_directory = temp("malformed-signature-input");
    let malformed_output = temp("malformed-signature-parent").join("report");
    fixture(&malformed_directory, "pass");
    let attestation_path = malformed_directory.join("codex-host-attestation.json");
    let mut attestation: Value =
        serde_json::from_slice(&fs::read(&attestation_path).unwrap()).unwrap();
    attestation["ed25519_signature"] = Value::from("not base64!");
    fs::write(
        &attestation_path,
        serde_json::to_vec_pretty(&attestation).unwrap(),
    )
    .unwrap();
    assert!(!aggregate(&malformed_directory, &malformed_output, true)
        .status
        .success());
    assert!(report(&malformed_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_host_attestation_signature:codex"));

    fs::remove_dir_all(noncanonical_directory).unwrap();
    fs::remove_dir_all(noncanonical_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(malformed_directory).unwrap();
    fs::remove_dir_all(malformed_output.parent().unwrap()).unwrap();
}

#[test]
fn nonfinite_manual_waiver_or_provider_budget_fails_closed() {
    let waiver_directory = temp("nonfinite-waiver-input");
    let waiver_output = temp("nonfinite-waiver-parent").join("report");
    fixture(&waiver_directory, "unobservable_cost");
    let review_path = waiver_directory.join("manual-review.json");
    let review = fs::read_to_string(&review_path).unwrap();
    let replaced = review.replacen("\"maximum_usd\": 25.0", "\"maximum_usd\": NaN", 1);
    assert_ne!(review, replaced);
    fs::write(&review_path, replaced).unwrap();
    let waiver = aggregate(&waiver_directory, &waiver_output, true);
    assert!(!waiver.status.success());
    assert!(String::from_utf8_lossy(&waiver.stderr).contains("non-finite JSON number: NaN"));
    assert!(!waiver_output.join("release-report.pointer.json").exists());

    let budget_directory = temp("nonfinite-budget-input");
    let budget_output = temp("nonfinite-budget-parent").join("report");
    fixture(&budget_directory, "pass");
    let summary_path = budget_directory.join("codex/summary.json");
    let summary = fs::read_to_string(&summary_path).unwrap();
    let replaced = summary.replacen("\"observed_usd\": 1.0", "\"observed_usd\": Infinity", 1);
    assert_ne!(summary, replaced);
    fs::write(&summary_path, replaced).unwrap();
    let budget = aggregate(&budget_directory, &budget_output, true);
    assert!(!budget.status.success());
    assert!(String::from_utf8_lossy(&budget.stderr).contains("non-finite JSON number: Infinity"));
    assert!(!budget_output.join("release-report.pointer.json").exists());

    fs::remove_dir_all(waiver_directory).unwrap();
    fs::remove_dir_all(waiver_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(budget_directory).unwrap();
    fs::remove_dir_all(budget_output.parent().unwrap()).unwrap();
}

#[test]
fn privileged_host_attester_verifies_evaluation_host_proof_and_binds_exact_run() {
    let directory = temp("host-attester-input");
    let output = temp("host-attester-parent").join("report");
    fixture(&directory, "pass");
    let evaluation_host_spki =
        fs::read_to_string(directory.join("codex-evaluation-host-public-spki-sha256.txt")).unwrap();
    let attestation = directory.join("codex-host-attestation.json");
    let result = Command::new("python3")
        .args(["scripts/fresh-agent-host-attest.py", "--summary"])
        .arg(directory.join("codex/summary.json"))
        .args(["--provider", "codex"])
        .args(["--private-key-file"])
        .arg(directory.join("codex-host-attestation-private.pem"))
        .args([
            "--key-id",
            "codex-cli-session-host-v1",
            "--evaluation-host-proof",
        ])
        .arg(directory.join("codex-evaluation-host-proof.json"))
        .arg("--evaluation-host-public-key")
        .arg(directory.join("codex-evaluation-host-public.pem"))
        .args([
            "--evaluation-host-key-id",
            "codex-evaluation-host-fixture-v1",
            "--evaluation-host-public-key-spki-sha256",
            evaluation_host_spki.trim(),
            "--provenance-file",
        ])
        .arg(directory.join("codex-expected-provenance.json"))
        .arg("--output")
        .arg(&attestation)
        .current_dir(root())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let aggregate = aggregate(&directory, &output, true);
    assert!(aggregate.status.success());
    assert_eq!(report(&output)["promotion_eligible"], true);
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn broker_cannot_replace_or_skip_provider_runner_host_proof() {
    let directory = temp("evaluation-host-proof-authority-input");
    fixture(&directory, "pass");
    let wrong_spki =
        fs::read_to_string(directory.join("claude-evaluation-host-public-spki-sha256.txt"))
            .unwrap();
    let result = Command::new("python3")
        .args(["scripts/fresh-agent-host-attest.py", "--summary"])
        .arg(directory.join("codex/summary.json"))
        .args(["--provider", "codex", "--private-key-file"])
        .arg(directory.join("codex-host-attestation-private.pem"))
        .args([
            "--key-id",
            "codex-cli-session-host-v1",
            "--evaluation-host-proof",
        ])
        .arg(directory.join("codex-evaluation-host-proof.json"))
        .arg("--evaluation-host-public-key")
        .arg(directory.join("claude-evaluation-host-public.pem"))
        .args([
            "--evaluation-host-key-id",
            "codex-evaluation-host-fixture-v1",
            "--evaluation-host-public-key-spki-sha256",
            wrong_spki.trim(),
            "--provenance-file",
        ])
        .arg(directory.join("codex-expected-provenance.json"))
        .arg("--output")
        .arg(directory.join("substituted-host-proof-attestation.json"))
        .current_dir(root())
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("evaluation-host proof signature is invalid"));
    assert!(!directory
        .join("substituted-host-proof-attestation.json")
        .exists());

    let missing = Command::new("python3")
        .args(["scripts/fresh-agent-host-attest.py", "--summary"])
        .arg(directory.join("codex/summary.json"))
        .args(["--provider", "codex", "--private-key-file"])
        .arg(directory.join("codex-host-attestation-private.pem"))
        .args(["--key-id", "codex-cli-session-host-v1"])
        .arg("--provenance-file")
        .arg(directory.join("codex-expected-provenance.json"))
        .arg("--output")
        .arg(directory.join("missing-host-proof-attestation.json"))
        .current_dir(root())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(!directory
        .join("missing-host-proof-attestation.json")
        .exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn finalizer_rechecks_the_retained_evaluation_host_proof_and_public_key() {
    let proof_directory = temp("retained-host-proof-substitution-input");
    let proof_output = temp("retained-host-proof-substitution-parent").join("report");
    fixture(&proof_directory, "pass");
    fs::copy(
        proof_directory.join("claude-evaluation-host-proof.json"),
        proof_directory.join("codex-evaluation-host-proof.json"),
    )
    .unwrap();
    let substituted_proof = aggregate(&proof_directory, &proof_output, true);
    assert!(!substituted_proof.status.success());
    assert!(report(&proof_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "evaluation_host_proof_hash_mismatch:codex"));

    let key_directory = temp("retained-host-key-substitution-input");
    let key_output = temp("retained-host-key-substitution-parent").join("report");
    fixture(&key_directory, "pass");
    fs::copy(
        key_directory.join("claude-evaluation-host-public.pem"),
        key_directory.join("codex-evaluation-host-public.pem"),
    )
    .unwrap();
    let substituted_key = aggregate(&key_directory, &key_output, true);
    assert!(!substituted_key.status.success());
    assert!(report(&key_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "evaluation_host_public_key_mismatch:codex"));

    fs::remove_dir_all(proof_directory).unwrap();
    fs::remove_dir_all(proof_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(key_directory).unwrap();
    fs::remove_dir_all(key_output.parent().unwrap()).unwrap();
}

#[test]
fn missing_unavailable_timeout_and_unresolved_review_cannot_pass() {
    for mode in [
        "missing",
        "provider_unavailable",
        "cli_session_unavailable",
        "timeout",
        "model_mismatch",
    ] {
        let directory = temp(&format!("{mode}-input"));
        let output = temp(&format!("{mode}-parent")).join("report");
        fixture(&directory, mode);
        let result = aggregate(&directory, &output, true);
        assert!(!result.status.success(), "{mode} unexpectedly passed");
        let report = report(&output);
        assert_eq!(report["status"], "fail");
        assert_eq!(report["promotion_eligible"], false);
        if mode == "model_mismatch" {
            assert!(report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|finding| finding == "provider_model_observation_mismatch:claude"));
        }
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

    let invalid_directory = temp("cost-waiver-invalid-input");
    fixture(&invalid_directory, "unobservable_cost_missing_limit");
    let invalid_output = temp("cost-waiver-invalid-parent").join("report");
    let invalid = aggregate(&invalid_directory, &invalid_output, true);
    assert!(!invalid.status.success());
    assert!(report(&invalid_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "invalid_cost_waiver:codex"));

    let limited_directory = temp("cost-waiver-limited-input");
    fixture(&limited_directory, "unobservable_cost_low_waiver");
    let limited_output = temp("cost-waiver-limited-parent").join("report");
    let limited = aggregate(&limited_directory, &limited_output, true);
    assert!(!limited.status.success());
    assert!(report(&limited_output)["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding == "cost_waiver_limit_exceeded:codex"));

    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(invalid_directory).unwrap();
    fs::remove_dir_all(limited_directory).unwrap();
    fs::remove_dir_all(valid_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(invalid_output.parent().unwrap()).unwrap();
    fs::remove_dir_all(limited_output.parent().unwrap()).unwrap();
}
