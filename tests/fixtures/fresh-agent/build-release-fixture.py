#!/usr/bin/env python3
"""Build deterministic synthetic provider evidence for aggregator tests."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def digest(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--mode",
        choices=["pass", "provider_unavailable", "missing", "timeout", "unobservable_cost"],
        default="pass",
    )
    args = parser.parse_args()
    manifest = json.loads((ROOT / "evals/fresh-agent/scenarios.v0.json").read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    run_hashes = {}
    for provider in ("codex", "claude"):
        run = args.output / provider
        run.mkdir()
        results = []
        scenarios = list(manifest["scenarios"])
        if args.mode == "missing" and provider == "codex":
            scenarios = scenarios[:-1]
        for index, scenario in enumerate(scenarios):
            evaluations = [
                {"kind": evaluator["kind"], "status": "pass", "detail": []}
                for evaluator in scenario["deterministic_evaluators"]
            ]
            evaluations.append(
                {"kind": "manual_judgment", "status": "manual_required", "detail": scenario["manual_judgments"][0]}
            )
            result = {
                "scenario_id": scenario["id"],
                "provider": {"provider": provider},
                "returncode": 0,
                "timed_out": args.mode == "timeout" and provider == "claude" and index == 0,
                "workspace_retained": True,
                "credential_material_scan": {"status": "pass", "affected_file_count": 0},
                "evaluations": evaluations,
            }
            destination = run / scenario["id"]
            destination.mkdir()
            (destination / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
            (destination / "raw.stdout").write_text(f"{provider}:{scenario['id']}\n")
            results.append(result)
        summary = {
            "schema": "casegraphen.eval.fresh_agent_run.v0",
            "status": "provider_unavailable" if args.mode == "provider_unavailable" and provider == "codex" else "completed",
            "provider": {
                "provider": provider,
                "available": args.mode != "provider_unavailable" or provider != "codex",
                "version_matches": True,
                "package_identity": f"fixture:{provider}@1",
                "expected_version": "1"
            },
            "manifest_hash": digest((ROOT / "evals/fresh-agent/scenarios.v0.json").read_bytes()),
            "budget": {"maximum_usd": 25.0, "observed_usd": 1.0,
                       "observable": args.mode != "unobservable_cost",
                       "per_scenario_timeout_seconds": 900},
            "results": [] if args.mode == "provider_unavailable" and provider == "codex" else results,
        }
        summary["content_hash"] = digest(canonical(summary))
        (run / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
        run_hashes[provider] = summary["content_hash"]
    judgments = [
        {"provider": provider, "scenario_id": scenario["id"], "outcome": "pass",
         "reviewer": "reviewer:fixture", "reason": "deterministic release fixture review"}
        for provider in ("codex", "claude") for scenario in manifest["scenarios"]
    ]
    manual = {"schema": "casegraphen.eval.fresh_agent_manual_review.v0",
              "run_content_hashes": run_hashes, "judgments": judgments}
    if args.mode == "unobservable_cost":
        manual["cost_waivers"] = [
            {"provider": provider, "reviewer": "reviewer:fixture",
             "reason": "provider does not expose cost telemetry in this bound run",
             "maximum_usd": 25.0}
            for provider in ("codex", "claude")
        ]
    (args.output / "manual-review.json").write_text(json.dumps(manual, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
