#!/usr/bin/env python3
"""Exercise the topology-to-review seam through MCP using only Python stdlib.

This is intentionally a protocol client, not a CaseGraphen library adapter. It
launches the published operational host, speaks newline-delimited JSON-RPC, and
proves that a complete runtime reconciliation still stops before acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path
from typing import Any


PROTOCOL_VERSION = "2025-06-18"
BASE_REVISION = "revision:independent-client-observed"
TRUST_BOUNDARY = "runtime_reported_untrusted_until_independently_validated_and_reviewed"

# ADR 0034 (issue #120): the seven-key claim vocabulary control_plane.response.v0
# pins at the top level of `result`, restated here by hand because this client
# is deliberately Python-stdlib-only (see module docstring) and cannot import
# the `jsonschema` package to validate against the shipped schema file
# directly. This is the independent half of layer 1: a consumer checking what
# it actually received, without trusting the host to have enforced anything.
WIRE_CLAIM_VOCABULARY: dict[str, Any] = {
    "accepted": False,
    "mutation_performed": False,
    "read_only": True,
    "accepted_runtime_output": False,
    "proofs_serialized": False,
    "review_status": "unreviewed",
    "generated_plan_review_status": "unreviewed",
}


def forbidden_wire_claim(result: Any) -> str | None:
    """Returns a description of the first top-level key that carries a value
    the wire vocabulary forbids, or None if `result` carries no such claim.
    Top-level only, matching the envelope's declared scope: a nested claim
    below this depth is payload semantics this independent client does not
    govern.

    Mirrors `src/control_plane.rs::wire_claim_violation`: a `dict` is the
    only legitimate successful `result`. Anything else — including `None` —
    is itself a violation here, not an exemption. `call()` already raises on
    any non-null `refusal` before this runs, so a `None` result reaching
    this function would mean both `result` and `refusal` are null on the
    wire, which the envelope's result/refusal exclusivity forbids. An
    earlier version special-cased `None` as "no violation" and, separately,
    checked `key in result` without an `isinstance` guard first — for a list
    `result`, Python's `in` tests membership over elements rather than keys,
    so a forged list result silently reported no violation. Both were the
    same blind spot: trusting `result`'s top-level shape instead of checking
    it."""
    if not isinstance(result, dict):
        kind = "null" if result is None else type(result).__name__
        return f"result is {kind}, but a successful call's result must be an object"
    for key, truthful in WIRE_CLAIM_VOCABULARY.items():
        if key in result and result[key] != truthful:
            return f"result.{key} = {result[key]!r}, but only {truthful!r} is truthful"
    return None


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class StdioMcpClient:
    """Minimal MCP client with no CaseGraphen or third-party dependencies."""

    def __init__(self, host: Path, root: Path) -> None:
        self.token = "independent-client-local-token"
        environment = os.environ.copy()
        environment["CASEGRAPHEN_INDEPENDENT_CLIENT_TOKEN"] = self.token
        self.process = subprocess.Popen(
            [
                str(host),
                "--state", str(root / "protocol-state.json"),
                "--store", str(root / "case-store"),
                "--artifacts", str(root / "host-artifacts"),
                "--auth-token-env", "CASEGRAPHEN_INDEPENDENT_CLIENT_TOKEN",
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=environment,
        )
        self.sequence = 0
        initialized = self.request("initialize", {"protocolVersion": PROTOCOL_VERSION})
        if initialized["protocolVersion"] != PROTOCOL_VERSION:
            raise RuntimeError(f"unexpected MCP protocol: {initialized}")
        self.notify("notifications/initialized", {})

    def notify(self, method: str, params: dict[str, Any]) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps({
            "jsonrpc": "2.0", "method": method, "params": params,
        }, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.write(json.dumps({
            "jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params,
        }, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            assert self.process.stderr is not None
            raise RuntimeError(f"MCP host stopped: {self.process.stderr.read()}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"MCP protocol error: {response['error']}")
        return response["result"]

    def call(self, tool: str, payload: dict[str, Any], request_suffix: str) -> dict[str, Any]:
        envelope = self.request("tools/call", {
            "authorization": self.token,
            "name": tool,
            "arguments": {
                "request_id": f"request:independent-client:{request_suffix}",
                "idempotency_key": f"idempotency:independent-client:{request_suffix}",
                "base_revision_id": BASE_REVISION,
                "caller_declared_audit_context": {
                    "declared_actor_id": "actor:independent-mcp-client",
                    "declared_capability_ids": ["capability:runtime-integration"],
                    "declared_operation_scope_id": "scope:issue-76-interoperability-evidence",
                    "declared_audience": "casegraphen-maintainers",
                    "declared_source_boundary_id": "boundary:external-python-client",
                },
                "payload": payload,
            },
        })
        structured = envelope["structuredContent"]
        if structured.get("refusal") is not None:
            raise RuntimeError(f"{tool} refused: {structured['refusal']}")
        claim = forbidden_wire_claim(structured["result"])
        if claim is not None:
            raise RuntimeError(f"{tool} response carried a forbidden wire claim: {claim}")
        return structured["result"]

    def close(self) -> None:
        if self.process.stdin is not None:
            self.process.stdin.close()
        return_code = self.process.wait(timeout=10)
        if return_code != 0:
            assert self.process.stderr is not None
            raise RuntimeError(f"MCP host exited {return_code}: {self.process.stderr.read()}")


def artifact(content: str) -> tuple[str, dict[str, Any]]:
    content_hash = digest(content.encode())
    artifact_id = f"artifact:sha256-{content_hash}"
    return artifact_id, {
        "kind": "artifact",
        "artifact_id": artifact_id,
        "media_type": "application/json",
        "content": content,
    }


def node_report(
    topology: dict[str, Any], topology_hash: str, node: dict[str, Any],
    output_id: str, parents: list[str], inputs: list[str], index: int,
) -> dict[str, Any]:
    node_id = node["node_id"]
    output_schema = node["outputs"][0]["schema_id"]
    return {
        "kind": "node_report",
        "report": {
            "schema": "casegraphen.experimental.runtime.node_report.v0",
            "schema_version": 0,
            "report_id": f"runtime_report:independent-client:{index}",
            "runtime_graph_id": topology["topology_id"],
            "runtime_graph_content_hash": topology_hash,
            "node_id": node_id,
            "attempt_id": f"attempt:independent-client:{index}",
            "retry_of_attempt_id": None,
            "round_id": "round:2" if parents else "round:1",
            "parent_node_ids": parents,
            "input_artifact_ids": inputs,
            "output_artifact_ids": [output_id],
            "expected_output_schema_id": output_schema,
            "actual_output_schema_id": output_schema,
            "started_at": f"2026-08-04T00:00:0{index}Z",
            "finished_at": f"2026-08-04T00:00:0{index + 1}Z",
            "status": "succeeded",
            "failure_kind": None,
            "runtime_identity": {
                "runtime_name": "independent-python-client",
                "runtime_version": "stdlib",
                "adapter_name": "generic-jsonl",
                "adapter_version": "0",
            },
            "reported_model": None,
            "reported_context_id": f"context:independent-client:{index}",
            "token_usage": None,
            "cost": {"amount": 0.0, "currency": "USD"},
            "resource_allocations": [],
            "worktree_id": None,
            "commit_sha": None,
            "verifier_report_ids": [],
            "trust_boundary": TRUST_BOUNDARY,
        },
    }


def runtime_records(topology: dict[str, Any], topology_hash: str) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    outputs: dict[str, str] = {}
    parents = {node["node_id"]: [] for node in topology["nodes"]}
    for edge in topology["edges"]:
        parents[edge["to"]].append(edge["from"])
    for index, node in enumerate(topology["nodes"]):
        node_id = node["node_id"]
        content = json.dumps({"node_id": node_id, "result": "observed"}, sort_keys=True)
        output_id, output_artifact = artifact(content)
        parent_ids = sorted(parents[node_id])
        input_ids = [outputs[parent] for parent in parent_ids]
        records.extend([
            output_artifact,
            node_report(topology, topology_hash, node, output_id, parent_ids, input_ids, index),
        ])
        outputs[node_id] = output_id
    return records


def compiler_request(topology: dict[str, Any]) -> dict[str, Any]:
    mappings = []
    for node in topology["nodes"]:
        mappings.append({
            "node_id": node["node_id"],
            "worker_binding_id": f"worker_binding:{node['node_id']}",
            "success_evidence_requirement_ids": [f"evidence_requirement:{node['node_id']}"],
            "allowed_transition_classes": [{
                "morphism_type": "update",
                "target_cell_types": ["work"],
                "to_lifecycles": ["resolved"],
            }],
        })
    budgets = {
        policy_id: {"policy_id": policy_id, "max_cost": 10}
        for policy_id in topology.get("budget_policy_ids", [])
    }
    return {
        "case_space_id": topology["case_space_id"],
        "base_revision_id": BASE_REVISION,
        "plan_id": "plan:independent-mcp-client",
        "node_plan_mappings": mappings,
        "verification_policies": {},
        "budget_policies": budgets,
        "expansion_policies": {},
    }


def assert_review_seam(result: dict[str, Any]) -> None:
    assert result["propose"]["accepted"] is False
    assert result["propose"]["review_status"] == "unreviewed"
    assert result["lint"]["accepted"] is False
    assert result["compiled"]["accepted"] is False
    assert result["compiled"]["deployment_authority"] == "proposal_only"
    assert result["compiled"]["generated_plan_review_status"] == "unreviewed"
    assert all(item["accepted"] is False for item in result["attachments"])
    assert all(item["review_status"] == "unreviewed" for item in result["attachments"])
    reconciliation = result["reconciliation"]
    assert reconciliation["completeness"]["complete"] is True
    assert reconciliation["halt"] == "needs_review"
    assert reconciliation["accepted"] is False
    assert reconciliation["proposals"]
    assert all(proposal["review_status"] == "unreviewed" for proposal in reconciliation["proposals"])


def run(host: Path, topology_path: Path) -> dict[str, Any]:
    topology = json.loads(topology_path.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="casegraphen-independent-mcp-") as directory:
        client = StdioMcpClient(host, Path(directory))
        try:
            topology_payload = {"topology_json": json.dumps(topology, separators=(",", ":"))}
            proposed = client.call("propose_execution_topology", topology_payload, "propose")
            linted = client.call("lint_execution_topology", topology_payload, "lint")
            topology_hash = linted["topology_content_hash"]
            compiled = client.call("compile_deployment_bundle", {
                **topology_payload, "compiler_request": compiler_request(topology),
            }, "compile")
            # The host directory is intentionally temporary in this evidence run.
            # Keep the content-addressed manifest, but omit that machine-local path
            # so identical inputs and binaries produce byte-identical reports.
            compiled.pop("bundle_directory", None)
            records = runtime_records(topology, topology_hash)
            lines = [json.dumps(record, sort_keys=True, separators=(",", ":")) for record in records]
            attachments = [
                client.call("attach_runtime_report", {"jsonl_record": line}, f"attach-{index}")
                for index, line in enumerate(lines)
            ]
            reconciliation = client.call("reconcile_run", {
                **topology_payload, "runtime_jsonl": "\n".join(lines),
            }, "reconcile")
        finally:
            client.close()
    result = {
        "schema": "casegraphen.evidence.independent_mcp_client.v0",
        "client_implementation": "python_stdlib_json_rpc",
        "custom_rust_client_code": False,
        "protocol_version": PROTOCOL_VERSION,
        "topology_source_name": topology_path.name,
        "topology_source_sha256": digest(topology_path.read_bytes()),
        "host_binary_sha256": digest(host.read_bytes()),
        "propose": proposed,
        "lint": linted,
        "compiled": compiled,
        "attachments": attachments,
        "reconciliation": reconciliation,
        "final_boundary": {
            "review_required": reconciliation["halt"] == "needs_review",
            "accepted": reconciliation["accepted"],
            "all_proposals_unreviewed": all(
                proposal["review_status"] == "unreviewed"
                for proposal in reconciliation["proposals"]
            ),
        },
    }
    assert_review_seam(result)

    # ADR 0034 / issue #120: prove forbidden_wire_claim is load-bearing
    # against a real, live response this run just received, not merely
    # reasoned about. `compiled` is untouched above; mutate a copy of it.
    assert forbidden_wire_claim(compiled) is None, "sanity: the real response must be clean first"
    forged = dict(compiled, accepted=True)
    forged_claim = forbidden_wire_claim(forged)
    assert forged_claim is not None, "a mutated copy claiming accepted: True must be caught"

    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host-bin", type=Path, required=True)
    parser.add_argument("--topology", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    report = run(arguments.host_bin.resolve(), arguments.topology.resolve())
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(encoded, encoding="utf-8")
    print(json.dumps({
        "report": str(arguments.output),
        "report_sha256": digest(encoded.encode()),
        "review_required": True,
        "accepted": False,
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
