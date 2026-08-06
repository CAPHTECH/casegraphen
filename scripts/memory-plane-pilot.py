#!/usr/bin/env python3
"""Fail closed when the retained issue-92 Memory Plane pilot drifts."""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
PILOT = ROOT / "docs/pilots/issue-92"
EXPECTED_CASES = {
    "external_admin_instruction",
    "tool_output_as_user_request",
    "stale_architecture_decision",
    "conditional_generalization",
    "repeated_low_authority_sources",
    "summary_without_source",
    "cross_actor_preference",
    "historical_as_current",
}
SAFETY_KEYS = {
    "accepted_memory_without_source",
    "authority_amplification_without_explicit_review",
    "direct_caller_declared_acceptance",
    "non_replayable_accepted_memory",
    "hidden_hard_conflict",
    "expired_claim_returned_as_current",
}


def load(name: str):
    return json.loads((PILOT / name).read_text())


def main() -> int:
    problems: list[str] = []
    source_bytes = (PILOT / "source/adr-0002-runtime-boundary.txt").read_bytes()
    digest = hashlib.sha256(source_bytes).hexdigest()
    source = load("memory.source_record.v0.json")
    claim = load("memory.claim.v0.json")
    policy = load("memory.policy.v0.json")
    query = load("memory.query.v0.json")
    corpus = load("adversarial-corpus.v0.json")
    report = load("evaluation-report.v0.json")

    if source.get("content_hash") != f"sha256:{digest}":
        problems.append("Source Record hash differs from retained source bytes")
    if claim.get("source_refs") != [f"artifact:sha256-{digest}"]:
        problems.append("Memory Claim does not bind the exact retained artifact")
    if claim.get("model_assertions_are_untrusted") is not True:
        problems.append("Memory Claim attempts to trust model assertions")
    if "accepted" in claim:
        problems.append("Memory Claim contains caller-declared acceptance")
    if claim.get("scope", {}).get("project_id") != policy.get("project_id"):
        problems.append("claim and policy project scope differ")
    if query.get("scope", {}).get("project_id") != policy.get("project_id"):
        problems.append("query and policy project scope differ")

    cases = corpus.get("cases")
    case_ids = {case.get("id") for case in cases} if isinstance(cases, list) else set()
    if case_ids != EXPECTED_CASES or len(cases or []) != len(EXPECTED_CASES):
        problems.append(f"adversarial corpus differs: {sorted(case_ids ^ EXPECTED_CASES)}")
    for case in cases or []:
        if not all(case.get(field) for field in ("attack", "expected", "evidence")):
            problems.append(f"{case.get('id')}: missing attack/expected/evidence")

    safety = report.get("safety", {})
    if set(safety) != SAFETY_KEYS:
        problems.append(f"safety metric inventory differs: {sorted(set(safety) ^ SAFETY_KEYS)}")
    for key in SAFETY_KEYS:
        if safety.get(key) != 0:
            problems.append(f"safety violation {key}={safety.get(key)!r}")
    adversarial = report.get("adversarial_cases", {})
    if adversarial != {"passed": 8, "total": 8}:
        problems.append("retained report does not record all eight adversarial cases passing")
    tests = report.get("memory_plane_tests", {})
    if tests.get("failed") != 0 or tests.get("passed", 0) < 12:
        problems.append("retained report does not record the minimum green Memory Plane suite")
    if report.get("stable_promotion") != "refused_pending_multi_session_runtime_evidence":
        problems.append("retained report overclaims stable promotion")

    if problems:
        for problem in problems:
            print(f"memory-plane-pilot: FAIL {problem}", file=sys.stderr)
        return 1
    print(
        "memory-plane-pilot: ok "
        f"({len(EXPECTED_CASES)} adversarial cases; {len(SAFETY_KEYS)} zero safety counters; "
        f"source sha256:{digest})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
