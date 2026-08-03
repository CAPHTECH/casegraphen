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
- state-changing host requests carry the client-observed `base_revision_id`
  and caller-declared audit context; the adapter never substitutes current and
  never represents that context as a validated CaseGraphen operation gate;
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

## Amendment: durable operational stdio host (2026-08-03)

Issue #69 adds `casegraphen-mcp-host` as the supported operational host package
without changing the reference adapter above. It remains an external stdio
process, but binds three production concerns that the reference binary
deliberately omits:

- an exact authorization token supplied by a named environment variable;
- an atomic, fsynced protocol journal that persists request/idempotency replay,
  notification cursors, and write-ahead pending effects across process restart;
- real resource owners backed by a configured CaseGraphen store and a configured
  content-addressed run/topology/halt projection directory.

Before a delegated effect, the host durably records a pending semantic
idempotency key. It commits the response before acknowledging it. A restart
that finds only the pending marker refuses `ambiguous_prior_effect` and never
delegates it again; an operator must reconcile the external effect. This is a
fail-closed at-most-once boundary, not a claim of distributed exactly-once
transactions.

ADR 0020 expands the operational read/proposal tool set to the complete
experimental Graph Engineering product-surface inventory: compile, generic
runtime integration/reconciliation, simulation, resource
reservation/reconciliation, bounded expansion, streaming reconciliation, and
redesign proposals. Acceptance-ledger mutation tools retain a typed refusal
until delegated to their existing CLI owner. The host does
not schedule, invoke models, retry work, interpret notifications as grants, or
auto-resume. `casegraphen-mcp` remains the stateless reference adapter and
continues to identify itself that way.

## Amendment: authorization vocabulary (2026-08-03)

ADR 0021 fixes the operational authorization model. The exact bearer token
authenticates and authorizes host-tool access. `caller_declared_audit_context`
is attribution only; its actor/capability/scope/audience/source declarations are
not checked by `check_operation_gate` and never claim canonical authority.
Operational responses record bearer authentication separately from canonical
CaseGraphen authorization (`not_evaluated`). Acceptance-ledger mutations remain
owned by the CLI/store canonical gate path and refused by this host.
