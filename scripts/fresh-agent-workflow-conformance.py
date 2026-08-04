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
DEFAULT_MANUAL_REVIEW_WORKFLOW = ROOT / ".github/workflows/fresh-agent-manual-review-sign.yml"
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
    if "github.sha == vars.CASEGRAPHEN_TRUSTED_VERIFIER_SHA" not in evaluate:
        errors.append("provider evaluation must run only at the protected trusted verifier SHA")
    if "environment: fresh-agent-cli-session-${{ matrix.provider }}" not in evaluate:
        errors.append("provider evaluation must use provider-specific protected environments")
    if text.count("python3 fresh-agent-bundle/scripts/fresh-agent-eval.py") != 2:
        errors.append("both provider lanes must execute the prepared evaluator artifact")
    for privileged_script in ("fresh-agent-host-attest.py", "fresh-agent-release.py"):
        if f"cp scripts/{privileged_script} fresh-agent-bundle" in text:
            errors.append(
                f"evaluated artifact must not carry privileged verifier code: {privileged_script}"
            )
    evaluator_upload = re.search(
        r"(?ms)name: fresh-agent-evaluator-\$\{\{ github\.sha \}\}.*?retention-days:\s*(\d+)",
        text,
    )
    if evaluator_upload is None or evaluator_upload.group(1) != "90":
        errors.append("immutable evaluator bundle must survive the 90-day review lifecycle")
    for artifact_template in (
        "fresh-agent-evaluator-${{ github.sha }}-run-${{ github.run_id }}-attempt-${{ github.run_attempt }}",
        "fresh-agent-${{ matrix.provider }}-${{ github.sha }}-run-${{ github.run_id }}-attempt-${{ github.run_attempt }}",
        "fresh-agent-evaluation-host-proof-${{ matrix.provider }}-${{ github.sha }}-run-${{ github.run_id }}-attempt-${{ github.run_attempt }}",
    ):
        if artifact_template not in text:
            errors.append("evaluation artifacts must bind exact workflow run id and run attempt")
    if text.count("$CASEGRAPHEN_EVALUATION_HOST_ATTESTOR\" attest-fresh-agent") != 1:
        errors.append("provider runner must invoke exactly one externally provisioned host attestor")
    for token in (
        "${{ steps.provider_artifact.outputs.artifact-id }}",
        "${{ steps.provider_artifact.outputs.artifact-digest }}",
        '--provider-artifact-digest "$PROVIDER_ARTIFACT_DIGEST"',
        '--source-run-attempt "$GITHUB_RUN_ATTEMPT"',
        '--source-head-sha "$GITHUB_SHA"',
        '--summary "fresh-agent-$PROVIDER/summary.json"',
    ):
        if token not in text:
            errors.append(f"evaluation-host proof is missing source binding: {token}")
    if "CASEGRAPHEN_EVALUATION_HOST_ATTESTOR: ${{ vars.CASEGRAPHEN_EVALUATION_HOST_ATTESTOR }}" not in text:
        errors.append("evaluation-host attestor must be provisioned by the protected runner environment")
    if text.count('--casegraphen-bin "$GITHUB_WORKSPACE/fresh-agent-bundle/bin/casegraphen"') != 2:
        errors.append("both provider lanes must pass an absolute prepared casegraphen binary path")
    if text.count('--model "$CASEGRAPHEN_MODEL"') != 2 or text.count(
        '--budget-usd "$CASEGRAPHEN_BUDGET_USD"'
    ) != 2:
        errors.append("model and budget inputs must be quoted argv values from the step environment")
    review_seam = re.search(
        r"(?ms)- name: Record the independent review seam\s+if: steps\.aggregate\.outcome != 'success'\s+run: \|(.*?)(?=\n\s*- name:|\Z)",
        text,
    )
    if review_seam is None or re.search(r"(?m)^\s*exit\s+[1-9]", review_seam.group(1)):
        errors.append(
            "a complete provider run must remain successful while waiting at the independent review seam"
        )

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


