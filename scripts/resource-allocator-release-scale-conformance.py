#!/usr/bin/env python3
"""Fail closed when retained Issue #88 scale evidence is absent or stale."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PILOT = ROOT / "docs/pilots/issue-88"
TARGETS = (512, 10_000, 100_000)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"resource-allocator-release-scale: {message}")


def main() -> int:
    for target in TARGETS:
        path = PILOT / f"resource-allocator-{target}.report.json"
        require(path.is_file(), f"missing {path.relative_to(ROOT)}")
        report = json.loads(path.read_text(encoding="utf-8"))
        prefix = f"{target}:"
        require(report.get("passed") is True, f"{prefix} report did not pass")
        require(report.get("accepted") is False, f"{prefix} report must not accept output")
        require(report.get("event_threshold") == target, f"{prefix} target mismatch")
        require(report.get("journal_event_count") == target, f"{prefix} event count mismatch")
        require(
            report["append_elapsed_ms"] <= report["append_threshold_ms"],
            f"{prefix} total append threshold exceeded",
        )
        require(
            report["append_pair_latency_ms"]["p95"]
            <= report["append_pair_p95_threshold_ms"],
            f"{prefix} append p95 threshold exceeded",
        )
        require(
            report["restart_replay_ms"] <= report["restart_replay_threshold_ms"],
            f"{prefix} restart threshold exceeded",
        )
        checkpoint = report["checkpoint_compaction"]
        require(checkpoint["implemented"] is True, f"{prefix} checkpoint absent")
        require(checkpoint["full_replay_equivalent"] is True, f"{prefix} replay divergence")
        require(
            checkpoint["checkpoint_size_bytes"] <= checkpoint["checkpoint_size_threshold_bytes"],
            f"{prefix} checkpoint size threshold exceeded",
        )
        for metric in (
            "checkpoint_create_ms",
            "checkpoint_independent_verify_ms",
            "compaction_ms",
        ):
            require(
                checkpoint[metric] <= checkpoint["checkpoint_operation_threshold_ms"],
                f"{prefix} {metric} threshold exceeded",
            )
        require(report["concurrent_grant_count"] == 1, f"{prefix} exclusivity diverged")
        workloads = report.get("workloads", {})
        require(workloads.get("passed") is True, f"{prefix} active/mixed workload failed")
        require(
            workloads.get("bounded_operation_snapshot") is True,
            f"{prefix} operation snapshot is not bounded",
        )
        require(
            workloads.get("all_active", {}).get("observed_active_count")
            == workloads.get("all_active", {}).get("reservation_count"),
            f"{prefix} all-active cardinality diverged",
        )
        require(
            workloads.get("mixed_churn", {}).get("observed_active_count")
            == workloads.get("all_active", {}).get("reservation_count"),
            f"{prefix} mixed-churn cardinality diverged",
        )
        require(report["release_observed"] is True, f"{prefix} release absent")
        require(report["supersede_active_successor"] is True, f"{prefix} supersede diverged")
        require(
            report["crash_before_publication_ignored"] is True
            and report["crash_after_publication_refused"] is True,
            f"{prefix} crash boundary diverged",
        )
        if target >= 10_000:
            envelope = report.get("release_scale_envelope", {})
            require(envelope.get("real_public_allocator_api") is True, f"{prefix} API bypass")
            require(
                envelope.get("long_lived_allocator_instance") is True,
                f"{prefix} allocator instance is not long-lived",
            )
            require(report["observed_peak_rss_bytes"] > 0, f"{prefix} RSS absent")
            require(
                len(envelope.get("host_identity_sha256", "")) == 64,
                f"{prefix} host identity is absent",
            )
            require(envelope.get("rustc_verbose"), f"{prefix} rustc metadata is absent")
            require(
                envelope.get("promotion_authority") is False,
                f"{prefix} unattested local evidence claimed promotion authority",
            )
            require(
                report["observed_peak_rss_bytes"]
                <= report["observed_peak_rss_threshold_bytes"],
                f"{prefix} RSS threshold exceeded",
            )
            require(
                envelope.get("harness_sha256")
                == digest(ROOT / "scripts/resource-allocator-release-scale-pilot.py"),
                f"{prefix} harness source changed; regenerate evidence",
            )
            require(
                envelope.get("allocator_source_sha256")
                == digest(ROOT / "src/resource_allocator.rs"),
                f"{prefix} allocator source changed; regenerate evidence",
            )
            require(
                envelope.get("pilot_source_sha256")
                == digest(ROOT / "examples/resource_allocator_durability_pilot.rs"),
                f"{prefix} pilot source changed; regenerate evidence",
            )
    print("resource-allocator-release-scale: ok (512, 10000, 100000)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
