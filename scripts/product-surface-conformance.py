#!/usr/bin/env python3
"""Keep the documented Graph Engineering product surface on one inventory."""

from __future__ import annotations

import json
import pathlib
import sys


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    inventory = json.loads((root / "docs/product-surface.v0.json").read_text())
    catalog = json.loads(
        (root / "schemas/experimental/control_plane.catalog.v0.schema.json").read_text()
    )["properties"]["tools"]["const"]
    request_tools = json.loads(
        (root / "schemas/experimental/control_plane.request.v0.schema.json").read_text()
    )["properties"]["tool"]["enum"]
    request_schema = json.loads(
        (root / "schemas/experimental/control_plane.request.v0.schema.json").read_text()
    )
    adr = (root / "docs/adr/0020-graph-engineering-product-surface.md").read_text()
    failures: list[str] = []

    if catalog != request_tools:
        failures.append("control-plane catalog and request schema tool order differ")
    request_properties = request_schema["properties"]
    if "operation_gate" in request_properties:
        failures.append("MCP request schema ambiguously exposes an operation gate")
    if "caller_declared_audit_context" not in request_properties:
        failures.append("MCP request schema omits caller-declared audit context")
    if not (root / "src/bin/casegraphen-mcp-host.rs").is_file():
        failures.append("operational host binary is missing")
    cargo = (root / "Cargo.toml").read_text()
    usage = (root / "src/cli_usage.txt").read_text()
    readme = (root / "README.md").read_text()
    if "casegraphen-mcp-host" not in cargo:
        failures.append("operational host is absent from the package manifest")
    if "casegraphen-mcp-host" not in usage:
        failures.append("operational host is absent from CLI usage")
    if "product-surface.v0.json" not in readme:
        failures.append("README does not link the canonical product-surface inventory")
    invariants = inventory["invariants"]
    for required_invariant in (
        "host_access_requires_bearer_authentication",
        "host_state_changes_require_caller_declared_audit_context",
        "caller_declared_audit_context_authorizes_nothing",
        "acceptance_ledger_mutations_require_canonical_operation_gates",
        "resource_allocator_state_is_host_canonical",
        "operational_resource_reservations_require_reviewed_deployment_authority",
        "resource_bearing_runtime_reconciliation_requires_a_versioned_expectation_bundle",
        "verification_lineage_proofs_are_canonical_opaque_and_never_serialized",
        "memory_tools_never_mutate_accepted_state",
        "memory_indexes_are_derived_and_non_authoritative",
    ):
        if invariants.get(required_invariant) is not True:
            failures.append(f"missing authority invariant: {required_invariant}")
    host_guide = (root / "docs/guides/mcp-operational-host.md").read_text()
    if "The bearer token authorizes access to host tools" not in host_guide:
        failures.append("host guide does not state the bearer authorization boundary")
    if "They are not a CaseGraphen operation gate" not in host_guide:
        failures.append("host guide does not deny audit-context authorization")

    seen: set[str] = set()
    for workflow in inventory["workflows"]:
        name, tool, owner = workflow["workflow"], workflow["tool"], workflow["owner"]
        if name in seen:
            failures.append(f"duplicate workflow: {name}")
        seen.add(name)
        if tool not in catalog:
            failures.append(f"{name}: {tool} is absent from the MCP catalog")
        if not (root / owner).is_file():
            failures.append(f"{name}: canonical owner does not exist: {owner}")
        if tool not in adr:
            failures.append(f"{name}: ADR omits {tool}")
        for skill in workflow["skills"]:
            skill_file = root / "skills" / skill / "SKILL.md"
            if not skill_file.is_file() or tool not in skill_file.read_text():
                failures.append(f"{name}: {skill} does not name {tool}")

    expected = {
        "compile", "reviewed_compile", "integrate_reconcile", "simulate", "resource_reserve",
        "resource_release", "resource_reconcile", "expansion", "streaming",
        "verification_lineage", "redesign", "memory_query", "memory_explain",
        "memory_history", "memory_conflicts", "memory_sources",
        "memory_propose_claim", "memory_propose_supersession",
        "memory_propose_retraction", "memory_propose_procedure",
    }
    if seen != expected:
        failures.append(f"workflow inventory differs: {sorted(seen ^ expected)}")

    if failures:
        for failure in failures:
            print(f"product-surface-conformance: {failure}", file=sys.stderr)
        return 1
    print(f"product-surface-conformance: ok ({len(seen)} workflows, {len(catalog)} tools)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
