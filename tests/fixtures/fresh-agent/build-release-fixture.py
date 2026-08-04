#!/usr/bin/env python3
"""Build deterministic synthetic provider evidence for aggregator tests."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
EVALUATED_COMMIT_SHA = "a" * 40
SOURCE_REPOSITORY = "CAPHTECH/casegraphen"
SOURCE_WORKFLOW = "Fresh Agent Release Evaluation"
SOURCE_RUN_ID = 424242
SOURCE_RUN_ATTEMPT = 1
SOURCE_HEAD_REF = "refs/heads/main"
REVIEWER_IDENTITY = "reviewer:fixture-independent"
REVIEWER_KEY_ID = "fresh-agent-reviewer-fixture-v1"


def canonical(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode()


def digest(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def generate_ed25519(private_key: Path, public_key: Path) -> None:
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(private_key)],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        [
            "openssl",
            "pkey",
            "-in",
            str(private_key),
            "-pubout",
            "-out",
            str(public_key),
        ],
        check=True,
        capture_output=True,
    )


def sign(private_key: Path, payload: dict) -> str:
    with tempfile.TemporaryDirectory(prefix="casegraphen-fixture-sign-") as directory:
        payload_path = Path(directory) / "payload"
        signature_path = Path(directory) / "signature"
        payload_path.write_bytes(canonical(payload))
        subprocess.run(
            [
                "openssl",
                "pkeyutl",
                "-sign",
                "-inkey",
                str(private_key),
                "-rawin",
                "-in",
                str(payload_path),
                "-out",
                str(signature_path),
            ],
            check=True,
            capture_output=True,
        )
        return base64.b64encode(signature_path.read_bytes()).decode("ascii")


def spki_hash(public_key: Path) -> str:
    process = subprocess.run(
        ["openssl", "pkey", "-pubin", "-in", str(public_key), "-outform", "DER"],
        check=True,
        capture_output=True,
    )
    return digest(process.stdout)


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
            "unobservable_cost_low_waiver",
            "unobservable_cost_missing_limit",
            "model_mismatch",
            "stale_provenance",
            "manual_stale_provenance",
        ],
        default="pass",
    )
    args = parser.parse_args()
    manifest = json.loads((ROOT / "evals/fresh-agent/scenarios.v0.json").read_text())
    policy = json.loads((ROOT / "evals/fresh-agent/release-policy.v0.json").read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    run_hashes = {}
    expected_provider_provenance = {}
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
                       "observable": not args.mode.startswith("unobservable_cost"),
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
        private_key = args.output / f"{provider}-host-attestation-private.pem"
        public_key = args.output / f"{provider}-host-attestation-public.pem"
        generate_ed25519(private_key, public_key)
        provenance = {
            "evaluated_commit_sha": EVALUATED_COMMIT_SHA,
            "repository": SOURCE_REPOSITORY,
            "source_workflow": SOURCE_WORKFLOW,
            "source_workflow_id": 7654321,
            "source_workflow_path": ".github/workflows/fresh-agent-release-eval.yml",
            "source_run_id": SOURCE_RUN_ID + (1 if args.mode == "stale_provenance" else 0),
            "source_run_attempt": SOURCE_RUN_ATTEMPT,
            "source_head_ref": SOURCE_HEAD_REF,
            "source_head_sha": EVALUATED_COMMIT_SHA,
            "source_event": "workflow_dispatch",
            "source_conclusion": "success",
            "provider_artifact": {
                "id": 1001 if provider == "codex" else 1002,
                "name": f"fresh-agent-{provider}-{EVALUATED_COMMIT_SHA}",
                "digest": summary["content_hash"],
            },
        }
        expected_provenance = dict(provenance)
        expected_provenance["source_run_id"] = SOURCE_RUN_ID
        (args.output / f"{provider}-expected-provenance.json").write_text(
            json.dumps(expected_provenance, indent=2, sort_keys=True) + "\n"
        )
        expected_provider_provenance[provider] = expected_provenance
        evaluation_host_private_key = args.output / f"{provider}-evaluation-host-private.pem"
        evaluation_host_public_key = args.output / f"{provider}-evaluation-host-public.pem"
        generate_ed25519(evaluation_host_private_key, evaluation_host_public_key)
        (args.output / f"{provider}-evaluation-host-public-spki-sha256.txt").write_text(
            spki_hash(evaluation_host_public_key) + "\n"
        )
        evaluation_host_proof = {
            "schema": "casegraphen.eval.evaluation_host_session_proof.v1",
            "signature_algorithm": "ed25519",
            "signing_key_id": f"{provider}-evaluation-host-fixture-v1",
            "provider": provider,
            "run_content_hash": summary["content_hash"],
            "host_attestation_challenge": summary["host_attestation_challenge"],
            "source": {
                "repository": provenance["repository"],
                "workflow": provenance["source_workflow"],
                "workflow_path": provenance["source_workflow_path"],
                "run_id": provenance["source_run_id"],
                "run_attempt": provenance["source_run_attempt"],
                "head_ref": provenance["source_head_ref"],
                "head_sha": provenance["source_head_sha"],
                "event": provenance["source_event"],
                "provider_artifact": provenance["provider_artifact"],
            },
            "runner_instance_id_hash": "sha256:"
            + hashlib.sha256(f"runner:{provider}".encode()).hexdigest(),
            "authentication_classification": summary["provider"]["authentication"]["classification"],
            "credential_boundary": "dedicated_provider_os_account_with_external_host_attestor",
            "agent_credential_read_access": False,
            "cli_session_verified": True,
        }
        evaluation_host_proof["ed25519_signature"] = sign(
            evaluation_host_private_key, evaluation_host_proof
        )
        (args.output / f"{provider}-evaluation-host-proof.json").write_text(
            json.dumps(evaluation_host_proof, indent=2, sort_keys=True) + "\n"
        )
        attestation = {
            "schema": "casegraphen.eval.cli_session_host_attestation.v1",
            "signature_algorithm": "ed25519",
            "signing_key_id": policy["runner_pins"][provider]["host_attestation_key_id"],
            "provider": provider,
            "run_content_hash": summary["content_hash"],
            "host_attestation_challenge": summary["host_attestation_challenge"],
            "authentication_classification": summary["provider"]["authentication"]["classification"],
            "credential_boundary": "evaluation_host_session_proven_by_external_attestor",
            "agent_credential_read_access": False,
            "runner_instance_id_hash": "sha256:" + hashlib.sha256(f"runner:{provider}".encode()).hexdigest(),
            "provenance": provenance,
            "evaluation_host_proof_content_hash": digest(canonical(evaluation_host_proof)),
            "evaluation_host_signing_key_id": evaluation_host_proof["signing_key_id"],
            "evaluation_host_public_key_spki_sha256": spki_hash(evaluation_host_public_key),
        }
        attestation["ed25519_signature"] = sign(private_key, attestation)
        (args.output / f"{provider}-host-attestation.json").write_text(
            json.dumps(attestation, indent=2, sort_keys=True, allow_nan=False) + "\n"
        )
    judgments = [
        {"provider": provider, "scenario_id": scenario["id"], "outcome": "pass",
         "reason": "deterministic release fixture review"}
        for provider in ("codex", "claude") for scenario in manifest["scenarios"]
    ]
    reviewer_private_key = args.output / "manual-review-private.pem"
    reviewer_public_key = args.output / "manual-review-public.pem"
    generate_ed25519(reviewer_private_key, reviewer_public_key)
    manual = {
        "schema": "casegraphen.eval.fresh_agent_manual_review.v1",
        "signature_algorithm": "ed25519",
        "reviewer_identity": REVIEWER_IDENTITY,
        "reviewer_key_id": REVIEWER_KEY_ID,
        "run_content_hashes": run_hashes,
        "expected_provider_provenance": expected_provider_provenance,
        "judgments": judgments,
    }
    if args.mode == "manual_stale_provenance":
        manual["expected_provider_provenance"] = json.loads(
            json.dumps(expected_provider_provenance)
        )
        manual["expected_provider_provenance"]["codex"]["source_run_id"] += 1
    if args.mode.startswith("unobservable_cost"):
        manual["cost_waivers"] = [
            {"provider": provider,
             "reason": "independent reviewer accepts unobservable-cost risk for this run",
             **({} if args.mode == "unobservable_cost_missing_limit" else {
                 "maximum_usd": 10.0
                 if args.mode == "unobservable_cost_low_waiver"
                 else 25.0
             })}
            for provider in ("codex", "claude")
        ]
    manual["ed25519_signature"] = sign(reviewer_private_key, manual)
    (args.output / "manual-review.json").write_text(
        json.dumps(manual, indent=2, sort_keys=True, allow_nan=False) + "\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
