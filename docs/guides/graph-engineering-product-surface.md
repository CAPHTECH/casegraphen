# Graph Engineering Plane through the operational MCP host

`casegraphen-mcp-host` is the supported standalone v0 entry point for the workflows listed in [`../product-surface.v0.json`](../product-surface.v0.json). Start it with durable replay state, a CaseGraphen store, an artifact directory, and an authorization-token environment variable:

```text
casegraphen-mcp-host --state state.json --store case-store --artifacts artifacts --auth-token-env CASEGRAPHEN_MCP_TOKEN
```

Use MCP `initialize`, `notifications/initialized`, then `tools/call`. Each tool argument contains `request_id`, `idempotency_key`, and `payload`. Case-bound workflows also carry the exact client-observed `base_revision_id`; host-managed state changes carry `caller_declared_audit_context` for attribution. Bearer authentication authorizes host access. The audit context is not a CaseGraphen operation gate, and the host never substitutes `current` for the observed revision.

The end-to-end sequence is:

```text
propose_execution_topology
→ lint_execution_topology
→ compile_deployment_bundle
→ attach_runtime_report
→ reconcile_run
→ accepted:false + unreviewed proposals
→ independent `casegraphen topology-review` / evidence review
```

The runtime JSONL must include the exact topology content hash and content-addressed artifact bytes. Missing reports, schema mismatches, resource mismatches, or unaccounted artifacts keep completeness false. Reconciliation never accepts runtime output. Expansion, simulation, streaming, and redesign likewise emit reports or unreviewed proposals only.

MCP domain refusals are returned in `structuredContent.refusal` with `isError: true`. Authentication and malformed JSON-RPC are protocol errors. A repeated idempotency key replays the durable result; a crash with an ambiguous prior effect refuses instead of delegating twice.