def validate_evidence_lifecycle(
    attestation: str, finalization: str, manual_review: str
) -> list[str]:
    errors: list[str] = []
    for name, text in (
        ("attestation", attestation),
        ("finalization", finalization),
        ("manual-review", manual_review),
    ):
        if any(
            re.search(rf"(?m)^  {event}:\s*$", text)
            for event in ("push", "pull_request", "pull_request_target", "schedule")
        ):
            errors.append(f"{name} workflow must remain workflow_dispatch-only")
        if "github.ref == 'refs/heads/main'" not in text:
            errors.append(f"{name} workflow must refuse non-main dispatch refs")
        if "github.sha == vars.CASEGRAPHEN_TRUSTED_VERIFIER_SHA" not in text:
            errors.append(f"{name} privileged workflow YAML must equal the protected trusted SHA")
        if "API_KEY" in text:
            errors.append(f"{name} workflow must not use provider API keys")
        errors.extend(immutable_actions(text, name))

    for name, text in (("attestation", attestation), ("finalization", finalization), ("manual-review", manual_review)):
        if "ref: ${{ vars.CASEGRAPHEN_TRUSTED_VERIFIER_SHA }}" not in text:
            errors.append(f"{name} must checkout the protected exact trusted verifier SHA")
        if "persist-credentials: false" not in text:
            errors.append(f"{name} trusted verifier checkout must not persist credentials")
        if "trusted-verifier/scripts/" not in text:
            errors.append(f"{name} must execute only trusted verifier source")
        if "fresh-agent-bundle/scripts/fresh-agent-host-attest.py" in text or "fresh-agent-bundle/scripts/fresh-agent-release.py" in text:
            errors.append(f"{name} must never execute privileged code from the evaluated artifact")

    for provider in ("codex", "claude"):
        if f"environment: fresh-agent-attestation-{provider}" not in attestation:
            errors.append(f"{provider} attestation must use its protected broker environment")
        expected_runner = (
            "runs-on: [self-hosted, linux, x64, casegraphen-fresh-agent-broker, "
            f"casegraphen-{provider}-attestation-broker]"
        )
        if expected_runner not in attestation:
            errors.append(f"{provider} attestation must use its dedicated broker runner")
        artifact = (
            f"fresh-agent-{provider}-${{{{ inputs.evaluated_commit_sha }}}}"
            "-run-${{ inputs.evidence_run_id }}-attempt-${{ inputs.evidence_run_attempt }}"
        )
        if artifact not in attestation:
            errors.append(f"{provider} attestation must download exact artifact {artifact}")
        proof_artifact = (
            f"fresh-agent-evaluation-host-proof-{provider}-${{{{ inputs.evaluated_commit_sha }}}}"
            "-run-${{ inputs.evidence_run_id }}-attempt-${{ inputs.evidence_run_attempt }}"
        )
        if proof_artifact not in attestation:
            errors.append(f"{provider} broker must observe exact evaluation-host proof artifact")
        if f"--provider {provider}" not in attestation:
            errors.append(f"{provider} broker must invoke the canonical host attester")
        if f"fresh-agent-host-attestation-{provider}-${{{{ inputs.evaluated_commit_sha }}}}-source-" not in attestation:
            errors.append(f"{provider} attestation artifact must bind the evaluated commit")
    if attestation.count("${{ secrets.CASEGRAPHEN_ATTESTATION_PRIVATE_KEY }}") != 2:
        errors.append("each broker lane must receive exactly its protected private signing key")
    if "--private-key-file" not in attestation or "--provenance-file" not in attestation:
        errors.append("broker attestations must use Ed25519 private keys and observed provenance files")
    for argument in (
        "--evaluation-host-proof",
        "--evaluation-host-key-id",
        "--evaluation-host-public-key-spki-sha256",
    ):
        if attestation.count(argument) != 2:
            errors.append(f"each broker lane must verify evaluation-host authority: {argument}")
    if len(re.findall(r"(?m)^\s+--evaluation-host-public-key\s", attestation)) != 2:
        errors.append("each broker lane must verify evaluation-host authority: --evaluation-host-public-key")
    if "--provider-cli" in attestation or "CASEGRAPHEN_RUNNER_INSTANCE_ID_HASH" in attestation:
        errors.append("broker must not substitute its own CLI session or runner identity for host proof")
    if attestation.count("${{ vars.CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY }}") != 2 or attestation.count(
        "${{ vars.CASEGRAPHEN_EVALUATION_HOST_PUBLIC_KEY_SPKI_SHA256 }}"
    ) != 2:
        errors.append("each broker lane must pin its provider-specific evaluation-host key and SPKI fingerprint")
    if "fresh-agent-run-provenance.py observe-run" not in attestation:
        errors.append("broker must independently observe the source GitHub run and artifact")
    if attestation.count("fresh-agent-run-provenance.py observe-run") != 4:
        errors.append("broker must independently observe provider evidence and host-proof artifacts")
    if attestation.count('[[ "$SOURCE_RUN_ID" =~ ^[1-9][0-9]*$ ]]') != 2 or attestation.count(
        '[[ "$EVALUATED_COMMIT_SHA" =~ ^[0-9a-f]{40}$ ]]'
    ) != 2:
        errors.append("broker lanes must validate numeric run ids and exact commit hashes")
    if "fresh-agent-eval.py" in attestation:
        errors.append("broker workflow must never execute provider evaluation")
    for provider in ("codex", "claude"):
        for retained in (
            f"host-attestation/{provider}-evaluation-host-proof.json",
            f"host-attestation/{provider}-evaluation-host-public.pem",
            f"host-attestation/{provider}-evaluation-host-key-provenance.json",
        ):
            if retained not in attestation:
                errors.append(f"{provider} broker artifact must retain {retained}")
    if len(re.findall(r"(?m)^\s+path: host-attestation\s*$", attestation)) != 2:
        errors.append("each broker artifact must upload the complete host-proof directory")

    if "environment: fresh-agent-release-verifier" not in finalization:
        errors.append("finalization must use the protected release-verifier environment")
    if "runs-on: ubuntu-latest" not in finalization:
        errors.append("finalization must use an ephemeral hosted verifier")
    for provider in ("codex", "claude"):
        for artifact in (
            f"fresh-agent-{provider}-${{{{ inputs.evaluated_commit_sha }}}}",
            f"fresh-agent-host-attestation-{provider}-${{{{ inputs.evaluated_commit_sha }}}}-source-",
        ):
            if artifact not in finalization:
                errors.append(f"finalization must download exact artifact {artifact}")
        provider_artifact = (
            f"fresh-agent-{provider}-${{{{ inputs.evaluated_commit_sha }}}}"
            "-run-${{ inputs.evidence_run_id }}-attempt-${{ inputs.evidence_run_attempt }}"
        )
        if provider_artifact not in finalization:
            errors.append(f"finalization must bind {provider} evidence to exact run attempt")
        for argument in (
            f"--provider-run provider-runs/{provider}",
            f"--host-attestation {provider}=host-attestations/{provider}/{provider}.json",
            f"--attestation-public-key {provider}=\"$key_dir/{provider}.pem\"",
            f"--expected-provenance {provider}=provenance/{provider}.json",
        ):
            if argument not in finalization:
                errors.append(f"finalization is missing canonical aggregate argument: {argument}")
    if "--manual-review signed-manual-review/manual-review.json" not in finalization:
        errors.append("finalization must pass the exact signed reviewer-authored artifact")
    if "--manual-review-public-key" not in finalization or "--expected-reviewer-identity" not in finalization:
        errors.append("finalization must cryptographically verify reviewer identity")
    if "CASEGRAPHEN_ATTESTATION_PRIVATE_KEY" in finalization or "CASEGRAPHEN_REVIEWER_PRIVATE_KEY" in finalization:
        errors.append("release verifier must never receive a private signing key")
    if finalization.count("verify-public-key") != 3 or "SPKI_SHA256" not in finalization:
        errors.append("release verifier must bind every public key to a protected SPKI fingerprint")
    if "if: steps.aggregate.outcome != 'success'" not in finalization:
        errors.append("finalization must fail closed when the strict aggregate does not pass")
    if "retention-days: 90" not in finalization:
        errors.append("final release evidence must be retained durably")
    if '[[ "$value" =~ ^[1-9][0-9]*$ ]]' not in finalization:
        errors.append("finalization must validate every numeric run and attempt coordinate")
    if '[[ "$value" =~ ^[0-9a-f]{40}$ ]]' not in finalization:
        errors.append("finalization must validate the exact evaluated commit hash")
    if finalization.count("fresh-agent-run-provenance.py observe-run") < 5:
        errors.append("finalization must independently observe all source runs and artifacts")
    for source in (
        'test "$CODEX_ATTESTATION_HEAD_SHA" = "$TRUSTED_VERIFIER_SHA"',
        'test "$CLAUDE_ATTESTATION_HEAD_SHA" = "$TRUSTED_VERIFIER_SHA"',
        'test "$MANUAL_REVIEW_HEAD_SHA" = "$TRUSTED_VERIFIER_SHA"',
    ):
        if source not in finalization:
            errors.append("finalization must bind every privileged source workflow to trusted SHA")
    if "record-trusted-source" not in finalization:
        errors.append("final report must retain trusted verifier source and script hashes")
    if "environment: fresh-agent-evidence-publisher" not in finalization or "gh release create" not in finalization:
        errors.append("finalization must publish durable content-addressed release evidence")
    if "gh release download" not in finalization or "verify-file" not in finalization:
        errors.append("durable release evidence must be re-downloaded and hash verified")
    if finalization.count("inspect-release") < 3 or "asset_state" not in finalization:
        errors.append("durable evidence publication must resume safely after create or upload crashes")
    if "verify-trusted-source" not in finalization:
        errors.append("durable publisher must match the finalizer's recorded trusted verifier source")
    if "--clobber" in finalization:
        errors.append("durable evidence publication must never overwrite an existing asset")
    for retained in (
        "authority-material/public-keys/codex.pem",
        "authority-material/public-keys/claude.pem",
        "authority-material/public-keys/reviewer.pem",
        "authority-material/contracts/release-policy.v0.json",
        "authority-material/contracts/release-baseline.v0.json",
        "authority-material/contracts/scenarios.v0.json",
        "authority-material/evaluation-host/codex",
        "authority-material/evaluation-host/claude",
    ):
        if retained not in finalization:
            errors.append(f"durable evidence must retain independent re-verification input: {retained}")

    if "environment: fresh-agent-manual-review-signer" not in manual_review:
        errors.append("manual review must use its protected signing environment")
    if "${{ secrets.CASEGRAPHEN_REVIEWER_PRIVATE_KEY }}" not in manual_review:
        errors.append("manual review signer alone must receive the reviewer private key")
    if "sign-review" not in manual_review or "fresh-agent-signed-manual-review-" not in manual_review:
        errors.append("manual review workflow must publish an Ed25519-signed review artifact")
    if manual_review.count("fresh-agent-run-provenance.py observe-run") != 2:
        errors.append("manual review signer must independently observe both provider artifacts")
    for argument in (
        "--allowed-review-root review-source/docs/evals/fresh-agent/reviews",
        "--expected-provenance codex=review-provenance/codex.json",
        "--expected-provenance claude=review-provenance/claude.json",
    ):
        if argument not in manual_review:
            errors.append(f"manual review signer is missing authority binding: {argument}")
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
    parser.add_argument(
        "--manual-review-workflow", type=pathlib.Path, default=DEFAULT_MANUAL_REVIEW_WORKFLOW
    )
    parser.add_argument("--policy", type=pathlib.Path, default=DEFAULT_POLICY)
    args = parser.parse_args()
    policy = json.loads(args.policy.resolve().read_text())
    errors = validate(args.workflow.resolve().read_text(), policy.get("runner_pins", {}))
    errors.extend(
        validate_evidence_lifecycle(
            args.attestation_workflow.resolve().read_text(),
            args.finalization_workflow.resolve().read_text(),
            args.manual_review_workflow.resolve().read_text(),
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
