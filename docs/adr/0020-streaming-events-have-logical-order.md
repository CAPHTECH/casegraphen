# ADR 0020: Streaming Events Have Logical Order, Not Acceptance Authority

## Status

Accepted on 2026-08-03. Resolves issue #53.

## Context

`run --frontier` deliberately waits for a frontier and appends results in plan
order. An external runtime can pipeline artifact production sooner, but arrival
order is nondeterministic and runtime output remains untrusted under ADR 0002.
Applying completion order directly to the CaseGraphen log would make replay,
readiness, and attribution vary with network timing.

## Decision

Streaming remains outside the core scheduler boundary. Runtime events carry a
topology id and content hash, node and attempt ids, an attempt-local sequence,
an event id, and a deployment-assigned logical order. Reconciliation first
deduplicates exact event identity, refuses identity collisions and graph-join
mismatches, and sorts by logical order plus stable identity. Arrival and
completion timestamps never select accepted log order.

Artifact chunks may produce an **unaccepted early-release proposal** only when:

1. the named edge is a typed data edge from the emitting node and its schema
   matches;
2. the source topology node declares streaming delivery;
3. the target has an opaque permit derived from the exact topology
   hash/node/claim declaration, reservation attempt, and canonical resource
   reconciliation emitted by runtime integration, rather than a caller-owned
   permission boolean or node map; and
4. every evidence/review/authority edge into the target is satisfied by an
   opaque readiness projection produced by CaseGraphen's canonical evaluator
   for the same case/topology revision.

The proposal does not dispatch work and does not append a morphism. A runtime
may consume it under its own scheduler; CaseGraphen later receives ordinary
unreviewed evidence and gated transitions. Resource reservations, not sibling
completion timing, decide whether parallel work can safely overlap.
The proposal also names the target attempt carried by the opaque permit, so a
runtime cannot safely reinterpret it as authority for a different attempt.

Events for distinct `(node_id, attempt_id, sequence)` identities are
commutative observations after canonical sorting. Conflicting bytes under one
event id are not commutative and are refused. A sibling result may enable a
runtime proposal only through an already accepted topology edge; it never
changes CaseGraphen readiness by itself.

The topology itself must join the expectation by topology id and content hash.
Different event ids claiming the same attempt-local sequence, repeated/gapped
chunk indices, or a final chunk before a later chunk make the stream prefix
ambiguous. No early-release proposal is emitted until that ambiguity is
resolved; stable sorting is not used to guess an authority order.

Partial status is explicit. `partially_progressing` means at least one safe
release proposal exists while terminal completeness is false. `collecting`
means no release is currently safe. Closing such a run yields
`incomplete_terminal`. Final `complete` is delegated unchanged to
`reconcile_runtime_reports`, including missing nodes, retry lineage, schemas,
failures, and artifact accounting.

Chunk payload bytes stay outside this contract. Each event names an artifact,
chunk index, schema, and SHA-256. The reconciler validates identity/hash/schema;
the integrator content-addresses bytes and the normal review path decides
acceptance. Partially completed rounds are reconstructed from the canonical
event set and terminal reports.

## Consequences

- Duplicate, delayed, and out-of-order delivery has deterministic replay.
- A slow sibling does not block safe runtime pipeline progress, and remains
  visible in `unfinished_node_ids` and completeness findings.
- Completion order cannot accidentally become accepted log order.
- The core gains no message bus, daemon, scheduler, retry engine, or model call.
- Logical-order assignment is a deployment protocol responsibility. A producer
  that equivocates on event identity is refused rather than guessed around.

## Rejected alternatives

- Append in completion order: faster locally, nondeterministic globally.
- Treat every data chunk as accepted evidence: violates ADR 0001/0002.
- Wait for all siblings before emitting any proposal: preserves the existing
  barrier but defeats the pipeline use case without adding safety beyond the
  typed edge/resource/acceptance checks above.
