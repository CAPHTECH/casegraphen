# Generic JSONL boundary

Each non-empty line is one strict JSON object. JSONL ordering carries no
authority and does not define retry lineage.

Artifact envelope:

```json
{"kind":"artifact","artifact_id":"artifact:sha256-<sha256-of-content-utf8>","media_type":"application/json","content":"{\"result\":true}"}
```

Node report envelope:

```json
{"kind":"node_report","report":{"schema":"casegraphen.experimental.runtime.node_report.v0"}}
```

Typed resource-allocation envelope:

```json
{"kind":"resource_allocation","allocation":{"schema":"casegraphen.experimental.runtime.resource_allocation.v0"}}
```

The full allocation is the separate `runtime.resource_allocation.v0` record.
The compact `resource_allocations` summaries inside a node report remain
runtime-declared metadata and never substitute for reservation reconciliation.
Supply the exact topology-bound declaration and granted reservation to
`reconcile_with_resources`; a missing, orphaned, or mismatched allocation keeps
the integration incomplete and emits no review proposal.

The complete nested report must satisfy `runtime.node_report.v0`; the abbreviated
example only shows envelope placement. `content` is UTF-8 text in v0 and its
exact bytes determine `artifact_id`. Binary payloads require a future typed
encoding rather than implicit base64 guessing.

The adapter deduplicates exact `report_id` and `artifact_id` replays. A reused
identifier with different canonical report content or bytes is a finding. A
complete reconciliation emits only content-addressed, unreviewed proposals and
halts for review. It never writes the case ledger.
