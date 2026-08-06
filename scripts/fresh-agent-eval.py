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
import secrets as secrets_module
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "evals/fresh-agent/scenarios.v0.json"
ORCHESTRATION_MANIFEST = ROOT / "evals/fresh-agent/skill-orchestration-scenarios.v0.json"
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
REQUIRED_ORCHESTRATION_SCENARIOS = {
    "direct-design-only",
    "direct-audit-only",
    "native-case-lifecycle",
    "external-jsonl-lifecycle",
    "end-to-end-two-review-seams",
    "must-stop-for-authority",
}
EVALUATOR_KINDS = {"graph_lint", "json_schema", "completeness_oracle", "json_assert"}
RUNNER_PROFILES = {
    "codex": [
        "codex",
        "exec",
        "--sandbox",
        "workspace-write",
        "--ignore-user-config",
        "--ephemeral",
        "--strict-config",
        "--skip-git-repo-check",
        "--color",
        "never",
        "-",
    ],
    "claude": [
        "claude",
        "--print",
        "--permission-mode",
        "acceptEdits",
        "--tools",
        "Read,Write,Edit",
        "--setting-sources",
        "project",
        "--disable-slash-commands",
        "--strict-mcp-config",
        "--output-format",
        "stream-json",
        "--verbose",
    ],
}
PROFILE_AUTH_STATUS_ARGS = {
    "codex": ["login", "status"],
    "claude": ["auth", "status", "--json"],
}
SECRET_MARKERS = (
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "API_KEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "COOKIE",
    "AUTHORIZATION",
    "BEARER",
    "CREDENTIAL",
)
CLI_SESSION_ENVIRONMENT_ALLOWLIST = {
    "HOME",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LOGNAME",
    "NO_COLOR",
    "PATH",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
}
CLAUDE_NON_API_SESSION_METHODS = {
    "claude.ai": "claude_subscription_session",
    "oauth": "claude_oauth_session",
    "subscription": "claude_subscription_session",
}


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


def cli_session_environment(workspace: pathlib.Path | None = None) -> dict[str, str]:
    """Expose only process basics needed by a pre-provisioned CLI session.

    HOME remains because provider CLIs may need their session store. The
    dedicated OS account/credential broker is therefore an external trust
    boundary; arbitrary ambient configuration and agent sockets are excluded.
    """
    environment = {
        key: value
        for key, value in os.environ.items()
        if key in CLI_SESSION_ENVIRONMENT_ALLOWLIST and value
    }
    environment["GIT_CONFIG_GLOBAL"] = os.devnull
    environment["GIT_CONFIG_NOSYSTEM"] = "1"
    environment["GIT_TERMINAL_PROMPT"] = "0"
    if workspace is not None:
        environment["CASEGRAPHEN_EVAL_WORKSPACE"] = str(workspace)
    return environment


def classify_cli_session(provider: str, output: str) -> str | None:
    """Return a non-API-key session class, or None for every unknown shape."""
    if provider == "claude":
        try:
            value = json.loads(output)
        except (json.JSONDecodeError, TypeError):
            return None
        if not isinstance(value, dict) or value.get("loggedIn") is not True:
            return None
        method = value.get("authMethod")
        return CLAUDE_NON_API_SESSION_METHODS.get(method) if isinstance(method, str) else None
    if provider == "codex":
        normalized = re.sub(r"\x1b\[[0-9;]*m", "", output).strip().casefold()
        lines = {line.strip() for line in normalized.splitlines() if line.strip()}
        if lines & {"logged in using chatgpt", "logged in using chatgpt account"}:
            return "codex_chatgpt_session"
        return None
    return None


