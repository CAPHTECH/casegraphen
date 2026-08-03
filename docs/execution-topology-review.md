# Execution topology review v0

The authority decision and exact reviewed-byte derivation are normative in
[ADR 0023](adr/0023-content-bound-topology-deployment-authority.md).

Execution topology acceptance is a dedicated, content-bound review contract.
It is not a plan review and it is not an evidence review. The canonical review
record fixes the topology id, canonical topology hash, canonical deployment
policy-manifest hash, case-space id, revision observed by the reviewer, claim
cell, and immutable content-addressed topology artifact. The policy manifest
binds every verification, budget, and expansion policy ID to canonical policy
bytes; a policy-ID-preserving content substitution therefore requires a new
review.

The CLI surface is:

```text
casegraphen topology-review accept  --target-id <claim> --input <topology.json> --policy-manifest <manifest.json> ...
casegraphen topology-review reject  --target-id <claim> --input <topology.json> --policy-manifest <manifest.json> ...
casegraphen topology-review reopen  --target-id <claim> --input <topology.json> --policy-manifest <manifest.json> ...
casegraphen topology-review inspect --target-id <claim> ...
```

Mutation commands require the existing `review` operation gate; their target
contract remains distinct from generic evidence and plan reviews. The
input bytes must reproduce the artifact id already joined to the claim by a
`derives_from` relation, and their parsed topology must reproduce the recorded
canonical topology hash. Artifact existence by itself never means acceptance.
Accept also runs the canonical deterministic topology validator. Its stable
finding code and JSON path are returned on refusal. Heuristic graph-lint
findings remain reviewer advice and cannot by themselves prevent acceptance.
Reject and reopen retain the exact content binding but do not require the
proposal to pass semantic validation, so invalid proposals remain auditable.

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
record. At compilation the policy documents are canonicalized into the same
manifest and must reproduce its accepted hash. Missing, extra, or substituted
policy content is refused. Current claim metadata and caller-supplied hashes
are consistency data, not authority.
