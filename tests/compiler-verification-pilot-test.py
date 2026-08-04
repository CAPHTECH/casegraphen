#!/usr/bin/env python3
"""Negative conformance tests for the compiler performance report verifier."""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
VERIFIER = ROOT / "scripts/compiler-verification-pilot.py"
CASES = (
    ("small", 4, 1_000, 96 * 1024),
    ("medium", 128, 2_000, 192 * 1024),
    ("large", 512, 8_000, 384 * 1024),
)


def content_hash(value: dict) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()
    return hashlib.sha256(encoded).hexdigest()


def file_hash(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def report() -> dict:
    value = {
        "schema": "casegraphen.experimental.graph_compiler.verification_performance_report.v0",
        "observed_compiler_binary_sha256": "a" * 64,
        "benchmark_source_sha256": file_hash(
            ROOT / "examples/compiler-verification-benchmark.rs"
        ),
        "compiler_source_sha256": file_hash(ROOT / "src/graph_compiler.rs"),
        "profile": "debug-bounded-pilot",
        "cases": [
            {
                "name": name,
                "node_count": nodes,
                "topology_edge_count": nodes // 2,
                "policy_document_count": nodes * 3,
                "artifact_count": 12,
                "verified_input_bytes": 1024,
                "canonical_verification_elapsed_micros": 1,
                "wall_elapsed_ms": 1.0,
                "peak_rss_kb": 1024,
                "recompile_count": 1,
                "latency_budget_ms": latency,
                "memory_budget_kb": memory,
                "passed": True,
            }
            for name, nodes, latency, memory in CASES
        ],
        "promotion_gate_passed": True,
        "authority": "none; test fixture grants no deployment authority",
    }
    value["report_content_hash"] = content_hash(value)
    return value


def verify(path: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(VERIFIER), "--verify-report", str(path)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="casegraphen-compiler-pilot-test-") as directory:
        path = pathlib.Path(directory) / "report.json"
        valid = report()
        path.write_text(json.dumps(valid, sort_keys=True))
        result = verify(path)
        if result.returncode != 0:
            raise SystemExit(f"valid compiler pilot fixture failed: {result.stderr}")

        forged = report()
        forged["cases"][0]["wall_elapsed_ms"] = 1_001.0
        forged["report_content_hash"] = content_hash(
            {key: value for key, value in forged.items() if key != "report_content_hash"}
        )
        path.write_text(json.dumps(forged, sort_keys=True))
        result = verify(path)
        if result.returncode == 0 or "result was not derived" not in result.stderr:
            raise SystemExit("rehashed over-budget compiler report was not rejected")

        forged = report()
        forged["cases"][1]["policy_document_count"] = 0
        forged["report_content_hash"] = content_hash(
            {key: value for key, value in forged.items() if key != "report_content_hash"}
        )
        path.write_text(json.dumps(forged, sort_keys=True))
        if verify(path).returncode == 0:
            raise SystemExit("rehashed workload substitution was not rejected")

        forged = report()
        forged["compiler_source_sha256"] = "b" * 64
        forged["report_content_hash"] = content_hash(
            {key: value for key, value in forged.items() if key != "report_content_hash"}
        )
        path.write_text(json.dumps(forged, sort_keys=True))
        if verify(path).returncode == 0:
            raise SystemExit("rehashed compiler-source substitution was not rejected")

    print("compiler verification pilot negative conformance: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
