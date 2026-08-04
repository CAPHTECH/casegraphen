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

For a native shell-worker claim, the subsequent read-only authority assessment
is `reconcile_verification_lineage`. It consumes the exact retained run files
and canonical review morphism IDs at the current revision, derives opaque
proofs inside the host, and returns only the verification-policy result with
`accepted:false`; it does not mutate the acceptance ledger.

`compile_deployment_bundle` remains the proposal-only inspection path. The reviewed tool accepts no caller-created mode or authority hash: it replays the exact case space and derives compilation authority from the canonical accepted topology review. Resource reservations additionally name the resulting bundle hash and persist that review/deployment binding in the allocator journal.

Bundle verification dispatches by the exact retained compiler-input schema and
compiler profile. Profile 1 binds its implementation identity plus topology,
plan, policy, manifest, report, and input contract identities. Historical
profile 0 is verified by its retained exact implementation; unknown and future
profiles are refused. A profile migration is an unaccepted proposal requiring
separate review, not an in-place reinterpretation. See
[ADR 0027](../adr/0027-exact-compiler-profile-compatibility.md).

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
proposals only. The v0 `streaming` workflow is specifically terminal-artifact
stage pipelining: it can release a stage after its canonical producer finishes,
but it does not authorize consumption of chunks from a running producer.

MCP domain refusals are returned in `structuredContent.refusal` with `isError: true`. Authentication and malformed JSON-RPC are protocol errors. A repeated idempotency key replays the durable result; a crash with an ambiguous prior effect refuses instead of delegating twice.
