# Operational MCP host

`casegraphen-mcp-host` is the durable authenticated host for the experimental
MCP-compatible control-plane boundary. `casegraphen-mcp` remains the stateless
reference adapter.

```sh
export CASEGRAPHEN_MCP_TOKEN='<injected by the service manager>'
casegraphen-mcp-host \
  --state /var/lib/casegraphen-mcp/protocol-state.json \
  --store /var/lib/casegraphen \
  --artifacts /var/lib/casegraphen-mcp/projections \
  --resource-journal /var/lib/casegraphen-mcp/resource-journal \
  --resource-capacities /etc/casegraphen/resource-capacities.json \
  --auth-token-env CASEGRAPHEN_MCP_TOKEN
```

Clients add `authorization` to each post-initialization request's `params`.
The token is held in memory and is excluded from journal state and responses.
Use filesystem ownership and service-manager secret injection; never put the
token in argv, a case store, a topology, or an artifact directory.

The bearer token authorizes access to host tools. For tools that change host-
managed state, clients also provide `caller_declared_audit_context` with
`declared_actor_id`, `declared_capability_ids`,
`declared_operation_scope_id`, `declared_audience`, and
`declared_source_boundary_id`. Those values are audit attribution supplied by
the caller. They are not a CaseGraphen operation gate and do not prove that an
actor holds a capability. Responses record bearer authentication separately
and report canonical CaseGraphen authorization as `not_evaluated`.

The host journal is one atomically replaced JSON file. The directory and file
must be private to one host instance. Request ids and semantic idempotency keys
survive restart. A crash after delegation but before durable acknowledgement
returns `ambiguous_prior_effect` on replay instead of duplicating the effect.
The operator reconciles the existing CaseGraphen/artifact state and submits a
new explicit request; the host never guesses.

Resource allocation has a separate append-only, hash-chained journal. The host
derives active reservations, release/expiry/supersede dispositions, and rate
capacity from that journal and its startup configuration. `reserve_resources`
therefore does not accept caller-supplied existing reservations, dispositions,
or capacities. `release_resources` records an explicit disposition. Atomic
event publication means an unpublished temporary event is ignored after a
crash, while a published event is replayed idempotently after restart. An
operational reservation must name a persisted reviewed deployment bundle,
claim cell, and exact accepted-review revision. The host verifies every bundle
artifact, re-derives authority from the CaseGraphen store, and journals the
topology, policy manifest, bundle, review, node, attempt, and declaration
hashes. Bearer authentication alone cannot reserve arbitrary topology work.

`compile_deployment_bundle` is proposal-only. After topology and policy
artifacts are attached and accepted through `casegraphen topology-review`, use
`compile_reviewed_deployment_bundle`; the host accepts the claim cell and
case-space identity, replays the exact revision, and derives the opaque mode
through the canonical compiler. It never accepts a caller-supplied mode,
review record, or authority hash.

For a resource-bearing `reconcile_run`, pass a
`runtime.resource_expectation_bundle.v0` naming the exact topology hash, base
revision, node/attempt joins, declarations, allocator-issued reservations,
runtime allocations, and any disposition evidence. The host rejects stale,
duplicate, substituted, or noncanonical records. Canonical reconciliation may
then reach `needs_review`; it never changes `accepted: false`. Resource-free
runs may omit the bundle and preserve the original v0 path.

Configured resources are projections:

- space status/frontier/reviews/revisions come from replay and canonical
  evaluators in the configured CaseGraphen store;
- `runs/{id}`, `topologies/{id}`, and space halt projections are exact JSON
  files in `runs/`, `topologies/`, and `halts/`, identity-checked and hashed by
  the host;
- none of these projections or notifications authorizes a mutation.

The operational host binds the workflows in
[`../product-surface.v0.json`](../product-surface.v0.json): topology
proposal/lint and compilation, content-addressed runtime JSONL attachment and
reconciliation, simulation, resource reservation/reconciliation, bounded
expansion, streaming reconciliation, and redesign proposals. Acceptance-ledger
mutations refuse `unsupported_operational_host_tool` and remain owned by the
main CLI. Host requests still require the client-observed base revision and
caller audit context where applicable. An actual acceptance-ledger mutation is
separately authorized by the CLI/store's canonical operation gate; the host
context cannot substitute for it.

`casegraphen-mcp-host --health-check` reports process capability without
opening a store or reading secrets. Protocol refusals and content hashes are
the audit surface; the host emits only newline-delimited JSON-RPC on stdout.
Run it under a supervisor for restart and stderr capture. It opens no network
listener, schedules no work, calls no model, performs no automatic retries,
and never resumes because of a notification.

## Independent client evidence

The repository includes a Python-standard-library client that exercises the
wire protocol without linking CaseGraphen or adding custom Rust client code:

```sh
cargo build --bin casegraphen-mcp-host
python3 scripts/independent-mcp-client.py \
  --host-bin target/debug/casegraphen-mcp-host \
  --topology pilots/runtime-integration/topologies/fanout-reduce.json \
  --output target/independent-mcp-client-report.json
```

The client performs initialize, topology proposal, lint, proposal compilation,
content-addressed runtime attachment, and complete runtime reconciliation. It
fails unless every pre-review artifact remains `accepted: false`, the complete
run halts with `needs_review`, and every resulting proposal is `unreviewed`.
The JSON report records the exact host and topology hashes so a release process
can retain and content-address the evidence. This is interoperability evidence,
not an acceptance action: topology/evidence review remains owned by the main
CaseGraphen CLI and its canonical gates.
