# Execution topology review v0

Execution topology acceptance is a dedicated, content-bound review contract.
It is not a plan review and it is not an evidence review. The canonical review
record fixes the topology id, canonical topology hash, case-space id, revision
observed by the reviewer, claim cell, and immutable content-addressed artifact.

The CLI surface is:

```text
casegraphen topology-review accept  --target-id <claim> --input <topology.json> ...
casegraphen topology-review reject  --target-id <claim> --input <topology.json> ...
casegraphen topology-review reopen  --target-id <claim> --input <topology.json> ...
casegraphen topology-review inspect --target-id <claim> ...
```

Mutation commands require the existing `review` operation gate; their target
contract remains distinct from generic evidence and plan reviews. The
input bytes must reproduce the artifact id already joined to the claim by a
`derives_from` relation, and their parsed topology must reproduce the recorded
canonical topology hash. Artifact existence by itself never means acceptance.

## Replay and migration

The native review envelope remains version 1. Execution-topology records add
the explicit `casegraphen.experimental.execution_topology_review.v0` nested
contract. Existing plan/evidence reviews replay unchanged, but they are never
interpreted as execution-topology authority. There is intentionally no
in-place migration: a topology accepted before this contract must be reviewed
again through `topology-review accept`. This preserves append-only history and
prevents an old target-id-only review from acquiring new authority after the
fact.

The graph compiler derives reviewed mode only from this canonical nested
record. Current claim metadata and caller-supplied hashes are consistency data,
not authority.
