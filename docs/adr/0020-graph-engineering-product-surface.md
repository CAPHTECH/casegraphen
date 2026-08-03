# ADR 0020: Graph Engineering product surface

Status: accepted for experimental v0

## Decision

The machine-readable source of truth is [`docs/product-surface.v0.json`](../product-surface.v0.json). The standalone supported entry point is the durable, authenticated `casegraphen-mcp-host`. It delegates to the canonical library owner; the host must not reimplement readiness, compilation, completeness, resource, expansion, streaming, or redesign decisions.

| Workflow | MCP tool | Canonical decision owner | Output/refusal boundary |
|---|---|---|---|
| compile | `compile_deployment_bundle` | `graph_compiler` | content-addressed deployment bundle; proposal mode; never accepted |
| integrate/reconcile | `reconcile_run` | `runtime_integration` | untrusted runtime integration report and reviewable proposals |
| simulate | `simulate_execution_topology` | `graph_simulation` | deterministic simulation report and unreviewed routing proposal |
| resource reserve/release | `reserve_resources`, `release_resources` | `resource_allocator` delegating to `resource_protocol` | atomic durable reservation/disposition; revision and caller-declared audit context required; caller allocator state is rejected |
| resource reconcile | `reconcile_resources` | `resource_protocol` | untrusted allocation reconciliation; incomplete on mismatch |
| expansion | `evaluate_expansion_round` | `dynamic_expansion` | bounded, typed, unreviewed topology proposals only |
| streaming | `reconcile_streaming_run` | `streaming_reconciliation` | exact current case revision, canonical readiness and resource permits |
| redesign | `propose_topology_redesign` | `topology_redesign` | content-bound unreviewed redesign proposal only |

All MCP calls return `casegraphen.experimental.control_plane.response.v0`. A domain refusal is a response with `result: null` and a typed `refusal`; JSON-RPC framing/authentication errors use JSON-RPC errors. The host process exits non-zero for invalid startup configuration and remains fail-closed for unsupported tools.

## End-to-end supported path

Without custom Rust code, an MCP client can:

1. call `propose_execution_topology`, then `lint_execution_topology`;
2. call `compile_deployment_bundle` with an explicit topology hash and observed base revision;
3. call `attach_runtime_report` for content-addressed JSONL bytes;
4. for a resource-bearing run, reserve through the host and call `reconcile_run` with an exact `runtime.resource_expectation_bundle.v0`; otherwise call it with the same topology and revision;
5. observe `accepted: false`, completeness findings, and content-addressed proposals;
6. stop at the independent CaseGraphen `topology-review` / evidence review seam.

The walkthrough and request transcript are in [`../guides/graph-engineering-product-surface.md`](../guides/graph-engineering-product-surface.md).

## Consequences

- The main `casegraphen` CLI remains the acceptance-ledger owner. The operational host exposes experimental graph-engineering workflows and does not become a scheduler, model caller, retry engine, or alternative acceptance authority.
- CLI/MCP parity is checked where both surfaces overlap (`graph lint`) and every additional MCP workflow is tested against its canonical Rust report boundary.
- Changing the supported surface requires updating the inventory; conformance fails if catalog schemas, ADR, README, Skills, package manifest, or usage drift.
