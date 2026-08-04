#![allow(missing_docs)]

use serde_json::Value;
use sha2::Digest;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn four_materially_distinct_runtime_families_fail_closed_at_the_operational_host() {
    let output =
        std::env::temp_dir().join(format!("casegraphen-runtime-pilots-{}", std::process::id()));
    let _ = fs::remove_dir_all(&output);
    let status = Command::new("python3")
        .arg("scripts/runtime-integration-pilots.py")
        .arg("--repo")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .arg("--host-bin")
        .arg(env!("CARGO_BIN_EXE_casegraphen-mcp-host"))
        .arg("--output")
        .arg(&output)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("run local-runtime pilot harness");
    assert!(status.success());

    let report: Value = serde_json::from_slice(
        &fs::read(output.join("pilot-report.json")).expect("pilot report exists"),
    )
    .expect("pilot report is JSON");
    assert_eq!(report["accepted"], false);
    assert_eq!(report["adapters"].as_array().unwrap().len(), 4);
    assert_eq!(
        report["scenarios"]["fanout_reduce_complete"]["halt"],
        "needs_review"
    );
    assert_eq!(
        report["scenarios"]["missing_report"]["completeness"]["missing_report_count"],
        1
    );
    assert_eq!(
        report["scenarios"]["worktree_isolation"]["halt"],
        "resource_reconciliation_incomplete"
    );
    for family in [
        "sqlite_resource_reconciliation",
        "async_resource_reconciliation",
    ] {
        assert_eq!(report["scenarios"][family]["halt"], "needs_review");
        assert_eq!(report["scenarios"][family]["accepted"], false);
        assert_eq!(report["scenarios"][family]["reconciliation_complete"], true);
    }
    assert!(report["assertions"]
        .as_object()
        .unwrap()
        .values()
        .all(|value| value == true));
    assert_eq!(report["redesign_proposal"]["accepted"], false);
    assert_eq!(
        report["next_version_proposal"]["review_status"],
        "unreviewed"
    );
    assert_eq!(report["promotion_report"]["promotion_recommended"], false);

    for name in [
        "process-jsonl.complete.jsonl",
        "file-drop.complete.jsonl",
        "sqlite-queue.complete.jsonl",
        "async-stream.complete.jsonl",
        "redesign-proposal.json",
        "v0-next-version-proposal.json",
        "promotion-report.json",
        "independent-mcp-client-report.json",
        "retained-evidence.manifest.json",
    ] {
        assert!(
            PathBuf::from(&output).join(name).is_file(),
            "missing {name}"
        );
    }
    let manifest: Value = serde_json::from_slice(
        &fs::read(output.join("retained-evidence.manifest.json"))
            .expect("retained evidence manifest exists"),
    )
    .expect("retained evidence manifest is JSON");
    assert_eq!(manifest["accepted"], false);
    assert_eq!(manifest["files"].as_array().unwrap().len(), 9);
    for entry in manifest["files"].as_array().unwrap() {
        let bytes = fs::read(output.join(entry["path"].as_str().unwrap())).unwrap();
        let observed = format!("sha256:{:x}", sha2::Sha256::digest(bytes));
        assert_eq!(entry["content_hash"], observed);
    }
    let independent: Value = serde_json::from_slice(
        &fs::read(output.join("independent-mcp-client-report.json"))
            .expect("independent MCP evidence exists"),
    )
    .unwrap();
    assert_eq!(
        independent["client_implementation"],
        "python_stdlib_json_rpc"
    );
    assert_eq!(independent["custom_rust_client_code"], false);
    assert_eq!(independent["final_boundary"]["review_required"], true);
    assert_eq!(independent["final_boundary"]["accepted"], false);
    let _ = fs::remove_dir_all(output);
}
