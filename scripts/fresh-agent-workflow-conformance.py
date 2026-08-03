#!/usr/bin/env python3
"""Fail closed when the opt-in provider workflow weakens credential isolation."""

from __future__ import annotations

import argparse
import pathlib
import re
import json

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_WORKFLOW = ROOT / ".github/workflows/fresh-agent-release-eval.yml"
DEFAULT_POLICY = ROOT / "evals/fresh-agent/release-policy.v0.json"


def validate(text: str, pins: dict[str, dict[str, str]]) -> list[str]:
    errors: list[str] = []
    if re.search(r"(?m)^    env:\s*$", text):
        errors.append("provider secrets must not be declared at job scope")
    if any(re.search(rf"(?m)^  {event}:\s*$", text) for event in ("push", "pull_request", "pull_request_target", "schedule")):
        errors.append("fresh-agent provider evaluation must remain workflow_dispatch-only")
    if "permissions:\n      contents: read" not in text:
        errors.append("provider job must have explicit read-only repository permission")

    expected = {
        "OPENAI_API_KEY": "Execute the Codex provider lane",
        "ANTHROPIC_API_KEY": "Execute the Claude provider lane",
    }
    step_blocks = re.split(r"(?m)(?=^      - name: )", text)
    for secret, step_name in expected.items():
        reference = "${{ secrets." + secret + " }}"
        if text.count(reference) != 1:
            errors.append(f"{secret} must occur exactly once")
            continue
        block = next((candidate for candidate in step_blocks if reference in candidate), "")
        if step_name not in block:
            errors.append(f"{secret} must be scoped to {step_name}")
        other = next(value for value in expected if value != secret)
        if "${{ secrets." + other + " }}" in block:
            errors.append(f"{step_name} must not receive {other}")

    for provider in ("codex", "claude"):
        pin = pins.get(provider, {})
        package = pin.get("package_identity", "")
        version = pin.get("version", "")
        if not package or not re.fullmatch(r"@[^/]+/[^@]+@[0-9]+\.[0-9]+\.[0-9]+", package):
            errors.append(f"release policy has no exact package pin for {provider}")
            continue
        if not version or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
            errors.append(f"release policy has no exact version pin for {provider}")
            continue
        if f"npm install --global {package}" not in text:
            errors.append(f"runner package is not pinned: {package}")
        for argument in (
            f"--expected-runner-version '{version}'",
            f"--runner-package-identity '{package}'",
        ):
            if argument not in text:
                errors.append(f"runner identity is not retained: {argument}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workflow", type=pathlib.Path, default=DEFAULT_WORKFLOW)
    parser.add_argument("--policy", type=pathlib.Path, default=DEFAULT_POLICY)
    args = parser.parse_args()
    policy = json.loads(args.policy.resolve().read_text())
    errors = validate(args.workflow.resolve().read_text(), policy.get("runner_pins", {}))
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    print("fresh-agent workflow credential boundary conforms")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
