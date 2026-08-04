#!/usr/bin/env python3
"""Measure semantic bundle verification without turning timing into authority."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import resource
import subprocess
import sys
import time
from typing import Any

import jsonschema


SCHEMA = "casegraphen.experimental.graph_compiler.verification_performance_report.v0"
DEFAULT_CASES = (
    ("small", 4, 1_000, 96 * 1024),
    ("medium", 128, 2_000, 192 * 1024),
    ("large", 512, 8_000, 384 * 1024),
)
EXPECTED_POLICY_DOCUMENTS_PER_NODE = 3
EXPECTED_ARTIFACT_COUNT = 12


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(),
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=lambda item: (_ for _ in ()).throw(ValueError(item)),
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid compiler verification report: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit("compiler verification report must be an object")
    return value


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def measure(binary: pathlib.Path, nodes: int) -> dict:
    started = time.monotonic_ns()
    completed = subprocess.run(
        [str(binary), str(nodes)],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
    )
    elapsed_ms = (time.monotonic_ns() - started) / 1_000_000
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    peak_rss_kb = int(usage.ru_maxrss / 1024) if sys.platform == "darwin" else int(usage.ru_maxrss)
    if completed.returncode != 0:
        raise SystemExit(completed.stderr or f"benchmark exited {completed.returncode}")
    metrics = json.loads(completed.stdout)
    return {"wall_elapsed_ms": elapsed_ms, "peak_rss_kb": peak_rss_kb, "metrics": metrics}


def canonical_hash(value: dict[str, Any]) -> str:
    encoded = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    ).encode()
    return hashlib.sha256(encoded).hexdigest()


def case_passed(
    case: dict[str, Any],
    expected_nodes: int,
    expected_latency_ms: int,
    expected_memory_kb: int,
) -> bool:
    return (
        case.get("node_count") == expected_nodes
        and case.get("topology_edge_count") == expected_nodes // 2
        and case.get("policy_document_count")
        == expected_nodes * EXPECTED_POLICY_DOCUMENTS_PER_NODE
        and case.get("artifact_count") == EXPECTED_ARTIFACT_COUNT
        and case.get("latency_budget_ms") == expected_latency_ms
        and case.get("memory_budget_kb") == expected_memory_kb
        and case.get("recompile_count") == 1
        and isinstance(case.get("wall_elapsed_ms"), (int, float))
        and not isinstance(case.get("wall_elapsed_ms"), bool)
        and case["wall_elapsed_ms"] <= expected_latency_ms
        and isinstance(case.get("peak_rss_kb"), int)
        and not isinstance(case.get("peak_rss_kb"), bool)
        and case["peak_rss_kb"] <= expected_memory_kb
    )


def verify_report(path: pathlib.Path) -> str:
    report = load_json(path)
    root = pathlib.Path(__file__).resolve().parents[1]
    schema_path = (
        pathlib.Path(__file__).resolve().parents[1]
        / "schemas/experimental/compiler.verification_performance_report.v0.schema.json"
    )
    jsonschema.validate(report, load_json(schema_path))
    claimed_hash = report.get("report_content_hash")
    hash_input = dict(report)
    hash_input.pop("report_content_hash", None)
    if report.get("schema") != SCHEMA or claimed_hash != canonical_hash(hash_input):
        raise SystemExit("compiler verification report content hash or schema mismatch")
    for field, source in (
        ("benchmark_source_sha256", root / "examples/compiler-verification-benchmark.rs"),
        ("compiler_source_sha256", root / "src/graph_compiler.rs"),
    ):
        if report.get(field) != sha256_file(source):
            raise SystemExit(f"compiler verification report {field} is stale or substituted")
    cases = report.get("cases")
    if not isinstance(cases, list) or len(cases) != len(DEFAULT_CASES):
        raise SystemExit("compiler verification report cases are incomplete")
    derived: list[bool] = []
    for case, (name, nodes, latency_ms, memory_kb) in zip(cases, DEFAULT_CASES):
        if not isinstance(case, dict) or case.get("name") != name:
            raise SystemExit("compiler verification report case order or identity mismatch")
        passed = case_passed(case, nodes, latency_ms, memory_kb)
        if case.get("passed") is not passed:
            raise SystemExit(f"compiler verification case result was not derived: {name}")
        derived.append(passed)
    overall = all(derived)
    if report.get("promotion_gate_passed") is not overall:
        raise SystemExit("compiler verification overall result was not derived from cases")
    if not overall:
        raise SystemExit("compiler verification report exceeded a retained budget")
    return str(claimed_hash)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--measure", nargs=2, metavar=("BINARY", "NODES"))
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--verify-report", type=pathlib.Path)
    args = parser.parse_args()
    if args.measure:
        print(json.dumps(measure(pathlib.Path(args.measure[0]), int(args.measure[1])), sort_keys=True))
        return 0
    if args.verify_report:
        claimed_hash = verify_report(args.verify_report)
        print(f"ok: verified compiler performance report {claimed_hash}")
        return 0

    root = pathlib.Path(__file__).resolve().parents[1]
    binary = root / "target/debug/examples/compiler-verification-benchmark"
    if not args.skip_build:
        subprocess.run(
            ["cargo", "build", "--quiet", "--example", "compiler-verification-benchmark"],
            cwd=root,
            check=True,
        )
    # Exclude one-time process loader and dynamic-linker startup from the
    # bounded verifier cases. The warm-up still executes the current benchmark
    # binary and fails the run if the representative workload cannot compile.
    subprocess.run(
        [str(binary), "1"],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    cases = []
    overall_pass = True
    for name, nodes, latency_budget_ms, memory_budget_kb in DEFAULT_CASES:
        completed = subprocess.run(
            [sys.executable, __file__, "--measure", str(binary), str(nodes)],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        )
        observation = json.loads(completed.stdout)
        metrics = observation["metrics"]
        passed = (
            observation["wall_elapsed_ms"] <= latency_budget_ms
            and observation["peak_rss_kb"] <= memory_budget_kb
            and metrics["recompile_count"] == 1
            and metrics["topology_node_count"] == nodes
            and metrics["policy_document_count"]
            == nodes * EXPECTED_POLICY_DOCUMENTS_PER_NODE
        )
        overall_pass = overall_pass and passed
        cases.append(
            {
                "name": name,
                "node_count": nodes,
                "topology_edge_count": metrics["topology_edge_count"],
                "policy_document_count": metrics["policy_document_count"],
                "artifact_count": metrics["artifact_count"],
                "verified_input_bytes": metrics["verified_input_bytes"],
                "canonical_verification_elapsed_micros": metrics["elapsed_micros"],
                "wall_elapsed_ms": observation["wall_elapsed_ms"],
                "peak_rss_kb": observation["peak_rss_kb"],
                "recompile_count": metrics["recompile_count"],
                "latency_budget_ms": latency_budget_ms,
                "memory_budget_kb": memory_budget_kb,
                "passed": passed,
            }
        )
    report = {
        "schema": SCHEMA,
        "observed_compiler_binary_sha256": sha256_file(binary),
        "benchmark_source_sha256": sha256_file(
            root / "examples/compiler-verification-benchmark.rs"
        ),
        "compiler_source_sha256": sha256_file(root / "src/graph_compiler.rs"),
        "profile": "debug-bounded-pilot",
        "cases": cases,
        "promotion_gate_passed": overall_pass,
        "authority": "none; timing and memory observations cannot grant deployment authority",
    }
    report["report_content_hash"] = canonical_hash(report)
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded)
    else:
        print(encoded, end="")
    return 0 if overall_pass else 1


if __name__ == "__main__":
    raise SystemExit(main())