def cli_session_status(command: list[str], provider: str) -> dict[str, Any]:
    """Ask the pinned CLI whether its disk-backed session is authenticated.

    Probe output exists only in process memory long enough to classify an exact
    non-API-key session shape. Raw output and account metadata are never retained.
    """
    executable = shutil.which(command[0])
    args = PROFILE_AUTH_STATUS_ARGS[provider]
    probe = [executable or command[0], *args]
    try:
        process = subprocess.run(
            probe,
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
            env=cli_session_environment(),
        )
        probe_output = (
            process.stdout
            if provider == "claude"
            else f"{process.stdout}\n{process.stderr}"
        )
        classification = (
            classify_cli_session(provider, probe_output)
            if executable is not None and process.returncode == 0
            else None
        )
        available = classification is not None
        exit_code = process.returncode
    except (FileNotFoundError, subprocess.TimeoutExpired):
        available = False
        exit_code = None
        classification = None
    return {
        "mode": "cli_session",
        "available": available,
        "classification": classification,
        "non_api_key_session_verified": classification is not None,
        "status_exit_code": exit_code,
        "probe_command_hash": sha256_bytes(json.dumps(args, separators=(",", ":")).encode()),
        "probe_output_retained": False,
        "child_environment_policy": "allowlisted_cli_session_environment_v1",
        "credential_boundary": "dedicated_provider_os_account_or_broker_required",
    }


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
            # Version discovery needs no provider authority. Removing
            # secret-like environment values also prevents a compromised probe
            # from echoing them into retained runner identity evidence.
            env=cli_session_environment(),
        )
        version_text = (version.stdout or version.stderr).strip()
        version_exit_code = version.returncode
    except subprocess.TimeoutExpired:
        version_text = "version probe timed out"
        version_exit_code = None
    return {
        "provider": provider,
        "model": model,
        "available": True,
        "executable": str(pathlib.Path(executable).resolve()),
        "version": version_text,
        "version_probe_exit_code": version_exit_code,
        "expected_version": expected_version,
        "version_matches": version_exit_code == 0
        and (
            expected_version is None
            or re.search(
                rf"(?<![0-9.]){re.escape(expected_version)}(?![0-9.])", version_text
            )
            is not None
        ),
        "declared_package_identity": package_identity,
        "command_hash": sha256_bytes(json.dumps(command, separators=(",", ":")).encode()),
    }


def secret_values(environment: dict[str, str]) -> list[str]:
    return [value for key, value in environment.items() if value and is_secret_key(key)]


def credential_canary_values() -> list[str]:
    """Read an operator-owned disk canary without exposing its path to children."""
    raw_path = os.environ.get("CASEGRAPHEN_EVAL_CREDENTIAL_CANARY_FILE")
    if not raw_path:
        return []
    path = pathlib.Path(raw_path)
    try:
        value = path.read_text()
    except (OSError, UnicodeError):
        return []
    return [value] if value and len(value.encode()) <= 4096 else []


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


def observed_models(results: list[dict[str, Any]], declared: str | None) -> dict[str, Any]:
    reported = sorted(
        {
            observation["model"]
            for result in results
            for observation in result.get("usage_observations", [])
            if isinstance(observation.get("model"), str) and observation["model"]
        }
    )
    return {
        "observable": bool(reported),
        "reported_models": reported,
        "matches_declared": bool(reported) and reported == [declared],
    }


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


def manifest_scenario_ids(path: pathlib.Path) -> set[str]:
    resolved = path.resolve()
    if resolved == DEFAULT_MANIFEST.resolve():
        return REQUIRED_SCENARIOS
    if resolved == ORCHESTRATION_MANIFEST.resolve():
        return REQUIRED_ORCHESTRATION_SCENARIOS
    raise ValueError("manifest must be a shipped, named fresh-agent suite")


