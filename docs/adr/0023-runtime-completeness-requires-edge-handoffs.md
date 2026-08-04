# ADR 0023: Runtime completeness requires edge handoff proof

## Status

Accepted for experimental v0.

## Context

Node reports could prove that every expected node had one successful terminal
retry attempt, but they did not prove that topology data edges carried any
artifact. Independently executed nodes could therefore be described as a
complete graph. Generic JSONL did verify output bytes before emitting a
proposal, but that adapter-local check did not join terminal source outputs to
terminal target inputs and was not shared by streaming reconciliation.

## Decision

The canonical execution topology is projected once into the strict
`runtime.graph_expectation.v0` contract. Its data-edge expectations bind edge
and endpoint IDs, output/input names, schema, and source delivery mode. Runtime
reconciliation selects the canonical terminal attempt from each linear retry
lineage, verifies exact parent lineage, and proves exactly one shared artifact
for every required data edge. Artifact observations can only be constructed by
presenting bytes whose SHA-256 matches the content-addressed ID.

`RuntimeCompleteness` exposes `node_complete` and `dataflow_complete`
separately. `complete` is their conjunction and additionally requires no
deterministic finding. Streaming uses the same topology projection, terminal
attempt selection, and byte observations; an early release cannot be proposed
from a nonterminal producer, nonfinal artifact, substituted edge, or absent
bytes.

## Consequences

- A successful node set without handoffs is diagnosable as node-complete but
  is not graph-complete.
- Fan-out may reuse one source artifact across multiple edges, while a single
  edge carrying multiple candidate artifacts is ambiguous and refused.
- Retry outputs from superseded attempts cannot satisfy downstream edges.
- Runtime output remains untrusted. Even graph-complete integration emits only
  unreviewed proposals and stops at the review seam.
- Historical pilot reports that predate this result shape are not evidence of
  edge completeness and must be rerun before promotion claims use them.
