#!/usr/bin/env python3
"""Build deterministic synthetic provider evidence for aggregator tests."""

from __future__ import annotations

import argparse
import hashlib
import hmac
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
        choices=[
            "pass",
            "provider_unavailable",
            "cli_session_unavailable",
            "missing",
            "timeout",
            "unobservable_cost",
            "model_mismatch",
        ],
        default="pass",
    )
    args = parser.parse_args()
    manifest = json.loads((ROOT / "evals/fresh-agent/scenarios.v0.json").read_text())
    policy = json.loads((ROOT / "evals/fresh-agent/release-policy.v0.json").read_text())
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
                "model": f"{provider}-fixture-model",
                "declared_package_identity": policy["runner_pins"][provider]["package_identity"],
                "expected_version": policy["runner_pins"][provider]["version"],
            "authentication": {
                    "mode": "cli_session",
                    "available": args.mode != "cli_session_unavailable" or provider != "codex",
                    "classification": "codex_chatgpt_session" if provider == "codex" else "claude_subscription_session",
                    "non_api_key_session_verified": args.mode != "cli_session_unavailable" or provider != "codex",
                    "status_exit_code": 1 if args.mode == "cli_session_unavailable" and provider == "codex" else 0,
                    "probe_command_hash": "sha256:" + "0" * 64,
                    "probe_output_retained": False
                }
            },
            "manifest_hash": digest((ROOT / "evals/fresh-agent/scenarios.v0.json").read_bytes()),
            "budget": {"maximum_usd": 25.0, "observed_usd": 1.0,
                       "observable": args.mode != "unobservable_cost",
                       "per_scenario_timeout_seconds": 900},
            "model_observation": {
                "observable": args.mode == "model_mismatch" and provider == "claude",
                "reported_models": ["substituted-model"]
                if args.mode == "model_mismatch" and provider == "claude"
                else [],
                "matches_declared": False,
            },
            "results": [] if args.mode == "provider_unavailable" and provider == "codex" else results,
            "host_attestation_challenge": hashlib.sha256(f"fixture:{provider}".encode()).hexdigest(),
        }
        summary["content_hash"] = digest(canonical(summary))
        (run / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
        run_hashes[provider] = summary["content_hash"]
        key = (f"fixture-host-key:{provider}:".encode() * 4)[:64]
        key_path = args.output / f"{provider}-host-attestation.key"
        key_path.write_bytes(key)
        attestation = {
            "schema": "casegraphen.eval.cli_session_host_attestation.v0",
            "provider": provider,
            "run_content_hash": summary["content_hash"],
            "host_attestation_challenge": summary["host_attestation_challenge"],
            "authentication_classification": summary["provider"]["authentication"]["classification"],
            "credential_boundary": "dedicated_provider_os_account_with_brokered_session",
            "agent_credential_read_access": False,
            "runner_instance_id_hash": "sha256:" + hashlib.sha256(f"runner:{provider}".encode()).hexdigest(),
            "key_id": policy["runner_pins"][provider]["host_attestation_key_id"],
        }
        attestation["hmac_sha256"] = hmac.new(key, canonical(attestation), hashlib.sha256).hexdigest()
        (args.output / f"{provider}-host-attestation.json").write_text(
            json.dumps(attestation, indent=2, sort_keys=True) + "\n"
        )
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
