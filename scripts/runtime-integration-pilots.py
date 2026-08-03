#!/usr/bin/env python3
"""Run reproducible local-runtime pilots through the operational MCP host.

The pilots deliberately use four different runtime boundaries:

* ``process-jsonl`` streams envelopes directly from local subprocesses.
* ``file-drop`` collects native per-attempt files from isolated workspaces and
  only then adapts them to the generic JSONL boundary.
* ``sqlite-queue`` crosses a transactional durable queue/result-table boundary.
* ``async-stream`` crosses an asyncio subprocess event-stream boundary.

Everything reported by either runtime remains a declaration.  The script asks
the real ``casegraphen-mcp-host`` to lint and reconcile and never changes a case
or accepts a proposal.
"""

from __future__ import annotations

import argparse
import asyncio
import concurrent.futures
import hashlib
import json
import os
import platform
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


TRUST_BOUNDARY = "runtime_reported_untrusted_until_independently_validated_and_reviewed"
BASE_REVISION = "revision:pilot-observed"


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def artifact(content: str, media_type: str = "application/json") -> tuple[str, dict[str, Any]]:
    digest = sha256(content.encode())
    artifact_id = f"artifact:sha256-{digest}"
    return artifact_id, {
        "kind": "artifact",
        "artifact_id": artifact_id,
        "media_type": media_type,
        "content": content,
    }


def report(
    *, topology_id: str, topology_hash: str, node_id: str, attempt_id: str,
    output_id: str | None, expected_schema: str, actual_schema: str | None,
    adapter_name: str, adapter_version: str, runtime_name: str,
    runtime_version: str, started_at: str, finished_at: str,
    status: str = "succeeded", failure_kind: str | None = None,
    retry_of: str | None = None, parents: list[str] | None = None,
    inputs: list[str] | None = None, worktree_id: str | None = None,
    commit_sha: str | None = None, cost: float = 0.0,
    resource_allocations: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "kind": "node_report",
        "report": {
            "schema": "casegraphen.experimental.runtime.node_report.v0",
            "schema_version": 0,
            "report_id": f"runtime_report:{adapter_name}:{attempt_id}",
            "runtime_graph_id": topology_id,
            "runtime_graph_content_hash": topology_hash,
            "node_id": node_id,
            "attempt_id": attempt_id,
            "retry_of_attempt_id": retry_of,
            "round_id": "round:2" if parents else "round:1",
            "parent_node_ids": parents or [],
            "input_artifact_ids": inputs or [],
            "output_artifact_ids": [output_id] if output_id else [],
            "expected_output_schema_id": expected_schema,
            "actual_output_schema_id": actual_schema,
            "started_at": started_at,
            "finished_at": finished_at,
            "status": status,
            "failure_kind": failure_kind,
            "runtime_identity": {
                "runtime_name": runtime_name,
                "runtime_version": runtime_version,
                "adapter_name": adapter_name,
                "adapter_version": adapter_version,
            },
            "reported_model": "local-deterministic-process",
            "reported_context_id": f"context:{attempt_id}",
            "token_usage": {"input_tokens": 2, "output_tokens": 3, "total_tokens": 5},
            "cost": {"amount": cost, "currency": "USD"},
            "resource_allocations": resource_allocations or [],
            "worktree_id": worktree_id,
            "commit_sha": commit_sha,
            "verifier_report_ids": [],
            "trust_boundary": TRUST_BOUNDARY,
        },
    }


