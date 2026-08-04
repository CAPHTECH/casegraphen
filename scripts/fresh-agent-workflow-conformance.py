#!/usr/bin/env python3
"""Fail closed when fresh-agent workflow weakens CLI-session isolation."""

from __future__ import annotations

import argparse
import pathlib
import re
import json

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_WORKFLOW = ROOT / ".github/workflows/fresh-agent-release-eval.yml"
DEFAULT_ATTESTATION_WORKFLOW = ROOT / ".github/workflows/fresh-agent-host-attest.yml"
DEFAULT_FINALIZATION_WORKFLOW = ROOT / ".github/workflows/fresh-agent-release-finalize.yml"
DEFAULT_POLICY = ROOT / "evals/fresh-agent/release-policy.v0.json"
RUST_TOOLCHAIN = re.search(
    r'^channel\s*=\s*"([^"]+)"',
    (ROOT / "rust-toolchain.toml").read_text(),
    re.MULTILINE,
).group(1)


def validate(text: str, pins: dict[str, dict[str, str]]) -> list[str]:
    errors: list[str] = []
    if "${{ secrets." in text or "API_KEY" in text:
        errors.append("provider workflow must not inject API keys or GitHub secrets")
    if any(re.search(rf"(?m)^  {event}:\s*$", text) for event in ("push", "pull_request", "pull_request_target", "schedule")):
        errors.append("fresh-agent provider evaluation must remain workflow_dispatch-only")
    if "permissions:\n      contents: read" not in text:
        errors.append("provider job must have explicit read-only repository permission")
    if text.count("persist-credentials: false") < 2:
        errors.append("checkout credentials must not persist into provider or aggregate worktrees")
    lines = text.splitlines()
    run_blocks: list[str] = []
    for index, line in enumerate(lines):
        match = re.match(r"^(\s*)run:\s*\|\s*$", line)
        if match is None:
            continue
        indentation = len(match.group(1))
        content: list[str] = []
        for following in lines[index + 1 :]:
            following_indentation = len(following) - len(following.lstrip())
            if following.strip() and following_indentation <= indentation:
                break
            content.append(following)
        run_blocks.append("\n".join(content))
    if any("${{ inputs." in block for block in run_blocks):
        errors.append("workflow inputs must reach shell commands only through step environment variables")

    evaluate_match = re.search(r"(?ms)^  evaluate:\s*\n(.*?)(?=^  aggregate:)", text)
    evaluate = evaluate_match.group(1) if evaluate_match else ""
    if not evaluate:
        errors.append("workflow is missing the provider evaluation job")
    if any(token in evaluate for token in ("actions/checkout@", "cargo build", "pip install")):
        errors.append("credentialed provider runners must consume only the prepared evaluator artifact")
    if "needs: prepare" not in evaluate:
        errors.append("provider evaluation must depend on the uncredentialed prepare job")
    if "if: github.ref == 'refs/heads/main'" not in evaluate:
        errors.append("provider evaluation must refuse non-main workflow dispatch refs")
    if "environment: fresh-agent-cli-session-${{ matrix.provider }}" not in evaluate:
        errors.append("provider evaluation must use provider-specific protected environments")
    if text.count("python3 fresh-agent-bundle/scripts/fresh-agent-eval.py") != 2:
        errors.append("both provider lanes must execute the prepared evaluator artifact")
    for retained_script in ("fresh-agent-host-attest.py", "fresh-agent-release.py"):
        if f"cp scripts/{retained_script} fresh-agent-bundle/scripts/{retained_script}" not in text:
            errors.append(f"immutable evaluator bundle must retain {retained_script}")
    evaluator_upload = re.search(
        r"(?ms)name: fresh-agent-evaluator-\$\{\{ github\.sha \}\}.*?retention-days:\s*(\d+)",
        text,
    )
    if evaluator_upload is None or evaluator_upload.group(1) != "90":
        errors.append("immutable evaluator bundle must survive the 90-day review lifecycle")
    if text.count('--casegraphen-bin "$GITHUB_WORKSPACE/fresh-agent-bundle/bin/casegraphen"') != 2:
        errors.append("both provider lanes must pass an absolute prepared casegraphen binary path")
    if text.count('--model "$CASEGRAPHEN_MODEL"') != 2 or text.count(
        '--budget-usd "$CASEGRAPHEN_BUDGET_USD"'
    ) != 2:
        errors.append("model and budget inputs must be quoted argv values from the step environment")

    for action, reference in re.findall(r"(?m)^\s*-?\s*uses:\s+([^@\s]+)@([^\s]+)\s*$", text):
        if not re.fullmatch(r"[0-9a-f]{40}", reference):
            errors.append(f"workflow action must use an immutable commit SHA: {action}@{reference}")
    rust_action = re.findall(
        r"(?m)^\s*- uses: dtolnay/rust-toolchain@[0-9a-f]{40}\s*\n"
        r"\s+with:\s*\n\s+toolchain:\s*([^\s#]+)",
        text,
    )
    if rust_action != [RUST_TOOLCHAIN]:
        errors.append("SHA-pinned rust-toolchain action must declare the repository toolchain input")

    expected_runs_on = 'runs-on: [self-hosted, linux, x64, casegraphen-fresh-agent, "${{ matrix.runner_label }}"]'
    if expected_runs_on not in text:
        errors.append("provider evaluation must use the labeled self-hosted CLI-session runner")

    matrix_pairs = re.findall(
        r"(?m)^\s{10}- provider:\s*([^\s]+)\s*\n\s{12}runner_label:\s*([^\s]+)\s*$",
        text,
    )
    expected_pairs: list[tuple[str, str]] = []
    for provider in ("codex", "claude"):
        pin = pins.get(provider, {})
        package = pin.get("package_identity", "")
        version = pin.get("version", "")
        auth_mode = pin.get("authentication_mode", "")
        auth_classes = pin.get("allowed_authentication_classifications", [])
        attestation_key_id = pin.get("host_attestation_key_id", "")
        runner_label = pin.get("self_hosted_runner_label", "")
        expected_pairs.append((provider, runner_label))
        if not package or not re.fullmatch(r"@[^/]+/[^@]+@[0-9]+\.[0-9]+\.[0-9]+", package):
            errors.append(f"release policy has no exact package pin for {provider}")
            continue
        if not version or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
            errors.append(f"release policy has no exact version pin for {provider}")
            continue
        if f"npm install --global {package}" in text:
            errors.append(f"authenticated runner CLI must be pre-provisioned, not installed in-job: {package}")
        if auth_mode != "cli_session":
            errors.append(f"release policy must require cli_session authentication for {provider}")
        if not auth_classes or any("api" in value.casefold() for value in auth_classes):
            errors.append(f"release policy must allow only explicit non-API CLI sessions for {provider}")
        if not attestation_key_id:
            errors.append(f"release policy must pin a host attestation key id for {provider}")
        if not runner_label:
            errors.append(f"release policy is missing the authenticated runner label for {provider}")
        for argument in (
            f"--expected-runner-version '{version}'",
            f"--runner-package-identity '{package}'",
            "--auth-mode cli-session",
        ):
            if argument not in text:
                errors.append(f"runner identity is not retained: {argument}")
    if sorted(matrix_pairs) != sorted(expected_pairs) or len(matrix_pairs) != len(expected_pairs):
        errors.append("provider matrix must exactly bind each provider to its policy-owned runner label")
    return errors


