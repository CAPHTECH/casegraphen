#![allow(missing_docs)]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
fn retained_remote_binary_scale_and_allocator_evidence_is_bounded_and_fail_closed() {
    let output = Path::new("docs/pilots/issue-85");

    let report: Value = serde_json::from_slice(
        &fs::read(output.join("durability-report.json")).expect("durability report exists"),
    )
    .expect("durability report is JSON");
    assert_eq!(report["accepted"], false);
    assert_eq!(report["promotion_eligible"], false);
    assert_eq!(report["all_thresholds_passed"], true);
    assert!(report["blockers"][0].as_str().unwrap().contains("#76"));

    let remote = &report["reports"]["remote"];
    assert_eq!(remote["passed"], true);
    for event in [
        "disconnect_observed",
        "duplicate_delivery_resumed",
        "timeout_observed",
        "process_restarted",
        "reconnected",
        "journal_resumed",
    ] {
        assert!(remote["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|observed| observed == event));
    }

    let binary = &report["reports"]["binary"];
    assert_eq!(binary["passed"], true);
    assert_eq!(binary["non_utf8_observed"], true);
    assert_eq!(binary["media_type"], "application/octet-stream");
    assert_eq!(binary["byte_length"], 65_536);
    assert_eq!(
        binary["artifact_id"],
        format!(
            "artifact:sha256-{}",
            binary["content_hash"].as_str().unwrap()
        )
    );

    let scale = &report["reports"]["scale"];
    assert_eq!(scale["node_count"], 512);
    assert_eq!(scale["edge_count"], 511);
    assert_eq!(scale["retry_count"], 128);
    assert_eq!(scale["report_count"], 640);
    assert_eq!(scale["complete"], true);
    assert!(
        scale["reconciliation_ms"].as_u64().unwrap()
            <= scale["thresholds"]["reconciliation_ms"].as_u64().unwrap()
    );
    assert!(
        scale["peak_memory_bytes"].as_u64().unwrap()
            <= scale["thresholds"]["peak_memory_bytes"].as_u64().unwrap()
    );

    let allocator = &report["reports"]["allocator"];
    assert_eq!(allocator["passed"], true);
    assert!(allocator["journal_event_count"].as_u64().unwrap() >= 512);
    assert_eq!(allocator["concurrent_grant_count"], 1);
    assert_eq!(allocator["restart_observed"], true);
    assert_eq!(allocator["crash_before_publication_ignored"], true);
    assert_eq!(allocator["crash_after_publication_refused"], true);
    assert_eq!(allocator["release_observed"], true);
    assert_eq!(allocator["supersede_active_successor"], true);
    assert_eq!(allocator["checkpoint_compaction"]["implemented"], false);
    assert!(
        allocator["append_elapsed_ms"].as_u64().unwrap()
            <= allocator["append_threshold_ms"].as_u64().unwrap()
    );

    let reviewed = &report["reports"]["reviewed_resource"];
    assert_eq!(reviewed["passed"], true);
    assert_eq!(reviewed["accepted"], false);
    assert_eq!(
        reviewed["reviewed_deployment_hash"],
        report["reviewed_deployment_hash"]
    );
    assert_eq!(reviewed["reconciliation_complete"], true);

    let manifest: Value = serde_json::from_slice(
        &fs::read(output.join("retained-evidence.manifest.json"))
            .expect("retained manifest exists"),
    )
    .unwrap();
    assert_eq!(manifest["accepted"], false);
    for entry in manifest["files"].as_array().unwrap() {
        let bytes = fs::read(output.join(entry["path"].as_str().unwrap())).unwrap();
        assert_eq!(
            entry["content_hash"],
            format!("sha256:{:x}", Sha256::digest(bytes))
        );
    }
}

#[test]
fn checked_in_edge_proof_pilot_is_current_not_historical_node_only_evidence() {
    let report: Value = serde_json::from_slice(
        &fs::read("docs/pilots/issue-76/pilot-report.json").expect("issue 76 report exists"),
    )
    .unwrap();
    let completeness = &report["scenarios"]["fanout_reduce_complete"]["completeness"];
    assert_eq!(completeness["node_complete"], true);
    assert_eq!(completeness["dataflow_complete"], true);
    assert_eq!(completeness["complete"], true);
    assert_eq!(completeness["expected_edge_count"], 2);
    assert_eq!(completeness["proven_edge_count"], 2);
    assert_eq!(completeness["edge_proofs"].as_array().unwrap().len(), 2);

    let promotion: Value = serde_json::from_slice(
        &fs::read("docs/pilots/issue-85/promotion-report.json")
            .expect("durability promotion report exists"),
    )
    .unwrap();
    assert_eq!(promotion["workflow_count"], 10);
    assert_eq!(promotion["promotion_recommended"], false);
    assert!(promotion["blockers"][0].as_str().unwrap().contains("#76"));
}
