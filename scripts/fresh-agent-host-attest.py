#!/usr/bin/env python3
"""Create a run-bound CLI-session attestation outside the evaluation agent.

Run this only from a dedicated provider-host credential broker or OS account.
The HMAC key must not be readable by the evaluation account and is never
written to the attestation or provider artifact directory.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import importlib.util
import json
import pathlib
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def summary_hash(summary: dict[str, Any]) -> str:
    value = dict(summary)
    claimed = value.pop("content_hash", None)
    actual = digest(canonical(value))
    if claimed != actual:
        raise ValueError("summary content hash mismatch")
    return actual


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", type=pathlib.Path, required=True)
    parser.add_argument("--provider", choices=["codex", "claude"], required=True)
    parser.add_argument("--key-file", type=pathlib.Path, required=True)
    parser.add_argument("--key-id", required=True)
    parser.add_argument("--runner-instance-id-hash", required=True)
    parser.add_argument("--provider-cli", help="pinned provider CLI executable")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    summary = json.loads(args.summary.read_text())
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
    if not args.runner_instance_id_hash.startswith("sha256:"):
        raise SystemExit("runner instance identity must be pre-hashed")
    eval_path = ROOT / "scripts/fresh-agent-eval.py"
    specification = importlib.util.spec_from_file_location("fresh_agent_eval", eval_path)
    if specification is None or specification.loader is None:
        raise SystemExit("cannot load canonical CLI-session classifier")
    eval_module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(eval_module)
    host_authentication = eval_module.cli_session_status(
        [args.provider_cli or args.provider], args.provider
    )
    if (
        host_authentication.get("non_api_key_session_verified") is not True
        or host_authentication.get("classification") != authentication.get("classification")
    ):
        raise SystemExit("host probe does not confirm the summary CLI-session class")
    key = args.key_file.read_bytes()
    if len(key) < 32:
        raise SystemExit("host attestation key must contain at least 32 bytes")
    payload = {
        "schema": "casegraphen.eval.cli_session_host_attestation.v0",
        "provider": args.provider,
        "run_content_hash": summary_hash(summary),
        "host_attestation_challenge": summary["host_attestation_challenge"],
        "authentication_classification": authentication["classification"],
        "credential_boundary": "dedicated_provider_os_account_with_brokered_session",
        "agent_credential_read_access": False,
        "runner_instance_id_hash": args.runner_instance_id_hash,
        "key_id": args.key_id,
    }
    payload["hmac_sha256"] = hmac.new(key, canonical(payload), hashlib.sha256).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
