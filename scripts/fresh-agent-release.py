#!/usr/bin/env python3
"""Aggregate a strict two-provider fresh-agent release evidence matrix.

The aggregator never invokes a provider. It verifies retained provider runs,
the scenario baseline and release threshold, and emits content-addressed
evidence. Missing/unavailable/timed-out lanes can never become passing evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import pathlib
import re
import shutil
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_POLICY = ROOT / "evals/fresh-agent/release-policy.v0.json"
DEFAULT_BASELINE = ROOT / "evals/fresh-agent/release-baseline.v0.json"
DEFAULT_MANIFEST = ROOT / "evals/fresh-agent/scenarios.v0.json"


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def load(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def validate_baseline(
    baseline: dict[str, Any], policy: dict[str, Any], manifest: dict[str, Any]
) -> list[str]:
    findings: list[str] = []
    required_providers = policy.get("required_providers")
    required_scenarios = policy.get("required_scenario_ids")
    if baseline.get("schema") != "casegraphen.eval.fresh_agent_release_baseline.v0":
        findings.append("baseline_schema_mismatch")
    if baseline.get("providers") != required_providers:
        findings.append("baseline_provider_set_mismatch")
    if baseline.get("manifest_schema") != manifest.get("schema"):
        findings.append("baseline_manifest_schema_mismatch")
    if sorted(baseline.get("scenarios", {})) != sorted(required_scenarios or []):
        findings.append("baseline_scenario_set_mismatch")
    manifest_scenarios = {item["id"]: item for item in manifest.get("scenarios", [])}
    for scenario_id in required_scenarios or []:
        expected = sorted(
            evaluator["kind"]
            for evaluator in manifest_scenarios.get(scenario_id, {}).get(
                "deterministic_evaluators", []
            )
        )
        if sorted(baseline.get("scenarios", {}).get(scenario_id, [])) != expected:
            findings.append(f"baseline_evaluator_mismatch:{scenario_id}")
    for field in (
        "maximum_new_deterministic_failures",
        "maximum_missing_scenarios",
        "maximum_unavailable_evaluators",
        "maximum_timeouts",
    ):
        if baseline.get(field) != 0:
            findings.append(f"nonzero_fail_open_baseline_threshold:{field}")
    return sorted(findings)


def summary_hash(summary: dict[str, Any]) -> str:
    value = dict(summary)
    claimed = value.pop("content_hash", None)
    actual = digest(canonical(value))
    if claimed != actual:
        raise ValueError(f"summary_content_hash_mismatch:{claimed}:{actual}")
    return actual


def verify_host_attestation(
    provider: str,
    summary: dict[str, Any],
    run_hash: str | None,
    policy: dict[str, Any],
    attestation_path: pathlib.Path | None,
    key_path: pathlib.Path | None,
) -> tuple[str | None, list[str]]:
    findings: list[str] = []
    if attestation_path is None or key_path is None:
        return None, [f"missing_host_attestation:{provider}"]
    try:
        attestation = load(attestation_path)
        key = key_path.read_bytes()
    except (OSError, ValueError, json.JSONDecodeError):
        return None, [f"unreadable_host_attestation:{provider}"]
    supplied_mac = attestation.pop("hmac_sha256", None)
    if not isinstance(supplied_mac, str) or len(key) < 32:
        return None, [f"invalid_host_attestation_signature:{provider}"]
    expected_mac = hmac.new(key, canonical(attestation), hashlib.sha256).hexdigest()
    if not hmac.compare_digest(supplied_mac, expected_mac):
        findings.append(f"invalid_host_attestation_signature:{provider}")
    authentication = summary.get("provider", {}).get("authentication", {})
    pin = policy["runner_pins"][provider]
    expected = {
        "schema": "casegraphen.eval.cli_session_host_attestation.v0",
        "provider": provider,
        "run_content_hash": run_hash,
        "host_attestation_challenge": summary.get("host_attestation_challenge"),
        "authentication_classification": authentication.get("classification"),
        "credential_boundary": "dedicated_provider_os_account_with_brokered_session",
        "agent_credential_read_access": False,
        "key_id": pin.get("host_attestation_key_id"),
    }
    for field, value in expected.items():
        if attestation.get(field) != value:
            findings.append(f"host_attestation_binding_mismatch:{provider}:{field}")
    runner_hash = attestation.get("runner_instance_id_hash")
    if not isinstance(runner_hash, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", runner_hash):
        findings.append(f"host_attestation_binding_mismatch:{provider}:runner_instance_id_hash")
    return digest(attestation_path.read_bytes()), findings


def evidence_inventory(
    run_paths: dict[str, pathlib.Path], retained: pathlib.Path
) -> tuple[list[dict[str, Any]], list[str]]:
    blobs = retained / "blobs"
    blobs.mkdir(parents=True, exist_ok=True)
    inventory: list[dict[str, Any]] = []
    findings: list[str] = []
    for provider, run_path in sorted(run_paths.items()):
        for path in sorted(run_path.rglob("*")):
            if path.is_symlink():
                findings.append(
                    f"symlinked_retained_evidence:{provider}:{path.relative_to(run_path)}"
                )
                continue
            if not path.is_file():
                continue
            data = path.read_bytes()
            content_hash = digest(data)
            blob = blobs / content_hash.replace(":", "-")
            if blob.exists() and blob.read_bytes() != data:
                raise ValueError(f"content_address_collision:{content_hash}")
            if not blob.exists():
                blob.write_bytes(data)
            inventory.append(
                {
                    "provider": provider,
                    "source_path": path.relative_to(run_path).as_posix(),
                    "content_hash": content_hash,
                    "byte_length": len(data),
                    "retained_blob": blob.relative_to(retained).as_posix(),
                }
            )
    return inventory, findings


def manual_resolutions(
    path: pathlib.Path | None, run_hashes: dict[str, str]
) -> tuple[dict[tuple[str, str], dict[str, Any]], dict[str, dict[str, Any]], list[str], str | None]:
    if path is None:
        return {}, {}, ["manual_review_missing"], None
    document = load(path)
    findings: list[str] = []
    if document.get("schema") != "casegraphen.eval.fresh_agent_manual_review.v0":
        findings.append("manual_review_schema_mismatch")
    if document.get("run_content_hashes") != run_hashes:
        findings.append("manual_review_run_binding_mismatch")
    resolved: dict[tuple[str, str], dict[str, Any]] = {}
    for judgment in document.get("judgments", []):
        key = (judgment.get("provider"), judgment.get("scenario_id"))
        if key in resolved:
            findings.append(f"duplicate_manual_judgment:{key[0]}:{key[1]}")
        if judgment.get("outcome") not in {"pass", "fail"}:
            findings.append(f"invalid_manual_judgment:{key[0]}:{key[1]}")
        if not judgment.get("reviewer") or not judgment.get("reason"):
            findings.append(f"incomplete_manual_judgment:{key[0]}:{key[1]}")
        resolved[key] = judgment
    cost_waivers: dict[str, dict[str, Any]] = {}
    for waiver in document.get("cost_waivers", []):
        provider = waiver.get("provider")
        if provider in cost_waivers:
            findings.append(f"duplicate_cost_waiver:{provider}")
        maximum_usd = waiver.get("maximum_usd")
        if (
            provider not in run_hashes
            or not waiver.get("reviewer")
            or not waiver.get("reason")
            or not isinstance(maximum_usd, (int, float))
            or isinstance(maximum_usd, bool)
            or maximum_usd <= 0
        ):
            findings.append(f"invalid_cost_waiver:{provider}")
        cost_waivers[provider] = waiver
    return resolved, cost_waivers, findings, digest(path.read_bytes())


def content_addressed_proposal(kind: str, payload: dict[str, Any]) -> dict[str, Any]:
    proposal_hash = digest(canonical({"kind": kind, "payload": payload}))
    return {
        "proposal_id": f"proposal:{proposal_hash.replace(':', '-')}",
        "kind": kind,
        "review_status": "unreviewed",
        "accepted": False,
        "payload": payload,
    }


def aggregate(
    run_paths: list[pathlib.Path], policy_path: pathlib.Path, baseline_path: pathlib.Path,
    manifest_path: pathlib.Path, manual_path: pathlib.Path | None, output: pathlib.Path,
    host_attestations: dict[str, pathlib.Path], attestation_keys: dict[str, pathlib.Path],
) -> tuple[dict[str, Any], int]:
    policy, baseline, manifest = load(policy_path), load(baseline_path), load(manifest_path)
    findings = validate_baseline(baseline, policy, manifest)
    summaries: dict[str, dict[str, Any]] = {}
    paths: dict[str, pathlib.Path] = {}
    run_hashes: dict[str, str] = {}
    for run_path in run_paths:
        summary_path = run_path / "summary.json"
        if not summary_path.is_file():
            findings.append(f"missing_summary:{run_path}")
            continue
        summary = load(summary_path)
        provider = summary.get("provider", {}).get("provider")
        if provider in summaries:
            findings.append(f"duplicate_provider_run:{provider}")
            continue
        if provider not in policy["required_providers"]:
            findings.append(f"unexpected_provider:{provider}")
            continue
        summaries[provider], paths[provider] = summary, run_path
        try:
            run_hashes[provider] = summary_hash(summary)
        except ValueError as error:
            findings.append(str(error))

    for provider in policy["required_providers"]:
        if provider not in summaries:
            findings.append(f"missing_provider:{provider}")
    host_attestation_hashes: dict[str, str] = {}
    for provider in policy["required_providers"]:
        summary = summaries.get(provider)
        if summary is None:
            continue
        attestation_hash, attestation_findings = verify_host_attestation(
            provider,
            summary,
            run_hashes.get(provider),
            policy,
            host_attestations.get(provider),
            attestation_keys.get(provider),
        )
        findings.extend(attestation_findings)
        if attestation_hash is not None:
            host_attestation_hashes[provider] = attestation_hash
    manual, cost_waivers, manual_findings, manual_hash = manual_resolutions(
        manual_path, run_hashes
    )
    findings.extend(manual_findings)
    matrix: list[dict[str, Any]] = []
    counts = {"deterministic_failures": 0, "timeouts": 0, "runner_failures": 0,
              "unavailable": 0, "missing_scenarios": 0, "manual_failures": 0,
              "manual_unresolved": 0}
    required_ids = set(policy["required_scenario_ids"])
    for provider in policy["required_providers"]:
        summary = summaries.get(provider)
        if summary is None:
            continue
        if summary.get("status") != "completed":
            findings.append(f"provider_not_completed:{provider}:{summary.get('status')}")
            counts["unavailable"] += 1
        identity = summary.get("provider", {})
        if identity.get("available") is not True or identity.get("version_matches") is not True:
            findings.append(f"provider_identity_unavailable:{provider}")
            counts["unavailable"] += 1
        pin = policy["runner_pins"][provider]
        if (
            identity.get("declared_package_identity") != pin["package_identity"]
            or identity.get("expected_version") != pin["version"]
        ):
            findings.append(f"provider_identity_unpinned:{provider}")
        authentication = identity.get("authentication", {})
        if (
            authentication.get("mode") != pin["authentication_mode"]
            or authentication.get("available") is not True
            or authentication.get("non_api_key_session_verified") is not True
            or not authentication.get("classification")
            or authentication.get("probe_output_retained") is not False
        ):
            findings.append(f"provider_cli_session_unavailable:{provider}")
            counts["unavailable"] += 1
        if authentication.get("classification") not in pin.get(
            "allowed_authentication_classifications", []
        ):
            findings.append(f"provider_cli_session_class_refused:{provider}")
            counts["unavailable"] += 1
        if summary.get("manifest_hash") != digest(manifest_path.read_bytes()):
            findings.append(f"scenario_manifest_hash_mismatch:{provider}")
        model_observation = summary.get("model_observation", {})
        if model_observation.get("observable") is True:
            reported_models = model_observation.get("reported_models")
            if (
                model_observation.get("matches_declared") is not True
                or reported_models != [identity.get("model")]
            ):
                findings.append(f"provider_model_observation_mismatch:{provider}")
        elif model_observation.get("observable") is not False:
            findings.append(f"provider_model_observation_missing:{provider}")
        results = summary.get("results") if isinstance(summary.get("results"), list) else []
        by_id: dict[str, dict[str, Any]] = {}
        for result in results:
            scenario_id = result.get("scenario_id")
            if scenario_id in by_id:
                findings.append(f"duplicate_scenario:{provider}:{scenario_id}")
            by_id[scenario_id] = result
        for missing in sorted(required_ids - set(by_id)):
            findings.append(f"missing_scenario:{provider}:{missing}")
            counts["missing_scenarios"] += 1
        for extra in sorted(set(by_id) - required_ids):
            findings.append(f"unexpected_scenario:{provider}:{extra}")
        for scenario_id in sorted(required_ids & set(by_id)):
            result = by_id[scenario_id]
            if result.get("provider", {}).get("provider") != provider:
                findings.append(f"result_provider_mismatch:{provider}:{scenario_id}")
            if result.get("workspace_retained") is not True or result.get(
                "credential_material_scan", {}
            ).get("status") != "pass":
                findings.append(f"credential_retention_boundary_failed:{provider}:{scenario_id}")
            retained_result = paths[provider] / scenario_id / "result.json"
            if not retained_result.is_file():
                findings.append(f"missing_retained_result:{provider}:{scenario_id}")
            else:
                try:
                    if load(retained_result) != result:
                        findings.append(f"retained_result_mismatch:{provider}:{scenario_id}")
                except (ValueError, json.JSONDecodeError) as error:
                    findings.append(f"invalid_retained_result:{provider}:{scenario_id}:{error}")
            statuses: dict[str, list[str]] = {}
            for evaluation in result.get("evaluations", []):
                statuses.setdefault(evaluation.get("kind"), []).append(evaluation.get("status"))
            expected_kinds = sorted(baseline["scenarios"][scenario_id])
            actual_deterministic = sorted(
                kind for kind in statuses if kind != "manual_judgment"
            )
            if actual_deterministic != expected_kinds:
                findings.append(f"evaluator_baseline_mismatch:{provider}:{scenario_id}")
                counts["deterministic_failures"] += 1
            deterministic_bad = sum(
                status != "pass" for kind in expected_kinds for status in statuses.get(kind, ["missing"])
            )
            counts["deterministic_failures"] += deterministic_bad
            counts["unavailable"] += sum(
                status == "unavailable" for values in statuses.values() for status in values
            )
            if result.get("timed_out") is True:
                counts["timeouts"] += 1
            if result.get("returncode") != 0:
                counts["runner_failures"] += 1
            judgment = manual.get((provider, scenario_id))
            if judgment is None:
                counts["manual_unresolved"] += 1
            elif judgment.get("outcome") != "pass":
                counts["manual_failures"] += 1
            matrix.append({
                "provider": provider, "scenario_id": scenario_id,
                "runner_returncode": result.get("returncode"),
                "timed_out": result.get("timed_out"),
                "deterministic_evaluators": statuses,
                "manual_outcome": judgment.get("outcome") if judgment else "unresolved",
                "result_content_hash": digest(canonical(result)),
            })

    threshold = policy["stable_promotion_threshold"]
    expected_manual = {
        (provider, scenario_id)
        for provider in policy["required_providers"]
        for scenario_id in policy["required_scenario_ids"]
    }
    for provider, scenario_id in sorted(set(manual) - expected_manual):
        findings.append(f"unexpected_manual_judgment:{provider}:{scenario_id}")
    if counts["deterministic_failures"] > threshold["deterministic_failures"]:
        findings.append("deterministic_failure_threshold_exceeded")
    if counts["timeouts"] > threshold["timeouts"]:
        findings.append("timeout_threshold_exceeded")
    if counts["runner_failures"] > threshold["runner_failures"]:
        findings.append("runner_failure_threshold_exceeded")
    if counts["unavailable"] > baseline["maximum_unavailable_evaluators"]:
        findings.append("unavailable_threshold_exceeded")
    if counts["missing_scenarios"] > baseline["maximum_missing_scenarios"]:
        findings.append("missing_scenario_threshold_exceeded")
    if threshold["manual_judgments_must_be_resolved"] and counts["manual_unresolved"]:
        findings.append("manual_judgments_unresolved")
    if counts["manual_failures"]:
        findings.append("manual_judgment_failed")
    for provider, summary in summaries.items():
        budget = summary.get("budget", {})
        if (
            threshold["cost_must_be_observed_or_explicitly_waived"]
            and not budget.get("observable")
            and provider not in cost_waivers
        ):
            findings.append(f"cost_unobserved:{provider}")
        waiver = cost_waivers.get(provider)
        if waiver is not None and isinstance(waiver.get("maximum_usd"), (int, float)):
            declared_maximum = budget.get("maximum_usd")
            if not isinstance(declared_maximum, (int, float)) or declared_maximum > waiver["maximum_usd"]:
                findings.append(f"cost_waiver_limit_exceeded:{provider}")
        if budget.get("observable") and budget.get("observed_usd", 0) > budget.get("maximum_usd", -1):
            findings.append(f"cost_budget_exceeded:{provider}")

    findings = sorted(set(findings))
    output.mkdir(parents=True, exist_ok=False)
    inventory, retention_findings = evidence_inventory(paths, output)
    if retention_findings:
        findings.extend(retention_findings)
        findings = sorted(set(findings))
        status = "fail"
    if manual_path is not None and manual_path.is_file():
        manual_bytes = manual_path.read_bytes()
        manual_content_hash = digest(manual_bytes)
        manual_blob = output / "blobs" / manual_content_hash.replace(":", "-")
        if not manual_blob.exists():
            manual_blob.write_bytes(manual_bytes)
        inventory.append({
            "provider": "independent_review",
            "source_path": manual_path.name,
            "content_hash": manual_content_hash,
            "byte_length": len(manual_bytes),
            "retained_blob": manual_blob.relative_to(output).as_posix(),
        })
    for provider, attestation_path in sorted(host_attestations.items()):
        if provider not in host_attestation_hashes or not attestation_path.is_file():
            continue
        attestation_bytes = attestation_path.read_bytes()
        attestation_content_hash = digest(attestation_bytes)
        blob = output / "blobs" / attestation_content_hash.replace(":", "-")
        if not blob.exists():
            blob.write_bytes(attestation_bytes)
        inventory.append({
            "provider": f"{provider}_host_attestation",
            "source_path": attestation_path.name,
            "content_hash": attestation_content_hash,
            "byte_length": len(attestation_bytes),
            "retained_blob": blob.relative_to(output).as_posix(),
        })
    status = "pass" if not findings else "fail"
    report: dict[str, Any] = {
        "schema": "casegraphen.eval.fresh_agent_release_report.v0",
        "status": status,
        "promotion_eligible": status == "pass",
        "accepted": False,
        "policy_content_hash": digest(policy_path.read_bytes()),
        "baseline_content_hash": digest(baseline_path.read_bytes()),
        "scenario_manifest_content_hash": digest(manifest_path.read_bytes()),
        "manual_review_content_hash": manual_hash,
        "provider_run_content_hashes": run_hashes,
        "host_attestation_content_hashes": host_attestation_hashes,
        "counts": counts,
        "findings": findings,
        "matrix": matrix,
        "evidence_inventory": inventory,
        "failure_proposals": [],
    }
    if findings:
        report["failure_proposals"] = [
            content_addressed_proposal("release_audit", {"finding_codes": findings}),
            content_addressed_proposal(
                "topology_redesign",
                {"finding_codes": findings, "automatic_topology_mutation": False},
            ),
        ]
    report_hash = digest(canonical(report))
    report["content_hash"] = report_hash
    report_name = f"{report_hash.replace(':', '-')}.release-report.json"
    (output / report_name).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (output / "release-report.pointer.json").write_text(json.dumps({
        "schema": "casegraphen.eval.fresh_agent_release_report_pointer.v0",
        "content_hash": report_hash,
        "path": report_name,
    }, indent=2, sort_keys=True) + "\n")
    return report, 0 if status == "pass" else 1


def discover_runs(root: pathlib.Path) -> list[pathlib.Path]:
    return sorted({path.parent for path in root.rglob("summary.json")})


def provider_paths(values: list[str], argument: str) -> dict[str, pathlib.Path]:
    parsed: dict[str, pathlib.Path] = {}
    for value in values:
        provider, separator, path = value.partition("=")
        if separator != "=" or provider not in {"codex", "claude"} or not path:
            raise ValueError(f"{argument} must use provider=/absolute/or/relative/path")
        if provider in parsed:
            raise ValueError(f"duplicate {argument} for {provider}")
        parsed[provider] = pathlib.Path(path)
    return parsed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider-run", action="append", type=pathlib.Path, default=[])
    parser.add_argument("--runs-root", type=pathlib.Path)
    parser.add_argument("--policy", type=pathlib.Path, default=DEFAULT_POLICY)
    parser.add_argument("--baseline", type=pathlib.Path, default=DEFAULT_BASELINE)
    parser.add_argument("--manifest", type=pathlib.Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--manual-review", type=pathlib.Path)
    parser.add_argument("--host-attestation", action="append", default=[])
    parser.add_argument("--attestation-key", action="append", default=[])
    parser.add_argument("--output-dir", type=pathlib.Path)
    parser.add_argument("--check-baseline", action="store_true")
    args = parser.parse_args()
    policy, baseline, manifest = load(args.policy), load(args.baseline), load(args.manifest)
    baseline_findings = validate_baseline(baseline, policy, manifest)
    if args.check_baseline:
        if baseline_findings:
            print("\n".join(baseline_findings), file=sys.stderr)
            return 1
        print("validated strict 2-provider x 10-scenario release baseline")
        return 0
    if args.output_dir is None:
        parser.error("--output-dir is required")
    runs = list(args.provider_run)
    if args.runs_root:
        runs.extend(discover_runs(args.runs_root))
    try:
        host_attestations = provider_paths(args.host_attestation, "--host-attestation")
        attestation_keys = provider_paths(args.attestation_key, "--attestation-key")
    except ValueError as error:
        parser.error(str(error))
    report, code = aggregate(
        runs,
        args.policy,
        args.baseline,
        args.manifest,
        args.manual_review,
        args.output_dir,
        host_attestations,
        attestation_keys,
    )
    print(json.dumps({"status": report["status"], "content_hash": report["content_hash"],
                      "finding_count": len(report["findings"])}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
