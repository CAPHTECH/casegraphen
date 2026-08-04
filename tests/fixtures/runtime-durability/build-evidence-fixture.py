#!/usr/bin/env python3
"""Create a bounded synthetic runtime-durability evidence directory."""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys


REVISION = "a" * 40
TOPOLOGY_HASH = "b" * 64
DEPLOYMENT_HASH = "c" * 64
ROLES = {
    "durability-report.json": "aggregate_report",
    "promotion-report.json": "promotion_report",
    "canonical-binary-artifact.bin": "binary_artifact",
    "canonical-canonical-runtime-report.json": "runtime_pilot_report",
    "canonical-execution.topology.json": "execution_topology",
    "canonical-runtime.completeness.json": "runtime_completeness",
    "canonical-runtime.expectation.json": "runtime_expectation",
    "canonical-runtime.reports.json": "runtime_node_reports",
    "allocator-durability-report.json": "allocator_report",
    "reviewed-resource-report.json": "reviewed_resource_report",
    "remote.journal.jsonl": "remote_journal",
}


def encode(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()


def main() -> int:
    output = pathlib.Path(sys.argv[1])
    output.mkdir(parents=True)
    reports = {
        "remote": {"passed": True, "threshold_ms": 10_000, "elapsed_ms": 1},
        "binary": {"passed": True, "byte_length": 4},
        "scale": {
            "passed": True,
            "node_count": 2,
            "edge_count": 1,
            "retry_count": 1,
            "reconciliation_ms": 1,
            "peak_memory_bytes": 1024,
            "thresholds": {"reconciliation_ms": 5_000, "peak_memory_bytes": 134_217_728},
        },
        "allocator": {
            "passed": True,
            "journal_event_count": 2,
            "append_elapsed_ms": 1,
            "append_threshold_ms": 5_000,
            "replay_elapsed_ms": 1,
            "replay_threshold_ms": 5_000,
        },
        "reviewed_resource": {"passed": True},
    }
    values: dict[str, bytes] = {
        "durability-report.json": encode({
            "schema": "casegraphen.experimental.runtime_durability_pilot.report.v0",
            "source_revision": REVISION,
            "source_worktree_dirty": False,
            "accepted": False,
            "promotion_eligible": False,
            "all_thresholds_passed": True,
            "topology_content_hash": TOPOLOGY_HASH,
            "reviewed_deployment_hash": DEPLOYMENT_HASH,
            "harness_content_hash": "d" * 64,
            "contract_content_hashes": {"fixture.schema.json": "e" * 64},
            "runtime_versions": {"python": "fixture", "platform": "bounded-ci"},
            "reports": reports,
        }),
        "promotion-report.json": encode({
            "accepted": False,
            "promotion_recommended": False,
            "durability_thresholds_passed": True,
        }),
        "canonical-binary-artifact.bin": b"\x00\xff\x01\xfe",
        "canonical-canonical-runtime-report.json": encode({"passed": True}),
        "canonical-execution.topology.json": encode({"topology_content_hash": TOPOLOGY_HASH}),
        "canonical-runtime.completeness.json": encode({"complete": True}),
        "canonical-runtime.expectation.json": encode({"edge_count": 1}),
        "canonical-runtime.reports.json": encode({"report_count": 3}),
        "allocator-durability-report.json": encode(reports["allocator"]),
        "reviewed-resource-report.json": encode({
            "passed": True, "reviewed_deployment_hash": DEPLOYMENT_HASH,
        }),
        "remote.journal.jsonl": b'{"accepted":false,"sequence":1}\n',
    }
    files = []
    for name, data in sorted(values.items()):
        (output / name).write_bytes(data)
        files.append({
            "path": name,
            "role": ROLES[name],
            "content_hash": "sha256:" + hashlib.sha256(data).hexdigest(),
            "byte_length": len(data),
        })
    (output / "retained-evidence.manifest.json").write_bytes(encode({
        "schema": "casegraphen.experimental.runtime_durability_pilot.evidence_manifest.v1",
        "schema_version": 1,
        "accepted": False,
        "files": files,
    }))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
