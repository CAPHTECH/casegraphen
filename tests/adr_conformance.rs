#![allow(missing_docs)]

use std::{path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn checker(fixture: &str) -> std::process::Output {
    let fixture = root().join("tests/fixtures/adr-conformance").join(fixture);
    Command::new("python3")
        .arg(root().join("scripts/adr-conformance.py"))
        .arg("--adr-dir")
        .arg(&fixture)
        .arg("--markdown-root")
        .arg(&fixture)
        .current_dir(root())
        .output()
        .expect("run ADR conformance checker")
}

#[test]
fn repository_adr_inventory_conforms() {
    let output = Command::new("python3")
        .arg(root().join("scripts/adr-conformance.py"))
        .args(["--index", "README.md"])
        .current_dir(root())
        .output()
        .expect("run ADR conformance checker");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn duplicate_identifier_is_refused_independent_of_directory_order() {
    let output = checker("duplicate");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate ADR 0002"));
}

#[test]
fn filename_heading_mismatch_is_refused() {
    let output = checker("heading-mismatch");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("filename ADR 0002 does not match heading ADR 0001"));
}

#[test]
fn missing_identifier_is_refused() {
    let output = checker("missing");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing ADR identifiers: 0002"));
}

#[test]
fn broken_adr_link_is_refused() {
    let output = checker("broken-link");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("broken ADR link"));
}
