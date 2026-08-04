#!/usr/bin/env python3
"""Conformance gate for bounded Git and retained runtime evidence."""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
PILOT = ROOT / "docs/pilots/issue-85"
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