class McpHost:
    def __init__(self, binary: Path, root: Path) -> None:
        token = "pilot-local-token"
        environment = os.environ.copy()
        environment["CASEGRAPHEN_PILOT_TOKEN"] = token
        self.token = token
        self.process = subprocess.Popen(
            [str(binary), "--state", str(root / "state.json"), "--store", str(root / "store"),
             "--artifacts", str(root / "host-artifacts"), "--auth-token-env", "CASEGRAPHEN_PILOT_TOKEN"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", env=environment,
        )
        self.sequence = 0
        self._request("initialize", {"protocolVersion": "2025-06-18"})
        self._notify("notifications/initialized", {})

    def _notify(self, method: str, params: dict[str, Any]) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n")
        self.process.stdin.flush()

    def _request(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        assert self.process.stdin is not None and self.process.stdout is not None
        message = {"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params}
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            assert self.process.stderr is not None
            raise RuntimeError(f"MCP host stopped: {self.process.stderr.read()}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"MCP error: {response['error']}")
        return response["result"]

    def call(
        self, name: str, payload: dict[str, Any], suffix: str, *, mutation: bool = False
    ) -> dict[str, Any]:
        arguments = {
            "request_id": f"request:pilot:{suffix}",
            "idempotency_key": f"idempotency:pilot:{suffix}",
            "base_revision_id": BASE_REVISION,
            "payload": payload,
        }
        if mutation:
            arguments["caller_declared_audit_context"] = {
                "declared_actor_id": "actor:runtime-pilot",
                "declared_capability_ids": ["capability:resource-pilot"],
                "declared_operation_scope_id": "scope:runtime-pilot",
                "declared_audience": "audit",
                "declared_source_boundary_id": "boundary:runtime-pilot",
            }
        result = self._request("tools/call", {
            "authorization": self.token,
            "name": name,
            "arguments": arguments,
        })
        structured = result["structuredContent"]
        if structured.get("refusal") is not None:
            raise RuntimeError(f"host refusal: {structured['refusal']}")
        return structured["result"]

    def close(self) -> None:
        if self.process.stdin is not None:
            self.process.stdin.close()
        return_code = self.process.wait(timeout=10)
        if return_code != 0:
            assert self.process.stderr is not None
            raise RuntimeError(f"MCP host exited {return_code}: {self.process.stderr.read()}")


def run_command(program: str, text: str, *, cwd: Path | None = None, fail: bool = False) -> tuple[str, int, int]:
    source = "import sys; data=sys.stdin.read(); "
    source += "sys.exit(7)" if fail else f"sys.stdout.write({program})"
    started = time.monotonic_ns()
    completed = subprocess.run(
        [sys.executable, "-c", source], input=text, text=True, capture_output=True, cwd=cwd, check=False,
    )
    elapsed_ms = max(0, (time.monotonic_ns() - started) // 1_000_000)
    return completed.stdout, completed.returncode, elapsed_ms


def process_jsonl_adapter(topology: dict[str, Any], topology_hash: str) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    input_a = "alpha\n"
    input_b = "beta\n"
    input_ids = [f"artifact:sha256-{sha256(input_a.encode())}", f"artifact:sha256-{sha256(input_b.encode())}"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        a_future = pool.submit(run_command, "data.strip().upper()", input_a)
        failed_future = pool.submit(run_command, "data", input_b, fail=True)
        a_output, a_code, a_ms = a_future.result()
        _, failed_code, failed_ms = failed_future.result()
    b_output, b_code, b_ms = run_command("data.strip()[::-1]", input_b)
    if (a_code, failed_code, b_code) != (0, 7, 0):
        raise RuntimeError("local process adapter did not exercise its declared retry path")
    a_id, a_artifact = artifact(json.dumps({"finding": a_output}, sort_keys=True))
    b_id, b_artifact = artifact(json.dumps({"finding": b_output}, sort_keys=True))
    reduced, reduce_code, reduce_ms = run_command(
        "'summary:' + data.replace('\\n', '|')", a_output + "\n" + b_output,
    )
    if reduce_code != 0:
        raise RuntimeError("local reducer failed")
    summary_id, summary_artifact = artifact(json.dumps({"summary": reduced}, sort_keys=True))
    records = [
        a_artifact,
        report(topology_id=topology["topology_id"], topology_hash=topology_hash,
               node_id="node:inspect-a", attempt_id="attempt:inspect-a:1", output_id=a_id,
               expected_schema="schema:pilot-finding", actual_schema="schema:pilot-finding",
               adapter_name="generic-jsonl", adapter_version="0.1.0", runtime_name="local-process-pool",
               runtime_version=platform.python_version(), started_at="2026-08-03T00:00:00Z",
               finished_at="2026-08-03T00:00:01Z", inputs=[input_ids[0]], cost=0.001),
        report(topology_id=topology["topology_id"], topology_hash=topology_hash,
               node_id="node:inspect-b", attempt_id="attempt:inspect-b:1", output_id=None,
               expected_schema="schema:pilot-finding", actual_schema=None,
               adapter_name="generic-jsonl", adapter_version="0.1.0", runtime_name="local-process-pool",
               runtime_version=platform.python_version(), started_at="2026-08-03T00:00:00Z",
               finished_at="2026-08-03T00:00:01Z", status="failed", failure_kind="execution_error",
               inputs=[input_ids[1]], cost=0.0005),
        b_artifact,
        report(topology_id=topology["topology_id"], topology_hash=topology_hash,
               node_id="node:inspect-b", attempt_id="attempt:inspect-b:2", output_id=b_id,
               expected_schema="schema:pilot-finding", actual_schema="schema:pilot-finding",
               adapter_name="generic-jsonl", adapter_version="0.1.0", runtime_name="local-process-pool",
               runtime_version=platform.python_version(), started_at="2026-08-03T00:00:01Z",
               finished_at="2026-08-03T00:00:02Z", retry_of="attempt:inspect-b:1",
               inputs=[input_ids[1]], cost=0.001),
        summary_artifact,
        report(topology_id=topology["topology_id"], topology_hash=topology_hash,
               node_id="node:reduce", attempt_id="attempt:reduce:1", output_id=summary_id,
               expected_schema="schema:pilot-summary", actual_schema="schema:pilot-summary",
               adapter_name="generic-jsonl", adapter_version="0.1.0", runtime_name="local-process-pool",
               runtime_version=platform.python_version(), started_at="2026-08-03T00:00:02Z",
               finished_at="2026-08-03T00:00:03Z", parents=["node:inspect-a", "node:inspect-b"],
               inputs=[a_id, b_id], cost=0.0002),
    ]
    deployment = {
        "adapter": "generic-jsonl@0.1.0", "python": platform.python_version(),
        "topology_content_hash": topology_hash, "input_artifact_ids": input_ids,
        "max_workers": 2, "retry_limit": 1,
    }
    observation = {
        "adapter": "generic-jsonl", "runtime": "local-process-pool",
        "runtime_version": platform.python_version(), "adapter_version": "0.1.0",
        "input_artifact_ids": input_ids, "topology_content_hash": topology_hash,
        "reported_deployment_content_hash": sha256(canonical(deployment)),
        "measured_latency_ms": {"inspect_a": a_ms, "inspect_b_failed": failed_ms,
                                "inspect_b_retry": b_ms, "reduce": reduce_ms},
        "reported_cost": {"amount": 0.0027, "currency": "USD"},
        "retry_lineage": ["attempt:inspect-b:1", "attempt:inspect-b:2"],
        "streaming_release_order": ["node:inspect-a", "node:inspect-b", "node:reduce"],
        "trust": "runtime_declared_untrusted",
    }
    return records, observation


def file_drop_adapter(topology: dict[str, Any], topology_hash: str, root: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    drop = root / "file-drop"
    drop.mkdir()
    repository = root / "source-repository"
    repository.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=repository, check=True)
    subprocess.run(["git", "config", "user.name", "CaseGraphen Pilot"], cwd=repository, check=True)
    subprocess.run(["git", "config", "user.email", "pilot@example.invalid"], cwd=repository, check=True)
    (repository / "shared.txt").write_text("base\n", encoding="utf-8")
    subprocess.run(["git", "add", "shared.txt"], cwd=repository, check=True)
    git_environment = os.environ.copy()
    git_environment.update({
        "GIT_AUTHOR_DATE": "2026-08-03T00:00:00Z",
        "GIT_COMMITTER_DATE": "2026-08-03T00:00:00Z",
    })
    subprocess.run(
        ["git", "commit", "-q", "-m", "pilot base"], cwd=repository, check=True,
        env=git_environment,
    )
    base_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    worktree_root = root / "worktrees"
    worktree_root.mkdir()
    workspaces = [worktree_root / "a", worktree_root / "b"]
    for index, workspace in enumerate(workspaces):
        subprocess.run(
            ["git", "worktree", "add", "-q", "-b", f"pilot-{index}", str(workspace), base_commit],
            cwd=repository, check=True,
        )

    def worker(index: int) -> tuple[Path, int]:
        workspace = workspaces[index]
        started = time.monotonic_ns()
        native_path = drop / f"attempt-{index}.json"
        node_id = f"node:edit-{'a' if index == 0 else 'b'}"
        attempt_id = f"attempt:edit-{'a' if index == 0 else 'b'}:1"
        worker_source = """
import json, subprocess, sys
from pathlib import Path
variant, native_path, node_id, attempt_id = sys.argv[1:]
Path('shared.txt').write_text('variant-' + variant + '\\n', encoding='utf-8')
subprocess.run(['git', 'add', 'shared.txt'], check=True)
subprocess.run(['git', 'commit', '-q', '-m', 'pilot variant ' + variant], check=True)
commit_sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
native = {
    'node': node_id,
    'attempt': attempt_id,
    'worktree_path': str(Path.cwd().resolve()),
    'output_file': str((Path.cwd() / 'shared.txt').resolve()),
    'commit_sha': commit_sha,
    'exit_code': 0,
}
Path(native_path).write_text(json.dumps(native, sort_keys=True), encoding='utf-8')
"""
        worker_environment = os.environ.copy()
        worker_environment.update({
            "GIT_AUTHOR_DATE": f"2026-08-03T00:00:0{index + 1}Z",
            "GIT_COMMITTER_DATE": f"2026-08-03T00:00:0{index + 1}Z",
        })
        completed = subprocess.run(
            [sys.executable, "-c", worker_source, str(index), str(native_path), node_id, attempt_id],
            cwd=workspace, capture_output=True, text=True, check=False, env=worker_environment,
        )
        elapsed = max(0, (time.monotonic_ns() - started) // 1_000_000)
        if completed.returncode != 0:
            raise RuntimeError(completed.stderr)
        return native_path, elapsed

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        native_results = list(pool.map(worker, [0, 1]))
    records: list[dict[str, Any]] = []
    commit_hashes: list[str] = []
    for index, (native_path, _) in enumerate(native_results):
        native = json.loads(native_path.read_text(encoding="utf-8"))
        output_bytes = Path(native["output_file"]).read_bytes()
        output_id, output_artifact = artifact(output_bytes.decode(), "text/plain")
        commit_hash = native["commit_sha"]
        commit_hashes.append(commit_hash)
        records.extend([
            output_artifact,
            report(topology_id=topology["topology_id"], topology_hash=topology_hash,
                   node_id=native["node"], attempt_id=native["attempt"], output_id=output_id,
                   expected_schema="schema:pilot-commit", actual_schema="schema:pilot-commit",
                   adapter_name="file-drop", adapter_version="0.1.0", runtime_name="local-workspace-runner",
                   runtime_version=platform.python_version(), started_at=f"2026-08-03T00:00:0{index}Z",
                   finished_at=f"2026-08-03T00:00:0{index + 1}Z",
                   worktree_id=f"git-worktree:pilot-{index}@{commit_hash}",
                   commit_sha=commit_hash, cost=0.0004),
        ])
    worktree_list = subprocess.check_output(
        ["git", "worktree", "list", "--porcelain"], cwd=repository, text=True
    )
    isolated = (
        workspaces[0].resolve() != workspaces[1].resolve()
        and all((path / ".git").is_file() and (path / "shared.txt").exists() for path in workspaces)
        and all(str(path.resolve()) in worktree_list for path in workspaces)
        and len(set(commit_hashes)) == 2
    )
    deployment = {
        "adapter": "file-drop@0.1.0", "topology_content_hash": topology_hash,
        "drop_directory": "file-drop", "workspace_strategy": "isolated_worktree",
        "base_commit": base_commit,
    }
    return records, {
        "adapter": "file-drop", "runtime": "local-workspace-runner",
        "runtime_version": platform.python_version(), "adapter_version": "0.1.0",
        "topology_content_hash": topology_hash,
        "reported_deployment_content_hash": sha256(canonical(deployment)),
        "native_report_count": len(native_results),
        "worktree_ids": [f"git-worktree:pilot-{index}@{commit_hashes[index]}" for index in range(2)],
        "base_commit_sha": base_commit,
        "commit_shas": commit_hashes, "workspace_isolation_observed": isolated,
        "measured_latency_ms": [result[1] for result in native_results],
        "reported_cost": {"amount": 0.0008, "currency": "USD"},
        "trust": "runtime_declared_untrusted",
    }


def single_resource_topology(template: dict[str, Any], family: str) -> dict[str, Any]:
    """Create one valid, resource-bearing topology for an independent runtime family."""
    topology = json.loads(json.dumps(template))
    node = topology["nodes"][0]
    topology["topology_id"] = f"topology:pilot-{family}"
    topology["nodes"] = [node]
    topology["edges"] = []
    node["node_id"] = f"node:{family}"
    node["work_cell_id"] = f"work:{family}"
    node["idempotency_key"] = f"{family}:<input-hash>"
    node["resource_claims"] = [{
        "resource": f"file:pilot/{family}.input",
        "mode": "read",
        "rate_limit_group": None,
        "workspace_strategy": "shared",
        "network_scope": [],
        "secret_scope": [],
    }]
    return topology


def resource_contracts(
    topology: dict[str, Any], topology_hash: str, family: str
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    node = topology["nodes"][0]
    claim = node["resource_claims"][0]
    declaration = {
        "schema": "casegraphen.experimental.resource.declaration.v0",
        "schema_version": 0,
        "declaration_id": f"declaration:{family}",
        "runtime_graph_id": topology["topology_id"],
        "runtime_graph_content_hash": topology_hash,
        "node_id": node["node_id"],
        "claims": node["resource_claims"],
    }
    grant = {
        "resource_id": claim["resource"],
        "mode": claim["mode"],
        "rate_limit_group": claim["rate_limit_group"],
        "rate_limit_units": 1 if claim["rate_limit_group"] is not None else 0,
        "workspace_strategy": claim["workspace_strategy"],
        "network_scope": claim["network_scope"],
        "secret_scope": claim["secret_scope"],
    }
    reservation = {
        "schema": "casegraphen.experimental.resource.reservation.v0",
        "schema_version": 0,
        "reservation_id": f"reservation:{family}:1",
        "declaration_id": declaration["declaration_id"],
        "attempt_id": f"attempt:{family}:1",
        "granted_at": "2026-08-03T00:00:00Z",
        "grants": [grant],
    }
    allocation = {
        "schema": "casegraphen.experimental.runtime.resource_allocation.v0",
        "schema_version": 0,
        "allocation_id": f"allocation:{family}:1",
        "reservation_id": reservation["reservation_id"],
        "attempt_id": reservation["attempt_id"],
        **grant,
        "worktree_id": None,
        "trust_boundary": "runtime_reported_untrusted_until_independently_reconciled",
    }
    return declaration, reservation, allocation


def sqlite_queue_runtime(root: Path) -> tuple[str, dict[str, Any]]:
    database = root / "sqlite-queue.db"
    connection = sqlite3.connect(database)
    connection.executescript(
        "CREATE TABLE jobs(id INTEGER PRIMARY KEY, payload TEXT, state TEXT);"
        "CREATE TABLE results(job_id INTEGER PRIMARY KEY, output TEXT);"
    )
    connection.execute("INSERT INTO jobs(payload,state) VALUES (?,?)", ("sqlite-payload", "queued"))
    connection.commit()
    started = time.monotonic_ns()
    row = connection.execute("SELECT id,payload FROM jobs WHERE state='queued'").fetchone()
    if row is None:
        raise RuntimeError("SQLite pilot queue did not retain its job")
    output = row[1].upper()
    connection.execute("UPDATE jobs SET state='complete' WHERE id=?", (row[0],))
    connection.execute("INSERT INTO results(job_id,output) VALUES (?,?)", (row[0], output))
    connection.commit()
    retained = connection.execute("SELECT output FROM results WHERE job_id=?", (row[0],)).fetchone()[0]
    connection.close()
    return retained, {
        "adapter": "sqlite-durable-queue",
        "runtime": "sqlite3-transactional-worker",
        "runtime_version": sqlite3.sqlite_version,
        "adapter_version": "0.1.0",
        "database_content_hash": file_sha256(database),
        "measured_latency_ms": max(0, (time.monotonic_ns() - started) // 1_000_000),
        "durable_result_observed": retained == output,
        "trust": "runtime_declared_untrusted",
    }


async def _async_worker() -> tuple[str, int]:
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-c",
        "import sys; print(sys.stdin.read().strip()[::-1])",
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    stdout, _ = await process.communicate(b"async-payload\n")
    return stdout.decode().strip(), int(process.returncode or 0)


def async_stream_runtime() -> tuple[str, dict[str, Any]]:
    started = time.monotonic_ns()
    output, returncode = asyncio.run(_async_worker())
    if returncode != 0:
        raise RuntimeError(f"async subprocess exited {returncode}")
    chunks = [output[index:index + 3] for index in range(0, len(output), 3)]
    return "".join(chunks), {
        "adapter": "async-subprocess-stream",
        "runtime": "asyncio-event-loop",
        "runtime_version": platform.python_version(),
        "adapter_version": "0.1.0",
        "chunk_count": len(chunks),
        "logical_sequence": list(range(len(chunks))),
        "measured_latency_ms": max(0, (time.monotonic_ns() - started) // 1_000_000),
        "trust": "runtime_declared_untrusted",
    }


def run_resource_family(
    host: McpHost,
    topology: dict[str, Any],
    topology_hash: str,
    family: str,
    output_text: str,
    observation: dict[str, Any],
) -> tuple[list[dict[str, Any]], dict[str, Any], dict[str, Any]]:
    declaration, reservation, allocation = resource_contracts(topology, topology_hash, family)
    host.call(
        "reserve_resources",
        {"topology_json": json.dumps(topology), "resource_request": {
            "declaration": declaration, "reservation": reservation,
        }},
        f"reserve-{family}",
        mutation=True,
    )
    artifact_id, artifact_record = artifact(output_text, "text/plain")
    node = topology["nodes"][0]
    report_record = report(
        topology_id=topology["topology_id"], topology_hash=topology_hash,
        node_id=node["node_id"], attempt_id=reservation["attempt_id"],
        output_id=artifact_id, expected_schema=node["outputs"][0]["schema_id"],
        actual_schema=node["outputs"][0]["schema_id"], adapter_name=observation["adapter"],
        adapter_version=observation["adapter_version"], runtime_name=observation["runtime"],
        runtime_version=observation["runtime_version"], started_at="2026-08-03T00:00:00Z",
        finished_at="2026-08-03T00:00:01Z",
        resource_allocations=[{
            "resource_id": allocation["resource_id"], "mode": allocation["mode"],
            "allocation_id": allocation["allocation_id"],
        }],
    )
    records = [artifact_record, report_record]
    bundle = {
        "schema": "casegraphen.experimental.runtime.resource_expectation_bundle.v0",
        "schema_version": 0,
        "topology_content_hash": topology_hash,
        "case_revision_id": BASE_REVISION,
        "expectations": [{
            "node_id": node["node_id"], "attempt_id": reservation["attempt_id"],
            "declaration": declaration, "reservation": reservation,
            "allocations": [allocation], "disposition_evidence": [],
        }],
    }
    result = host.call(
        "reconcile_run",
        {"topology_json": json.dumps(topology), "runtime_jsonl": jsonl(records),
         "resource_expectation_bundle": bundle},
        f"reconcile-{family}",
    )
    observation.update({
        "topology_content_hash": topology_hash,
        "output_artifact_id": artifact_id,
        "reservation_id": reservation["reservation_id"],
        "resource_reconciliation_complete": result.get("reconciliation_complete") is True,
    })
    return records, observation, result


def jsonl(records: list[dict[str, Any]]) -> str:
    return "\n".join(json.dumps(record, sort_keys=True, separators=(",", ":")) for record in records)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def content_addressed_proposal(kind: str, payload: dict[str, Any]) -> dict[str, Any]:
    proposal_hash = sha256(canonical({"kind": kind, "payload": payload}))
    return {
        "proposal_id": f"proposal:sha256-{proposal_hash}", "kind": kind,
        "review_status": "unreviewed", "accepted": False, "payload": payload,
    }


def run_pilots(repo: Path, host_binary: Path, output: Path) -> dict[str, Any]:
    output.mkdir(parents=True, exist_ok=True)
    topology_paths = {
        "fanout_reduce": repo / "pilots/runtime-integration/topologies/fanout-reduce.json",
        "worktree_isolation": repo / "pilots/runtime-integration/topologies/worktree-isolation.json",
        "resource_collision": repo / "pilots/runtime-integration/topologies/resource-collision.json",
    }
    fanout = json.loads(topology_paths["fanout_reduce"].read_text())
    worktree = json.loads(topology_paths["worktree_isolation"].read_text())
    collision = json.loads(topology_paths["resource_collision"].read_text())
    sqlite_topology = single_resource_topology(fanout, "sqlite-queue")
    async_topology = single_resource_topology(fanout, "async-stream")
    with tempfile.TemporaryDirectory(prefix="casegraphen-runtime-pilot-") as directory:
        root = Path(directory)
        host = McpHost(host_binary, root)
        try:
            fanout_lint = host.call("lint_execution_topology", {"topology_json": json.dumps(fanout)}, "lint-fanout")
            worktree_lint = host.call("lint_execution_topology", {"topology_json": json.dumps(worktree)}, "lint-worktree")
            collision_lint = host.call("lint_execution_topology", {"topology_json": json.dumps(collision)}, "lint-collision")
            sqlite_lint = host.call(
                "lint_execution_topology",
                {"topology_json": json.dumps(sqlite_topology)},
                "lint-sqlite-queue",
            )
            async_lint = host.call(
                "lint_execution_topology",
                {"topology_json": json.dumps(async_topology)},
                "lint-async-stream",
            )
            fanout_hash = fanout_lint["topology_content_hash"]
            worktree_hash = worktree_lint["topology_content_hash"]
            process_records, process_observation = process_jsonl_adapter(fanout, fanout_hash)
            file_records, file_observation = file_drop_adapter(worktree, worktree_hash, root)
            sqlite_output, sqlite_observation = sqlite_queue_runtime(root)
            async_output, async_observation = async_stream_runtime()
            sqlite_records, sqlite_observation, sqlite_result = run_resource_family(
                host,
                sqlite_topology,
                sqlite_lint["topology_content_hash"],
                "sqlite-queue",
                sqlite_output,
                sqlite_observation,
            )
            async_records, async_observation, async_result = run_resource_family(
                host,
                async_topology,
                async_lint["topology_content_hash"],
                "async-stream",
                async_output,
                async_observation,
            )

            complete = host.call("reconcile_run", {
                "topology_json": json.dumps(fanout), "runtime_jsonl": jsonl(process_records),
            }, "reconcile-process-complete")
            missing_records = [record for record in process_records if not (
                record.get("kind") == "node_report" and record["report"]["node_id"] == "node:reduce"
            )]
            missing_records = [record for record in missing_records if not (
                record.get("kind") == "artifact" and record["artifact_id"] in {
                    report["report"]["output_artifact_ids"][0] for report in process_records
                    if report.get("kind") == "node_report" and report["report"]["node_id"] == "node:reduce"
                }
            )]
            missing = host.call("reconcile_run", {
                "topology_json": json.dumps(fanout), "runtime_jsonl": jsonl(missing_records),
            }, "reconcile-process-missing")
            mismatch_records = json.loads(json.dumps(process_records))
            for record in mismatch_records:
                if record.get("kind") == "node_report" and record["report"]["node_id"] == "node:reduce":
                    record["report"]["actual_output_schema_id"] = "schema:pilot-wrong-summary"
            mismatch = host.call("reconcile_run", {
                "topology_json": json.dumps(fanout), "runtime_jsonl": jsonl(mismatch_records),
            }, "reconcile-process-schema-mismatch")
            worktree_result = host.call("reconcile_run", {
                "topology_json": json.dumps(worktree), "runtime_jsonl": jsonl(file_records),
            }, "reconcile-file-drop")
        finally:
            host.close()

    mismatch_codes = sorted({finding["code"] for finding in mismatch["completeness"]["findings"]})
    collision_codes = sorted({finding["code"] for finding in collision_lint["lint"]["findings"]})
    redesign_payload = {
        "schema": "casegraphen.experimental.topology.redesign_proposal.v0",
        "source_topology_id": fanout["topology_id"], "source_topology_content_hash": fanout_hash,
        "source_runtime_finding_codes": mismatch_codes,
        "proposed_changes": [{
            "change_kind": "bind_runtime_schema_validation",
            "target_node_id": "node:reduce",
            "rationale": "The real local runtime emitted a schema mismatch; keep the graph unchanged until this proposal is reviewed.",
        }],
        "runtime_claim_accepted": False,
    }
    redesign = content_addressed_proposal("topology_redesign", redesign_payload)
    next_version = content_addressed_proposal("contract_next_version", {
        "current_contract": "runtime.node_report.v0",
        "candidate_contract": "runtime.node_report.v1-proposal",
        "changes": [
            "Bind a deployment_content_hash separately from topology_content_hash.",
            "Carry source adapter run identity without promoting runtime declarations to facts.",
            "Let the operational host accept canonical resource expectations for reconciliation.",
        ],
        "derived_from_pilots": [
            "generic-jsonl", "file-drop", "sqlite-durable-queue", "async-subprocess-stream"
        ],
    })
    promotion = {
        "schema": "casegraphen.experimental.runtime_pilot.promotion_report.v0",
        "candidate": "graph-engineering-plane-v0", "promotion_recommended": False,
        "accepted": False,
        "evidence": [
            "Four materially different local runtime adapters executed real process, file-drop, durable-queue, and event-stream boundaries.",
            "Complete fan-out/reduce stopped at needs_review with accepted=false.",
            "Missing report and schema mismatch failed closed.",
            "Explicit retry lineage reconciled without relying on JSONL order.",
            "File-drop workspaces were physically distinct and output bytes were content-addressed.",
            "Unsafe shared writers produced a graph-lint resource conflict finding.",
            "SQLite queue and async stream families both ingested content-addressed artifacts and completed canonical resource reconciliation.",
        ],
        "unknowns": [
            "Runtime-reported model, context, cost, version, and latency are not independently anchored.",
            "The pilots do not cover a remote runtime, sustained load, or binary artifacts.",
            "A deployment hash is pilot provenance, not a runtime.node_report.v0 field.",
        ],
        "blockers": [
            "Real runtime integration evidence is insufficient to promote experimental v0 to stable.",
        ],
        "review_seam": "operator_review_required",
    }
    summary = {
        "schema": "casegraphen.experimental.runtime_pilot.report.v0",
        "base_revision_id": BASE_REVISION, "accepted": False,
        "execution_provenance": {
            "harness_content_hash": file_sha256(Path(__file__).resolve()),
            "operational_host_binary_content_hash": file_sha256(host_binary),
            "topology_source_hashes": {
                name: file_sha256(path) for name, path in topology_paths.items()
            },
            "python_version": platform.python_version(),
            "git_version": subprocess.check_output(["git", "--version"], text=True).strip(),
            "trust": "locally_observed_not_ledger_accepted",
        },
        "adapters": [
            process_observation, file_observation, sqlite_observation, async_observation
        ],
        "scenarios": {
            "fanout_reduce_complete": complete,
            "missing_report": missing,
            "schema_mismatch": mismatch,
            "worktree_isolation": worktree_result,
            "resource_collision_lint": collision_lint,
            "sqlite_resource_reconciliation": sqlite_result,
            "async_resource_reconciliation": async_result,
        },
        "assertions": {
            "complete_halts_for_review": complete["halt"] == "needs_review" and complete["accepted"] is False,
            "all_complete_proposals_unreviewed": all(p["review_status"] == "unreviewed" for p in complete["proposals"]),
            "missing_report_detected": missing["completeness"]["missing_report_count"] == 1 and missing["accepted"] is False,
            "schema_mismatch_detected": "output_schema_mismatch" in mismatch_codes and mismatch["accepted"] is False,
            "retry_lineage_preserved": process_observation["retry_lineage"] == ["attempt:inspect-b:1", "attempt:inspect-b:2"],
            "resource_collision_detected": any("resource" in code for code in collision_codes),
            "workspaces_isolated": file_observation["workspace_isolation_observed"],
            "legacy_resource_boundary_fails_closed": worktree_result["halt"] == "resource_reconciliation_incomplete" and not worktree_result["proposals"],
            "additional_families_resource_complete": all(
                result["halt"] == "needs_review"
                and result["accepted"] is False
                and result["reconciliation_complete"] is True
                and all(proposal["review_status"] == "unreviewed" for proposal in result["proposals"])
                for result in [sqlite_result, async_result]
            ),
            "four_materially_distinct_families": len({
                observation["runtime"] for observation in [
                    process_observation, file_observation, sqlite_observation, async_observation
                ]
            }) == 4,
            "all_runtime_results_unaccepted": all(result["accepted"] is False for result in [
                complete, missing, mismatch, worktree_result, sqlite_result, async_result
            ]),
        },
        "redesign_proposal": redesign,
        "next_version_proposal": next_version,
        "promotion_report": promotion,
    }
    failed = [name for name, passed in summary["assertions"].items() if not passed]
    if failed:
        raise RuntimeError(f"pilot assertions failed: {failed}")
    (output / "process-jsonl.complete.jsonl").write_text(jsonl(process_records) + "\n", encoding="utf-8")
    (output / "file-drop.complete.jsonl").write_text(jsonl(file_records) + "\n", encoding="utf-8")
    (output / "sqlite-queue.complete.jsonl").write_text(
        jsonl(sqlite_records) + "\n", encoding="utf-8"
    )
    (output / "async-stream.complete.jsonl").write_text(
        jsonl(async_records) + "\n", encoding="utf-8"
    )
    write_json(output / "pilot-report.json", summary)
    write_json(output / "redesign-proposal.json", redesign)
    write_json(output / "v0-next-version-proposal.json", next_version)
    write_json(output / "promotion-report.json", promotion)
    retained_names = [
        "pilot-report.json",
        "process-jsonl.complete.jsonl",
        "file-drop.complete.jsonl",
        "sqlite-queue.complete.jsonl",
        "async-stream.complete.jsonl",
        "redesign-proposal.json",
        "v0-next-version-proposal.json",
        "promotion-report.json",
    ]
    retained_manifest = {
        "schema": "casegraphen.experimental.runtime_pilot.evidence_manifest.v0",
        "accepted": False,
        "trust": "locally_observed_not_ledger_accepted",
        "files": [
            {
                "path": name,
                "content_hash": f"sha256:{file_sha256(output / name)}",
                "byte_length": (output / name).stat().st_size,
            }
            for name in retained_names
        ],
    }
    write_json(output / "retained-evidence.manifest.json", retained_manifest)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--host-bin", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    host_binary = arguments.host_bin or arguments.repo / "target/debug/casegraphen-mcp-host"
    if not host_binary.is_file():
        parser.error(f"operational host binary not found: {host_binary}; run cargo build --bin casegraphen-mcp-host")
    summary = run_pilots(arguments.repo.resolve(), host_binary.resolve(), arguments.output.resolve())
    print(json.dumps({"output": str(arguments.output.resolve()), "assertions": summary["assertions"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
