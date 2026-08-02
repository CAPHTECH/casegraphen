# ADR 0019: The MCP Control Plane Is an External Adapter

## Status

Accepted on 2026-08-03. Resolves issue #52.

## Context

Graph-engineering clients need long-lived resources, tools, notifications,
reconnect, and replay. ADR 0002 simultaneously excludes a daemon, message bus,
scheduler, model caller, and retry engine from the CaseGraphen core crate. An
MCP server placed inside core would erase that boundary and invite a second
implementation of readiness, gates, review state, completeness, compilation,
verification, and resource compatibility.

## Decision

`casegraphen-mcp` is an **external process/package adapter**. This repository
ships a transport-neutral protocol/state library, wire schemas, and a reference
newline-delimited stdio binary. The binary is a child-process transport, not a
daemon: it opens no listener, has no background lifecycle, and exits at stdin
EOF. An embedding server package may supply the decision/resource delegate and
own authentication, subscriptions, durable persistence, and process lifetime.
Every decision must call the existing CaseGraphen APIs or CLI adapter.

The trust boundary is explicit:

- MCP clients, runtime reports, model/context identity, notifications, and
  reconnect cursors are untrusted inputs;
- resource reads are projections of CaseGraphen/runtime state, not new facts;
- mutating requests carry the client-observed `base_revision_id` and operation
  gate; the adapter never substitutes current;
- stale revision is a structured refusal containing supplied and current ids;
- notifications describe observed state changes and grant no authority;
- request ids/content hashes make reconnect and replay idempotent; an id reused
  for different content is refused;
- runtime ingestion, compilation, reconciliation, resource reservation, and
  verification delegate to their existing owners;
- scheduling, model calls, retries, automatic resume, and time-based release
  remain outside both core and the adapter.

The transport-neutral library may remember request results, cursors,
notifications, ingest identities, and reservation identities. This is protocol
state, not an acceptance ledger. Production persistence and atomicity belong to
the external package and must preserve the same idempotency keys.

The reference stdio process preserves replay and idempotency only for its own
process lifetime. Restarting it starts an empty protocol state. It deliberately
does not imply crash-safe exactly-once behavior: a production wrapper must
persist request and semantic-idempotency records atomically with delegated
effects before acknowledging them. The reference binary binds deterministic
topology lint directly to its existing owner; operations and resource
projections without a configured external owner fail closed.

## Consequences

Core gains no listener, async runtime, daemon lifecycle, or dependency. The
stdio adapter implements MCP initialization, resource/tool discovery and calls,
plus explicit CaseGraphen replay/notification methods. MCP and CLI can be
compared at the decision/report boundary because both call the same
implementation. An external package has more integration work, but it cannot
silently reinterpret a CLI refusal or make notifications authoritative.

Adding a network transport, authentication scheme, durable state store, or
persistent server is a separate package decision. Moving any decision rule into
that package requires amending this ADR and ADR 0002.
