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
→ attach topology/policy artifacts + topology-review accept
→ compile_reviewed_deployment_bundle
→ attach_runtime_report
→ reconcile_run
→ accepted:false + unreviewed proposals
→ independent `casegraphen topology-review` / evidence review
```

`compile_deployment_bundle` remains the proposal-only inspection path. The reviewed tool accepts no caller-created mode or authority hash: it replays the exact case space and derives compilation authority from the canonical accepted topology review. Resource reservations additionally name the resulting bundle hash and persist that review/deployment binding in the allocator journal.

The runtime JSONL must include the exact topology content hash and
content-addressed artifact bytes. The host derives
`runtime.graph_expectation.v0` from the canonical topology; callers do not
supply a second edge rule. `node_complete` means each node has one valid
terminal retry attempt. `dataflow_complete` additionally means every data edge
has exactly one artifact produced by the terminal source and consumed by the
terminal target, with matching parent lineage, output/input/schema/delivery
binding, content hash, and ingested bytes. Only their conjunction is
`complete`. Missing reports, schema mismatches, resource mismatches,
substituted/un-ingested handoffs, or unaccounted artifacts keep graph
completeness false. Reconciliation never accepts runtime output. Expansion,
simulation, streaming, and redesign likewise emit reports or unreviewed
proposals only.

MCP domain refusals are returned in `structuredContent.refusal` with `isError: true`. Authentication and malformed JSON-RPC are protocol errors. A repeated idempotency key replays the durable result; a crash with an ambiguous prior effect refuses instead of delegating twice.
