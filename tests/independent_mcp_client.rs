#![allow(missing_docs)]

use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn python_stdlib_client_reaches_review_seam_without_accepting_runtime_output() {
    let outputs = ["a", "b"].map(|suffix| {
        std::env::temp_dir().join(format!(
            "casegraphen-independent-mcp-client-{}-{suffix}.json",
            std::process::id()
        ))
    });
    for output in &outputs {
        let _ = fs::remove_file(output);
        let status = Command::new("python3")
            .arg("scripts/independent-mcp-client.py")
            .arg("--host-bin")
            .arg(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
            .arg("--topology")
            .arg("pilots/runtime-integration/topologies/fanout-reduce.json")
            .arg("--output")
            .arg(output)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .expect("run independent Python MCP client");
        assert!(status.success());
    }

    let evidence_bytes = fs::read(&outputs[0]).expect("evidence report exists");
    assert_eq!(
        evidence_bytes,
        fs::read(&outputs[1]).expect("repeated evidence report exists"),
        "identical host and topology bytes must produce reproducible evidence"
    );

    let report: Value = serde_json::from_slice(&evidence_bytes).expect("evidence report is JSON");
    assert_eq!(report["client_implementation"], "python_stdlib_json_rpc");
    assert_eq!(report["custom_rust_client_code"], false);
    assert_eq!(report["final_boundary"]["review_required"], true);
    assert_eq!(report["final_boundary"]["accepted"], false);
    assert_eq!(report["final_boundary"]["all_proposals_unreviewed"], true);
    assert_eq!(report["reconciliation"]["completeness"]["complete"], true);
    assert_eq!(report["reconciliation"]["halt"], "needs_review");
    assert!(report["attachments"]
        .as_array()
        .expect("attachments")
        .iter()
        .all(|attachment| attachment["accepted"] == false));
    for output in outputs {
        let _ = fs::remove_file(output);
    }
}
