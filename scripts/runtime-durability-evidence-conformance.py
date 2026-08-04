#!/usr/bin/env python3
"""Conformance gate for bounded Git and retained runtime evidence."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
PILOT = ROOT / "docs/pilots/issue-85"
RETAINED_RECORD = ROOT / "docs/pilots/issue-89/retained-release-record.json"
MAX_LEGACY_EVIDENCE_BYTES = 2_200_000
MAX_LEGACY_FILE_BYTES = 1_000_000
EXPECTED_DIRECTORY = {
    "README.md",
    "release-evidence.json",
    "retained-evidence.manifest.json",
    "durability-report.json",
    "promotion-report.json",
    "canonical-binary-artifact.bin",
    "canonical-canonical-runtime-report.json",
    "canonical-execution.topology.json",
    "canonical-runtime.completeness.json",
    "canonical-runtime.expectation.json",
    "canonical-runtime.reports.json",
    "allocator-durability-report.json",
    "reviewed-resource-report.json",
    "remote.journal.jsonl",
}


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def main() -> int:
    failures: list[str] = []
    actual = {path.name for path in PILOT.iterdir() if path.is_file()}
    if actual != EXPECTED_DIRECTORY:
        failures.append(
            f"issue-85 checked-in inventory changed: missing={sorted(EXPECTED_DIRECTORY-actual)} "
            f"unexpected={sorted(actual-EXPECTED_DIRECTORY)}"
        )
    pointer = json.loads((PILOT / "release-evidence.json").read_text())
    manifest_path = PILOT / "retained-evidence.manifest.json"
    manifest_bytes = manifest_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    baseline = pointer.get("legacy_baseline", {})
    if pointer.get("retention_state") != "last_checked_in_baseline":
        failures.append("issue-85 legacy evidence must be explicitly marked last_checked_in_baseline")
    if pointer.get("accepted") is not False or pointer.get("promotion_recommended") is not False:
        failures.append("issue-85 retained evidence must not claim acceptance or promotion")
    if baseline.get("manifest_content_hash") != digest(manifest_bytes):
        failures.append("issue-85 legacy manifest hash does not match release pointer")
    if baseline.get("manifest_byte_length") != len(manifest_bytes):
        failures.append("issue-85 legacy manifest length does not match release pointer")
    files = manifest.get("files", [])
    total = 0
    for item in files:
        path = PILOT / str(item.get("path", ""))
        if not path.is_file() or path.is_symlink():
            failures.append(f"issue-85 legacy evidence is missing or unsafe: {item.get('path')}")
            continue
        data = path.read_bytes()
        total += len(data)
        if item.get("content_hash") != digest(data) or item.get("byte_length") != len(data):
            failures.append(f"issue-85 legacy evidence mismatch: {item.get('path')}")
        if len(data) > MAX_LEGACY_FILE_BYTES:
            failures.append(f"issue-85 legacy file exceeds repository budget: {item.get('path')}")
    if total != baseline.get("evidence_byte_length") or len(files) != baseline.get("evidence_file_count"):
        failures.append("issue-85 legacy evidence count/size does not match release pointer")
    if total > MAX_LEGACY_EVIDENCE_BYTES:
        failures.append("issue-85 legacy evidence exceeds checked-in repository budget")
    future = pointer.get("future_evidence_contract", {})
    if future.get("content_addressed_release_required") is not True or future.get(
        "additional_generated_evidence_must_not_be_committed"
    ) is not True:
        failures.append("future runtime evidence must be Release-only")
    latest = pointer.get("latest_retained_release", {})
    retained_bytes = RETAINED_RECORD.read_bytes() if RETAINED_RECORD.is_file() else b""
    if latest.get("record_path") != "../issue-89/retained-release-record.json":
        failures.append("latest retained runtime evidence path is not canonical")
    if latest.get("record_content_hash") != digest(retained_bytes):
        failures.append("latest retained runtime evidence record hash mismatch")
    if latest.get("record_byte_length") != len(retained_bytes):
        failures.append("latest retained runtime evidence record length mismatch")
    try:
        retained = json.loads(retained_bytes)
    except (ValueError, json.JSONDecodeError):
        retained = {}
        failures.append("latest retained runtime evidence record is invalid JSON")
    release = retained.get("release", {})
    provenance = retained.get("provenance", {})
    evidence = retained.get("evidence", {})
    package_hash = release.get("package_sha256")
    bare_hash = str(package_hash).removeprefix("sha256:")
    if (
        retained.get("schema")
        != "casegraphen.experimental.runtime_durability.retention_record.v1"
        or retained.get("schema_version") != 1
        or retained.get("retention_state") != "retained_release"
        or retained.get("accepted") is not False
        or retained.get("promotion_recommended") is not False
        or evidence.get("all_thresholds_passed") is not True
    ):
        failures.append("latest retained runtime evidence authority boundary is invalid")
    if (
        re.fullmatch(r"sha256:[0-9a-f]{64}", str(package_hash)) is None
        or release.get("tag") != f"runtime-durability-evidence-{bare_hash}"
        or release.get("asset_name") != f"sha256-{bare_hash}.tar.gz"
    ):
        failures.append("latest retained runtime evidence is not content-addressed")
    for field in ("evaluated_commit_sha", "workflow_run_id", "workflow_run_attempt"):
        if latest.get(field) != provenance.get(field):
            failures.append(f"latest retained runtime evidence provenance mismatch: {field}")
    if latest.get("release_tag") != release.get("tag") or latest.get(
        "package_sha256"
    ) != package_hash:
        failures.append("latest retained runtime evidence Release identity mismatch")
    if latest.get("offline_verified") is not True:
        failures.append("latest retained runtime evidence lacks offline verification")
    workflow = (ROOT / ".github/workflows/runtime-durability-evidence.yml").read_text()
    required_workflow = [
        "fresh-agent-run-provenance.py inspect-release",
        "fresh-agent-run-provenance.py verify-file",
        "runtime-durability-evidence.py verify-offline",
        "gh release upload",
        "runtime-durability-evidence-publisher",
    ]
    for required in required_workflow:
        if required not in workflow:
            failures.append(f"runtime evidence workflow omits shared publication rule: {required}")
    readme = (PILOT / "README.md").read_text()
    for required in ("last checked-in baseline", "verify-offline", "2,200,000"):
        if required not in readme:
            failures.append(f"issue-85 README omits retention contract: {required}")
    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        return 1
    print(
        f"runtime durability evidence conforms: legacy={total} bytes, "
        "future evidence is content-addressed Release-only"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
