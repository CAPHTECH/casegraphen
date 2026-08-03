#!/usr/bin/env python3
"""Opt-in fresh-process behavior evaluation for shipped CaseGraphen Skills.

Normal CI calls only --check-manifest. A release/operator run must explicitly
provide an external runner command and output directory.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "evals/fresh-agent/scenarios.v0.json"
DEFAULT_RELEASE_POLICY = ROOT / "evals/fresh-agent/release-policy.v0.json"
REQUIRED_SCENARIOS = {
    "independent-20-file-fanout",
    "same-file-resource-edge",
    "correlated-verifier-context",
    "hierarchical-1000-fanin",
    "missing-one-of-200-reports",
    "dynamic-loop-dedupe-all-seen",
    "evidence-requires-review",
    "stale-revision-no-auto-rebase",
    "tool-failure-versus-domain-halt",
    "proposal-not-direct-mutation",
}
EVALUATOR_KINDS = {"graph_lint", "json_schema", "completeness_oracle", "json_assert"}
RUNNER_PROFILES = {
    "codex": [
        "codex",
        "exec",
        "--sandbox",
        "workspace-write",
        "--skip-git-repo-check",
        "--color",
        "never",
        "-",
    ],
    "claude": [
        "claude",
        "--print",
        "--permission-mode",
        "bypassPermissions",
        "--output-format",
        "stream-json",
        "--verbose",
    ],
}
PROFILE_CREDENTIAL_ENV = {
    "codex": "OPENAI_API_KEY",
    "claude": "ANTHROPIC_API_KEY",
}
SECRET_MARKERS = ("TOKEN", "SECRET", "PASSWORD", "API_KEY", "CREDENTIAL")


def utc_now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha256_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def hash_tree(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    for item in sorted(candidate for candidate in path.rglob("*") if candidate.is_file()):
        digest.update(item.relative_to(path).as_posix().encode())
        digest.update(b"\0")
        digest.update(item.read_bytes())
        digest.update(b"\0")
    return "sha256:" + digest.hexdigest()


def is_secret_key(key: str) -> bool:
    return any(marker in key.upper() for marker in SECRET_MARKERS)


def provider_environment(provider: str, workspace: pathlib.Path | None = None) -> dict[str, str]:
    """Build a provider environment containing at most that provider's credential."""
    credential_key = PROFILE_CREDENTIAL_ENV.get(provider)
    environment = {key: value for key, value in os.environ.items() if not is_secret_key(key)}
    if credential_key and os.environ.get(credential_key):
        environment[credential_key] = os.environ[credential_key]
    if workspace is not None:
        environment["CASEGRAPHEN_EVAL_WORKSPACE"] = str(workspace)
    return environment


def runner_identity(
    command: list[str],
    provider: str,
    model: str | None,
    expected_version: str | None,
    package_identity: str | None,
) -> dict[str, Any]:
    executable = shutil.which(command[0])
    if executable is None:
        return {"provider": provider, "model": model, "available": False, "executable": command[0]}
    try:
        version = subprocess.run(
            [executable, "--version"],
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
            # Version discovery needs no provider authority. Keeping the
            # credential out also prevents a compromised probe from echoing it
            # into retained runner identity evidence.
            env=provider_environment("identity-probe"),
        )
        version_text = (version.stdout or version.stderr).strip()
    except subprocess.TimeoutExpired:
        version_text = "version probe timed out"
    return {
        "provider": provider,
        "model": model,
        "available": True,
        "executable": str(pathlib.Path(executable).resolve()),
        "version": version_text,
        "expected_version": expected_version,
        "version_matches": expected_version is None
        or re.search(rf"(?<![0-9.]){re.escape(expected_version)}(?![0-9.])", version_text) is not None,
        "declared_package_identity": package_identity,
        "command_hash": sha256_bytes(json.dumps(command, separators=(",", ":")).encode()),
    }


