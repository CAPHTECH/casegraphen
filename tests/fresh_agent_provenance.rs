#![allow(missing_docs)]

use std::{path::Path, process::Command};

#[test]
fn artifact_redirects_never_forward_the_github_token_cross_origin() {
    let output = Command::new("python3")
        .arg("tests/fixtures/fresh-agent/run-provenance-redirect-self-test.py")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("run redirect boundary self-test");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
