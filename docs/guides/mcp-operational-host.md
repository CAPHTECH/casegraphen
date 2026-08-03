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
  --auth-token-env CASEGRAPHEN_MCP_TOKEN
```

Clients add `authorization` to each post-initialization request's `params`.
The token is held in memory and is excluded from journal state and responses.
Use filesystem ownership and service-manager secret injection; never put the
token in argv, a case store, a topology, or an artifact directory.

The host journal is one atomically replaced JSON file. The directory and file
must be private to one host instance. Request ids and semantic idempotency keys
survive restart. A crash after delegation but before durable acknowledgement
returns `ambiguous_prior_effect` on replay instead of duplicating the effect.
The operator reconciles the existing CaseGraphen/artifact state and submits a
new explicit request; the host never guesses.

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
main CLI. Mutations still require the client-observed base revision and
operation gate.

`casegraphen-mcp-host --health-check` reports process capability without
opening a store or reading secrets. Protocol refusals and content hashes are
the audit surface; the host emits only newline-delimited JSON-RPC on stdout.
Run it under a supervisor for restart and stderr capture. It opens no network
listener, schedules no work, calls no model, performs no automatic retries,
and never resumes because of a notification.