def immutable_actions(text: str, workflow: str) -> list[str]:
    return [
        f"{workflow} action must use an immutable commit SHA: {action}@{reference}"
        for action, reference in re.findall(
            r"(?m)^\s*-?\s*uses:\s+([^@\s]+)@([^\s]+)\s*$", text
        )
        if not re.fullmatch(r"[0-9a-f]{40}", reference)
    ]


def validate_evidence_lifecycle(attestation: str, finalization: str) -> list[str]:
    errors: list[str] = []
    for name, text in (("attestation", attestation), ("finalization", finalization)):
        if any(
            re.search(rf"(?m)^  {event}:\s*$", text)
            for event in ("push", "pull_request", "pull_request_target", "schedule")
        ):
            errors.append(f"{name} workflow must remain workflow_dispatch-only")
        if "if: github.ref == 'refs/heads/main'" not in text:
            errors.append(f"{name} workflow must refuse non-main dispatch refs")
        if "API_KEY" in text:
            errors.append(f"{name} workflow must not use provider API keys")
        errors.extend(immutable_actions(text, name))

    for provider in ("codex", "claude"):
        if f"environment: fresh-agent-attestation-{provider}" not in attestation:
            errors.append(f"{provider} attestation must use its protected broker environment")
        expected_runner = (
            "runs-on: [self-hosted, linux, x64, casegraphen-fresh-agent-broker, "
            f"casegraphen-{provider}-attestation-broker]"
        )
        if expected_runner not in attestation:
            errors.append(f"{provider} attestation must use its dedicated broker runner")
        for artifact in (
            "fresh-agent-evaluator-${{ inputs.evaluated_commit_sha }}",
            f"fresh-agent-{provider}-${{{{ inputs.evaluated_commit_sha }}}}",
        ):
            if artifact not in attestation:
                errors.append(f"{provider} attestation must download exact artifact {artifact}")
        if f"--provider {provider}" not in attestation:
            errors.append(f"{provider} broker must invoke the canonical host attester")
        if f"fresh-agent-host-attestation-{provider}-${{{{ inputs.evaluated_commit_sha }}}}" not in attestation:
            errors.append(f"{provider} attestation artifact must bind the evaluated commit")
    if attestation.count("${{ secrets.CASEGRAPHEN_ATTESTATION_KEY }}") != 2:
        errors.append("each broker lane must receive exactly its protected attestation key")
    if attestation.count("${{ vars.CASEGRAPHEN_RUNNER_INSTANCE_ID_HASH }}") != 2:
        errors.append("each broker lane must bind its protected runner instance identity")
    if attestation.count('[[ "$EVIDENCE_RUN_ID" =~ ^[1-9][0-9]*$ ]]') != 2 or attestation.count(
        '[[ "$EVALUATED_COMMIT_SHA" =~ ^[0-9a-f]{40}$ ]]'
    ) != 2:
        errors.append("broker lanes must validate numeric run ids and exact commit hashes")
    if "fresh-agent-eval.py" in attestation:
        errors.append("broker workflow must never execute provider evaluation")

    if "environment: fresh-agent-release-verifier" not in finalization:
        errors.append("finalization must use the protected release-verifier environment")
    if "runs-on: ubuntu-latest" not in finalization:
        errors.append("finalization must use an ephemeral hosted verifier")
    for provider in ("codex", "claude"):
        for artifact in (
            f"fresh-agent-{provider}-${{{{ inputs.evaluated_commit_sha }}}}",
            f"fresh-agent-host-attestation-{provider}-${{{{ inputs.evaluated_commit_sha }}}}",
        ):
            if artifact not in finalization:
                errors.append(f"finalization must download exact artifact {artifact}")
        for argument in (
            f"--provider-run provider-runs/{provider}",
            f"--host-attestation {provider}=host-attestations/{provider}/{provider}.json",
            f"--attestation-key {provider}=\"$key_dir/{provider}.key\"",
        ):
            if argument not in finalization:
                errors.append(f"finalization is missing canonical aggregate argument: {argument}")
    if "docs/evals/fresh-agent/reviews/*.json" not in finalization:
        errors.append("manual review path must be confined to the reviewed repository directory")
    if '--manual-review "$CASEGRAPHEN_MANUAL_REVIEW_PATH"' not in finalization:
        errors.append("finalization must pass the exact reviewer-authored document")
    if finalization.count("${{ secrets.CASEGRAPHEN_") != 2:
        errors.append("release verifier must receive exactly the two HMAC verification keys")
    if "if: steps.aggregate.outcome != 'success'" not in finalization:
        errors.append("finalization must fail closed when the strict aggregate does not pass")
    if "retention-days: 90" not in finalization:
        errors.append("final release evidence must be retained durably")
    for coordinate in (
        "EVIDENCE_RUN_ID",
        "CODEX_ATTESTATION_RUN_ID",
        "CLAUDE_ATTESTATION_RUN_ID",
    ):
        if f'[[ "${coordinate}" =~ ^[1-9][0-9]*$ ]]' not in finalization:
            errors.append(f"finalization must validate numeric coordinate {coordinate}")
    if '[[ "$EVALUATED_COMMIT_SHA" =~ ^[0-9a-f]{40}$ ]]' not in finalization:
        errors.append("finalization must validate the exact evaluated commit hash")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workflow", type=pathlib.Path, default=DEFAULT_WORKFLOW)
    parser.add_argument(
        "--attestation-workflow", type=pathlib.Path, default=DEFAULT_ATTESTATION_WORKFLOW
    )
    parser.add_argument(
        "--finalization-workflow", type=pathlib.Path, default=DEFAULT_FINALIZATION_WORKFLOW
    )
    parser.add_argument("--policy", type=pathlib.Path, default=DEFAULT_POLICY)
    args = parser.parse_args()
    policy = json.loads(args.policy.resolve().read_text())
    errors = validate(args.workflow.resolve().read_text(), policy.get("runner_pins", {}))
    errors.extend(
        validate_evidence_lifecycle(
            args.attestation_workflow.resolve().read_text(),
            args.finalization_workflow.resolve().read_text(),
        )
    )
    if errors:
        for error in errors:
            print(f"FAIL {error}")
        return 1
    print("fresh-agent workflow CLI-session boundary conforms")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
