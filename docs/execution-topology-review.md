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
Accept runs the canonical graph linter after typed JSON parsing. The linter is
the single decision owner for both intrinsic semantic-contract findings (for
example an unknown node or invalid data binding) and graph-shape findings (for
example a dependency cycle). Review blocks exactly findings whose published
classification is `deterministic` and whose severity is `error`; stable code,
location, and detail fields are returned as structured CLI refusal data.
Deterministic warnings and informational findings are not implicitly promoted
to blockers. `heuristic` findings remain reviewer advice, are retained in the
accepted review record, and cannot by themselves prevent acceptance.

Reject and reopen retain the exact content binding but do not require the
proposal to pass semantic or graph-lint acceptance checks, so invalid proposals
remain auditable. The operational MCP host does not mutate the acceptance
ledger; a future host delegate must preserve the same typed `NativeReviewError`
findings rather than parse or reconstruct lint decisions.

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

After acceptance, an operational client may call
`compile_reviewed_deployment_bundle` with the exact accepted revision,
case-space ID, and claim-cell ID. The host replays the store and derives the
opaque mode itself; callers cannot submit a mode, review record, or accepted
hash. The generated execution plan and all later runtime output remain
unreviewed and still stop at their independent review seams.
