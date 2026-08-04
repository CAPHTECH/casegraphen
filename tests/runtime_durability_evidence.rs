#![allow(missing_docs)]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn temp(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("casegraphen-{label}-{nonce}"))
}

fn fixture(root: &Path) {
    let result = Command::new("python3")
        .arg("tests/fixtures/runtime-durability/build-evidence-fixture.py")
        .arg(root)
        .output()
        .expect("build bounded runtime evidence fixture");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn build(evidence: &Path, output: &Path) {
    let result = Command::new("python3")
        .arg("scripts/runtime-durability-evidence.py")
        .arg("build-package")
        .args(["--evidence-dir", evidence.to_str().unwrap()])
        .args(["--repository", "CAPHTECH/casegraphen"])
        .args(["--evaluated-commit", &"a".repeat(40)])
        .args(["--workflow-run-id", "17"])
        .args(["--workflow-run-attempt", "2"])
        .args(["--output-dir", output.to_str().unwrap()])
        .output()
        .expect("build runtime durability package");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn verify(output: &Path) -> std::process::Output {
    Command::new("python3")
        .arg("scripts/runtime-durability-evidence.py")
        .arg("verify-offline")
        .args([
            "--manifest",
            output.join("retention-record.json").to_str().unwrap(),
        ])
        .args([
            "--asset",
            output
                .join("runtime-durability-evidence.tar.gz")
                .to_str()
                .unwrap(),
        ])
        .output()
        .expect("verify runtime durability package")
}

#[test]
fn bounded_fixture_builds_deterministically_and_verifies_offline() {
    let evidence = temp("runtime-evidence-fixture");
    let first = temp("runtime-evidence-first");
    let second = temp("runtime-evidence-second");
    fixture(&evidence);
    build(&evidence, &first);
    build(&evidence, &second);

    let first_bytes = fs::read(first.join("runtime-durability-evidence.tar.gz")).unwrap();
    let second_bytes = fs::read(second.join("runtime-durability-evidence.tar.gz")).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert!(verify(&first).status.success());

    let record: Value =
        serde_json::from_slice(&fs::read(first.join("retention-record.json")).unwrap()).unwrap();
    assert_eq!(record["accepted"], false);
    assert_eq!(record["promotion_recommended"], false);
    assert_eq!(
        record["release"]["package_sha256"],
        format!("sha256:{:x}", Sha256::digest(first_bytes))
    );
}

#[test]
fn substituted_package_fails_offline_verification() {
    let evidence = temp("runtime-evidence-substitution-fixture");
    let output = temp("runtime-evidence-substitution");
    fixture(&evidence);
    build(&evidence, &output);
    let package = output.join("runtime-durability-evidence.tar.gz");
    let mut bytes = fs::read(&package).unwrap();
    bytes.push(0);
    fs::write(&package, &bytes).unwrap();
    let mut record: Value =
        serde_json::from_slice(&fs::read(output.join("retention-record.json")).unwrap()).unwrap();
    let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
    let bare = hash.trim_start_matches("sha256:").to_owned();
    record["release"]["package_sha256"] = hash.into();
    record["release"]["byte_length"] = bytes.len().into();
    record["release"]["tag"] = format!("runtime-durability-evidence-{bare}").into();
    record["release"]["asset_name"] = format!("sha256-{bare}.tar.gz").into();
    record["release"]["url"] = format!(
        "https://github.com/CAPHTECH/casegraphen/releases/tag/runtime-durability-evidence-{bare}"
    )
    .into();
    fs::write(
        output.join("retention-record.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let result = verify(&output);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not the deterministic canonical"));
}
