# ADR 0023: Content-bound topology deployment authority

- Status: Accepted
- Date: 2026-08-04

## Context

An execution topology names verification, budget, and expansion policies by
ID. Reviewing only the topology bytes would allow a compiler caller to retain
the reviewed IDs while substituting different policy content. Conversely,
embedding all policy documents in every review morphism would duplicate large
documents in the append-only ledger.

The authority boundary must also distinguish raw artifact provenance from
semantic canonicalization. Formatting changes to JSON must not create a new
policy meaning, while a substituted policy value must always require review.

## Decision

The indivisible authority unit for reviewed compilation is the canonical
`ExecutionTopologyReviewTarget` retained by an accepted canonical review. It
contains exactly:

```text
topology_id
topology_content_hash
policy_manifest_content_hash
case_space_id
observed_base_revision_id
claim_cell_id
artifact_id
expansion_proposal_id (when applicable)
```

The reviewed bytes and their derivation are:

1. `artifact_id` binds the raw topology artifact bytes already attached to the
   claim through validated `derives_from` lineage.
2. `topology_content_hash` binds the repository-owned canonical serialization
   of the parsed execution topology.
3. `policy_manifest_content_hash` binds the canonical deployment-policy
   manifest. The manifest fixes topology ID/hash and every declared
   verification, budget, and expansion policy ID to the SHA-256 of that policy
   document's canonical JSON bytes. Binding arrays are sorted before manifest
   hashing, so representation order is not authority.
4. `case_space_id`, `observed_base_revision_id`, claim and artifact provenance
   bind the review to the exact ledger observation. A stale review target is
   refused.

The dedicated topology-review constructor is the sole owner of these checks.
It verifies artifact/claim lineage, topology identity, exact policy-ID sets,
manifest content hashes, and the canonical graph-lint result before an accept
morphism can be created. Intrinsic semantic-contract findings and graph-shape
findings block only when the linter classifies them as `deterministic` errors;
deterministic warnings are non-blocking and heuristic findings remain retained
review advice. Reject and reopen retain the same identity but may record a
disposition for semantically or structurally invalid content.

`reviewed_compilation_mode` may construct its opaque binding only from an
accepted canonical execution-topology review. At compilation, the compiler
rebuilds the policy manifest from the actual policy documents and requires the
same manifest hash, topology hash, case space, and accepted review revision.
Missing, extra, duplicate, cross-topology, or same-ID substituted policy
content is refused. Expansion and redesign proposals do not inherit authority
after topology or policy content changes; they require another canonical
review.

Compilation still emits an unreviewed execution plan/deployment bundle. This
ADR grants no runtime dispatch, evidence acceptance, or topology mutation
authority.

A persisted bundle gains deployment authority only after semantic provenance
verification. Every bundle retains `compiler.inputs.json`, containing the
canonical compiler target, mapping, policy documents, and the exact reviewed
binding used for lowering. The verifier treats that record as untrusted,
reconstructs the compiler request internally, deterministically recompiles the
reviewed topology, and requires equality of the manifest plus every artifact
byte. Hash-consistent substitutions of a plan, runtime deployment, resource
manifest, policy set, compiler report, analysis, mapping, topology hash, or
retained input are therefore refused. Deserializing retained inputs never
constructs ledger authority outside this verification boundary.

## Consequences

- Old target-ID-only or topology-only reviews cannot authorize reviewed
  compilation and must be repeated through the dedicated CLI.
- Clients provide both `--input <topology.json>` and
  `--policy-manifest <manifest.json>` and record the manifest hash in the claim
  metadata as consistency data. Metadata alone is never authority.
- The experimental canonical JSON profile may change before stable promotion;
  such a change requires schema/example/test migration and a new review.
- Review morphisms stay compact while the actual compiler policy documents
  remain substitution-resistant.
- Bundle verification costs one deterministic recompile. This deliberate
  authority-boundary cost prevents an artifact writer from manufacturing a
  self-consistent bundle that the compiler never emitted.

## Rejected alternatives

- Policy IDs only: cannot distinguish same-ID content substitution.
- Caller-supplied opaque hash: cannot prove which policy set was reviewed.
- Full policy documents embedded in every review: correct but unnecessarily
  duplicates content-addressed documents in append-only history.
- Compiler-only policy validation: checks shape but cannot establish reviewer
  authorization.
