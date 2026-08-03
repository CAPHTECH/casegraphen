# ADR 0022: Atomic resource allocation and runtime expectations

Status: accepted for experimental v0

## Decision

The operational MCP host owns a durable resource-allocation journal. A client
may submit a topology-bound declaration and requested reservation, but it may
not submit the active reservation set, disposition set, or rate-limit
capacities. Those inputs are reconstructed from the allocator journal and the
host configuration. Compatibility, conflict, capacity, and active-disposition
rules remain canonical in `resource_protocol`.

Allocator events are append-only and hash chained. Publication uses an atomic
create-new filesystem boundary after the complete event has been written and
synced. A crash before publication leaves an ignored temporary file; a crash
after publication leaves a complete replayable event. Reusing an idempotency
key with identical content replays the event, while different content refuses.
Release and expiry are explicit disposition events. Supersession additionally
requires the replacement reservation to exist and remain active. No wall-clock
expiry is inferred by replay.

Resource-bearing runtime reconciliation uses
`runtime.resource_expectation_bundle.v0`. The bundle binds the exact topology
content hash and client-observed case revision to node/attempt identities,
declarations, reservations, allocations, and disposition evidence. The host
checks reservation and disposition records against the allocator journal,
then delegates allocation reconciliation to `runtime_integration` and
`resource_protocol`. A missing or mismatched resource boundary cannot become a
complete run. A complete run still emits only unreviewed proposals and stops at
`needs_review`.

## Consequences

- Multiple host processes sharing a journal cannot grant conflicting exclusive
  reservations or exceed configured rate capacity.
- Changing capacity configuration can make replay refuse; it cannot silently
  weaken an earlier capacity boundary.
- Resource-free `reconcile_run` requests retain their v0 behavior. Resource-
  bearing requests need the versioned bundle to reach the review seam.
- The journal is authoritative operational state, not acceptance-ledger truth.
  Allocation and runtime reports remain untrusted observations.