def secret_values(environment: dict[str, str]) -> list[str]:
    return [value for key, value in environment.items() if value and is_secret_key(key)]


def redact(text: str, secrets: list[str]) -> str:
    for secret in secrets:
        text = text.replace(secret, "[REDACTED]")
    return text


def redact_value(value: Any, secrets: list[str]) -> Any:
    if isinstance(value, str):
        return redact(value, secrets)
    if isinstance(value, list):
        return [redact_value(item, secrets) for item in value]
    if isinstance(value, dict):
        return {key: redact_value(item, secrets) for key, item in value.items()}
    return value


def files_containing_secrets(path: pathlib.Path, secrets: list[str]) -> list[pathlib.Path]:
    encoded = [secret.encode() for secret in secrets if secret]
    if not encoded:
        return []
    affected: list[pathlib.Path] = []
    for item in path.rglob("*"):
        if item.is_file():
            content = item.read_bytes()
            if any(secret in content for secret in encoded):
                affected.append(item)
    return affected


def usage_observations(stdout: str) -> list[dict[str, Any]]:
    """Retain provider-emitted usage/cost objects without interpreting them."""
    observed: list[dict[str, Any]] = []
    for line in stdout.splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(value, dict):
            continue
        selected = {
            key: value[key]
            for key in ("usage", "token_usage", "cost", "total_cost_usd", "model")
            if key in value
        }
        if selected:
            observed.append(selected)
    return observed


def observed_cost_usd(results: list[dict[str, Any]]) -> tuple[float, bool]:
    values: list[float] = []
    for result in results:
        for observation in result.get("usage_observations", []):
            for key in ("total_cost_usd", "cost"):
                value = observation.get(key)
                if isinstance(value, (int, float)):
                    values.append(float(value))
    return sum(values), bool(values)


