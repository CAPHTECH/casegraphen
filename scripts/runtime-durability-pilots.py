#!/usr/bin/env python3
"""Bounded durability pilots for the experimental runtime graph boundary.

These pilots are deliberately local and reproducible.  They exercise real
process/TCP and filesystem boundaries, but remain untrusted promotion evidence.
Provider-host provenance is owned by issue #76 and is never inferred here.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import resource
import selectors
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


SCHEMA = "casegraphen.experimental.runtime_durability_pilot.report.v0"
LIMITS = {
    "remote_total_ms": 10_000,
    "binary_bytes": 65_536,
    "scale_nodes": 512,
    "scale_edges": 511,
    "scale_retries": 128,
    "scale_reconcile_ms": 5_000,
    "scale_peak_bytes": 128 * 1024 * 1024,
    "allocator_events": 512,
    "allocator_replay_ms": 5_000,
}


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


REMOTE_SERVER = r'''
import json, os, socketserver, sys, time
journal = sys.argv[1]
delay_marker = sys.argv[2]
disconnect_marker = sys.argv[3]

def append(record):
    with open(journal, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")
        handle.flush(); os.fsync(handle.fileno())

def known(key):
    if not os.path.exists(journal): return None
    for line in open(journal, encoding="utf-8"):
        item = json.loads(line)
        if item["idempotency_key"] == key: return item
    return None

class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        request = json.loads(self.rfile.readline())
        key = request["idempotency_key"]
        prior = known(key)
        if prior:
            response = {"status":"resumed", "duplicate":True, "record":prior}
        else:
            record = {"idempotency_key":key, "payload_hash":request["payload_hash"], "accepted":False}
            append(record)
            response = {"status":"recorded", "duplicate":False, "record":record}
        if request.get("delay_once") and not os.path.exists(delay_marker):
            open(delay_marker, "wb").close(); time.sleep(0.25)
        if request.get("disconnect_once") and not os.path.exists(disconnect_marker):
            open(disconnect_marker, "wb").close(); return
        self.wfile.write((json.dumps(response, sort_keys=True) + "\n").encode())

class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True
    block_on_close = False
with Server(("127.0.0.1", 0), Handler) as server:
    print(server.server_address[1], flush=True)
    server.serve_forever()
'''


def start_server(root: Path) -> tuple[subprocess.Popen[str], int]:
    process = subprocess.Popen(
        [sys.executable, "-c", REMOTE_SERVER, str(root / "remote.journal"),
         str(root / "delay.marker"), str(root / "disconnect.marker")],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    assert process.stdout is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    if not selector.select(timeout=5):
        process.kill(); process.wait(timeout=3)
        raise TimeoutError("remote pilot server startup timed out")
    line = process.stdout.readline().strip()
    selector.close()
    if not line:
        assert process.stderr is not None
        raise RuntimeError(process.stderr.read())
    return process, int(line)


def remote_call(port: int, request: dict[str, Any], timeout: float) -> dict[str, Any]:
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as connection:
        connection.settimeout(timeout)
        connection.sendall(canonical(request) + b"\n")
        stream = connection.makefile("rb")
        line = stream.readline()
        if not line:
            raise ConnectionError("peer disconnected before response")
        return json.loads(line)


def remote_pilot(root: Path) -> dict[str, Any]:
    started = time.monotonic_ns()
    server, port = start_server(root)
    disconnect = {"idempotency_key":"remote:disconnect", "payload_hash":digest(b"one"),
                  "disconnect_once":True}
    timeout = {"idempotency_key":"remote:timeout", "payload_hash":digest(b"two"),
               "delay_once":True}
    events: list[str] = []
    try:
        try:
            remote_call(port, disconnect, 1.0)
        except ConnectionError:
            events.append("disconnect_observed")
        resumed = remote_call(port, disconnect, 1.0)
        events.append("duplicate_delivery_resumed")
        try:
            remote_call(port, timeout, 0.05)
        except (TimeoutError, socket.timeout):
            events.append("timeout_observed")
    finally:
        server.terminate(); server.wait(timeout=3)
    restarted, new_port = start_server(root)
    try:
        timeout_resumed = remote_call(new_port, timeout, 1.0)
        events.extend(["process_restarted", "reconnected", "journal_resumed"])
    finally:
        restarted.terminate(); restarted.wait(timeout=3)
    journal = [json.loads(line) for line in (root / "remote.journal").read_text().splitlines()]
    elapsed = (time.monotonic_ns() - started) // 1_000_000
    required = {"disconnect_observed", "duplicate_delivery_resumed", "timeout_observed",
                "process_restarted", "reconnected", "journal_resumed"}
    passed = (required <= set(events) and resumed["duplicate"] and timeout_resumed["duplicate"]
              and len(journal) == 2 and elapsed <= LIMITS["remote_total_ms"]
              and all(item["accepted"] is False for item in journal))
    return {"passed":passed, "threshold_ms":LIMITS["remote_total_ms"],
            "elapsed_ms":elapsed, "events":events, "journal_record_count":len(journal),
            "duplicate_delivery_count":2, "accepted":False,
            "halt":"operator_review_required"}


def canonical_runtime_pilot(directory: Path, peak_memory_bytes: int) -> tuple[dict[str, Any], dict[str, Any]]:
    report = json.loads((directory / "canonical-runtime-report.json").read_text())
    binary = (directory / "binary-artifact.bin").read_bytes()
    binary_hash = digest(binary)
    binary_passed = (report["passed"] and report["non_utf8_observed"]
                     and report["binary_byte_length"] == len(binary)
                     and report["binary_artifact_id"] == f"artifact:sha256-{binary_hash}")
    binary_result = {"passed":binary_passed, "accepted":False,
                     "artifact_id":report["binary_artifact_id"],
                     "content_hash":binary_hash, "byte_length":len(binary),
                     "media_type":report["binary_media_type"],
                     "non_utf8_observed":report["non_utf8_observed"],
                     "canonical_edge_proof_set_hash":report["edge_proof_set_hash"],
                     "halt":"operator_review_required"}
    scale_passed = (report["passed"] and report["complete"]
                    and report["node_count"] == LIMITS["scale_nodes"]
                    and report["edge_count"] == LIMITS["scale_edges"]
                    and report["retry_count"] == LIMITS["scale_retries"]
                    and report["proven_edge_count"] == LIMITS["scale_edges"]
                    and report["reconciliation_ms"] <= LIMITS["scale_reconcile_ms"]
                    and peak_memory_bytes <= LIMITS["scale_peak_bytes"])
    scale_result = {"passed":scale_passed, "accepted":False,
                    "node_count":report["node_count"], "edge_count":report["edge_count"],
                    "retry_count":report["retry_count"], "report_count":report["report_count"],
                    "proven_edge_count":report["proven_edge_count"],
                    "node_complete":report["node_complete"],
                    "dataflow_complete":report["dataflow_complete"],
                    "complete":report["complete"],
                    "edge_proof_set_hash":report["edge_proof_set_hash"],
                    "reconciliation_ms":report["reconciliation_ms"],
                    "peak_memory_bytes":peak_memory_bytes,
                    "thresholds":{"reconciliation_ms":LIMITS["scale_reconcile_ms"],
                                  "peak_memory_bytes":LIMITS["scale_peak_bytes"]},
                    "halt":"operator_review_required"}
    return binary_result, scale_result


def contract_hashes(repo: Path) -> dict[str, str]:
    names = ["execution.topology.v0.schema.json", "runtime.node_report.schema.json",
             "runtime.graph_expectation.v0.schema.json", "runtime.integration_report.v0.schema.json",
             "resource.allocator_event.v0.schema.json"]
    return {name:digest((repo / "schemas/experimental" / name).read_bytes()) for name in names}


def run(repo: Path, output: Path, allocator_report: Path, canonical_runtime_dir: Path,
        reviewed_resource_report: Path, canonical_peak_memory_bytes: int) -> dict[str, Any]:
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="casegraphen-durability-") as directory:
        root = Path(directory)
        remote = remote_pilot(root)
        shutil.copy2(root / "remote.journal", output / "remote.journal.jsonl")
    binary, scale = canonical_runtime_pilot(canonical_runtime_dir, canonical_peak_memory_bytes)
    allocator = json.loads(allocator_report.read_text())
    reviewed_resource = json.loads(reviewed_resource_report.read_text())
    canonical_report = json.loads((canonical_runtime_dir / "canonical-runtime-report.json").read_text())
    authority_matches = (reviewed_resource["passed"] and
                         reviewed_resource["reviewed_deployment_hash"] == canonical_report["reviewed_deployment_hash"])
    reviewed_resource["passed"] = bool(authority_matches)
    reports = {"remote":remote, "binary":binary, "scale":scale, "allocator":allocator,
               "reviewed_resource":reviewed_resource}
    all_passed = all(report["passed"] is True for report in reports.values())
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    summary = {"schema":SCHEMA, "schema_version":0, "accepted":False,
               "promotion_eligible":False, "all_thresholds_passed":all_passed,
               "source_revision":head, "source_worktree_dirty":bool(subprocess.check_output(
                   ["git", "status", "--porcelain", "--untracked-files=no"],
                   cwd=repo, text=True).strip()),
               "harness_content_hash":digest(Path(__file__).read_bytes()),
               "contract_content_hashes":contract_hashes(repo),
               "runtime_versions":{"python":platform.python_version(), "platform":platform.platform()},
               "topology_content_hash":canonical_report["topology_content_hash"],
               "reviewed_deployment_hash":canonical_report["reviewed_deployment_hash"],
               "reports":reports,
               "blockers":["#76 provider-specific broker-signed host/session attestations are absent"],
               "failure_disposition":"audit_or_redesign_proposal_only",
               "proposals":[{"kind":"runtime_durability_audit", "review_status":"unreviewed",
                             "accepted":False, "finding_codes":[] if all_passed else [
                                 name for name, report in reports.items() if not report["passed"]]}]}
    evidence_names = []
    for source in sorted(canonical_runtime_dir.iterdir()):
        if source.is_file():
            name = "canonical-" + source.name
            shutil.copy2(source, output / name)
            evidence_names.append(name)
    shutil.copy2(allocator_report, output / "allocator-durability-report.json")
    shutil.copy2(reviewed_resource_report, output / "reviewed-resource-report.json")
    evidence_names.extend(["allocator-durability-report.json", "reviewed-resource-report.json",
                           "remote.journal.jsonl"])
    write_json(output / "durability-report.json", summary)
    write_json(output / "promotion-report.json", {"accepted":False, "promotion_recommended":False,
               "durability_thresholds_passed":all_passed, "blockers":summary["blockers"],
               "workflow_count":10, "review_seam":"operator_review_required"})
    names = ["durability-report.json", "promotion-report.json", *evidence_names]
    manifest = {"schema":"casegraphen.experimental.runtime_durability_pilot.evidence_manifest.v0",
                "accepted":False, "files":[{"path":name,
                    "content_hash":"sha256:" + digest((output / name).read_bytes()),
                    "byte_length":(output / name).stat().st_size} for name in names]}
    write_json(output / "retained-evidence.manifest.json", manifest)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--allocator-report", type=Path, required=True)
    parser.add_argument("--canonical-runtime-dir", type=Path, required=True)
    parser.add_argument("--reviewed-resource-report", type=Path, required=True)
    parser.add_argument("--canonical-peak-memory-bytes", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    summary = run(args.repo.resolve(), args.output.resolve(), args.allocator_report.resolve(),
                  args.canonical_runtime_dir.resolve(), args.reviewed_resource_report.resolve(),
                  args.canonical_peak_memory_bytes)
    print(json.dumps({"all_thresholds_passed":summary["all_thresholds_passed"],
                      "promotion_eligible":summary["promotion_eligible"]}, sort_keys=True))
    return 0 if summary["all_thresholds_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