def validate_manifest(manifest: dict[str, Any], required_scenarios: set[str]) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema") != "casegraphen.eval.fresh_agent_manifest.v0":
        errors.append("unsupported manifest schema")
    if manifest.get("version") != 0:
        errors.append("manifest version must be 0")
    scenarios = manifest.get("scenarios")
    if not isinstance(scenarios, list):
        return errors + ["scenarios must be an array"]
    ids = [scenario.get("id") for scenario in scenarios if isinstance(scenario, dict)]
    if set(ids) != required_scenarios or len(ids) != len(required_scenarios):
        errors.append("manifest must contain each required scenario id exactly once")
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
        if identity["provider"] in PROFILE_AUTH_STATUS_ARGS:
            environment = cli_session_environment(workspace)
        else:
            environment = {**os.environ, "CASEGRAPHEN_EVAL_WORKSPACE": str(workspace)}
            environment.pop("CASEGRAPHEN_EVAL_CREDENTIAL_CANARY_FILE", None)
        environment["CASEGRAPHEN_EVAL_SKILL"] = str(workspace / "skill" / scenario["skill"])
        # Scan against the parent environment too: secret-like values must
        # neither reach the provider process nor survive in retained evidence.
        secrets = list(dict.fromkeys(secret_values(dict(os.environ)) + credential_canary_values()))
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
        output_match_detected = any(secret in stdout or secret in stderr for secret in secrets)
        stdout = redact(stdout, secrets)
        stderr = redact(stderr, secrets)
        destination = output_root / scenario["id"]
        destination.mkdir(parents=True, exist_ok=False)
        (destination / "raw.stdout").write_text(stdout)
        (destination / "raw.stderr").write_text(stderr)
        (destination / "prompt.md").write_text(prompt_file.read_text())
        evaluation = evaluate(scenario, workspace, casegraphen_bin)
        affected = files_containing_secrets(workspace, secrets)
        workspace_retained = not affected and not output_match_detected
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
                "output_match_detected": output_match_detected,
                "disk_canary_configured": bool(credential_canary_values()),
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
    parser.add_argument("--auth-mode", choices=["cli-session"])
    parser.add_argument("--model", help="provider model id; recorded exactly as supplied")
    parser.add_argument("--expected-runner-version", help="exact version required for a real runner profile")
    parser.add_argument("--runner-package-identity", help="exact pinned package identity retained with evidence")
    parser.add_argument("--output-dir", type=pathlib.Path)
    parser.add_argument("--scenario", action="append", default=[])
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--budget-usd", type=float, help="declared aggregate release-run budget")
    parser.add_argument("--casegraphen-bin", default="casegraphen")
    args = parser.parse_args()
    manifest_path = args.manifest.resolve()
    try:
        required_scenarios = manifest_scenario_ids(manifest_path)
    except ValueError as error:
        parser.error(str(error))
    manifest = load_manifest(manifest_path)
    errors = validate_manifest(manifest, required_scenarios)
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
    unknown = selected - required_scenarios
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
    if args.runner_profile and not args.model:
        parser.error("real runner profiles require an exact --model id")
    if args.runner_profile and args.auth_mode != "cli-session":
        parser.error("real runner profiles require --auth-mode cli-session")
    if args.runner_profile:
        policy = json.loads(DEFAULT_RELEASE_POLICY.read_text())
        pin = policy["runner_pins"][args.runner_profile]
        if args.expected_runner_version != pin["version"] or args.runner_package_identity != pin["package_identity"]:
            parser.error("real runner identity must exactly match evals/fresh-agent/release-policy.v0.json")
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
    if args.runner_profile:
        authentication = cli_session_status(runner, provider)
        identity["authentication"] = authentication
        if not authentication["available"]:
            unavailable = {
                "schema": "casegraphen.eval.fresh_agent_run.v0",
                "status": "cli_session_unavailable",
                "provider": identity,
                "results": [],
            }
            (output_root / "summary.json").write_text(
                json.dumps(unavailable, indent=2, sort_keys=True) + "\n"
            )
            return 3
    results = [
        run_scenario(scenario, runner, output_root, args.timeout, args.casegraphen_bin, identity, args.model)
        for scenario in scenarios
    ]
    total_cost_usd, cost_observable = observed_cost_usd(results)
    model_observation = observed_models(results, args.model)
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
        "model_observation": model_observation,
        "results": results,
        "host_attestation_challenge": secrets_module.token_hex(32),
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
    if model_observation["observable"] and not model_observation["matches_declared"]:
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