def load_manifest(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError("manifest must be an object")
    return value


def safe_relative(value: str, field: str) -> pathlib.Path:
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or not path.parts:
        raise ValueError(f"{field} must be a non-traversing relative path: {value!r}")
    return pathlib.Path(*path.parts)


def validate_manifest(manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema") != "casegraphen.eval.fresh_agent_manifest.v0":
        errors.append("unsupported manifest schema")
    if manifest.get("version") != 0:
        errors.append("manifest version must be 0")
    scenarios = manifest.get("scenarios")
    if not isinstance(scenarios, list):
        return errors + ["scenarios must be an array"]
    ids = [scenario.get("id") for scenario in scenarios if isinstance(scenario, dict)]
    if set(ids) != REQUIRED_SCENARIOS or len(ids) != len(REQUIRED_SCENARIOS):
        errors.append("manifest must contain each of the ten required scenario ids exactly once")
    for index, scenario in enumerate(scenarios):
        location = f"scenarios[{index}]"
        if not isinstance(scenario, dict):
            errors.append(f"{location} must be an object")
            continue
        scenario_id = scenario.get("id")
        if not isinstance(scenario_id, str) or not scenario_id:
            errors.append(f"{location}.id must be non-empty")
        skill = scenario.get("skill")
        if not isinstance(skill, str) or not (ROOT / "skills" / skill / "SKILL.md").is_file():
            errors.append(f"{location}.skill does not name a shipped Skill")
        if not isinstance(scenario.get("task"), str) or not scenario["task"].strip():
            errors.append(f"{location}.task must be non-empty")
        targets: set[pathlib.Path] = set()
        sources: set[pathlib.Path] = set()
        for artifact_index, artifact in enumerate(scenario.get("artifacts", [])):
            try:
                source = safe_relative(artifact["source"], f"{location}.artifacts[{artifact_index}].source")
                target = safe_relative(artifact["target"], f"{location}.artifacts[{artifact_index}].target")
                if not (ROOT / source).is_file():
                    errors.append(f"{location} artifact source does not exist: {source}")
                if target in targets:
                    errors.append(f"{location} repeats artifact target: {target}")
                targets.add(target)
                sources.add(source)
            except (KeyError, TypeError, ValueError) as error:
                errors.append(str(error))
        outputs = scenario.get("expected_outputs")
        if not isinstance(outputs, list) or not outputs:
            errors.append(f"{location}.expected_outputs must be non-empty")
        else:
            for output in outputs:
                try:
                    safe_relative(output, f"{location}.expected_outputs")
                except (TypeError, ValueError) as error:
                    errors.append(str(error))
        evaluators = scenario.get("deterministic_evaluators")
        if not isinstance(evaluators, list) or not evaluators:
            errors.append(f"{location}.deterministic_evaluators must be non-empty")
        else:
            for evaluator in evaluators:
                if not isinstance(evaluator, dict) or evaluator.get("kind") not in EVALUATOR_KINDS:
                    errors.append(f"{location} has unknown deterministic evaluator")
                    continue
                try:
                    safe_relative(evaluator["output"], f"{location}.evaluator.output")
                    for field in ("schema", "oracle"):
                        if field in evaluator:
                            source = safe_relative(evaluator[field], f"{location}.evaluator.{field}")
                            if not (ROOT / source).is_file():
                                errors.append(f"{location} evaluator {field} does not exist: {source}")
                            if field == "oracle" and source in sources:
                                errors.append(f"{location} evaluator oracle must not be exposed as a task artifact")
                except (KeyError, TypeError, ValueError) as error:
                    errors.append(str(error))
        manual = scenario.get("manual_judgments")
        if not isinstance(manual, list) or not manual or not all(isinstance(item, str) and item for item in manual):
            errors.append(f"{location}.manual_judgments must explicitly list human judgments")
    return errors


def render_task(scenario: dict[str, Any]) -> str:
    inputs = "\n".join(f"- `{artifact['target']}`" for artifact in scenario["artifacts"])
    outputs = "\n".join(f"- `{output}`" for output in scenario["expected_outputs"])
    return (
        f"# Task\n\n{scenario['task']}\n\n"
        f"Use the Skill at `skill/{scenario['skill']}/SKILL.md`. Read only its references needed for this task.\n\n"
        f"## Inputs\n\n{inputs}\n\n## Required files\n\n{outputs}\n"
    )


def prepare_workspace(scenario: dict[str, Any], workspace: pathlib.Path) -> None:
    skill = scenario["skill"]
    shutil.copytree(ROOT / "skills" / skill, workspace / "skill" / skill)
    for artifact in scenario["artifacts"]:
        destination = workspace / safe_relative(artifact["target"], "artifact target")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / safe_relative(artifact["source"], "artifact source"), destination)
    (workspace / "TASK.md").write_text(render_task(scenario))


def json_pointer(value: Any, pointer: str) -> Any:
    if pointer == "":
        return value
    if not pointer.startswith("/"):
        raise ValueError(f"invalid JSON pointer: {pointer}")
    current = value
    for raw in pointer[1:].split("/"):
        token = raw.replace("~1", "/").replace("~0", "~")
        current = current[int(token)] if isinstance(current, list) else current[token]
    return current


def evaluate_assertions(value: Any, assertions: list[dict[str, Any]]) -> list[str]:
    failures: list[str] = []
    for assertion in assertions:
        pointer = assertion["pointer"]
        try:
            actual = json_pointer(value, pointer)
            expected = assertion["value"]
            operation = assertion["op"]
            passed = actual == expected if operation == "eq" else actual <= expected if operation == "le" else False
            if not passed:
                failures.append(f"{pointer}: expected {operation} {expected!r}, got {actual!r}")
        except (KeyError, IndexError, TypeError, ValueError) as error:
            failures.append(f"{pointer}: {error}")
    return failures


def evaluate(scenario: dict[str, Any], workspace: pathlib.Path, casegraphen_bin: str) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for output in scenario["expected_outputs"]:
        if not (workspace / safe_relative(output, "expected output")).is_file():
            results.append({"kind": "expected_output", "output": output, "status": "fail", "detail": "missing"})
    for evaluator in scenario["deterministic_evaluators"]:
        kind = evaluator["kind"]
        output = workspace / safe_relative(evaluator["output"], "evaluator output")
        if not output.is_file():
            results.append({"kind": kind, "status": "fail", "detail": f"missing {evaluator['output']}"})
            continue
        try:
            if kind == "graph_lint":
                process = subprocess.run(
                    [casegraphen_bin, "graph", "lint", "--input", str(output), "--format", "json"],
                    cwd=workspace,
                    capture_output=True,
                    text=True,
                    timeout=60,
                    check=False,
                )
                if process.returncode != 0:
                    raise ValueError(f"graph lint refused: {process.stderr.strip()}")
                report = json.loads(process.stdout)
                failures = evaluate_assertions(report, evaluator.get("assertions", []))
                codes = {finding.get("code") for finding in report.get("findings", [])}
                failures.extend(f"missing finding {code}" for code in evaluator.get("required_finding_codes", []) if code not in codes)
                failures.extend(f"forbidden finding {code}" for code in evaluator.get("forbidden_finding_codes", []) if code in codes)
            elif kind == "json_schema":
                process = subprocess.run(
                    [sys.executable, "-m", "jsonschema", "-i", str(output), str(ROOT / evaluator["schema"])],
                    capture_output=True,
                    text=True,
                    timeout=60,
                    check=False,
                )
                if "No module named jsonschema" in process.stderr:
                    results.append({"kind": kind, "status": "unavailable", "detail": "python jsonschema module is not installed"})
                    continue
                failures = [] if process.returncode == 0 else [process.stderr.strip()]
            elif kind == "completeness_oracle":
                actual_document = json.loads(output.read_text())
                actual = json_pointer(actual_document, evaluator.get("output_pointer", ""))
                oracle = json.loads((ROOT / evaluator["oracle"]).read_text())
                failures = [
                    f"{field}: expected canonical {oracle.get(field)!r}, got {actual.get(field)!r}"
                    for field in evaluator["fields"]
                    if actual.get(field) != oracle.get(field)
                ]
            else:
                failures = evaluate_assertions(json.loads(output.read_text()), evaluator["assertions"])
            results.append({"kind": kind, "status": "pass" if not failures else "fail", "detail": failures})
        except (KeyError, TypeError, ValueError, json.JSONDecodeError, subprocess.TimeoutExpired) as error:
            results.append({"kind": kind, "status": "fail", "detail": str(error)})
    results.extend(
        {"kind": "manual_judgment", "status": "manual_required", "detail": judgment}
        for judgment in scenario["manual_judgments"]
    )
    return results


def run_scenario(
    scenario: dict[str, Any],
    runner: list[str],
    output_root: pathlib.Path,
    timeout: int,
    casegraphen_bin: str,
    identity: dict[str, Any],
    model: str | None,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"casegraphen-eval-{scenario['id']}-") as temporary:
        workspace = pathlib.Path(temporary) / "workspace"
        workspace.mkdir()
        prepare_workspace(scenario, workspace)
        prompt_file = workspace / "TASK.md"
        replacements = {
            "{workspace}": str(workspace),
            "{prompt_file}": str(prompt_file),
            "{skill_path}": str(workspace / "skill" / scenario["skill"] / "SKILL.md"),
        }
        command = [replacements.get(token, token) for token in runner]
        if model:
            if identity["provider"] == "codex":
                command[2:2] = ["--model", model]
            elif identity["provider"] == "claude":
                command.extend(["--model", model])
        if identity["provider"] in PROFILE_CREDENTIAL_ENV:
            environment = provider_environment(identity["provider"], workspace)
        else:
            environment = {**os.environ, "CASEGRAPHEN_EVAL_WORKSPACE": str(workspace)}
        environment["CASEGRAPHEN_EVAL_SKILL"] = str(workspace / "skill" / scenario["skill"])
        # Scan against the parent environment too: an unrelated credential must
        # neither reach the provider process nor survive in retained evidence.
        secrets = secret_values(dict(os.environ))
        declared_input_hash = hash_tree(workspace)
        started_at = utc_now()
        started = time.monotonic()
        timed_out = False
        try:
            process = subprocess.run(
                command,
                cwd=workspace,
                input=prompt_file.read_text(),
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
                env=environment,
            )
            returncode, stdout, stderr = process.returncode, process.stdout, process.stderr
        except subprocess.TimeoutExpired as error:
            timed_out = True
            returncode = None
            stdout = error.stdout.decode() if isinstance(error.stdout, bytes) else error.stdout or ""
            stderr = error.stderr.decode() if isinstance(error.stderr, bytes) else error.stderr or ""
        elapsed_ms = int((time.monotonic() - started) * 1000)
        finished_at = utc_now()
        stdout = redact(stdout, secrets)
        stderr = redact(stderr, secrets)
        destination = output_root / scenario["id"]
        destination.mkdir(parents=True, exist_ok=False)
        (destination / "raw.stdout").write_text(stdout)
        (destination / "raw.stderr").write_text(stderr)
        (destination / "prompt.md").write_text(prompt_file.read_text())
        evaluation = evaluate(scenario, workspace, casegraphen_bin)
        affected = files_containing_secrets(workspace, secrets)
        workspace_retained = not affected
        if workspace_retained:
            shutil.copytree(workspace, destination / "workspace")
        else:
            evaluation.append(
                {
                    "kind": "credential_material_scan",
                    "status": "fail",
                    "detail": "generated workspace withheld because credential material was detected",
                }
            )
        evaluation = redact_value(evaluation, secrets)
        result = {
            "scenario_id": scenario["id"],
            "provider": identity,
            "runner_argv": command,
            "prompt_hash": sha256_bytes(prompt_file.read_bytes()),
            "skill_hash": hash_tree(workspace / "skill" / scenario["skill"]),
            "declared_input_hash": declared_input_hash,
            "produced_workspace_hash": hash_tree(workspace) if workspace_retained else None,
            "workspace_retained": workspace_retained,
            "credential_material_scan": {
                "status": "pass" if workspace_retained else "fail",
                "affected_file_count": len(affected),
            },
            "raw_stdout_hash": sha256_bytes(stdout.encode()),
            "raw_stderr_hash": sha256_bytes(stderr.encode()),
            "usage_observations": usage_observations(stdout),
            "started_at": started_at,
            "finished_at": finished_at,
            "returncode": returncode,
            "timed_out": timed_out,
            "elapsed_ms": elapsed_ms,
            "evaluations": evaluation,
        }
        (destination / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=pathlib.Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--check-manifest", action="store_true")
    parser.add_argument("--runner-json", help="JSON array command; exact {workspace}, {prompt_file}, and {skill_path} tokens are replaced")
    parser.add_argument("--runner-profile", choices=sorted(RUNNER_PROFILES))
    parser.add_argument("--model", help="provider model id; recorded exactly as supplied")
    parser.add_argument("--expected-runner-version", help="exact version required for a real runner profile")
    parser.add_argument("--runner-package-identity", help="exact pinned package identity retained with evidence")
    parser.add_argument("--output-dir", type=pathlib.Path)
    parser.add_argument("--scenario", action="append", default=[])
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--budget-usd", type=float, help="declared aggregate release-run budget")
    parser.add_argument("--casegraphen-bin", default="casegraphen")
    args = parser.parse_args()
    manifest = load_manifest(args.manifest.resolve())
    errors = validate_manifest(manifest)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if args.check_manifest:
        print(f"validated {len(manifest['scenarios'])} fresh-agent scenarios")
        return 0
    if bool(args.runner_json) == bool(args.runner_profile) or args.output_dir is None:
        parser.error("release evaluation requires exactly one of --runner-json/--runner-profile and --output-dir")
    runner = json.loads(args.runner_json) if args.runner_json else RUNNER_PROFILES[args.runner_profile]
    if not isinstance(runner, list) or not runner or not all(isinstance(token, str) and token for token in runner):
        parser.error("--runner-json must be a non-empty JSON array of command tokens")
    selected = set(args.scenario)
    unknown = selected - REQUIRED_SCENARIOS
    if unknown:
        parser.error(f"unknown scenarios: {sorted(unknown)}")
    scenarios = [scenario for scenario in manifest["scenarios"] if not selected or scenario["id"] in selected]
    output_root = args.output_dir.resolve()
    output_root.mkdir(parents=True, exist_ok=False)
    provider = args.runner_profile or "custom"
    if args.runner_profile and (args.budget_usd is None or args.budget_usd <= 0):
        parser.error("real runner profiles require a positive --budget-usd")
    if args.runner_profile and (not args.expected_runner_version or not args.runner_package_identity):
        parser.error("real runner profiles require --expected-runner-version and --runner-package-identity")
    if args.runner_profile:
        policy = json.loads(DEFAULT_RELEASE_POLICY.read_text())
        pin = policy["runner_pins"][args.runner_profile]
        if args.expected_runner_version != pin["version"] or args.runner_package_identity != pin["package_identity"]:
            parser.error("real runner identity must exactly match evals/fresh-agent/release-policy.v0.json")
    credential_key = PROFILE_CREDENTIAL_ENV.get(provider)
    if credential_key and not os.environ.get(credential_key):
        unavailable = {
            "schema": "casegraphen.eval.fresh_agent_run.v0",
            "status": "credential_unavailable",
            "provider": {
                "provider": provider,
                "model": args.model,
                "available": False,
                "credential_environment": credential_key,
                "declared_package_identity": args.runner_package_identity,
                "expected_version": args.expected_runner_version,
            },
            "results": [],
        }
        (output_root / "summary.json").write_text(json.dumps(unavailable, indent=2, sort_keys=True) + "\n")
        return 3
    identity = runner_identity(
        runner,
        provider,
        args.model,
        args.expected_runner_version,
        args.runner_package_identity,
    )
    if not identity["available"]:
        unavailable = {
            "schema": "casegraphen.eval.fresh_agent_run.v0",
            "status": "provider_unavailable",
            "provider": identity,
            "results": [],
        }
        (output_root / "summary.json").write_text(json.dumps(unavailable, indent=2, sort_keys=True) + "\n")
        return 3
    if not identity["version_matches"]:
        mismatch = {
            "schema": "casegraphen.eval.fresh_agent_run.v0",
            "status": "runner_version_mismatch",
            "provider": identity,
            "results": [],
        }
        (output_root / "summary.json").write_text(json.dumps(mismatch, indent=2, sort_keys=True) + "\n")
        return 3
    results = [
        run_scenario(scenario, runner, output_root, args.timeout, args.casegraphen_bin, identity, args.model)
        for scenario in scenarios
    ]
    total_cost_usd, cost_observable = observed_cost_usd(results)
    summary = {
        "schema": "casegraphen.eval.fresh_agent_run.v0",
        "status": "completed",
        "provider": identity,
        "manifest_hash": sha256_bytes(args.manifest.resolve().read_bytes()),
        "budget": {
            "maximum_usd": args.budget_usd,
            "observed_usd": total_cost_usd if cost_observable else None,
            "observable": cost_observable,
            "per_scenario_timeout_seconds": args.timeout,
        },
        "results": results,
    }
    summary["content_hash"] = sha256_bytes(
        json.dumps(summary, separators=(",", ":"), sort_keys=True).encode()
    )
    (output_root / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    failed = any(
        result["returncode"] != 0
        or result["timed_out"]
        or any(item["status"] in {"fail", "unavailable"} for item in result["evaluations"])
        for result in results
    )
    if args.budget_usd is not None and cost_observable and total_cost_usd > args.budget_usd:
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
