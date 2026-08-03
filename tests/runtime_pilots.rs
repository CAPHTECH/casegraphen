#![allow(missing_docs)]

use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn two_real_local_runtime_adapters_fail_closed_at_the_operational_host() {
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
    assert_eq!(report["adapters"].as_array().unwrap().len(), 2);
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
        "redesign-proposal.json",
        "v0-next-version-proposal.json",
        "promotion-report.json",
    ] {
        assert!(
            PathBuf::from(&output).join(name).is_file(),
            "missing {name}"
        );
    }
    let _ = fs::remove_dir_all(output);
}
