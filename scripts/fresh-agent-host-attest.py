#!/usr/bin/env python3
"""Broker-sign a proof emitted by the provider evaluation host.

The broker does not run or inspect the provider CLI. It verifies an opaque,
provider-specific Ed25519 proof created by an externally provisioned attestor
on the runner which performed the evaluation, then binds that proof to GitHub
run provenance observed independently by the broker.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile
from typing import Any


def canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode()


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_object(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(
        path.read_text(),
        object_pairs_hook=reject_duplicate_keys,
        parse_constant=lambda value: (_ for _ in ()).throw(
            ValueError(f"non-finite JSON number: {value}")
        ),
    )
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def summary_hash(summary: dict[str, Any]) -> str:
    value = dict(summary)
    claimed = value.pop("content_hash", None)
    actual = digest(canonical(value))
    if claimed != actual:
        raise ValueError("summary content hash mismatch")
    return actual


def ed25519_sign(private_key: pathlib.Path, payload: bytes) -> str:
    key_type = subprocess.run(
        [
            "openssl",
            "pkey",
            "-in",
            str(private_key),
            "-pubout",
            "-text_pub",
            "-noout",
        ],
        capture_output=True,
        check=False,
    )
    if key_type.returncode != 0 or not key_type.stdout.startswith(b"ED25519 Public-Key:"):
        raise SystemExit("host attestation signing key must be Ed25519")
    with tempfile.TemporaryDirectory(prefix="casegraphen-attestation-sign-") as directory:
        payload_path = pathlib.Path(directory) / "payload"
        signature_path = pathlib.Path(directory) / "signature"
        payload_path.write_bytes(payload)
        process = subprocess.run(
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
            capture_output=True,
            check=False,
        )
        if process.returncode != 0:
            raise SystemExit("Ed25519 signing failed")
        return base64.b64encode(signature_path.read_bytes()).decode("ascii")


def canonical_base64(value: Any) -> bytes:
    if not isinstance(value, str):
        raise SystemExit("evaluation-host proof signature must be base64")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (ValueError, base64.binascii.Error):
        raise SystemExit("evaluation-host proof signature must be canonical base64")
    if base64.b64encode(decoded).decode("ascii") != value:
        raise SystemExit("evaluation-host proof signature must be canonical base64")
    return decoded


def verify_ed25519(
    public_key: pathlib.Path,
    expected_spki_hash: str,
    payload: bytes,
    signature: Any,
) -> None:
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", expected_spki_hash):
        raise SystemExit("evaluation-host public key fingerprint must be SHA-256")
    key_type = subprocess.run(
        ["openssl", "pkey", "-pubin", "-in", str(public_key), "-text_pub", "-noout"],
        capture_output=True,
        check=False,
    )
    if key_type.returncode != 0 or not key_type.stdout.startswith(b"ED25519 Public-Key:"):
        raise SystemExit("evaluation-host proof key must be Ed25519")
    der = subprocess.run(
        ["openssl", "pkey", "-pubin", "-in", str(public_key), "-outform", "DER"],
        capture_output=True,
        check=False,
    )
    if der.returncode != 0 or digest(der.stdout) != expected_spki_hash:
        raise SystemExit("evaluation-host public key fingerprint mismatch")
    with tempfile.TemporaryDirectory(prefix="casegraphen-host-proof-verify-") as directory:
        payload_path = pathlib.Path(directory) / "payload"
        signature_path = pathlib.Path(directory) / "signature"
        payload_path.write_bytes(payload)
        signature_path.write_bytes(canonical_base64(signature))
        verified = subprocess.run(
            [
                "openssl",
                "pkeyutl",
                "-verify",
                "-pubin",
                "-inkey",
                str(public_key),
                "-rawin",
                "-in",
                str(payload_path),
                "-sigfile",
                str(signature_path),
            ],
            capture_output=True,
            check=False,
        )
    if verified.returncode != 0:
        raise SystemExit("evaluation-host proof signature is invalid")


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be a positive integer")
    return parsed


def validated_provenance(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise SystemExit("provenance must be a JSON object")
    required_strings = (
        "evaluated_commit_sha",
        "repository",
        "source_workflow",
        "source_workflow_path",
        "source_head_ref",
        "source_head_sha",
        "source_event",
        "source_conclusion",
    )
    if set(value) != {
        *required_strings,
        "source_workflow_id",
        "source_run_id",
        "source_run_attempt",
        "provider_artifact",
    }:
        raise SystemExit("provenance has missing or unknown fields")
    if any(not isinstance(value.get(field), str) or not value[field].strip() for field in required_strings):
        raise SystemExit("provenance string fields must be non-empty")
    if not re.fullmatch(r"[0-9a-f]{40}", value["evaluated_commit_sha"]):
        raise SystemExit("evaluated commit must be an exact 40-character SHA")
    if value["source_head_sha"] != value["evaluated_commit_sha"]:
        raise SystemExit("source head SHA must equal evaluated commit")
    if not re.fullmatch(r"[^/\s]+/[^/\s]+", value["repository"]):
        raise SystemExit("source repository must use owner/name")
    for field in ("source_workflow_id", "source_run_id", "source_run_attempt"):
        if not isinstance(value.get(field), int) or isinstance(value[field], bool) or value[field] <= 0:
            raise SystemExit(f"{field} must be a positive integer")
    artifact = value.get("provider_artifact")
    if not isinstance(artifact, dict) or set(artifact) != {"id", "name", "digest"}:
        raise SystemExit("provider artifact provenance is invalid")
    if not isinstance(artifact.get("id"), int) or isinstance(artifact["id"], bool) or artifact["id"] <= 0:
        raise SystemExit("provider artifact id must be a positive integer")
    if not isinstance(artifact.get("name"), str) or not artifact["name"].strip():
        raise SystemExit("provider artifact name must be non-empty")
    if not isinstance(artifact.get("digest"), str) or not re.fullmatch(
        r"sha256:[0-9a-f]{64}", artifact["digest"]
    ):
        raise SystemExit("provider artifact digest must be SHA-256")
    return value


def expected_proof_source(provenance: dict[str, Any]) -> dict[str, Any]:
    artifact = provenance["provider_artifact"]
    return {
        "repository": provenance["repository"],
        "workflow": provenance["source_workflow"],
        "workflow_path": provenance["source_workflow_path"],
        "run_id": provenance["source_run_id"],
        "run_attempt": provenance["source_run_attempt"],
        "head_ref": provenance["source_head_ref"],
        "head_sha": provenance["source_head_sha"],
        "event": provenance["source_event"],
        "provider_artifact": {
            "id": artifact["id"],
            "name": artifact["name"],
            "digest": artifact["digest"],
        },
    }


def validated_evaluation_host_proof(
    value: dict[str, Any],
    *,
    provider: str,
    summary: dict[str, Any],
    provenance: dict[str, Any],
    expected_key_id: str,
    public_key: pathlib.Path,
    expected_spki_hash: str,
) -> dict[str, Any]:
    signature = value.get("ed25519_signature")
    payload = dict(value)
    payload.pop("ed25519_signature", None)
    expected_fields = {
        "schema",
        "signature_algorithm",
        "signing_key_id",
        "provider",
        "run_content_hash",
        "host_attestation_challenge",
        "source",
        "runner_instance_id_hash",
        "authentication_classification",
        "credential_boundary",
        "agent_credential_read_access",
        "cli_session_verified",
    }
    if set(payload) != expected_fields or "ed25519_signature" not in value:
        raise SystemExit("evaluation-host proof has missing or unknown fields")
    authentication = summary.get("provider", {}).get("authentication", {})
    expected = {
        "schema": "casegraphen.eval.evaluation_host_session_proof.v1",
        "signature_algorithm": "ed25519",
        "signing_key_id": expected_key_id,
        "provider": provider,
        "run_content_hash": summary_hash(summary),
        "host_attestation_challenge": summary.get("host_attestation_challenge"),
        "source": expected_proof_source(provenance),
        "authentication_classification": authentication.get("classification"),
        "credential_boundary": "dedicated_provider_os_account_with_external_host_attestor",
        "agent_credential_read_access": False,
        "cli_session_verified": True,
    }
    for field, expected_value in expected.items():
        if payload.get(field) != expected_value:
            raise SystemExit(f"evaluation-host proof binding mismatch: {field}")
    runner_hash = payload.get("runner_instance_id_hash")
    if not isinstance(runner_hash, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", runner_hash):
        raise SystemExit("evaluation-host runner identity must be a SHA-256 digest")
    verify_ed25519(public_key, expected_spki_hash, canonical(payload), signature)
    return payload


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", type=pathlib.Path, required=True)
    parser.add_argument("--provider", choices=["codex", "claude"], required=True)
    parser.add_argument("--private-key-file", type=pathlib.Path, required=True)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--evaluation-host-proof", type=pathlib.Path, required=True)
    parser.add_argument("--evaluation-host-public-key", type=pathlib.Path, required=True)
    parser.add_argument("--evaluation-host-key-id", required=True)
    parser.add_argument("--evaluation-host-public-key-spki-sha256", required=True)
    parser.add_argument("--provenance-file", type=pathlib.Path)
    parser.add_argument("--evaluated-commit-sha")
    parser.add_argument("--source-repository")
    parser.add_argument("--source-workflow")
    parser.add_argument("--source-workflow-id", type=positive_integer)
    parser.add_argument("--source-workflow-path")
    parser.add_argument("--source-run-id", type=positive_integer)
    parser.add_argument("--source-run-attempt", type=positive_integer)
    parser.add_argument("--source-head-ref")
    parser.add_argument("--source-head-sha")
    parser.add_argument("--source-event")
    parser.add_argument("--source-conclusion")
    parser.add_argument("--provider-artifact-id", type=positive_integer)
    parser.add_argument("--provider-artifact-name")
    parser.add_argument("--provider-artifact-digest")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    summary = load_object(args.summary)
    identity = summary.get("provider", {})
    authentication = identity.get("authentication", {})
    if (
        summary.get("status") != "completed"
        or identity.get("provider") != args.provider
        or authentication.get("non_api_key_session_verified") is not True
        or not authentication.get("classification")
        or not summary.get("host_attestation_challenge")
    ):
        raise SystemExit("summary does not contain a verified CLI-session run")
    scalar_fields = (
        args.evaluated_commit_sha,
        args.source_repository,
        args.source_workflow,
        args.source_workflow_id,
        args.source_workflow_path,
        args.source_run_id,
        args.source_run_attempt,
        args.source_head_ref,
        args.source_head_sha,
        args.source_event,
        args.source_conclusion,
        args.provider_artifact_id,
        args.provider_artifact_name,
        args.provider_artifact_digest,
    )
    if args.provenance_file is not None:
        if any(value is not None for value in scalar_fields):
            raise SystemExit("--provenance-file cannot be combined with scalar provenance arguments")
        provenance = validated_provenance(load_object(args.provenance_file))
    else:
        if any(value is None for value in scalar_fields):
            raise SystemExit("complete source and artifact provenance is required")
        provenance = validated_provenance(
            {
                "evaluated_commit_sha": args.evaluated_commit_sha,
                "repository": args.source_repository,
                "source_workflow": args.source_workflow,
                "source_workflow_id": args.source_workflow_id,
                "source_workflow_path": args.source_workflow_path,
                "source_run_id": args.source_run_id,
                "source_run_attempt": args.source_run_attempt,
                "source_head_ref": args.source_head_ref,
                "source_head_sha": args.source_head_sha,
                "source_event": args.source_event,
                "source_conclusion": args.source_conclusion,
                "provider_artifact": {
                    "id": args.provider_artifact_id,
                    "name": args.provider_artifact_name,
                    "digest": args.provider_artifact_digest,
                },
            }
        )

    proof_document = load_object(args.evaluation_host_proof)
    proof = validated_evaluation_host_proof(
        proof_document,
        provider=args.provider,
        summary=summary,
        provenance=provenance,
        expected_key_id=args.evaluation_host_key_id,
        public_key=args.evaluation_host_public_key,
        expected_spki_hash=args.evaluation_host_public_key_spki_sha256,
    )

    payload = {
        "schema": "casegraphen.eval.cli_session_host_attestation.v1",
        "signature_algorithm": "ed25519",
        "signing_key_id": args.key_id,
        "provider": args.provider,
        "run_content_hash": summary_hash(summary),
        "host_attestation_challenge": summary["host_attestation_challenge"],
        "authentication_classification": authentication["classification"],
        "credential_boundary": "evaluation_host_session_proven_by_external_attestor",
        "agent_credential_read_access": False,
        "runner_instance_id_hash": proof["runner_instance_id_hash"],
        "provenance": provenance,
        "evaluation_host_proof_content_hash": digest(canonical(proof_document)),
        "evaluation_host_signing_key_id": proof["signing_key_id"],
        "evaluation_host_public_key_spki_sha256": args.evaluation_host_public_key_spki_sha256,
    }
    document = dict(payload)
    document["ed25519_signature"] = ed25519_sign(args.private_key_file, canonical(payload))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
